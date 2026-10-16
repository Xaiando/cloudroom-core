//! Durable, tool-free conversations linked to separately authorized native execution.
use super::{Error, Manager, Result};
use crate::platform::{DirBuilderExt, OpenOptionsExt};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    harness: String,
    model: String,
    provider: String,
    endpoint: String,
    #[serde(default)]
    api_key: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Turn {
    input: Value,
    mode: String,
    state: String,
    output: String,
    usage: Value,
    execution_input: Value,
    #[serde(default)]
    handoff_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Conversation {
    id: String,
    start: Value,
    harness: String,
    model: String,
    mode: String,
    workspace: crate::workspace::Workspace,
    work: Option<String>,
    handoffs: Vec<Value>,
    switches: BTreeMap<String, Value>,
    turns: BTreeMap<String, Turn>,
    order: Vec<String>,
    closed: bool,
}

pub(crate) struct Store {
    root: PathBuf,
    conversations: Mutex<BTreeMap<String, Conversation>>,
    profiles: Mutex<Vec<Profile>>,
    transitions: tokio::sync::Mutex<()>,
}

fn invalid(message: &'static str) -> Error {
    Error::Conflict(message)
}
fn key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 48
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid("invalid Cloud Bee identity"));
    }
    Ok(())
}
fn save(path: &FsPath, value: &impl Serialize) -> io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("pending");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .unix_mode(0o600)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)?;
    crate::platform::sync_directory(
        path.parent()
            .ok_or_else(|| io::Error::other("missing Bee directory"))?,
    )
}
impl Store {
    pub(crate) fn automation_target(
        &self,
        id: &str,
        require_work_view: bool,
    ) -> Result<(String, crate::workspace::Workspace)> {
        let c = self.read(id)?;
        if c.closed || require_work_view && c.mode != "work" {
            return Err(invalid(
                "select an authorized Work conversation before creating an agent automation",
            ));
        }
        Ok((
            c.work
                .ok_or(invalid("agent automation requires authorized Work"))?,
            c.workspace,
        ))
    }
    pub(super) fn open(state: &FsPath) -> io::Result<Self> {
        let root = state.join("bee");
        fs::DirBuilder::new()
            .recursive(true)
            .unix_mode(0o700)
            .create(&root)?;
        let mut conversations = BTreeMap::new();
        for entry in fs::read_dir(&root)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json")
                || path.file_name().and_then(|s| s.to_str()) == Some("profiles.json")
            {
                continue;
            }
            let mut conversation: Conversation = serde_json::from_slice(&fs::read(&path)?)?;
            key(&conversation.id).map_err(|_| io::Error::other("invalid saved Bee identity"))?;
            for turn in conversation.turns.values_mut() {
                if turn.mode == "chat" && turn.state == "accepted" {
                    turn.state = "unknown_after_restart".into();
                }
            }
            save(&path, &conversation)?;
            conversations.insert(conversation.id.clone(), conversation);
        }
        let profiles = match fs::read(root.join("profiles.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            root,
            conversations: Mutex::new(conversations),
            profiles: Mutex::new(profiles),
            transitions: tokio::sync::Mutex::new(()),
        })
    }
    fn read(&self, id: &str) -> Result<Conversation> {
        self.conversations
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn update<T>(&self, id: &str, apply: impl FnOnce(&mut Conversation) -> Result<T>) -> Result<T> {
        let mut all = self.conversations.lock().unwrap();
        let mut next = all.get(id).cloned().ok_or(Error::NotFound)?;
        let result = apply(&mut next)?;
        save(&self.root.join(format!("{id}.json")), &next)?;
        all.insert(id.into(), next);
        Ok(result)
    }
    fn profile(&self, harness: &str, model: &str) -> Option<Profile> {
        self.profiles
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.harness == harness && p.model == model)
            .cloned()
    }
    pub(crate) fn redact(&self, text: &str, token: &str) -> String {
        let mut clean = text.replace(token, "[redacted]");
        for profile in self.profiles.lock().unwrap().iter() {
            if !profile.api_key.is_empty() {
                clean = clean.replace(&profile.api_key, "[redacted]");
            }
        }
        clean
    }
}

pub(crate) fn routes() -> Router<Arc<Manager>> {
    Router::new()
        .route("/v1/bee/capabilities", get(capabilities))
        .route("/v1/bee/profiles", post(configure))
        .route("/v1/bee/sessions", get(list).post(create))
        .route("/v1/bee/sessions/{id}", get(status))
        .route("/v1/bee/sessions/{id}/events", get(events))
        .route("/v1/bee/sessions/{id}/mode", post(mode))
        .route("/v1/bee/sessions/{id}/{command}", post(command))
}

async fn capabilities(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let mut core = manager.capabilities().await;
    let mut harnesses = core["harnesses"].as_array().cloned().unwrap_or_default();
    for harness in &mut harnesses {
        harness["work_models"] = if harness["id"] == "codex" {
            json!(
                harness["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m["model"].as_str())
                    .collect::<Vec<_>>()
            )
        } else {
            json!([])
        };
    }
    let profiles = manager.bee.profiles.lock().unwrap().clone();
    for profile in &profiles {
        if !harnesses.iter().any(|h| h["id"] == profile.harness) {
            harnesses.push(json!({"id":profile.harness,"model":profile.model,"models":[]}));
        }
        let harness = harnesses
            .iter_mut()
            .find(|h| h["id"] == profile.harness)
            .unwrap();
        if let Some(models) = harness["models"].as_array_mut() {
            if !models.iter().any(|m| m["model"] == profile.model) {
                models.push(json!({"model":profile.model,"reasoning_levels":[]}));
            }
        } else {
            harness["models"] = json!([{"model":profile.model,"reasoning_levels":[]}]);
        }
    }
    for harness in &mut harnesses {
        let id = harness["id"].as_str().unwrap_or("").to_owned();
        let work = id == "codex"
            && manager
                .config
                .harnesses
                .contains_key(&crate::runtime::Kind::Codex);
        harness["work"] = json!(work);
        harness["chat_models"] = json!(
            profiles
                .iter()
                .filter(|p| p.harness == id)
                .map(|p| &p.model)
                .collect::<Vec<_>>()
        );
        let chat = profiles.iter().any(|p| p.harness == id);
        harness["modes"] = json!(match (chat, work) {
            (true, true) => "CHAT_AND_WORK",
            (true, false) => "CHAT_ONLY",
            (false, true) => "WORK_ONLY",
            (false, false) => "UNAVAILABLE",
        });
        harness["mode_note"] = json!(if work {
            "Work uses Codex workspace-write with untrusted approvals. Unsupported approval requests are denied. Windows account isolation is not provided."
        } else {
            "Managed Work unavailable: this harness's permission contract is not verified. Existing native sessions remain accessible."
        });
    }
    core["harnesses"] = json!(harnesses);
    core["council"] = json!("broadcast_comparison");
    core["chat_note"] =
        json!("Chat requires its own API configuration; consumer subscriptions are separate.");
    Json(core)
}

async fn configure(
    State(manager): State<Arc<Manager>>,
    Json(profile): Json<Profile>,
) -> Result<Json<Value>> {
    if profile.model.is_empty()
        || profile.model.len() > 256
        || profile.model.chars().any(char::is_control)
        || profile.api_key.len() > 4096
        || profile.api_key.chars().any(char::is_control)
    {
        return Err(invalid("invalid Chat profile"));
    }
    let url =
        reqwest::Url::parse(&profile.endpoint).map_err(|_| invalid("invalid Chat endpoint"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("invalid Chat endpoint"));
    }
    let valid = match (profile.provider.as_str(), profile.harness.as_str()) {
        ("openai", "codex") => {
            profile.endpoint == "https://api.openai.com/v1/chat/completions"
                && !profile.api_key.is_empty()
        }
        ("xai", "grok") => {
            profile.endpoint == "https://api.x.ai/v1/chat/completions"
                && !profile.api_key.is_empty()
        }
        ("local", "local" | "qwen" | "codex") => {
            (profile.harness != "codex"
                || std::env::var("CLOUDROOM_UNPROTECTED_TEST_MODE").as_deref() == Ok("1"))
                && url.scheme() == "http"
                && url.host_str().is_some_and(|h| {
                    h.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
                })
        }
        _ => false,
    };
    if !valid {
        return Err(invalid("unsupported Chat provider or endpoint"));
    }
    let mut profiles = manager.bee.profiles.lock().unwrap();
    let mut next = profiles.clone();
    next.retain(|p| p.harness != profile.harness || p.model != profile.model);
    next.push(profile);
    save(&manager.bee.root.join("profiles.json"), &next)?;
    *profiles = next;
    Ok(Json(json!({"configured":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    request_id: String,
    harness: String,
    model: String,
    reasoning: Option<String>,
    workspace: Option<String>,
    workspace_name: Option<String>,
    group: Option<String>,
}
async fn create(
    State(manager): State<Arc<Manager>>,
    Json(start): Json<Start>,
) -> Result<Json<Value>> {
    key(&start.request_id)?;
    if start.request_id.len() > 32 {
        return Err(invalid("Cloud Bee start IDs must be at most 32 characters"));
    }
    let id = format!("bee_{}", start.request_id);
    let input = json!({"harness":start.harness,"model":start.model,"reasoning":start.reasoning,"workspace":start.workspace,"workspace_name":start.workspace_name,"group":start.group});
    let _gate = manager.bee.transitions.lock().await;
    if let Ok(saved) = manager.bee.read(&id) {
        if saved.start != input {
            return Err(invalid("request_id already has different content"));
        }
        return Ok(Json(json!({"session_id":id})));
    }
    let chat = manager.bee.profile(&start.harness, &start.model).is_some();
    let work = start.harness == "codex"
        && manager
            .config
            .harnesses
            .contains_key(&crate::runtime::Kind::Codex);
    if !chat && !work {
        return Err(invalid("no managed mode is available for this model"));
    }
    if start.model.is_empty()
        || start.model.len() > 256
        || start.model.chars().any(char::is_control)
    {
        return Err(invalid("invalid model"));
    }
    let workspace = match start.workspace {
        Some(workspace) => manager
            .workspaces
            .resolve(&workspace, start.workspace_name.as_deref())
            .map_err(|_| invalid("invalid workspace"))?,
        None => crate::workspace::Workspace {
            id: "legacy".into(),
            path: manager.config.repository.canonicalize()?,
            parent: None,
        },
    };
    let conversation = Conversation {
        id: id.clone(),
        start: input,
        harness: start.harness,
        model: start.model,
        mode: "chat".into(),
        workspace,
        work: None,
        handoffs: vec![],
        switches: BTreeMap::new(),
        turns: BTreeMap::new(),
        order: vec![],
        closed: false,
    };
    save(&manager.bee.root.join(format!("{id}.json")), &conversation)?;
    manager
        .bee
        .conversations
        .lock()
        .unwrap()
        .insert(id.clone(), conversation);
    Ok(Json(json!({"session_id":id})))
}

async fn view(manager: &Manager, c: &Conversation) -> Result<Value> {
    let work_available = c.harness == "codex"
        && manager
            .codex_models
            .lock()
            .await
            .as_ref()
            .is_some_and(|(_, models)| models.iter().any(|m| m.model == c.model));
    let native = match &c.work {
        Some(id) => manager.session(id).await.ok(),
        None => None,
    };
    let mut result = if c.mode == "work" {
        native
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(io::Error::other)?
            .unwrap_or(json!({}))
    } else {
        json!({})
    };
    result["session_id"] = json!(c.id);
    result["conversation_id"] = json!(c.id);
    result["harness"] = json!(c.harness);
    result["model"] = json!(c.model);
    result["mode"] = json!(c.mode);
    result["group"] = c.start["group"].clone();
    result["workspace"] = json!(c.workspace);
    result["execution_session_id"] = json!(c.work);
    result["handoffs"] = json!(c.handoffs);
    result["chat_available"] = json!(manager.bee.profile(&c.harness, &c.model).is_some());
    result["work_available"] = json!(work_available);
    result["chat_usage"] = c
        .order
        .iter()
        .rev()
        .find_map(|id| {
            let t = &c.turns[id];
            (t.mode == "chat" && !t.usage.is_null()).then(|| t.usage.clone())
        })
        .unwrap_or(Value::Null);
    result["work_state"] = json!(native.as_ref().map(|s| s.state.as_str()));
    if c.mode == "chat" {
        let busy = c
            .turns
            .values()
            .any(|t| t.mode == "chat" && t.state == "accepted");
        result["state"] = json!(if c.closed {
            "closed"
        } else if busy {
            "running"
        } else {
            "idle"
        });
        result["receipts"] = json!(c.turns);
    }
    if c.closed {
        result["state"] = json!("closed");
    }
    Ok(result)
}
async fn list(State(manager): State<Arc<Manager>>) -> Result<Json<Value>> {
    let conversations: Vec<_> = manager
        .bee
        .conversations
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect();
    let mut sessions = vec![];
    for c in &conversations {
        sessions.push(view(&manager, c).await?);
    }
    Ok(Json(json!({"total":sessions.len(),"sessions":sessions})))
}
async fn status(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"session":view(&manager,&manager.bee.read(&id)?).await?}),
    ))
}

#[derive(Deserialize)]
struct After {
    #[serde(default)]
    after: u64,
}
async fn events(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Query(query): Query<After>,
) -> Result<Json<Value>> {
    let c = manager.bee.read(&id)?;
    if c.mode == "work" {
        let native = c.work.as_ref().ok_or(invalid("authorize Work first"))?;
        return Ok(Json(
            json!({"events":manager.records(native,query.after).await?}),
        ));
    }
    let mut records = vec![];
    for (index, id) in c.order.iter().enumerate() {
        let turn = &c.turns[id];
        if turn.mode != "chat" {
            continue;
        }
        records.push(json!({"sequence":index*2+1,"kind":"receipt","data":{"request_id":id,"command":"prompt","state":turn.state,"input":turn.input}}));
        if !turn.output.is_empty() {
            records.push(json!({"sequence":index*2+2,"kind":"item_completed","data":{"item_id":id,"text":turn.output}}));
        }
    }
    records.retain(|r| r["sequence"].as_u64().unwrap() > query.after);
    Ok(Json(json!({"events":records})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Switch {
    request_id: String,
    mode: String,
    #[serde(default)]
    authorize_work: bool,
    handoff: Value,
}
async fn mode(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(input): Json<Switch>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if !matches!(input.mode.as_str(), "chat" | "work") {
        return Err(invalid("invalid mode"));
    }
    let raw = serde_json::to_string(&input.handoff).map_err(io::Error::other)?;
    if raw.len() > 12000 || !input.handoff.is_object() {
        return Err(invalid("handoff must be an object within 12000 bytes"));
    }
    let handoff: Value = serde_json::from_str(&manager.bee.redact(&raw, &manager.config.token))
        .map_err(io::Error::other)?;
    for field in [
        "objective",
        "context",
        "requirements",
        "constraints",
        "acceptance",
        "artifacts",
    ] {
        if !handoff[field].is_string() {
            return Err(invalid(
                "handoff requires objective, context, requirements, constraints, acceptance and artifacts text",
            ));
        }
    }
    let payload =
        json!({"mode":input.mode,"authorize_work":input.authorize_work,"handoff":handoff});
    let _gate = manager.bee.transitions.lock().await;
    let c = manager.bee.read(&id)?;
    if c.closed {
        return Err(invalid("conversation is closed"));
    }
    if let Some(prior) = c.switches.get(&input.request_id) {
        if prior != &payload {
            return Err(invalid("request_id already has different content"));
        }
        return Ok(Json(json!({"session_id":id,"mode":c.mode})));
    }
    if input.mode == "chat" && manager.bee.profile(&c.harness, &c.model).is_none() {
        return Err(invalid(
            "Chat unavailable: configure a tool-free API profile for this model",
        ));
    }
    let mut work = c.work.clone();
    if input.mode == "work" {
        if !input.authorize_work || c.harness != "codex" {
            return Err(invalid(
                "explicit authorization and supported managed Work harness required",
            ));
        }
        if handoff["objective"].as_str().unwrap().trim().is_empty() {
            return Err(invalid("review a Work objective first"));
        }
        if !manager
            .codex_models()
            .await?
            .iter()
            .any(|m| m.model == c.model)
        {
            return Err(invalid("managed Work unavailable for this exact model"));
        }
        let request = format!("bw_{}", c.id.trim_start_matches("bee_"));
        let native = format!("cr_{request}");
        if work.is_none() {
            manager.bee.update(&id, |next| {
                next.work = Some(native.clone());
                Ok(())
            })?;
        }
        let workspace = if c.workspace.id == "legacy" {
            None
        } else {
            Some(c.workspace.id.clone())
        };
        manager
            .start(
                request,
                Some(crate::runtime::Kind::Codex),
                Some(c.model.clone()),
                c.start["reasoning"].as_str().map(str::to_owned),
                (workspace, None),
                (None, Some(false), Some(false), None, true),
            )
            .await?;
        work = Some(native);
    }
    manager.bee.update(&id, |next| {
        next.mode = input.mode.clone();
        next.work = work;
        let mut handoff = handoff.clone();
        handoff["destination"] = json!(input.mode);
        handoff["workspace"] = json!(next.workspace.path);
        handoff["execution_session_id"] = json!(next.work);
        handoff["request_id"] = json!(input.request_id);
        next.handoffs.push(handoff);
        next.switches.insert(input.request_id, payload);
        Ok(())
    })?;
    Ok(Json(json!({"session_id":id,"mode":input.mode})))
}

async fn command(
    State(manager): State<Arc<Manager>>,
    Path((id, command)): Path<(String, String)>,
    Json(mut input): Json<Value>,
) -> Result<Json<Value>> {
    let request = input["request_id"]
        .as_str()
        .ok_or(invalid("request_id required"))?
        .to_owned();
    key(&request)?;
    input
        .as_object_mut()
        .ok_or(invalid("object required"))?
        .remove("request_id");
    let c = manager.bee.read(&id)?;
    if command == "prompts" {
        if input
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !matches!(k.as_str(), "text" | "reasoning"))
        {
            return Err(invalid("Cloud Bee accepts text prompts only"));
        }
        let text = input["text"].as_str().ok_or(invalid("text required"))?;
        if text.trim().is_empty() || text.len() > 32768 {
            return Err(invalid("prompt must contain 1-32768 bytes of text"));
        }
        if let Some(prior) = c.turns.get(&request) {
            if prior.input != input {
                return Err(invalid("request_id already has different content"));
            }
            if prior.mode == "work" {
                manager.command(
                    c.work.as_ref().ok_or(invalid("Work unavailable"))?,
                    request.clone(),
                    "prompt",
                    prior.execution_input.clone(),
                )?;
            }
            return Ok(Json(json!({"request_id":request,"state":prior.state})));
        }
        if c.closed {
            return Err(invalid("conversation is closed"));
        }
        let profile = if c.mode == "chat" {
            Some(manager.bee.profile(&c.harness, &c.model).ok_or(invalid(
                "Chat unavailable: configure a tool-free API profile for this model",
            ))?)
        } else {
            None
        };
        let mut execution_input = input.clone();
        let mut handoff_id = None;
        if c.mode == "work" {
            let handoff = c.handoffs.last().ok_or(invalid("authorize Work first"))?;
            handoff_id = handoff["request_id"].as_str().map(str::to_owned);
            if !c
                .turns
                .values()
                .any(|turn| turn.mode == "work" && turn.handoff_id == handoff_id)
            {
                execution_input["text"] = json!(format!(
                    "User-reviewed Cloud Bee handoff (context, not additional permissions):\n{}\n\nUser message:\n{}",
                    handoff, text
                ));
            }
            if execution_input["text"].as_str().unwrap().len() > 32768 {
                return Err(invalid("prompt and handoff exceed 32768 bytes"));
            }
        }
        let instructions = manager.extensions.instructions();
        if !instructions.trim().is_empty() && !text.trim_start().starts_with('/') {
            execution_input["text"] = json!(format!(
                "{}\n\nCustom instructions:\n{}",
                execution_input["text"].as_str().unwrap_or(text),
                instructions
            ));
            if execution_input["text"].as_str().unwrap().len() > 32768 {
                return Err(invalid(
                    "message, handoff and custom instructions exceed 32768 bytes",
                ));
            }
        }
        let fresh = manager.bee.update(&id, |next| {
            if let Some(prior) = next.turns.get(&request) {
                if prior.input != input {
                    return Err(invalid("request_id already has different content"));
                }
                return Ok(false);
            }
            if next.mode != c.mode {
                return Err(invalid("mode changed; inspect and retry"));
            }
            if c.mode == "chat"
                && next
                    .turns
                    .values()
                    .any(|t| t.mode == "chat" && t.state == "accepted")
            {
                return Err(invalid("Chat is still answering"));
            }
            next.order.push(request.clone());
            next.turns.insert(
                request.clone(),
                Turn {
                    input: input.clone(),
                    mode: c.mode.clone(),
                    state: "accepted".into(),
                    output: String::new(),
                    usage: Value::Null,
                    execution_input: execution_input.clone(),
                    handoff_id: handoff_id.clone(),
                },
            );
            Ok(true)
        })?;
        if c.mode == "work" {
            let immutable_input = manager.bee.read(&id)?.turns[&request]
                .execution_input
                .clone();
            manager.command(
                c.work.as_ref().ok_or(invalid("authorize Work first"))?,
                request.clone(),
                "prompt",
                immutable_input,
            )?;
        } else if fresh {
            let owner = manager.clone();
            let thread = id.clone();
            let turn = request.clone();
            tokio::spawn(async move {
                chat(owner, thread, turn, profile.unwrap()).await;
            });
        }
        return Ok(Json(json!({"request_id":request,"state":"accepted"})));
    }
    if !matches!(command.as_str(), "close" | "stop" | "resume" | "interrupt") {
        return Err(invalid("unsupported Cloud Bee command"));
    }
    // Work cancellation remains available while viewing Chat; it never creates execution.
    if let Some(native) = &c.work {
        if command == "interrupt"
            && input["target_request_id"]
                .as_str()
                .is_none_or(str::is_empty)
        {
            input["target_request_id"] = json!(manager.session(native).await?.current_request);
        }
        manager.command(native, request, &command, input)?;
    } else if command != "close" {
        return Err(invalid("no active Work session"));
    }
    if command == "close" {
        manager.bee.update(&id, |next| {
            next.closed = true;
            Ok(())
        })?;
    }
    Ok(Json(json!({"accepted":true})))
}

async fn chat(manager: Arc<Manager>, id: String, request: String, profile: Profile) {
    let result = async {
        let c = manager.bee.read(&id)?;
        let mut messages = vec![];
        if let Some(handoff) = c.handoffs.last() {
            messages
                .push(json!({"role":"user","content":format!("User-reviewed handoff: {handoff}")}));
        }
        // Bound the provider context, retaining the complete transcript on disk.
        let mut prior: Vec<_> = c
            .order
            .iter()
            .filter(|r| c.turns[*r].mode == "chat")
            .rev()
            .take(12)
            .collect();
        prior.reverse();
        for turn in prior {
            messages.push(json!({"role":"user","content":c.turns[turn].execution_input["text"]}));
            if !c.turns[turn].output.is_empty() {
                messages.push(json!({"role":"assistant","content":c.turns[turn].output}));
            }
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|_| invalid("Chat transport unavailable"))?;
        let mut call = client.post(&profile.endpoint).json(
            &json!({"model":profile.model,"messages":messages,"tool_choice":"none","stream":false}),
        );
        if !profile.api_key.is_empty() {
            call = call.bearer_auth(&profile.api_key);
        }
        let mut response = call.send().await.map_err(|_| {
            invalid("Chat request outcome uncertain; inspect history before sending again")
        })?;
        if !response.status().is_success() {
            return Err(invalid(
                "Chat API rejected the request; check model and separate API credentials",
            ));
        }
        let mut bytes = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| invalid("Chat response interrupted"))?
        {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(invalid("Chat response too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid Chat response"))?;
        let message = &body["choices"][0]["message"];
        if message["tool_calls"]
            .as_array()
            .is_some_and(|calls| !calls.is_empty())
            || !message["function_call"].is_null()
        {
            return Err(invalid(
                "Chat provider attempted a tool call; nothing was executed",
            ));
        }
        let output = message["content"]
            .as_str()
            .ok_or(invalid("Chat returned no text"))?;
        let mut usage = json!({});
        for field in ["prompt_tokens", "completion_tokens", "total_tokens"] {
            if let Some(value) = body["usage"][field].as_u64() {
                usage[field] = json!(value);
            }
        }
        Ok((manager.bee.redact(output, &manager.config.token), usage))
    }
    .await;
    let _ = manager.bee.update(&id,|c| {
        let turn=c.turns.get_mut(&request).ok_or(Error::NotFound)?;
        match result { Ok((output,usage)) => { turn.output=output; turn.usage=usage; turn.state="completed".into(); }, Err(_) => { turn.output="Chat failed or its outcome is uncertain. No tools were executed. Check the API profile and saved history before sending a new request.".into(); turn.state="failed".into(); } }
        Ok(())
    });
}
