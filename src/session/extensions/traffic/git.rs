use super::*;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

fn binary() -> Result<PathBuf> {
    let mut candidates = vec![];
    if let Some(p) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(p).join("Git/cmd/git.exe"));
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if dir.is_absolute() {
            candidates.push(dir.join("git.exe"));
        }
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or(invalid("Git for Windows is required for isolated Work"))
}
async fn capture(mut stream: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut out = vec![];
    let mut buf = [0; 8192];
    let mut oversized = false;
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        let keep = (256 * 1024usize).saturating_sub(out.len()).min(n);
        out.extend_from_slice(&buf[..keep]);
        oversized |= keep < n;
    }
    if oversized {
        return Err(io::Error::other("Git output exceeded its bound"));
    }
    Ok(out)
}
pub(super) async fn run(root: &FsPath, args: &[&str]) -> Result<String> {
    let mut command = Command::new(binary()?);
    command
        .args([
            "-c",
            "core.hooksPath=NUL",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
            "-c",
            "core.longpaths=true",
            "--no-pager",
        ])
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for k in [
        "SystemRoot",
        "WINDIR",
        "PATH",
        "PATHEXT",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "HOME",
        "LOCALAPPDATA",
        "APPDATA",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        if let Some(v) = std::env::var_os(k) {
            command.env(k, v);
        }
    }
    let (mut child, job) = crate::runtime::linux::spawn(&mut command)?;
    let mut stdout = tokio::spawn(capture(
        child.stdout.take().ok_or(invalid("Git stdout missing"))?,
    ));
    let mut stderr = tokio::spawn(capture(
        child.stderr.take().ok_or(invalid("Git stderr missing"))?,
    ));
    let status = tokio::time::timeout(Duration::from_secs(60), child.wait()).await;
    let stopped = job.stop().await;
    if child.try_wait()?.is_none() {
        let _ = child.start_kill();
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    let output = tokio::time::timeout(Duration::from_secs(2), &mut stdout).await;
    let error = tokio::time::timeout(Duration::from_secs(2), &mut stderr).await;
    if output.is_err() {
        stdout.abort();
    }
    if error.is_err() {
        stderr.abort();
    }
    stopped?;
    if !matches!(status,Ok(Ok(ref s)) if s.success()) {
        return Err(invalid(
            "Git operation failed or timed out; inspect the preserved repository/worktree",
        ));
    }
    let bytes = output
        .map_err(|_| invalid("Git output incomplete"))?
        .map_err(|_| invalid("Git output worker stopped"))??;
    String::from_utf8(bytes).map_err(|_| invalid("Git returned a non-UTF-8 path or result"))
}
pub(super) async fn head(root: &FsPath) -> Result<String> {
    let value = run(root, &["rev-parse", "--verify", "HEAD"])
        .await?
        .trim()
        .to_owned();
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("Git HEAD is unavailable"));
    }
    Ok(value)
}
pub(super) async fn check_project(root: &FsPath, path: &str) -> Result<String> {
    files::validate_root(root)?;
    let top = run(root, &["rev-parse", "--show-toplevel"]).await?;
    if canonical_key(&PathBuf::from(top.trim()).canonicalize()?)
        != canonical_key(&root.canonicalize()?)
    {
        return Err(invalid("select the Git project root as the Council folder"));
    }
    if !run(
        root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )
    .await?
    .trim()
    .is_empty()
    {
        return Err(invalid(
            "start isolated Work from a clean Git project; preserve or commit existing changes first",
        ));
    }
    let tracked = run(
        root,
        &["--literal-pathspecs", "ls-files", "--stage", "--", path],
    )
    .await?;
    let mut lines = tracked.lines();
    let line = lines
        .next()
        .ok_or(invalid("choose an existing tracked regular text file"))?;
    if lines.next().is_some() || !(line.starts_with("100644 ") || line.starts_with("100755 ")) {
        return Err(invalid(
            "integration supports one existing tracked regular file",
        ));
    }
    head(root).await
}
pub(super) async fn create(t: &Task) -> Result<()> {
    if t.worktree.path.exists() {
        return Err(invalid(
            "isolated worktree already exists; inspect it before retrying",
        ));
    }
    let destination = t
        .worktree
        .path
        .to_str()
        .ok_or(invalid("worktree path is not Unicode"))?;
    // Git's path parser rejects the Win32 extended prefix even though cwd accepts it.
    let destination = destination.strip_prefix("\\\\?\\").unwrap_or(destination);
    run(
        &t.root,
        &["worktree", "add", "--detach", destination, &t.base_commit],
    )
    .await?;
    files::validate_root(&t.worktree.path)?;
    Ok(())
}
