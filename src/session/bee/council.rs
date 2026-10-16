//! Persisted Council conversations, a bounded dependency scheduler and scoped file operations.
use super::*;
use tokio::sync::watch;
mod runner;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Member {
    harness: String,
    model: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Access {
    Discuss,
    Read,
    Edit,
}
#[derive(Clone, Serialize, Deserialize)]
struct Reply {
    member: usize,
    state: String,
    text: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Task {
    title: String,
    owner: usize,
    depends_on: Vec<usize>,
    instructions: String,
    state: String,
    activity: String,
    result: String,
    steps: usize,
    #[serde(default)]
    operations: Vec<Value>,
    started_at: Option<u64>,
    finished_at: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Round {
    #[serde(default)]
    members: Vec<Member>,
    #[serde(default)]
    instructions: String,
    request_id: String,
    text: String,
    coordinate: bool,
    state: String,
    replies: Vec<Reply>,
    tasks: Vec<Task>,
    summary: String,
    started_at: u64,
    finished_at: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Council {
    id: String,
    title: String,
    members: Vec<Member>,
    workspace: Option<PathBuf>,
    access: Access,
    start: Value,
    rounds: Vec<Round>,
}
pub(super) struct Store {
    root: PathBuf,
    all: Mutex<BTreeMap<String, Council>>,
    active: Mutex<BTreeMap<String, watch::Sender<bool>>>,
}
fn active(state: &str) -> bool {
    matches!(
        state,
        "broadcasting" | "planning" | "executing" | "synthesizing" | "cancelling"
    )
}
fn now() -> u64 {
    crate::observability::now_ms()
}
impl Store {
    pub(super) fn open(root: &FsPath) -> io::Result<Self> {
        let root = root.join("councils");
        fs::DirBuilder::new()
            .recursive(true)
            .unix_mode(0o700)
            .create(&root)?;
        let mut all = BTreeMap::new();
        for entry in fs::read_dir(&root)? {
            let path = entry?.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let mut group: Council = serde_json::from_slice(&fs::read(&path)?)?;
            key(group
                .id
                .strip_prefix("council_")
                .ok_or_else(|| io::Error::other("invalid Council identity"))?)
            .map_err(|_| io::Error::other("invalid Council identity"))?;
            for round in &mut group.rounds {
                if active(&round.state) {
                    round.state = "unknown_after_restart".into();
                    round.finished_at = Some(now());
                    for reply in &mut round.replies {
                        if reply.state == "running" || reply.state == "queued" {
                            reply.state = "unknown_after_restart".into();
                        }
                    }
                    for task in &mut round.tasks {
                        if task.state == "running" || task.state == "queued" {
                            task.state = "unknown_after_restart".into();
                            task.activity="Interrupted; inspect files and recovery copies before starting another message".into();
                        }
                        for operation in &mut task.operations {
                            if operation["state"] == "running" {
                                operation["state"] = json!("unknown_after_restart");
                            }
                        }
                    }
                }
            }
            save(&path, &group)?;
            all.insert(group.id.clone(), group);
        }
        Ok(Self {
            root,
            all: Mutex::new(all),
            active: Mutex::new(BTreeMap::new()),
        })
    }
    fn read(&self, id: &str) -> Result<Council> {
        self.all
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn update<T>(&self, id: &str, f: impl FnOnce(&mut Council) -> Result<T>) -> Result<T> {
        let mut all = self.all.lock().unwrap();
        let mut c = all.get(id).cloned().ok_or(Error::NotFound)?;
        let result = f(&mut c)?;
        save(&self.root.join(format!("{id}.json")), &c)?;
        all.insert(id.into(), c);
        Ok(result)
    }
    fn round<T>(
        &self,
        id: &str,
        request: &str,
        f: impl FnOnce(&mut Round) -> Result<T>,
    ) -> Result<T> {
        self.update(id, |c| {
            f(c.rounds
                .iter_mut()
                .find(|r| r.request_id == request)
                .ok_or(Error::NotFound)?)
        })
    }
}
impl super::Store {
    pub(crate) fn council_work_context(
        &self,
        id: &str,
        round: &str,
        index: usize,
    ) -> Result<(PathBuf, String)> {
        let c = self.councils.read(id)?;
        if c.access != Access::Edit {
            return Err(invalid("Council has not authorized isolated Work"));
        }
        let r = c
            .rounds
            .iter()
            .find(|r| r.request_id == round)
            .ok_or(Error::NotFound)?;
        let t = r.tasks.get(index).ok_or(Error::NotFound)?;
        Ok((
            c.workspace.ok_or(invalid("select a Council workspace"))?,
            format!(
                "{}\n\nCouncil objective: {}\n\nMicrotask: {}\n{}\n\nResearch findings: {}",
                r.instructions, r.text, t.title, t.instructions, t.result
            ),
        ))
    }
    pub(crate) fn council_workspace(&self, id: &str) -> Result<PathBuf> {
        self.councils
            .read(id)?
            .workspace
            .ok_or(invalid("Council has no selected workspace"))
    }
}
pub(super) fn routes() -> Router<Arc<Manager>> {
    Router::new()
        .route("/v1/bee/councils", get(list).post(create))
        .route("/v1/bee/councils/catalog", get(catalog))
        .route("/v1/bee/councils/{id}", get(status))
        .route(
            "/v1/bee/councils/{id}/messages",
            post(message).layer(axum::extract::DefaultBodyLimit::max(128 * 1024)),
        )
        .route("/v1/bee/councils/{id}/cancel", post(cancel))
}
async fn catalog(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let profiles = manager.bee.profiles.lock().unwrap();
    Json(
        json!({"models":profiles.iter().map(|p|json!({"harness":p.harness,"model":p.model,"provider":p.provider})).collect::<Vec<_>>(),"access":["discuss","read","edit"],"max_members":4,"max_tasks":12,"max_steps":8}),
    )
}
async fn list(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let all = manager.bee.councils.all.lock().unwrap();
    Json(
        json!({"councils":all.values().map(|c|json!({"id":c.id,"title":c.title,"members":c.members,"workspace":c.workspace,"access":c.access,"state":c.rounds.last().map(|r|r.state.as_str()).unwrap_or("idle")})).collect::<Vec<_>>()}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    before: Option<usize>,
}
async fn status(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Query(history): Query<History>,
) -> Result<Json<Value>> {
    let mut c = manager.bee.councils.read(&id)?;
    let total = c.rounds.len();
    let end = history.before.unwrap_or(total).min(total);
    let start = end.saturating_sub(3);
    let latest = c.rounds.last().cloned();
    c.rounds = c.rounds[start..end].to_vec();
    Ok(Json(
        json!({"council":c,"latest":latest,"rounds_total":total,"rounds_offset":start}),
    ))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Start {
    request_id: String,
    title: String,
    members: Vec<Member>,
    workspace: Option<PathBuf>,
    access: Access,
    authorize_access: bool,
}
fn workspace(path: &FsPath) -> Result<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    let text = path.to_string_lossy();
    if !path.is_absolute() || text.starts_with("\\\\") || text.chars().any(char::is_control) {
        return Err(invalid("choose an ordinary local Windows folder"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| invalid("workspace is unavailable"))?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(invalid(
            "workspace root must be an ordinary local directory",
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| invalid("workspace is unavailable"))?;
    if canonical
        .to_string_lossy()
        .to_uppercase()
        .starts_with("\\\\?\\UNC\\")
    {
        return Err(invalid(
            "choose a local folder rather than a mapped network share",
        ));
    }
    Ok(canonical)
}
async fn create(
    State(manager): State<Arc<Manager>>,
    Json(input): Json<Start>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if input.request_id.len() > 32
        || input.title.is_empty()
        || input.title.len() > 160
        || input.members.is_empty()
        || input.members.len() > 4
    {
        return Err(invalid(
            "choose 1–4 configured models and a Council title within 160 bytes",
        ));
    }
    let id = format!("council_{}", input.request_id);
    let start = serde_json::to_value(&input).map_err(io::Error::other)?;
    let mut all = manager.bee.councils.all.lock().unwrap();
    if let Some(c) = all.get(&id) {
        if c.start != start {
            return Err(invalid("Council request ID already has different settings"));
        }
        return Ok(Json(json!({"council_id":id})));
    }
    for (i, member) in input.members.iter().enumerate() {
        if input.members[..i].contains(member)
            || manager
                .bee
                .profile(&member.harness, &member.model)
                .is_none()
        {
            return Err(invalid(
                "Council members must be distinct configured Chat API models",
            ));
        }
    }
    if input.access != Access::Discuss && (!input.authorize_access || input.workspace.is_none()) {
        return Err(invalid(
            "explicitly authorize the selected shared folder access",
        ));
    }
    let workspace = input.workspace.as_deref().map(workspace).transpose()?;
    if all.len() >= 100 {
        return Err(invalid("Council storage limit reached (100 conversations)"));
    }
    let c = Council {
        id: id.clone(),
        title: input.title,
        members: input.members,
        workspace,
        access: input.access,
        start,
        rounds: vec![],
    };
    save(&manager.bee.councils.root.join(format!("{id}.json")), &c)?;
    all.insert(id.clone(), c);
    Ok(Json(json!({"council_id":id})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    request_id: String,
    text: String,
    coordinate: bool,
    #[serde(default)]
    members: Option<Vec<Member>>,
}
async fn message(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(input): Json<Message>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if input.text.trim().is_empty() || input.text.len() > 16000 {
        return Err(invalid("Council message must contain 1–16000 UTF-8 bytes"));
    }
    let mut active_runs = manager.bee.councils.active.lock().unwrap();
    let c = manager.bee.councils.read(&id)?;
    if let Some(r) = c.rounds.iter().find(|r| r.request_id == input.request_id) {
        if r.text != input.text
            || r.coordinate != input.coordinate
            || input.members.as_ref().is_some_and(|m| *m != r.members)
        {
            return Err(invalid("message ID already has different content"));
        }
        return Ok(Json(json!({"request_id":input.request_id,"state":r.state})));
    }
    if active_runs.contains_key(&id) || active_runs.len() >= 4 {
        return Err(invalid(
            "Council is still working or the four-Council concurrency limit is reached",
        ));
    }
    if c.rounds.len() >= 100 {
        return Err(invalid(
            "start a new Council after 100 messages; this history is retained",
        ));
    }
    if let Some(root) = &c.workspace {
        crate::session::extensions::files::validate_root(root)?;
    }
    let members = input.members.unwrap_or_else(|| c.members.clone());
    if members.is_empty()
        || members.len() > 4
        || members
            .iter()
            .enumerate()
            .any(|(i, m)| members[..i].contains(m))
    {
        return Err(invalid("enable 1–4 distinct Council models"));
    }
    let profiles = members
        .iter()
        .map(|m| {
            manager
                .bee
                .profile(&m.harness, &m.model)
                .ok_or(invalid("a Council model connection is unavailable"))
        })
        .collect::<Result<Vec<_>>>()?;
    let r = Round {
        members: members.clone(),
        instructions: council_instructions(&manager, &c)?,
        request_id: input.request_id.clone(),
        text: input.text,
        coordinate: input.coordinate,
        state: "broadcasting".into(),
        replies: (0..members.len())
            .map(|member| Reply {
                member,
                state: "queued".into(),
                text: String::new(),
            })
            .collect(),
        tasks: vec![],
        summary: String::new(),
        started_at: now(),
        finished_at: None,
    };
    manager.bee.councils.update(&id, |c| {
        c.members = members;
        c.rounds.push(r);
        Ok(())
    })?;
    let (signal, receiver) = watch::channel(false);
    active_runs.insert(id.clone(), signal);
    drop(active_runs);
    let owner = manager.clone();
    let request = input.request_id.clone();
    tokio::spawn(async move {
        runner::run(owner, id, request, profiles, receiver).await;
    });
    Ok(Json(
        json!({"request_id":input.request_id,"state":"broadcasting"}),
    ))
}
fn council_instructions(manager: &Manager, c: &Council) -> Result<String> {
    let workspace = if let Some(root) = &c.workspace {
        if manager.config.repository.canonicalize().ok().as_ref() == Some(root) {
            crate::workspace::Workspace {
                id: "legacy".into(),
                path: root.clone(),
                parent: None,
            }
        } else {
            manager
                .workspaces
                .list()?
                .into_iter()
                .find(|w| w.path == *root)
                .unwrap_or_else(|| crate::workspace::Workspace {
                    id: c.id.clone(),
                    path: root.clone(),
                    parent: None,
                })
        }
    } else {
        crate::workspace::Workspace {
            id: c.id.clone(),
            path: PathBuf::new(),
            parent: None,
        }
    };
    Ok(manager.extensions.instructions_for(&workspace))
}
async fn cancel(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    if body.as_object().is_none_or(|o| !o.is_empty()) {
        return Err(invalid("cancel takes an empty object"));
    }
    let all = manager.bee.councils.active.lock().unwrap();
    if let Some(signal) = all.get(&id) {
        manager.bee.councils.update(&id, |c| {
            if let Some(r) = c.rounds.last_mut() {
                r.state = "cancelling".into();
            }
            Ok(())
        })?;
        let _ = signal.send(true);
    } else {
        manager.bee.councils.read(&id)?;
    }
    Ok(Json(json!({"cancel_requested":true})))
}
