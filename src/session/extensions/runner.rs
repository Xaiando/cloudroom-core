use super::*;
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

pub(super) fn interpreter(name: &str) -> Result<PathBuf> {
    let program = match name {
        "powershell" => {
            if !cfg!(windows) {
                return Err(invalid("native PowerShell automation requires Windows"));
            }
            return Ok(PathBuf::from(
                std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()),
            )
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        }
        "node" => {
            if cfg!(windows) {
                "node.exe"
            } else {
                "node"
            }
        }
        "python" => {
            if cfg!(windows) {
                "python.exe"
            } else {
                "python3"
            }
        }
        _ => return Err(invalid("choose powershell, node or python")),
    };
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(invalid(
        "selected script interpreter is not installed or not on Core's PATH",
    ))
}
async fn capture(mut stream: impl AsyncRead + Unpin) -> io::Result<String> {
    let mut output = Vec::new();
    let mut chunk = [0; 8192];
    let mut truncated = false;
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        let keep = (64 * 1024usize).saturating_sub(output.len()).min(n);
        output.extend_from_slice(&chunk[..keep]);
        truncated |= keep < n;
    }
    let mut text = String::from_utf8_lossy(&output).into_owned();
    if truncated {
        text.push_str("\n[Output truncated at 64 KiB]");
    }
    Ok(text)
}
pub(super) async fn run(manager: Arc<Manager>, run: Run, mut cancel: watch::Receiver<bool>) {
    let result = match &run.execution {
        Execution::Script {
            interpreter: name,
            source,
            timeout_ms,
        } => script(&manager, &run, name, source, *timeout_ms, &mut cancel).await,
        Execution::Agent {
            conversation_id,
            prompt,
        } => agent(&manager, &run, conversation_id, prompt, &mut cancel).await,
    };
    let (state,output,error,code)=match result { Ok(outcome)=>outcome,Err(_)=>("failed".into(),String::new(),"Execution could not complete. Check the saved interpreter, workspace or linked Work session.".into(),None) };
    let output = manager.bee.redact(&output, &manager.config.token);
    let error = manager.bee.redact(&error, &manager.config.token);
    let _ = manager.extensions.transact(|data| {
        let saved = data.runs.get_mut(&run.id).ok_or(Error::NotFound)?;
        saved.state = state;
        saved.output = output;
        saved.error = error;
        saved.exit_code = code;
        saved.finished_at = Some(now());
        let mut old: Vec<_> = data
            .runs
            .values()
            .filter(|r| r.automation_id == run.automation_id && r.state != "running")
            .map(|r| (r.started_at, r.id.clone()))
            .collect();
        old.sort();
        let remove = old.len().saturating_sub(100);
        for (_, id) in old.into_iter().take(remove) {
            data.runs.remove(&id);
        }
        Ok(())
    });
    manager.extensions.active.lock().unwrap().remove(&run.id);
}
type Outcome = (String, String, String, Option<i32>);
async fn script(
    manager: &Manager,
    run: &Run,
    name: &str,
    source: &str,
    timeout_ms: u64,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Outcome> {
    let directory = manager.extensions.root.join("scripts").join(&run.id);
    fs::DirBuilder::new()
        .recursive(true)
        .unix_mode(0o700)
        .create(&directory)?;
    let filename = directory.join(match name {
        "powershell" => "script.ps1",
        "node" => "script.cjs",
        _ => "script.py",
    });
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .unix_mode(0o600)
        .open(&filename)?;
    if name == "powershell" {
        // Windows PowerShell 5.1 needs a BOM to read UTF-8 source; its pipe encoding is independent.
        file.write_all(b"\xef\xbb\xbf[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)\r\n$OutputEncoding = [Console]::OutputEncoding\r\n")?;
    }
    file.write_all(source.as_bytes())?;
    file.sync_all()?;
    drop(file);
    let mut command = Command::new(interpreter(name)?);
    if name == "powershell" {
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
    }
    command
        .arg(&filename)
        .current_dir(run.workspace.path.canonicalize()?)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in [
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
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("CLOUD_BEE_AUTOMATION_ID", &run.automation_id)
        .env("CLOUD_BEE_RUN_ID", &run.id)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1");
    #[cfg(windows)]
    let (mut child, job) = crate::runtime::linux::spawn(&mut command)?;
    #[cfg(not(windows))]
    let mut child = command.spawn()?;
    let stdout = tokio::spawn(capture(
        child
            .stdout
            .take()
            .ok_or_else(|| invalid("missing script stdout"))?,
    ));
    let stderr = tokio::spawn(capture(
        child
            .stderr
            .take()
            .ok_or_else(|| invalid("missing script stderr"))?,
    ));
    let (state, code) = if *cancel.borrow() {
        ("cancelled", None)
    } else {
        tokio::select! {
            status=child.wait()=>{let status=status?;(if status.success(){"completed"}else{"failed"},status.code())},
            _=tokio::time::sleep(Duration::from_millis(timeout_ms))=>("timed_out",None),
            _=cancel.changed()=>("cancelled",None),
        }
    };
    #[cfg(windows)]
    job.stop().await?;
    if child.try_wait()?.is_none() {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    let output = finish_capture(stdout).await;
    let error = finish_capture(stderr).await;
    Ok((state.into(), output, error, code))
}
async fn finish_capture(mut task: tokio::task::JoinHandle<io::Result<String>>) -> String {
    match tokio::time::timeout(Duration::from_secs(2), &mut task).await {
        Ok(Ok(Ok(output))) => output,
        _ => {
            task.abort();
            "[Output capture incomplete]".into()
        }
    }
}
async fn agent(
    manager: &Arc<Manager>,
    run: &Run,
    conversation: &str,
    prompt: &str,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Outcome> {
    let (native, workspace) = manager.bee.automation_target(conversation, false)?;
    if run.native_session.as_deref() != Some(&native) || workspace.path != run.workspace.path {
        return Err(invalid("automation Work authorization changed"));
    }
    let request = format!("aut_{}", stable_id(&run.id));
    let instructions = if run.instructions.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nCustom instructions:\n{}", run.instructions)
    };
    manager.command(&native,request.clone(),"prompt",json!({"text":format!("[Cloud Bee automation {}]\n\n{}{}",run.automation_id,prompt,instructions)}))?;
    loop {
        if *cancel.borrow() || manager.is_stopping() {
            if let Some(receipt) = manager.receipt(&native, &request) {
                let session = manager.session(&native).await?;
                if session.current_request.as_deref() == Some(&request) {
                    let _ = manager.command(
                        &native,
                        format!("cancel_{}", stable_id(&run.id)),
                        "interrupt",
                        json!({"target_request_id":request}),
                    );
                } else if receipt.state == "accepted" {
                    let _ = manager.command(
                        &native,
                        format!("cancel_{}", stable_id(&run.id)),
                        "cancel",
                        json!({"target_request_id":request}),
                    );
                }
            }
            return Ok(("cancelled".into(), String::new(), String::new(), None));
        }
        if let Some(receipt) = manager.receipt(&native, &request)
            && matches!(
                receipt.state.as_str(),
                "completed" | "failed" | "interrupted" | "unknown" | "unknown_after_restart"
            )
        {
            let output = format!(
                "Work request {request}: {}. Open the linked Work transcript for its full output. Conversation: {conversation}; native session: {native}.",
                receipt.state
            );
            return Ok((
                receipt.state,
                output,
                receipt.error.unwrap_or_default(),
                None,
            ));
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(200))=>{},_=cancel.changed()=>{}}
    }
}
fn stable_id(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}
