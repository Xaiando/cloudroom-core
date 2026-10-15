//! Native workload ownership. Start suspended, assign to a non-inheritable kill-on-close Job,
//! then resume. Descendants cannot run before membership and cannot opt out of the Job.
use crate::workspace::storage::Policy;
use std::{
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    sync::watch,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::*,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

pub struct Workload {
    job: OwnedHandle,
    paused: watch::Sender<bool>,
}
impl Workload {
    fn new() -> io::Result<Self> {
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limit = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limit.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&limit as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            job,
            paused: watch::channel(false).0,
        })
    }
    pub fn create(_: &Policy) -> io::Result<Self> {
        Self::new()
    }
    pub fn attach(&self, _: &mut Command, _: &Policy) -> io::Result<()> {
        Err(io::Error::other(
            "Linux identity switching is unavailable on Windows",
        ))
    }
    pub fn paused(&self) -> watch::Receiver<bool> {
        self.paused.subscribe()
    }
    pub async fn freeze(&self, _: bool) -> io::Result<()> {
        Err(io::Error::other("Windows workload freezing is unavailable"))
    }
    pub fn process_count(&self) -> Option<usize> {
        let mut value = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let ok = unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut value as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(value.ActiveProcesses as usize)
    }
    pub fn kill_top_writer(&self, _: u32) -> io::Result<Option<String>> {
        Err(io::Error::other(
            "Linux disk rescue is unavailable on Windows",
        ))
    }
    pub fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub async fn stop(&self) -> io::Result<()> {
        self.terminate()?;
        tokio::time::timeout(super::SHUTDOWN_GRACE, async {
            loop {
                match self.process_count() {
                    Some(0) => return Ok(()),
                    None => return Err(io::Error::other("Job accounting failed")),
                    _ => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .map_err(|_| io::Error::other("Job exit unconfirmed"))?
    }
    pub async fn clear(_: &Policy) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Arc<Workload>)> {
    let group = Arc::new(Workload::new()?);
    command
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let result = (|| {
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("child has no process handle"))?;
        if unsafe { AssignProcessToJobObject(group.job.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        resume(
            child
                .id()
                .ok_or_else(|| io::Error::other("child has no PID"))?,
        )
    })();
    if let Err(error) = result {
        let _ = child.start_kill();
        return Err(error);
    }
    Ok((child, group))
}

fn resume(pid: u32) -> io::Result<()> {
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut thread = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut thread) };
    while more != 0 {
        if thread.th32OwnerProcessID == pid {
            let raw: HANDLE = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread.th32ThreadID) };
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            if unsafe { ResumeThread(handle.as_raw_handle()) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut thread) };
    }
    Err(io::Error::other("suspended child thread not found"))
}

pub(crate) fn as_agent(
    _: &mut Command,
    _: u32,
    _: u32,
    _: Option<&Path>,
    _: bool,
) -> io::Result<()> {
    Err(io::Error::other(
        "Linux identity switching is unavailable on Windows",
    ))
}

/// Pausing work must not turn an in-flight native RPC into a timeout/replayed task.
pub(super) async fn reply<T>(
    mut result: tokio::sync::oneshot::Receiver<T>,
    mut paused: watch::Receiver<bool>,
    timeout: Duration,
) -> io::Result<T> {
    let mut remaining = timeout;
    loop {
        if *paused.borrow() {
            tokio::select! {
                value = &mut result => return value.map_err(|_| io::Error::other("harness response lost; outcome uncertain")),
                changed = paused.changed() => if changed.is_err() { return Err(io::Error::other("workload owner lost")); },
            }
        } else {
            let start = tokio::time::Instant::now();
            tokio::select! {
                value = &mut result => return value.map_err(|_| io::Error::other("harness response lost; outcome uncertain")),
                _ = tokio::time::sleep(remaining) => return Err(io::Error::new(io::ErrorKind::TimedOut, "harness response timed out; outcome uncertain")),
                changed = paused.changed() => if changed.is_err() { return Err(io::Error::other("workload owner lost")); },
            }
            remaining = remaining.saturating_sub(start.elapsed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn a_job_owns_detached_descendants_and_drop_kills_them() {
        let mut command = Command::new("node");
        command.args(["-e", r#"const c = require('node:child_process').spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], {stdio:'ignore', detached:true, windowsHide:true}); console.log(c.pid); setInterval(()=>{},1000);"#])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let (mut child, group) = spawn(&mut command).unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        let pid: u32 = line.trim().parse().unwrap();
        assert!(group.process_count().unwrap() >= 2);
        drop(group);
        // Windows kill-on-close may report exit code zero; bounded exit is the guarantee.
        tokio::time::timeout(Duration::from_secs(4), child.wait())
            .await
            .unwrap()
            .unwrap();
        super::super::wait_for_exit(pid).await.unwrap();
    }

    #[tokio::test]
    async fn stopping_a_job_confirms_quiescence() {
        let mut command = Command::new("node");
        command
            .args(["-e", "setInterval(()=>{},1000)"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let (mut child, group) = spawn(&mut command).unwrap();
        assert_eq!(group.process_count(), Some(1));
        group.stop().await.unwrap();
        assert_eq!(group.process_count(), Some(0));
        assert!(!child.wait().await.unwrap().success());
    }
}
