//! Hive Traffic Control: durable arbitration and reviewed single-file integration.
//! Isolated Git worktrees are cooperative boundaries, not Windows account isolation.
use super::*;
use sha2::{Digest, Sha256};
use std::path::Path as FsPath;
mod git;
mod leases;
mod worker;
use leases::{Engine, Owner, Resource};

fn short(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..24].into()
}
fn random_token() -> Result<String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut bytes = [0u8; 32];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } != 0
    {
        return Err(io::Error::other("lease entropy unavailable").into());
    }
    Ok(bytes.iter().map(|v| format!("{v:02x}")).collect())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    request_id: String,
    council_id: String,
    round_id: String,
    microtask: usize,
    harness: String,
    model: String,
    path: String,
    authorize_work: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    start: Start,
    root: PathBuf,
    worktree: crate::workspace::Workspace,
    resource: String,
    private_resource: String,
    objective: String,
    state: String,
    activity: String,
    revision: u64,
    attempt: u32,
    session_id: String,
    #[serde(default)]
    owned_pid: Option<u32>,
    #[serde(default)]
    owned_created: Option<u64>,
    quiescent: bool,
    base_commit: String,
    base: Value,
    proposed: Option<Value>,
    reservation: Option<String>,
    created_at: u64,
    updated_at: u64,
    error: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Data {
    engine: Engine,
    tasks: BTreeMap<String, Task>,
    ledger: BTreeMap<String, (Value, Value)>,
}
pub(super) struct HiveTrafficCoordinator {
    path: PathBuf,
    data: Mutex<Data>,
    gate: tokio::sync::Mutex<()>,
    active: Mutex<BTreeMap<String, watch::Sender<String>>>,
    file_changes: Arc<Mutex<()>>,
}
fn persist(path: &FsPath, data: &Data) -> io::Result<()> {
    use std::io::Write;
    let pending = path.with_extension("pending");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .unix_mode(0o600)
        .open(&pending)?;
    f.write_all(&serde_json::to_vec(data)?)?;
    f.sync_all()?;
    drop(f);
    fs::rename(pending, path)?;
    crate::platform::sync_directory(
        path.parent()
            .ok_or_else(|| io::Error::other("traffic directory missing"))?,
    )
}
impl HiveTrafficCoordinator {
    pub fn open(root: &FsPath, file_changes: Arc<Mutex<()>>) -> io::Result<Self> {
        let path = root.join("traffic.json");
        let mut data: Data = match fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e),
        };
        data.engine
            .restart()
            .map_err(|_| io::Error::other("traffic recovery failed"))?;
        for t in data.tasks.values_mut() {
            if matches!(
                t.state.as_str(),
                "preparing" | "executing" | "stopping" | "waiting" | "verification" | "integrating"
            ) {
                t.state = "recovery".into();
                t.activity="Core restarted; inspect preserved work and confirm owned execution termination".into();
                t.revision += 1;
                t.updated_at = now();
            }
        }
        persist(&path, &data)?;
        Ok(Self {
            path,
            data: Mutex::new(data),
            gate: tokio::sync::Mutex::new(()),
            active: Mutex::new(BTreeMap::new()),
            file_changes,
        })
    }
    fn transact<T>(&self, f: impl FnOnce(&mut Data) -> Result<T>) -> Result<T> {
        let _files = self.file_changes.lock().unwrap();
        let mut d = self.data.lock().unwrap();
        let mut next = d.clone();
        let out = f(&mut next)?;
        persist(&self.path, &next)?;
        *d = next;
        Ok(out)
    }
    fn task(&self, id: &str) -> Result<Task> {
        self.data
            .lock()
            .unwrap()
            .tasks
            .get(id)
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn update(&self, id: &str, f: impl FnOnce(&mut Task)) -> Result<()> {
        self.transact(|d| {
            let t = d.tasks.get_mut(id).ok_or(Error::NotFound)?;
            f(t);
            t.revision += 1;
            t.updated_at = now();
            Ok(())
        })
    }
    // Called under the existing file-save mutex immediately before a Core-owned write.
    pub(super) fn authorize_file_write(
        &self,
        root: &FsPath,
        path: &FsPath,
        permit: Option<&Permit>,
    ) -> Result<()> {
        let target = resource(root, path, "file", false)?;
        let d = self.data.lock().unwrap();
        if let Some(p) = permit {
            let l = d.engine.validate(&p.lease, &p.token, p.generation, now())?;
            let t = d.tasks.get(&p.task).ok_or(Error::NotFound)?;
            if l.owner.task_id != t.id
                || l.state != "integrating"
                || !t.quiescent
                || t.state != "integrating"
                || !l.resources.contains(&t.resource)
                || target.id != t.resource
            {
                return Err(invalid("integration ownership changed"));
            }
        } else if d
            .engine
            .traffic(
                &d.engine
                    .resources
                    .values()
                    .filter(|r| leases::overlap(r, &target))
                    .map(|r| r.id.clone())
                    .collect::<Vec<_>>(),
            )
            .0
            != "green"
        {
            return Err(invalid(
                "Hive Traffic Control owns this resource; inspect its task before editing",
            ));
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(super) struct Permit {
    task: String,
    lease: String,
    token: String,
    generation: u64,
}
fn canonical_key(path: &FsPath) -> String {
    path.to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}
fn resource(root: &FsPath, path: &FsPath, kind: &str, directory: bool) -> Result<Resource> {
    let root = root.canonicalize()?;
    files::validate_root(&root)?;
    let dir = files::directory(&root)?;
    let rel = dir
        .canonicalize(path)
        .map_err(|_| invalid("resource is unavailable or outside its workspace"))?;
    let absolute = root.join(&rel);
    let key = canonical_key(&absolute);
    Ok(Resource {
        id: format!("resource_{}", short(&key)),
        workspace_id: format!("workspace_{}", short(&canonical_key(&root))),
        path: rel.to_string_lossy().replace('\\', "/"),
        kind: kind.into(),
        key,
        directory,
    })
}
fn owner(t: &Task) -> Owner {
    Owner {
        agent_id: format!("codex:{}", t.start.model),
        session_id: t.session_id.clone(),
        task_id: t.id.clone(),
        can_work: true,
    }
}
pub(super) fn routes() -> Router<Arc<Manager>> {
    Router::new()
        .route("/v1/bee/traffic", get(status))
        .route("/v1/bee/traffic/catalog", get(catalog))
        .route("/v1/bee/traffic/tasks", post(create))
        .route("/v1/bee/traffic/tasks/{id}", get(detail))
        .route("/v1/bee/traffic/tasks/{id}/{action}", post(action))
}
fn summary(t: &Task, e: &Engine) -> Value {
    let resources = vec![
        if (matches!(t.state.as_str(), "preparing" | "executing" | "stopping") && !t.quiescent)
            || (t.state == "recovery"
                && e.leases.values().any(|l| {
                    l.owner.task_id == t.id
                        && leases::held(&l.state)
                        && l.resources.contains(&t.private_resource)
                }))
        {
            t.private_resource.clone()
        } else {
            t.resource.clone()
        },
    ];
    let (light, lease) = e.traffic(&resources);
    let waiting = e
        .reservations
        .iter()
        .filter(|r| r.state == "waiting")
        .collect::<Vec<_>>();
    let position = waiting
        .iter()
        .position(|r| r.owner.task_id == t.id)
        .map(|n| n + 1);
    json!({"id":t.id,"council_id":t.start.council_id,"round_id":t.start.round_id,"microtask":t.start.microtask,"model":t.start.model,"harness":"codex","path":t.start.path,"workspace":t.root,"worktree":t.worktree.path,"session_id":t.session_id,"state":t.state,"activity":t.activity,"revision":t.revision,"attempt":t.attempt,"quiescent":t.quiescent,"error":t.error,"created_at":t.created_at,"updated_at":t.updated_at,"traffic":light,"queue_position":position,"lease":lease.map(|l|json!({"id":l.id,"state":l.state,"generation":l.generation,"owner":l.owner,"acquired_at":l.acquired_at,"expires_at":l.expires_at,"heartbeat_at":l.heartbeat_at,"resources":l.resources}))})
}
async fn status(State(m): State<Arc<Manager>>) -> Json<Value> {
    let d = m.extensions.traffic.data.lock().unwrap();
    Json(
        json!({"tasks":d.tasks.values().map(|t|summary(t,&d.engine)).collect::<Vec<_>>(),"resources":d.engine.resources.values().map(|r|{let (light,owner)=d.engine.traffic(std::slice::from_ref(&r.id));json!({"id":r.id,"workspace_id":r.workspace_id,"path":r.path,"kind":r.kind,"traffic":light,"owner":owner.map(|l|&l.owner)})}).collect::<Vec<_>>(),"generation":d.engine.generation,"integration":"reviewed_existing_utf8_file","isolation":"cooperative_git_worktrees_same_windows_account"}),
    )
}
async fn catalog(State(m): State<Arc<Manager>>) -> Json<Value> {
    let models = m.codex_models().await.unwrap_or_default();
    Json(
        json!({"models":models,"harness":"codex","integration":"one tracked UTF-8 file per task","automatic_tests":false}),
    )
}
async fn detail(State(m): State<Arc<Manager>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let d = m.extensions.traffic.data.lock().unwrap();
    let t = d.tasks.get(&id).ok_or(Error::NotFound)?;
    Ok(Json(
        json!({"task":summary(t,&d.engine),"base":t.base,"proposed":t.proposed,"base_commit":t.base_commit,"objective":t.objective,"checks":["owned native execution stopped before review","Git HEAD and baseline SHA-256 must match at integration","regular UTF-8 file within 256 KiB; exclusive file handle","human review required; compilation and project tests are not automatic"]}),
    ))
}
async fn create(State(m): State<Arc<Manager>>, Json(input): Json<Start>) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if !input.authorize_work || input.harness != "codex" {
        return Err(invalid(
            "only explicitly authorized native Codex Work can execute; Council API Chat models cannot write",
        ));
    }
    let coordinator = &m.extensions.traffic;
    let _gate = coordinator.gate.lock().await;
    let id = format!("hive_{}", input.request_id);
    let payload = serde_json::to_value(&input).map_err(io::Error::other)?;
    if let Ok(t) = coordinator.task(&id) {
        if serde_json::to_value(&t.start).map_err(io::Error::other)? != payload {
            return Err(invalid(
                "task request identity already has different content",
            ));
        }
        return Ok(Json(json!({"id":id,"state":t.state})));
    }
    if coordinator.active.lock().unwrap().len() >= 4 {
        return Err(invalid("four isolated Work tasks are already active"));
    }
    if coordinator.data.lock().unwrap().tasks.len() >= 200 {
        return Err(invalid(
            "traffic task retention limit reached; preserve the data directory",
        ));
    }
    let (root, objective) =
        m.bee
            .council_work_context(&input.council_id, &input.round_id, input.microtask)?;
    let path = files::relative(&input.path)?;
    if input.path.is_empty() || input.path.starts_with('.') {
        return Err(invalid(
            "choose an existing tracked project file outside Git metadata",
        ));
    }
    let models = m.codex_models().await?;
    if !models.iter().any(|v| v.model == input.model) {
        return Err(invalid("choose an installed native Codex model"));
    }
    let base_commit = git::check_project(&root, &input.path).await?;
    let base = files::council_action(
        m.clone(),
        input.council_id.clone(),
        "inspect",
        &input.path,
        json!({}),
    )
    .await?;
    let public = resource(&root, &path, "file", false)?;
    let worktree = m
        .workspaces
        .resolve(&format!("htc_{}", short(&id)), Some("hive-isolated-work"))?;
    let private = Resource {
        id: format!("resource_{}", short(&canonical_key(&worktree.path))),
        workspace_id: worktree.id.clone(),
        path: ".".into(),
        kind: "worktree".into(),
        key: canonical_key(&worktree.path),
        directory: true,
    };
    let task = Task {
        id: id.clone(),
        start: input,
        root,
        worktree,
        resource: public.id.clone(),
        private_resource: private.id.clone(),
        objective,
        state: "preparing".into(),
        activity: "Creating an isolated Git worktree".into(),
        revision: 1,
        attempt: 1,
        session_id: format!("cr_htc_{}_1", short(&id)),
        owned_pid: None,
        owned_created: None,
        quiescent: false,
        base_commit,
        base,
        proposed: None,
        reservation: None,
        created_at: now(),
        updated_at: now(),
        error: String::new(),
    };
    coordinator.transact(|d| {
        d.engine.register(public)?;
        d.engine.register(private)?;
        d.tasks.insert(id.clone(), task);
        Ok(())
    })?;
    worker::spawn(&m, &id)?;
    Ok(Json(json!({"id":id,"state":"preparing"})))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Action {
    request_id: String,
    expected_revision: u64,
    #[serde(default)]
    proposed_revision: String,
    #[serde(default)]
    current_revision: String,
    #[serde(default)]
    authorize: bool,
}
async fn action(
    State(m): State<Arc<Manager>>,
    Path((id, action)): Path<(String, String)>,
    Json(input): Json<Action>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    let c = &m.extensions.traffic;
    let _gate = c.gate.lock().await;
    let identity = format!("{}:{}", id, input.request_id);
    let payload = json!({"action":action,"input":input});
    if let Some((before, result)) = c.data.lock().unwrap().ledger.get(&identity) {
        if *before != payload {
            return Err(invalid(
                "action request identity already has different content",
            ));
        }
        return Ok(Json(result.clone()));
    }
    let t = c.task(&id)?;
    if input.expected_revision != t.revision {
        return Err(invalid("task changed; refresh before controlling it"));
    }
    if !matches!(
        action.as_str(),
        "pause" | "cancel" | "reject" | "retry" | "approve" | "recover"
    ) {
        return Err(invalid("unsupported traffic task action"));
    }
    if action == "approve" {
        if !input.authorize || t.state != "verification" || !t.quiescent {
            return Err(invalid(
                "inspect and authorize a verified, stopped proposal",
            ));
        }
        let proposed = t.proposed.as_ref().ok_or(invalid("proposal is missing"))?;
        if proposed["revision"].as_str() != Some(input.proposed_revision.as_str()) {
            return Err(invalid("proposal revision changed; inspect it again"));
        }
        worker::integrate(&m, &t).await?;
    } else if action == "recover" {
        if !input.authorize || t.state != "recovery" {
            return Err(invalid("inspect disk and explicitly authorize recovery"));
        }
        worker::close_owned(&m, &t).await?;
        let current = files::council_action(
            m.clone(),
            t.start.council_id.clone(),
            "inspect",
            &t.start.path,
            json!({}),
        )
        .await?;
        if current["revision"].as_str() != Some(input.current_revision.as_str()) {
            return Err(invalid("shared file changed; inspect recovery state again"));
        }
        worker::release_all(c, &t)?;
        c.update(&id, |t| {
            t.state = "rejected".into();
            t.quiescent = true;
            t.activity =
                "Recovery reviewed; current shared file retained and private work preserved".into();
        })?;
    } else if action == "retry" {
        if c.active.lock().unwrap().contains_key(&id) {
            return Err(invalid("wait for the owned runner to finish stopping"));
        }
        if !input.authorize
            || !matches!(t.state.as_str(), "failed" | "paused" | "cancelled")
            || !t.quiescent
            || t.attempt >= 3
        {
            return Err(invalid(
                "retry requires stopped failed/paused work, authorization and fewer than three attempts",
            ));
        }
        if t.proposed.is_some() {
            c.transact(|d| {
                let request = format!("retry_{}", short(&identity));
                d.engine
                    .reserve(&request, owner(&t), vec![t.resource.clone()], 30000, now())?;
                let lease = d.engine.reservation_lease(&request);
                if let Some(l) = &lease {
                    d.engine
                        .phase(&l.id, &l.token, l.generation, "verification", true, now())?;
                }
                let t = d.tasks.get_mut(&id).ok_or(Error::NotFound)?;
                t.attempt += 1;
                t.reservation = Some(request);
                t.state = if lease.is_some() {
                    "verification"
                } else {
                    "waiting"
                }
                .into();
                t.activity = "Requeued the preserved proposal without another model call".into();
                t.error.clear();
                t.revision += 1;
                t.updated_at = now();
                Ok(())
            })?;
        } else {
            if c.active.lock().unwrap().len() >= 4 {
                return Err(invalid("four isolated Work tasks are already active"));
            }
            c.update(&id, |t| {
                t.attempt += 1;
                t.session_id = format!("cr_htc_{}_{}", short(&id), t.attempt);
                t.owned_pid = None;
                t.owned_created = None;
                t.state = "preparing".into();
                t.activity = "Retrying preserved isolated work".into();
                t.error.clear();
                t.reservation = None;
                t.proposed = None;
                t.quiescent = false;
            })?;
            worker::spawn(&m, &id)?;
        }
    } else {
        if matches!(t.state.as_str(), "integrated" | "rejected" | "recovery") {
            return Err(invalid(
                "completed or recovery tasks require explicit inspection",
            ));
        }
        let sender = c.active.lock().unwrap().get(&id).cloned();
        if let Some(sender) = sender {
            // Persist the desired stop before notifying the owned runner.
            c.update(&id, |t| {
                t.state = "stopping".into();
                t.activity = format!(
                    "{} requested; waiting for owned execution termination",
                    action
                );
            })?;
            sender.send_replace(
                if action == "pause" {
                    "paused"
                } else {
                    "cancelled"
                }
                .into(),
            );
        } else {
            worker::close_owned(&m, &t).await?;
            worker::release_all(c, &t)?;
            c.update(&id, |t| {
                t.state = if action == "pause" {
                    "paused"
                } else if action == "reject" {
                    "rejected"
                } else {
                    "cancelled"
                }
                .into();
                t.quiescent = true;
                t.activity = "Owned execution stopped; isolated work and proposal retained".into();
            })?;
        }
    }
    let t = c.task(&id)?;
    let result = json!({"id":id,"state":t.state,"revision":t.revision});
    c.transact(|d| {
        if d.ledger.len() >= 4096 {
            return Err(invalid("traffic action retention limit reached"));
        }
        d.ledger.insert(identity, (payload, result.clone()));
        Ok(())
    })?;
    Ok(Json(result))
}
pub(super) fn start_monitor(m: &Arc<Manager>) {
    let weak = Arc::downgrade(m);
    tokio::spawn(async move {
        let mut clock = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            clock.tick().await;
            let Some(m) = weak.upgrade() else { break };
            if m.is_stopping() {
                break;
            }
            let c = &m.extensions.traffic;
            let _gate = c.gate.lock().await;
            let needed = {
                let d = c.data.lock().unwrap();
                d.engine.leases.values().any(|l| {
                    matches!(l.state.as_str(), "reserved" | "active" | "verification")
                        && (l.expires_at <= now() || l.heartbeat_at.saturating_add(10000) <= now())
                }) || d
                    .engine
                    .reservations
                    .iter()
                    .any(|r| r.state == "waiting" && r.expires_at <= now())
                    || d.tasks.values().any(|t| {
                        t.state == "waiting"
                            && t.reservation.as_deref().is_some_and(|request| {
                                d.engine
                                    .reservations
                                    .iter()
                                    .any(|r| r.id == request && r.state != "waiting")
                            })
                    })
            };
            if !needed {
                continue;
            }
            let result=c.transact(|d|{
                d.engine.expire(now())?;
                for t in d.tasks.values_mut() {
                    let private=format!("private_{}_{}",short(&t.id),t.attempt);
                    for request in [Some(private.as_str()),t.reservation.as_deref()].into_iter().flatten() {
                        if let Some(l)=d.engine.reservation_lease(request) {
                            if l.state=="recovery" {if !matches!(t.state.as_str(),"integrated"|"rejected"|"cancelled"|"paused"|"failed"|"recovery"){t.state="recovery".into();t.activity="Lease expired; ownership retained until execution and disk are reconciled".into();t.revision+=1;}}
                            else if matches!(l.state.as_str(),"active"|"verification"|"reserved") {d.engine.renew(&l.id,&l.token,l.generation,now())?;}
                        }
                    }
                    if t.state=="waiting" {
                        if let Some(l)=t.reservation.as_deref().and_then(|r|d.engine.reservation_lease(r)) {d.engine.phase(&l.id,&l.token,l.generation,"verification",true,now())?;t.state="verification".into();t.activity="Ownership acquired; inspect the exact proposed file before approval".into();t.revision+=1;}
                        else if d.engine.reservations.iter().any(|r|Some(r.id.as_str())==t.reservation.as_deref()&&r.state=="expired") {t.state="failed".into();t.error="Integration queue expired after two minutes; isolated work preserved".into();t.revision+=1;}
                    }
                }
                Ok(())
            });
            if result.is_err() {
                for cancel in c.active.lock().unwrap().values() {
                    cancel.send_replace("recovery".into());
                }
            }
            let recovery: Vec<_> = c
                .data
                .lock()
                .unwrap()
                .tasks
                .values()
                .filter(|t| t.state == "recovery")
                .map(|t| t.id.clone())
                .collect();
            for id in recovery {
                if let Some(s) = c.active.lock().unwrap().get(&id) {
                    s.send_replace("recovery".into());
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_aliases_and_durable_restart_keep_write_ownership() {
        let base = std::env::temp_dir().canonicalize().unwrap();
        let root = base.join(format!("cloudbee-traffic-{}", random_token().unwrap()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("Note.txt"), b"fixture").unwrap();
        let r = resource(&root, FsPath::new("Note.txt"), "file", false).unwrap();
        let alias = resource(&root, FsPath::new("NOTE.TXT"), "file", false).unwrap();
        assert_eq!(r.id, alias.id);
        let directory = resource(&root, FsPath::new("."), "directory", true).unwrap();
        assert!(leases::overlap(&directory, &r));
        assert!(resource(&root, FsPath::new("../outside"), "file", false).is_err());
        let store = HiveTrafficCoordinator::open(&root, Arc::new(Mutex::new(()))).unwrap();
        let owned = Owner {
            agent_id: "codex:fixture".into(),
            session_id: "owned".into(),
            task_id: "fixture".into(),
            can_work: true,
        };
        store
            .transact(|d| {
                d.engine.register(r.clone())?;
                d.engine
                    .reserve("request", owned.clone(), vec![r.id.clone()], 30000, now())?;
                Ok(())
            })
            .unwrap();
        let old = store
            .data
            .lock()
            .unwrap()
            .engine
            .reservation_lease("request")
            .unwrap();
        assert!(
            store
                .authorize_file_write(&root, FsPath::new("NOTE.TXT"), None)
                .is_err()
        );
        drop(store);
        let restored = HiveTrafficCoordinator::open(&root, Arc::new(Mutex::new(()))).unwrap();
        let data = restored.data.lock().unwrap();
        assert_eq!(data.engine.traffic(std::slice::from_ref(&r.id)).0, "blue");
        assert!(
            data.engine
                .validate(&old.id, &old.token, old.generation, now())
                .is_err()
        );
        assert_ne!(random_token().unwrap(), old.token);
        drop(data);
        drop(restored);
        assert_eq!(root.canonicalize().unwrap().parent(), Some(base.as_path()));
        assert!(
            root.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("cloudbee-traffic-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
