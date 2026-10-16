use super::*;
use crate::platform::OpenOptionsExt as _;
use cap_std::fs::OpenOptionsExt;
use std::{
    io::{Seek, SeekFrom, Write},
    os::windows::io::AsRawHandle,
};

#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    request: String,
    session: String,
    root: PathBuf,
    path: String,
    payload_hash: String,
    before_hash: String,
    after_hash: String,
    before: Option<String>,
    after: Option<String>,
    state: String,
    at: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Save {
    request_id: String,
    path: String,
    expected_revision: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bom: Option<bool>,
    authorize_write: bool,
}
fn folder(root: &FsPath) -> PathBuf {
    root.join("file-edits")
}
fn journal(root: &FsPath, request: &str) -> PathBuf {
    folder(root).join(format!("save_{request}.json"))
}
fn persist(root: &FsPath, saved: &Saved) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .unix_mode(0o700)
        .create(folder(root))?;
    let path = journal(root, &saved.request);
    let pending = path.with_extension("pending");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .unix_mode(0o600)
        .open(&pending)?;
    file.write_all(&serde_json::to_vec(saved)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(pending, path)?;
    crate::platform::sync_directory(&folder(root))
}
fn records(root: &FsPath) -> io::Result<Vec<Saved>> {
    if !folder(root).exists() {
        return Ok(vec![]);
    }
    let mut result = vec![];
    for item in fs::read_dir(folder(root))? {
        let path = item?.path();
        if path.extension().is_some_and(|e| e == "json") {
            result.push(serde_json::from_slice(&fs::read(path)?)?);
        }
    }
    Ok(result)
}
pub(in crate::session::extensions) fn recover(root: &FsPath) -> io::Result<()> {
    for mut saved in records(root)? {
        if saved.state == "prepared" {
            saved.state = "unknown_after_restart".into();
            persist(root, &saved)?;
        }
    }
    Ok(())
}
fn prune(root: &FsPath) -> io::Result<()> {
    let mut saved = records(root)?;
    saved.sort_by_key(|r| std::cmp::Reverse(r.at));
    let mut counts = BTreeMap::new();
    let mut total = 0;
    for mut record in saved {
        if record.state != "saved" || record.before.is_none() {
            continue;
        }
        let count = counts
            .entry((record.root.clone(), record.path.clone()))
            .or_insert(0);
        *count += 1;
        total += 1;
        if *count > 10 || total > 200 {
            record.before = None;
            record.after = None;
            persist(root, &record)?;
        }
    }
    Ok(())
}
fn response(saved: &Saved) -> Value {
    json!({"request_id":saved.request,"path":saved.path,"revision":saved.after_hash,"state":saved.state})
}
pub(super) async fn save(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(input): Json<Save>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if !input.authorize_write
        || input.text.len() as u64 > LIMIT
        || input.text.contains('\0')
        || input.expected_revision.len() != 64
        || !input
            .expected_revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid(
            "review and authorize a UTF-8 save within 256 KiB with its loaded revision",
        ));
    }
    let path = relative(&input.path)?;
    let root = root(&manager, &id).await?;
    let result = tokio::task::spawn_blocking(move || save_file(&manager, id, root, path, input))
        .await
        .map_err(|_| {
            invalid("file save worker stopped; inspect disk and saved receipts before retrying")
        })??;
    Ok(Json(result))
}
fn save_file(
    manager: &Manager,
    id: String,
    root: PathBuf,
    path: PathBuf,
    input: Save,
) -> Result<Value> {
    let _guard = manager.extensions.file_changes.lock().unwrap();
    let payload_hash = revision(
        &serde_json::to_vec(&json!({"session":id,"input":input})).map_err(io::Error::other)?,
    );
    let record_path = journal(&manager.extensions.root, &input.request_id);
    let prior: Option<Saved> = match fs::read(record_path) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes).map_err(io::Error::other)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(saved) = &prior {
        if saved.payload_hash != payload_hash || saved.root != root {
            return Err(invalid(
                "save request_id already has different content or workspace",
            ));
        }
        if saved.state == "saved" {
            return Ok(response(saved));
        }
    }
    let dir = directory(&root)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).share_mode(0);
    let mut file = dir
        .open_with(path, &options)
        .map_err(|_| invalid("file is busy, read-only, unavailable or outside the workspace"))?
        .into_std();
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err(invalid(
            "editing requires an existing regular UTF-8 file within 256 KiB",
        ));
    }
    let mut info = windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION::default();
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
            file.as_raw_handle(),
            &mut info,
        )
    } == 0
        || info.nNumberOfLinks != 1
    {
        return Err(invalid("editing hard-linked files is unavailable"));
    }
    let mut bytes = vec![];
    (&mut file).take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT || bytes.contains(&0) {
        return Err(invalid("editing requires UTF-8 text within 256 KiB"));
    }
    let current = revision(&bytes);
    if let Some(saved) = &prior
        && current == saved.after_hash
    {
        file.sync_all()?;
        let mut saved = saved.clone();
        saved.state = "saved".into();
        persist(&manager.extensions.root, &saved)?;
        return Ok(response(&saved));
    }
    if current != input.expected_revision {
        return Err(invalid(
            "file changed or a previous save is uncertain; reload disk and review the recovery copy before saving",
        ));
    }
    let before = String::from_utf8(bytes.clone())
        .map_err(|_| invalid("editing non-UTF-8 files is unavailable"))?;
    let after = if input
        .bom
        .unwrap_or_else(|| bytes.starts_with(b"\xef\xbb\xbf"))
    {
        format!("\u{feff}{}", input.text)
    } else {
        input.text.clone()
    };
    if after.len() as u64 > LIMIT {
        return Err(invalid("file and UTF-8 BOM exceed 256 KiB"));
    }
    let mut saved = Saved {
        request: input.request_id,
        session: id,
        root,
        path: input.path.replace('\\', "/"),
        payload_hash,
        before_hash: current,
        after_hash: revision(after.as_bytes()),
        before: Some(before),
        after: Some(after.clone()),
        state: "prepared".into(),
        at: now(),
    };
    persist(&manager.extensions.root, &saved)?;
    let write = |file: &mut fs::File, value: &[u8]| -> io::Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(value)?;
        file.set_len(value.len() as u64)?;
        file.sync_all()
    };
    if write(&mut file, after.as_bytes()).is_err() {
        if write(&mut file, &bytes).is_ok() {
            saved.state = "failed_restored".into();
            let _ = persist(&manager.extensions.root, &saved);
        }
        return Err(invalid(
            "save failed; a private recovery copy was retained; inspect disk before retrying",
        ));
    }
    saved.state = "saved".into();
    persist(&manager.extensions.root, &saved)?;
    // Retention failure must not turn a completed project write into an apparent failed save.
    let _ = prune(&manager.extensions.root);
    Ok(response(&saved))
}
pub(super) async fn history(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Query(location): Query<Location>,
) -> Result<Json<Value>> {
    relative(&location.path)?;
    let root = root(&manager, &id).await?;
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let _guard = manager.extensions.file_changes.lock().unwrap();
        let mut rows: Vec<_> = records(&manager.extensions.root)?
            .into_iter()
            .filter(|r| r.root == root && r.path == location.path.replace('\\', "/"))
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.at));
        rows.truncate(20);
        Ok(json!({"saves":rows}))
    })
    .await
    .map_err(|_| invalid("file history worker stopped"))??;
    Ok(Json(result))
}
pub(super) async fn receipt(
    State(manager): State<Arc<Manager>>,
    Path((id, request)): Path<(String, String)>,
) -> Result<Json<Value>> {
    key(&request)?;
    let root = root(&manager, &id).await?;
    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let _guard = manager.extensions.file_changes.lock().unwrap();
        let bytes =
            fs::read(journal(&manager.extensions.root, &request)).map_err(|_| Error::NotFound)?;
        let saved: Saved = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if saved.root != root || saved.session != id {
            return Err(Error::NotFound);
        }
        Ok(json!({"save":saved}))
    })
    .await
    .map_err(|_| invalid("save receipt worker stopped"))??;
    Ok(Json(result))
}
