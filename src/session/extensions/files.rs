//! Workspace-relative file previews and explicitly reviewed text edits.
mod edits;
use super::*;
use axum::extract::Query;
use std::{io::Read, os::windows::fs::MetadataExt, path::Path as FsPath};

const LIMIT: u64 = 256 * 1024;
pub(super) use edits::recover;
fn revision(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Location {
    #[serde(default)]
    path: String,
}
pub(super) fn routes() -> Router<Arc<Manager>> {
    Router::new()
        .route("/v1/bee/files/{id}/list", get(list))
        .route("/v1/bee/files/{id}/preview", get(preview))
        .route(
            "/v1/bee/files/{id}/save",
            post(edits::save).layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route("/v1/bee/files/{id}/history", get(edits::history))
        .route("/v1/bee/files/{id}/saves/{request}", get(edits::receipt))
}
fn relative(value: &str) -> Result<PathBuf> {
    if value.len() > 4096 {
        return Err(invalid("file path is too long"));
    }
    if value.is_empty() {
        return Ok(PathBuf::from("."));
    }
    let normalized = value.replace('\\', "/");
    for part in normalized.split('/') {
        let name = part.split('.').next().unwrap_or("").to_uppercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || ":*?\"<>|".contains(c))
            || ["CON", "CONIN$", "CONOUT$", "PRN", "AUX", "NUL", "CLOCK$"].contains(&name.as_str())
            || ["COM", "LPT"].iter().any(|prefix| {
                name.strip_prefix(prefix).is_some_and(|s| {
                    s.len() == 1 && "123456789".contains(s) || matches!(s, "¹" | "²" | "³")
                })
            })
        {
            return Err(invalid(
                "use a normal workspace-relative path without traversal, devices or alternate streams",
            ));
        }
    }
    Ok(PathBuf::from(normalized))
}
async fn root(manager: &Manager, id: &str) -> Result<PathBuf> {
    if manager.bee.contains(id) {
        return Ok(manager.bee.workspace(id)?.path);
    }
    Ok(manager
        .session(id)
        .await?
        .workspace
        .map(|w| w.path)
        .unwrap_or_else(|| manager.config.repository.clone()))
}
fn directory(root: &FsPath) -> Result<cap_std::fs::Dir> {
    let metadata =
        fs::symlink_metadata(root).map_err(|_| invalid("workspace directory is unavailable"))?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(invalid("workspace root must be an ordinary directory"));
    }
    cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|_| invalid("cannot open workspace directory"))
}
async fn list(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Query(location): Query<Location>,
) -> Result<Json<Value>> {
    let path = relative(&location.path)?;
    let root = root(&manager, &id).await?;
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let dir = directory(&root)?.open_dir(path).map_err(|_| invalid("directory is unavailable or outside the workspace"))?;
        let mut entries = vec![];
        for entry in dir.entries().map_err(|_| invalid("cannot list directory"))?.take(501) {
            let entry = entry.map_err(|_| invalid("cannot read directory entry"))?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            let kind = entry.file_type().map_err(|_| invalid("cannot read entry type"))?;
            entries.push(json!({"name":name,"directory":kind.is_dir(),"link":kind.is_symlink()}));
        }
        let truncated = entries.len() > 500;
        entries.truncate(500);
        entries.sort_by_key(|e| (!e["directory"].as_bool().unwrap_or(false),e["name"].as_str().unwrap_or("").to_lowercase()));
        Ok(json!({"path":location.path.replace('\\',"/"),"entries":entries,"truncated":truncated,"read_only":true}))
    }).await.map_err(|_| invalid("file listing worker stopped"))??;
    Ok(Json(result))
}
async fn preview(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Query(location): Query<Location>,
) -> Result<Json<Value>> {
    let path = relative(&location.path)?;
    let root = root(&manager, &id).await?;
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let file = directory(&root)?.open(&path).map_err(|_| invalid("file is unavailable or outside the workspace"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > LIMIT { return Err(invalid("preview requires a regular file no larger than 256 KiB")); }
        let mut bytes = vec![];
        file.take(LIMIT+1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT || bytes.contains(&0) { return Err(invalid("preview requires UTF-8 text no larger than 256 KiB")); }
        let size = bytes.len();
        let revision = revision(&bytes);
        let bom = bytes.starts_with(b"\xef\xbb\xbf");
        let text = String::from_utf8(bytes).map_err(|_| invalid("binary or non-UTF-8 preview is not supported"))?;
        let extension = path.extension().and_then(|p|p.to_str()).unwrap_or("").to_lowercase();
        let language = match extension.as_str() { "rs"=>"rust","cs"=>"csharp","py"=>"python","js"|"cjs"|"mjs"=>"javascript","ts"|"tsx"=>"typescript","ps1"=>"powershell","json"=>"json",_=>"text" };
        let newline = if text.contains("\r\n") && !text.replace("\r\n", "").contains(['\r', '\n']) { "crlf" } else { "lf" };
        Ok(json!({"path":location.path.replace('\\',"/"),"text":text.strip_prefix('\u{feff}').unwrap_or(&text),"revision":revision,"bom":bom,"newline":newline,"size":size,"markdown":matches!(extension.as_str(),"md"|"markdown"),"language":language,"read_only":false}))
    }).await.map_err(|_| invalid("file preview worker stopped"))??;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_file_paths_reject_ambiguous_and_device_spellings() {
        for path in [
            "../key",
            "a/../key",
            "C:\\key",
            "//host/share/key",
            "a:stream",
            "CON.txt",
            "aux",
            "com1",
            "LPT².txt",
            "a./b",
            "a /b",
            "a//b",
            "a\n",
        ] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert_eq!(
            relative("notes\\Unicode ø.md").unwrap(),
            PathBuf::from("notes/Unicode ø.md")
        );
    }
}
