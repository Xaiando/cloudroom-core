//! Native plugin ports and desktop organization, served by the existing Core API.
#[cfg(windows)]
mod files;
mod library;
mod plugins;
mod runner;
mod schedule;
mod threads;
use super::{Error, Manager, Result};
use crate::platform::{DirBuilderExt, OpenOptionsExt};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use schedule::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Execution {
    Script {
        interpreter: String,
        source: String,
        timeout_ms: u64,
    },
    Agent {
        conversation_id: String,
        prompt: String,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    name: String,
    enabled: bool,
    schedule: Schedule,
    execution: Execution,
    workspace: Option<String>,
    workspace_name: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Automation {
    id: String,
    definition: Definition,
    workspace: crate::workspace::Workspace,
    native_session: Option<String>,
    next_run_at: Option<u64>,
    deleted: bool,
    revision: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Run {
    id: String,
    automation_id: String,
    trigger: String,
    started_at: u64,
    finished_at: Option<u64>,
    state: String,
    execution: Execution,
    workspace: crate::workspace::Workspace,
    native_session: Option<String>,
    output: String,
    error: String,
    exit_code: Option<i32>,
    instructions: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    threads: BTreeMap<String, threads::Organization>,
    #[serde(default)]
    library: BTreeMap<String, library::Entry>,
    automations: BTreeMap<String, Automation>,
    runs: BTreeMap<String, Run>,
    ledger: BTreeMap<String, (Value, Value)>,
    plugins: BTreeMap<String, plugins::Plugin>,
}
pub(crate) struct Extensions {
    root: PathBuf,
    data: Mutex<Data>,
    active: Mutex<BTreeMap<String, watch::Sender<bool>>>,
    power_status: Mutex<String>,
    stopped: std::sync::atomic::AtomicBool,
}
fn invalid(message: &'static str) -> Error {
    Error::Conflict(message)
}
fn now() -> u64 {
    crate::observability::now_ms()
}
fn key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 48
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(invalid("invalid extension identity"));
    }
    Ok(())
}
fn save(path: &std::path::Path, data: &Data) -> io::Result<()> {
    use std::io::Write;
    let pending = path.with_extension("pending");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .unix_mode(0o600)
        .open(&pending)?;
    file.write_all(&serde_json::to_vec(data)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(pending, path)?;
    crate::platform::sync_directory(
        path.parent()
            .ok_or_else(|| io::Error::other("missing extension directory"))?,
    )
}
impl Extensions {
    pub(super) fn open(state: &std::path::Path) -> io::Result<Self> {
        let root = state.join("extensions");
        fs::DirBuilder::new()
            .recursive(true)
            .unix_mode(0o700)
            .create(&root)?;
        let path = root.join("state.json");
        let mut data: Data = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e),
        };
        for run in data.runs.values_mut() {
            if run.state == "running" {
                run.state = "unknown_after_restart".into();
                run.finished_at = Some(now());
                run.error="Core restarted; this run will not be replayed. Inspect its linked session or workspace.".into();
            }
        }
        plugins::initialize(&mut data);
        save(&path, &data)?;
        Ok(Self {
            root,
            data: Mutex::new(data),
            active: Mutex::new(BTreeMap::new()),
            power_status: Mutex::new("disabled".into()),
            stopped: std::sync::atomic::AtomicBool::new(false),
        })
    }
    fn transact<T>(&self, update: impl FnOnce(&mut Data) -> Result<T>) -> Result<T> {
        let mut data = self.data.lock().unwrap();
        let mut next = data.clone();
        let result = update(&mut next)?;
        save(&self.root.join("state.json"), &next)?;
        *data = next;
        Ok(result)
    }
    pub(crate) fn instructions_for(&self, workspace: &crate::workspace::Workspace) -> String {
        let data = self.data.lock().unwrap();
        let custom = data
            .plugins
            .get("custom-instructions")
            .filter(|p| p.installed && p.enabled)
            .and_then(|p| p.settings["instructions"].as_str())
            .unwrap_or("")
            .to_owned();
        let memory = library::context(&data, &workspace.path);
        [custom, memory]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    fn enabled(&self, id: &str) -> bool {
        self.data
            .lock()
            .unwrap()
            .plugins
            .get(id)
            .is_some_and(|p| p.installed && p.enabled)
    }
    pub(crate) fn start(manager: &Arc<Manager>) {
        if !cfg!(windows) {
            return;
        }
        plugins::start_power(manager);
        let weak = Arc::downgrade(manager);
        tokio::spawn(async move {
            let mut clock = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                clock.tick().await;
                let Some(manager) = weak.upgrade() else { break };
                if manager.is_stopping()
                    || manager
                        .extensions
                        .stopped
                        .load(std::sync::atomic::Ordering::Relaxed)
                {
                    break;
                }
                if !manager.extensions.enabled("automations") || manager.storage.blocks() {
                    continue;
                }
                let due: Vec<_> = manager
                    .extensions
                    .data
                    .lock()
                    .unwrap()
                    .automations
                    .values()
                    .filter(|a| {
                        !a.deleted
                            && a.definition.enabled
                            && a.next_run_at.is_some_and(|t| t <= now())
                    })
                    .map(|a| (a.id.clone(), a.next_run_at.unwrap()))
                    .collect();
                for (id, due) in due {
                    let _ = claim(&manager, &id, format!("due_{due}"), "scheduled");
                }
            }
        });
    }
    pub(crate) fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
        for cancel in self.active.lock().unwrap().values() {
            cancel.send_replace(true);
        }
    }
}
pub(crate) fn routes() -> Router<Arc<Manager>> {
    if !cfg!(windows) {
        return Router::new();
    }
    Router::new()
        .merge(file_routes())
        .route("/v1/bee/threads", get(threads::list))
        .route("/v1/bee/threads/{id}/organize", post(threads::write))
        .route("/v1/bee/library", get(library::list).post(library::create))
        .route("/v1/bee/library/{id}", get(library::detail))
        .route("/v1/bee/library/{id}/{action}", post(library::action))
        .route("/v1/bee/automations", get(list).post(create))
        .route("/v1/bee/automations/{id}", get(detail))
        .route("/v1/bee/automations/{id}/{action}", post(action))
        .route("/v1/bee/plugins", get(plugins::list))
        .route("/v1/bee/plugins/{id}/{action}", post(plugins::action))
}
#[cfg(windows)]
fn file_routes() -> Router<Arc<Manager>> {
    files::routes()
}
#[cfg(not(windows))]
fn file_routes() -> Router<Arc<Manager>> {
    Router::new()
}
async fn list(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let workspaces = manager.workspaces.list().unwrap_or_default();
    let data = manager.extensions.data.lock().unwrap();
    let automations:Vec<_>=data.automations.values().filter(|a| !a.deleted).map(|a| {
        let last=data.runs.values().filter(|r| r.automation_id==a.id).max_by_key(|r| r.started_at);
        json!({"id":a.id,"name":a.definition.name,"enabled":a.definition.enabled,"schedule":a.definition.schedule,"execution_type":match a.definition.execution { Execution::Script{..}=>"script",Execution::Agent{..}=>"agent" },"next_run_at":a.next_run_at,"workspace":a.workspace,"revision":a.revision,"last_run":last.map(|r|json!({"id":r.id,"state":r.state,"started_at":r.started_at,"finished_at":r.finished_at,"exit_code":r.exit_code}))})
    }).collect();
    Json(
        json!({"automations":automations,"workspaces":workspaces,"default_workspace":manager.config.repository,"plugin_enabled":data.plugins.get("automations").is_some_and(|p| p.installed&&p.enabled),"local":true}),
    )
}
async fn detail(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let data = manager.extensions.data.lock().unwrap();
    let automation = data.automations.get(&id).ok_or(Error::NotFound)?;
    let mut runs: Vec<_> = data
        .runs
        .values()
        .filter(|r| r.automation_id == id)
        .cloned()
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    Ok(Json(json!({"automation":automation,"runs":runs})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    request_id: String,
    automation: Definition,
    authorize_execution: bool,
}
async fn validate(
    manager: &Manager,
    definition: &Definition,
    authorize: bool,
) -> Result<(crate::workspace::Workspace, Option<String>)> {
    if !authorize {
        return Err(invalid(
            "explicit automation execution authorization required",
        ));
    }
    if definition.name.trim().is_empty() || definition.name.len() > 200 {
        return Err(invalid("use an automation name within 200 bytes"));
    }
    if definition.schedule.next(now())?.is_none() {
        return Err(invalid("one-shot run time must be in the future"));
    }
    match &definition.execution {
        Execution::Script {
            interpreter,
            source,
            timeout_ms,
        } => {
            if source.trim().is_empty()
                || source.len() > 32768
                || !(1000..=600_000).contains(timeout_ms)
            {
                return Err(invalid(
                    "script requires 1-32768 bytes and a timeout of 1000-600000 milliseconds",
                ));
            }
            runner::interpreter(interpreter)?;
            let workspace = match &definition.workspace {
                Some(id) => manager
                    .workspaces
                    .resolve(id, definition.workspace_name.as_deref())
                    .map_err(|_| invalid("invalid automation workspace"))?,
                None => crate::workspace::Workspace {
                    id: "legacy".into(),
                    path: manager.config.repository.canonicalize()?,
                    parent: None,
                },
            };
            Ok((workspace, None))
        }
        Execution::Agent {
            conversation_id,
            prompt,
        } => {
            if prompt.trim().is_empty() || prompt.len() > 28000 {
                return Err(invalid("agent prompt requires 1-28000 bytes"));
            }
            let (native, workspace) = manager.bee.automation_target(conversation_id, true)?;
            let state = manager.session(&native).await?;
            if matches!(state.state.as_str(), "closed" | "failed" | "process_lost") {
                return Err(invalid(
                    "agent automation requires a usable authorized Work session",
                ));
            }
            Ok((workspace, Some(native)))
        }
    }
}
fn replay(data: &Data, key: &str, input: &Value) -> Result<Option<Value>> {
    if let Some((prior, result)) = data.ledger.get(key) {
        if prior != input {
            return Err(invalid("request_id already has different content"));
        }
        return Ok(Some(result.clone()));
    }
    Ok(None)
}
async fn create(
    State(manager): State<Arc<Manager>>,
    Json(input): Json<Write>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    let id = format!("auto_{}", input.request_id);
    let payload = json!({"action":"create","automation":input.automation,"authorize":input.authorize_execution});
    if let Some(prior) = replay(
        &manager.extensions.data.lock().unwrap(),
        &input.request_id,
        &payload,
    )? {
        return Ok(Json(prior));
    }
    let (workspace, native_session) =
        validate(&manager, &input.automation, input.authorize_execution).await?;
    let next_run_at = if input.automation.enabled {
        input.automation.schedule.next(now())?
    } else {
        None
    };
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        if data.automations.values().filter(|a| !a.deleted).count() >= 100 {
            return Err(invalid("limit of 100 automations reached"));
        }
        let automation = Automation {
            id: id.clone(),
            definition: input.automation,
            workspace,
            native_session,
            next_run_at,
            deleted: false,
            revision: 1,
        };
        data.automations.insert(id.clone(), automation);
        let result = json!({"id":id});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    request_id: String,
    automation: Option<Definition>,
    expected_revision: Option<u64>,
    #[serde(default)]
    authorize_execution: bool,
    run_id: Option<String>,
}
async fn action(
    State(manager): State<Arc<Manager>>,
    Path((id, action)): Path<(String, String)>,
    Json(input): Json<Action>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if action == "run" {
        return claim(&manager, &id, input.request_id, "manual").map(Json);
    }
    if action == "cancel" {
        let run = input.run_id.ok_or(invalid("run_id required"))?;
        {
            let data = manager.extensions.data.lock().unwrap();
            let saved = data.runs.get(&run).ok_or(Error::NotFound)?;
            if saved.automation_id != id {
                return Err(invalid("run belongs to another automation"));
            }
        }
        if let Some(signal) = manager.extensions.active.lock().unwrap().get(&run) {
            signal.send_replace(true);
        }
        return Ok(Json(json!({"cancel_requested":true})));
    }
    let payload = json!({"action":action,"id":id,"automation":input.automation,"revision":input.expected_revision,"authorize":input.authorize_execution});
    if let Some(prior) = replay(
        &manager.extensions.data.lock().unwrap(),
        &input.request_id,
        &payload,
    )? {
        return Ok(Json(prior));
    }
    let validated = if action == "update" {
        Some(
            validate(
                &manager,
                input
                    .automation
                    .as_ref()
                    .ok_or(invalid("automation required"))?,
                input.authorize_execution,
            )
            .await?,
        )
    } else {
        None
    };
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        let active = data
            .runs
            .values()
            .any(|r| r.automation_id == id && r.state == "running");
        let saved = data
            .automations
            .get_mut(&id)
            .filter(|a| !a.deleted)
            .ok_or(Error::NotFound)?;
        if input.expected_revision != Some(saved.revision) {
            return Err(invalid("automation changed; reload before editing"));
        }
        match action.as_str() {
            "pause" => {
                saved.definition.enabled = false;
                saved.next_run_at = None;
            }
            "enable" => {
                if !input.authorize_execution {
                    return Err(invalid(
                        "explicit automation execution authorization required",
                    ));
                }
                saved.next_run_at = saved.definition.schedule.next(now())?;
                if saved.next_run_at.is_none() {
                    return Err(invalid(
                        "edit the expired one-shot schedule before enabling",
                    ));
                }
                saved.definition.enabled = true;
            }
            "delete" => {
                if active {
                    return Err(invalid("cancel the active run before deleting"));
                }
                saved.deleted = true;
                saved.definition.enabled = false;
                saved.next_run_at = None;
            }
            "update" => {
                let (workspace, native) = validated.unwrap();
                saved.definition = input.automation.unwrap();
                saved.workspace = workspace;
                saved.native_session = native;
                saved.next_run_at = if saved.definition.enabled {
                    saved.definition.schedule.next(now())?
                } else {
                    None
                };
            }
            _ => return Err(invalid("unsupported automation action")),
        }
        saved.revision += 1;
        let result = json!({"id":id,"revision":saved.revision});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
fn claim(manager: &Arc<Manager>, id: &str, request: String, trigger: &str) -> Result<Value> {
    if !cfg!(windows) {
        return Err(invalid("native automations require Windows"));
    }
    if manager.is_stopping()
        || manager.storage.blocks()
        || manager
            .extensions
            .stopped
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Err(invalid("new execution is paused"));
    }
    key(&request)?;
    let run_id = format!("run_{}_{}", id.trim_start_matches("auto_"), request);
    let (cancel, receiver) = watch::channel(false);
    let mut active = manager.extensions.active.lock().unwrap();
    let run = manager.extensions.transact(|data| {
        if data.ledger.contains_key(&format!("run:{run_id}")) {
            return Ok(None);
        }
        if !data
            .plugins
            .get("automations")
            .is_some_and(|p| p.installed && p.enabled)
        {
            return Err(invalid("Automations plugin is disabled"));
        }
        if active.len() >= 4
            || data
                .runs
                .values()
                .any(|r| r.automation_id == id && r.state == "running")
        {
            return Err(invalid(
                "automation already running or worker limit reached",
            ));
        }
        let automation = data
            .automations
            .get_mut(id)
            .filter(|a| !a.deleted)
            .ok_or(Error::NotFound)?;
        if trigger == "scheduled" {
            if !automation.definition.enabled
                || !automation.next_run_at.is_some_and(|at| at <= now())
            {
                return Err(invalid("automation is not due"));
            }
            automation.next_run_at = match automation.definition.schedule {
                Schedule::Once { .. } => {
                    automation.definition.enabled = false;
                    automation.revision += 1;
                    None
                }
                _ => automation.definition.schedule.next(now())?,
            };
        }
        let automation = automation.clone();
        let custom = data
            .plugins
            .get("custom-instructions")
            .filter(|p| p.installed && p.enabled)
            .and_then(|p| p.settings["instructions"].as_str())
            .unwrap_or("")
            .to_owned();
        let memory = library::context(data, &automation.workspace.path);
        let instructions = [custom, memory]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let run = Run {
            id: run_id.clone(),
            automation_id: id.into(),
            trigger: trigger.into(),
            started_at: now(),
            finished_at: None,
            state: "running".into(),
            execution: automation.definition.execution.clone(),
            workspace: automation.workspace.clone(),
            native_session: automation.native_session.clone(),
            output: String::new(),
            error: String::new(),
            exit_code: None,
            instructions,
        };
        data.runs.insert(run_id.clone(), run.clone());
        data.ledger.insert(
            format!("run:{run_id}"),
            (
                json!({"automation":id,"trigger":trigger}),
                json!({"run_id":run_id}),
            ),
        );
        Ok(Some(run))
    })?;
    if let Some(run) = run {
        active.insert(run_id.clone(), cancel);
        let owner = manager.clone();
        tokio::spawn(async move {
            runner::run(owner, run, receiver).await;
        });
    }
    Ok(json!({"run_id":run_id}))
}
