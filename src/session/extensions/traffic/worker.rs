use super::*;
use std::time::{Duration, Instant};

pub(super) fn spawn(m: &Arc<Manager>, id: &str) -> Result<()> {
    let (tx, rx) = watch::channel(String::new());
    if m.extensions
        .traffic
        .active
        .lock()
        .unwrap()
        .insert(id.into(), tx)
        .is_some()
    {
        return Err(invalid("task already has an owned runner"));
    }
    let m = m.clone();
    let id = id.to_owned();
    tokio::spawn(async move {
        let final_stop = rx.clone();
        let outcome = execute(&m, &id, rx).await;
        if let Err(error) = outcome
            && let Ok(t) = m.extensions.traffic.task(&id)
        {
            let stopped = close_owned(&m, &t).await.is_ok();
            let c = &m.extensions.traffic;
            let _gate = c.gate.lock().await;
            let mut recovered = c.task(&id).is_ok_and(|t| t.state == "recovery");
            if stopped && !recovered {
                recovered = release_all(c, &t).is_err();
            }
            let _=c.update(&id,|t|{t.state=if stopped&&!recovered{"failed"}else{"recovery"}.into();t.quiescent=stopped;t.error=match error { Error::Conflict(reason)=>reason.to_owned(), _=>"Work did not complete verification. Inspect the linked session, shared file and preserved worktree.".into() };t.activity="No automatic integration or replay".into();});
        }
        let c = &m.extensions.traffic;
        let _gate = c.gate.lock().await;
        let reason = final_stop.borrow().clone();
        if !reason.is_empty()
            && let Ok(t) = c.task(&id)
            && t.state != reason
        {
            let _ = stop_task(&m, &t, reason).await;
        }
        c.active.lock().unwrap().remove(&id);
    });
    Ok(())
}
pub(super) async fn close_owned(m: &Arc<Manager>, t: &Task) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let session = match m.session(&t.session_id).await {
            Ok(s) => s,
            Err(Error::NotFound) => return Ok(()),
            Err(e) => return Err(e),
        };
        if matches!(session.state.as_str(), "closed" | "failed" | "process_lost")
            && session.handle.is_none()
        {
            if let Some(pid) = session.harness_pid {
                let created = process_identity(pid)?;
                if created.is_some()
                    && !(t.owned_pid == Some(pid)
                        && t.owned_created.is_some()
                        && t.owned_created != created)
                {
                    return Err(invalid(
                        "owned process termination is unconfirmed; recovery ownership retained",
                    ));
                }
            }
            return Ok(());
        }
        // A failed/lost runtime has already closed its Job; close records its final session state.
        let _ = m.command(
            &t.session_id,
            format!("htc_close_{}_{}", short(&t.id), t.attempt),
            "close",
            json!({}),
        );
        if Instant::now() >= deadline {
            return Err(invalid(
                "owned execution termination is unconfirmed; ownership retained",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
fn process_identity(pid: u32) -> Result<Option<u64>> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{FILETIME, GetLastError},
        System::Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        if unsafe { GetLastError() } == 87 {
            return Ok(None);
        }
        return Err(invalid("cannot confirm owned process identity"));
    }
    let _handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut code = 0;
    if unsafe { GetExitCodeProcess(raw, &mut code) } == 0 {
        return Err(invalid("cannot inspect owned process exit"));
    }
    if code != 259 {
        return Ok(None);
    }
    let (mut created, mut exit, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    if unsafe { GetProcessTimes(raw, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(invalid("cannot inspect owned process creation"));
    }
    Ok(Some(
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
    ))
}
pub(super) fn release_all(c: &HiveTrafficCoordinator, t: &Task) -> Result<()> {
    c.transact(|d| {
        for l in d
            .engine
            .leases
            .values()
            .filter(|l| l.owner.task_id == t.id && leases::held(&l.state))
            .cloned()
            .collect::<Vec<_>>()
        {
            d.engine.recover_release(&l.id, true, now())?;
        }
        for r in d
            .engine
            .reservations
            .iter()
            .filter(|r| r.owner.task_id == t.id && r.state == "waiting")
            .map(|r| r.id.clone())
            .collect::<Vec<_>>()
        {
            d.engine.cancel_wait(&r, now())?;
        }
        Ok(())
    })
}
async fn execute(m: &Arc<Manager>, id: &str, mut stop: watch::Receiver<String>) -> Result<()> {
    let c = &m.extensions.traffic;
    let t = c.task(id)?;
    let private = format!("private_{}_{}", short(id), t.attempt);
    {
        let _gate = c.gate.lock().await;
        if !stop.borrow().is_empty() {
            let reason = stop.borrow().clone();
            return stop_task(m, &t, reason).await;
        }
        c.transact(|d| {
            d.engine.reserve(
                &private,
                owner(&t),
                vec![t.private_resource.clone()],
                30000,
                now(),
            )?;
            let l = d
                .engine
                .reservation_lease(&private)
                .ok_or(invalid("private worktree is still owned"))?;
            d.engine
                .phase(&l.id, &l.token, l.generation, "active", false, now())?;
            let t = d.tasks.get_mut(id).ok_or(Error::NotFound)?;
            t.state = "preparing".into();
            t.activity = "Core prepares the private Git worktree under an exclusive lease".into();
            t.revision += 1;
            Ok(())
        })?;
    }
    if t.attempt == 1 {
        git::create(&t).await?;
    } else {
        files::validate_root(&t.worktree.path)?;
    }
    {
        let _gate = c.gate.lock().await;
        if !stop.borrow().is_empty() {
            let reason = stop.borrow().clone();
            return stop_task(m, &t, reason).await;
        }
        c.transact(|d| {
            let l = d
                .engine
                .reservation_lease(&private)
                .ok_or(Error::NotFound)?;
            d.engine.validate(&l.id, &l.token, l.generation, now())?;
            let task = d.tasks.get_mut(id).ok_or(Error::NotFound)?;
            task.state = "executing".into();
            task.activity =
                "Native Codex modifies its isolated worktree; shared integration is separate"
                    .into();
            task.revision += 1;
            Ok(())
        })?;
        let request = t
            .session_id
            .strip_prefix("cr_")
            .ok_or(Error::NotFound)?
            .to_owned();
        m.start(request,Some(crate::runtime::Kind::Codex),Some(t.start.model.clone()),None,(Some(t.worktree.id.clone()),None),(None,Some(false),Some(false),Some("You are a Cloud Bee isolated coding worker. Work only inside the current worktree. Do not change other workspaces, Git metadata, branches or the source project. Only the reviewed target file can be integrated by HTC.".into()),true)).await?;
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if !stop.borrow().is_empty() || m.is_stopping() {
            let reason = stop.borrow().clone();
            return stop_task(m, &t, reason).await;
        }
        let session = m.session(&t.session_id).await?;
        if session.state == "idle" {
            let pid = session
                .harness_pid
                .ok_or(invalid("native Work process identity missing"))?;
            let created =
                process_identity(pid)?.ok_or(invalid("native Work process already exited"))?;
            c.update(id, |t| {
                t.owned_pid = Some(pid);
                t.owned_created = Some(created);
            })?;
            break;
        }
        if matches!(session.state.as_str(), "failed" | "closed" | "process_lost")
            || Instant::now() >= deadline
        {
            return Err(invalid("native Work could not start"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let request = format!("htc_prompt_{}_{}", short(id), t.attempt);
    let prompt = format!(
        "[Cloud Bee isolated Work]\nTarget file: {}\n\n{}\n\nMake the requested change in this worktree. Preserve other projects. Report verification performed and any limitations. Shared integration is handled separately by HTC after human review.",
        t.start.path, t.objective
    );
    m.command(
        &t.session_id,
        request.clone(),
        "prompt",
        json!({"text":prompt}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(1800);
    loop {
        if !stop.borrow().is_empty() || m.is_stopping() {
            let reason = stop.borrow().clone();
            return stop_task(m, &t, reason).await;
        }
        if let Some(r) = m.receipt(&t.session_id, &request) {
            if r.state == "completed" {
                break;
            }
            if matches!(
                r.state.as_str(),
                "failed" | "interrupted" | "unknown" | "unknown_after_restart"
            ) {
                return Err(invalid("native Work did not report completion"));
            }
        }
        if Instant::now() >= deadline {
            return Err(invalid("isolated Work exceeded its 30-minute limit"));
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(200))=>{},_=stop.changed()=>{}}
    }
    close_owned(m, &t).await?;
    // The entire native Job is closed before looking at a candidate for shared integration.
    let proposed = files::council_action(
        m.clone(),
        t.session_id.clone(),
        "inspect",
        &t.start.path,
        json!({}),
    )
    .await?;
    if proposed["revision"] == t.base["revision"] {
        return Err(invalid(
            "target file did not change; no integration proposal",
        ));
    }
    if git::head(&t.worktree.path).await? != t.base_commit {
        return Err(invalid(
            "worker changed Git HEAD; inspect its preserved worktree",
        ));
    }
    let _gate = c.gate.lock().await;
    if !stop.borrow().is_empty() {
        let reason = stop.borrow().clone();
        return stop_task(m, &t, reason).await;
    }
    let request = format!("shared_{}_{}", short(id), t.attempt);
    c.transact(|d| {
        let l = d
            .engine
            .reservation_lease(&private)
            .ok_or(Error::NotFound)?;
        d.engine
            .release(&l.id, &l.token, l.generation, true, now())?;
        d.engine
            .reserve(&request, owner(&t), vec![t.resource.clone()], 30000, now())?;
        let acquired = d.engine.reservation_lease(&request);
        if let Some(l) = &acquired {
            d.engine
                .phase(&l.id, &l.token, l.generation, "verification", true, now())?;
        }
        let task = d.tasks.get_mut(id).ok_or(Error::NotFound)?;
        task.quiescent = true;
        task.proposed = Some(proposed);
        task.reservation = Some(request);
        task.state = if acquired.is_some() {
            "verification"
        } else {
            "waiting"
        }
        .into();
        task.activity = if acquired.is_some() {
            "Inspect the exact proposed file; compilation/tests require separate review"
        } else {
            "Waiting for conflicting shared integration; private work is preserved"
        }
        .into();
        task.revision += 1;
        task.updated_at = now();
        Ok(())
    })?;
    Ok(())
}
async fn stop_task(m: &Arc<Manager>, t: &Task, state: String) -> Result<()> {
    close_owned(m, t).await?;
    let c = &m.extensions.traffic;
    if state != "recovery" {
        release_all(c, t)?;
    }
    c.update(&t.id, |t| {
        t.state = if state.is_empty() {
            "recovery".into()
        } else {
            state
        };
        t.quiescent = true;
        t.activity = "Owned execution terminated; unfinished isolated work preserved".into();
    })
}
pub(super) async fn integrate(m: &Arc<Manager>, t: &Task) -> Result<()> {
    let c = &m.extensions.traffic;
    close_owned(m, t).await?;
    if git::head(&t.root).await? != t.base_commit {
        return Err(invalid(
            "shared Git HEAD changed; proposal must be reviewed against the new baseline",
        ));
    }
    let proposed = t.proposed.as_ref().ok_or(Error::NotFound)?;
    let current = files::council_action(
        m.clone(),
        t.start.council_id.clone(),
        "inspect",
        &t.start.path,
        json!({}),
    )
    .await?;
    if current["revision"] != t.base["revision"] {
        return Err(invalid(
            "shared file changed; this proposal cannot overwrite another integration",
        ));
    }
    let permit = c.transact(|d| {
        let l = t
            .reservation
            .as_deref()
            .and_then(|r| d.engine.reservation_lease(r))
            .ok_or(Error::NotFound)?;
        d.engine
            .phase(&l.id, &l.token, l.generation, "integrating", true, now())?;
        let task = d.tasks.get_mut(&t.id).ok_or(Error::NotFound)?;
        task.state = "integrating".into();
        task.activity =
            "Applying the reviewed proposal with an exclusive revision-checked file save".into();
        task.revision += 1;
        Ok(Permit {
            task: t.id.clone(),
            lease: l.id,
            token: l.token,
            generation: l.generation,
        })
    })?;
    let input = json!({"request_id":format!("htc_save_{}_{}",short(&t.id),t.attempt),"expected_revision":t.base["revision"],"text":proposed["text"],"bom":proposed["bom"],"authorize_write":true,"path":t.start.path});
    let result =
        files::traffic_save(m.clone(), t.start.council_id.clone(), input, permit.clone()).await;
    match result {
        Ok(_) => c.transact(|d| {
            d.engine
                .release(&permit.lease, &permit.token, permit.generation, true, now())?;
            let t = d.tasks.get_mut(&t.id).ok_or(Error::NotFound)?;
            t.state = "integrated".into();
            t.activity =
                "Reviewed file integrated; original and proposal retained in recovery history"
                    .into();
            t.revision += 1;
            t.updated_at = now();
            Ok(())
        }),
        Err(e) => {
            let _ = c.update(&t.id, |t| {
                t.state = "recovery".into();
                t.activity =
                    "Integration uncertain or rejected; inspect file save history and current disk"
                        .into();
            });
            Err(e)
        }
    }
}
