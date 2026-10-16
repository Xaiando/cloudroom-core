//! Council text via existing CLI accounts. Never resume a native Work session.
use super::*;
use crate::{config::HarnessConfig, runtime::Kind};
use sha2::{Digest, Sha256};
use std::{
    process::Stdio,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const LIMIT: u64 = 2 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);
static VERIFIED: tokio::sync::Mutex<Vec<String>> = tokio::sync::Mutex::const_new(Vec::new());

fn harness(manager: &Manager, kind: Kind) -> Option<&HarnessConfig> {
    manager
        .config
        .harnesses
        .get(&kind)
        .filter(|p| p.binary.is_file())
}
fn codex_catalog(home: &FsPath) -> Option<Vec<Value>> {
    let bytes = fs::read(home.join("models_cache.json")).ok()?;
    if bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice::<Value>(&bytes).ok()?["models"]
        .as_array()
        .cloned()
}
fn grok_clean(home: &FsPath) -> bool {
    // Grok has no global customization bypass. Refuse custom hooks/MCP/plugins
    // rather than executing them before the headless tool allowlist takes effect.
    if [
        "hooks",
        "hooks.json",
        "plugins",
        "mcp.json",
        "managed_config.toml",
        "requirements.toml",
    ]
    .iter()
    .any(|name| home.join(name).exists())
    {
        return false;
    }
    match fs::read_to_string(home.join("config.toml")) {
        Ok(text) => grok_settings_safe(&text),
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}
fn grok_settings_safe(text: &str) -> bool {
    // Fail closed on unknown TOML syntax/sections. Match keys rather than values:
    // the stock marketplace URL contains "plugins" but is not an installed plugin.
    let mut section = "";
    for line in text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        if line.starts_with('[') {
            if ![
                "[cli]",
                "[marketplace]",
                "[[marketplace.sources]]",
                "[ui]",
                "[privacy]",
                "[models]",
            ]
            .contains(&line)
            {
                return false;
            }
            section = line;
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            return false;
        };
        let allowed: &[&str] = match section {
            "[cli]" => &["installer"],
            "[marketplace]" => &[
                "default_skills_installs_purged",
                "official_marketplace_auto_installed",
            ],
            "[[marketplace.sources]]" => &["name", "git"],
            "[ui]" => &[
                "max_thoughts_width",
                "fork_secondary_model",
                "yolo",
                "compact_mode",
                "permission_mode",
            ],
            "[privacy]" => &["privacy_banner_acked"],
            "[models]" => &["default", "default_reasoning_effort"],
            _ => &[],
        };
        if !allowed.contains(&key.trim()) {
            return false;
        }
    }
    true
}
pub(super) fn models(manager: &Manager) -> Vec<Profile> {
    let mut all = Vec::new();
    if let Some(p) = harness(manager, Kind::Codex)
        && p.home.join("auth.json").is_file()
        && let Some(models) = codex_catalog(&p.home)
    {
        for model in models {
            if model["visibility"] == "hide" {
                continue;
            }
            if let Some(id) = model["slug"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 200)
            {
                all.push(Profile {
                    harness: "codex".into(),
                    model: id.into(),
                    provider: "codex_cli".into(),
                    endpoint: String::new(),
                    api_key: String::new(),
                });
            }
        }
    }
    if let Some(p) = harness(manager, Kind::Grok)
        && p.home.join("auth.json").is_file()
        && p.home.canonicalize().ok()
            == manager
                .config
                .account_home
                .join(".grok")
                .canonicalize()
                .ok()
        && grok_clean(&p.home)
    {
        for model in crate::runtime::grok_models() {
            all.push(Profile {
                harness: "grok".into(),
                model: model.model,
                provider: "grok_cli".into(),
                endpoint: String::new(),
                api_key: String::new(),
            });
        }
    }
    all.extend(super::claude::models(manager));
    all
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(manager: &Manager) -> Result<Self> {
        let parent = manager.config.state_dir.join("council-cli");
        fs::create_dir_all(&parent)?;
        let path = parent.join(format!(
            "{}-{}-{}",
            std::process::id(),
            now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::create_dir(path.join("workspace"))?;
        fs::create_dir(path.join("probe-home"))?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn command(manager: &Manager, binary: &FsPath, root: &FsPath) -> Command {
    let mut cmd = Command::new(binary);
    cmd.env_clear();
    // Only OS startup variables; neither Core credentials nor saved API keys.
    for name in [
        "SystemRoot",
        "WINDIR",
        "SystemDrive",
        "COMSPEC",
        "PATHEXT",
        "PATH",
        "TEMP",
        "TMP",
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        if let Some(value) = std::env::var_os(name) {
            cmd.env(name, value);
        }
    }
    cmd.env("USERPROFILE", &manager.config.account_home)
        .env("HOME", &manager.config.account_home)
        .current_dir(root.join("workspace"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    cmd
}
pub(super) async fn run(mut cmd: Command, input: &[u8]) -> Result<Vec<u8>> {
    let (mut child, job) = crate::runtime::linux::spawn(&mut cmd)
        .map_err(|_| invalid("Council CLI could not start"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or(invalid("Council CLI input unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(invalid("Council CLI output unavailable"))?;
    let result = tokio::time::timeout(Duration::from_secs(120), async {
        let write = async { stdin.write_all(input).await?; stdin.shutdown().await?; drop(stdin); Ok::<_, io::Error>(()) };
        let read = async { let mut bytes = Vec::new(); stdout.take(LIMIT + 1).read_to_end(&mut bytes).await?; Ok::<_,io::Error>(bytes) };
        let (_, bytes) = tokio::try_join!(write, read).map_err(|_| invalid("Council CLI pipe failed"))?;
        if bytes.len() as u64 > LIMIT { return Err(invalid("Council CLI output limit exceeded")); }
        if !child.wait().await?.success() { return Err(invalid("Council CLI request failed; check its login and model access. No API fallback was called.")); }
        Ok(bytes)
    }).await.map_err(|_| invalid("Council CLI timed out; its process tree was stopped"));
    // The Job also kills descendants on cancellation/future drop.
    let stopped = job.stop().await;
    stopped.map_err(|_| invalid("Council CLI process exit unconfirmed"))?;
    result?
}

fn sanitized_model(mut model: Value) -> Value {
    for (key, value) in [
        ("shell_type", json!("disabled")),
        ("apply_patch_tool_type", Value::Null),
        ("experimental_supported_tools", json!([])),
        ("supports_search_tool", json!(false)),
        ("include_skills_usage_instructions", json!(false)),
        ("include_plugin_usage_instructions", json!(false)),
        ("include_apps_usage_instructions", json!(false)),
        ("tool_mode", json!("direct")),
    ] {
        model[key] = value;
    }
    model
}
fn codex_command(
    manager: &Manager,
    p: &HarnessConfig,
    model: &str,
    root: &FsPath,
    home: &FsPath,
) -> Command {
    let mut cmd = command(manager, &p.binary, root);
    cmd.env("CODEX_HOME", home).args([
        "exec",
        "--ignore-user-config",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--json",
        "--model",
        model,
    ]);
    for feature in [
        "shell_tool",
        "multi_agent",
        "multi_agent_v2",
        "view_image",
        "browser_use",
        "plugins",
        "remote_plugin",
        "enable_mcp_apps",
        "hooks",
        "skill_mcp_dependency_install",
        "code_mode",
        "code_mode_only",
        "deferred_executor",
        "sleep_tool",
        "tool_suggest",
        "current_time_reminder",
    ] {
        cmd.args(["-c", &format!("features.{feature}=false")]);
    }
    for setting in [
        "web_search=\"disabled\"",
        "mcp_servers={}",
        "apps._default.enabled=false",
        "project_doc_max_bytes=0",
    ] {
        cmd.args(["-c", setting]);
    }
    cmd.args([
        "-c",
        &format!("model_catalog_json={}", json!(root.join("models.json"))),
    ]);
    cmd.arg("-");
    cmd
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn verify_codex(
    manager: &Manager,
    p: &HarnessConfig,
    model: &str,
    root: &FsPath,
    catalog: &[u8],
) -> Result<()> {
    let mut hash = Sha256::new();
    hash.update(fs::read(&p.binary)?);
    hash.update(catalog);
    let key = format!("{:x}", hash.finalize());
    let mut cache = VERIFIED.lock().await;
    if cache.contains(&key) {
        return Ok(());
    }
    let checked = Arc::new(AtomicBool::new(false));
    let received = checked.clone();
    let expected = model.to_owned();
    let app = Router::new().route("/v1/responses", post(move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
        let received = received.clone(); let expected = expected.clone();
        async move {
            let safe = !headers.contains_key(axum::http::header::AUTHORIZATION) && body.get("tools").is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)) && body["model"] == expected;
            received.store(safe, Ordering::SeqCst);
            if !safe { return (axum::http::StatusCode::BAD_REQUEST, [("content-type", "text/plain")], "tool policy rejected".to_owned()); }
            let item = json!({"id":"fixture_message","type":"message","role":"assistant","content":[{"type":"output_text","text":"CLOUD_BEE_POLICY_OK"}]});
            let events = [json!({"type":"response.created","response":{"id":"fixture"}}), json!({"type":"response.output_item.done","item":item}), json!({"type":"response.completed","response":{"id":"fixture","output":[],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}})];
            let text = events.iter().map(|e| format!("event: {}\ndata: {e}\n\n",e["type"].as_str().unwrap())).collect::<String>();
            (axum::http::StatusCode::OK, [("content-type", "text/event-stream")], text)
        }
    })).layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let _server = Server(tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    }));
    let mut cmd = codex_command(manager, p, model, root, &root.join("probe-home"));
    for setting in [
        "model_provider=\"cloud_bee_policy\"".to_owned(),
        "model_providers.cloud_bee_policy.name=\"Cloud Bee local policy check\"".into(),
        format!("model_providers.cloud_bee_policy.base_url=\"http://{address}/v1\""),
        "model_providers.cloud_bee_policy.wire_api=\"responses\"".into(),
        "model_providers.cloud_bee_policy.requires_openai_auth=false".into(),
    ] {
        cmd.args(["-c", &setting]);
    }
    let bytes = tokio::time::timeout(
        Duration::from_secs(20),
        run(cmd, b"Reply CLOUD_BEE_POLICY_OK"),
    )
    .await
    .map_err(|_| invalid("Installed Codex CLI could not verify Council's tool-free policy"))??;
    let text = codex_text(&bytes)?;
    if !checked.load(Ordering::SeqCst) || text != "CLOUD_BEE_POLICY_OK" {
        return Err(invalid(
            "Installed Codex CLI failed Council's tool-free policy check",
        ));
    }
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.push(key);
    Ok(())
}
fn codex_text(bytes: &[u8]) -> Result<String> {
    let mut answer = String::new();
    let mut completed = false;
    for line in bytes.split(|b| *b == b'\n').filter(|b| !b.is_empty()) {
        let event: Value =
            serde_json::from_slice(line).map_err(|_| invalid("Invalid Codex CLI output"))?;
        match event["type"].as_str() {
            Some("turn.completed") => completed = true,
            Some("thread.started" | "turn.started") => {}
            Some("item.completed" | "item.started" | "item.updated") => {
                match event["item"]["type"].as_str() {
                    Some("agent_message") if event["type"] == "item.completed" => {
                        if !answer.is_empty() {
                            answer.push('\n');
                        }
                        answer.push_str(
                            event["item"]["text"]
                                .as_str()
                                .ok_or(invalid("Missing CLI text"))?,
                        );
                    }
                    Some("agent_message" | "reasoning") => {}
                    _ => {
                        return Err(invalid(
                            "Council CLI returned an unsupported tool or error event",
                        ));
                    }
                }
            }
            _ => return Err(invalid("Council CLI returned an unsupported event")),
        }
    }
    if !completed || answer.trim().is_empty() {
        return Err(invalid("Council CLI did not complete a text response"));
    }
    Ok(answer)
}
pub(super) async fn complete(
    manager: &Manager,
    profile: &Profile,
    messages: &[Value],
) -> Result<(String, Value)> {
    let scratch = Scratch::new(manager)?;
    let root = &scratch.0;
    let prompt = serde_json::to_vec(&json!({"instruction":"Respond to the final user message in this conversation. Text only; no tool use. JSON requested by the Council protocol is ordinary response text.","messages":messages})).map_err(io::Error::other)?;
    match profile.provider.as_str() {
        "claude_cli" => super::claude::complete(manager, profile, root, &prompt).await,
        "codex_cli" => {
            let p = harness(manager, Kind::Codex).ok_or(invalid("Codex CLI is unavailable"))?;
            let model = codex_catalog(&p.home)
                .and_then(|m| m.into_iter().find(|m| m["slug"] == profile.model))
                .ok_or(invalid("Codex model catalog changed; refresh Council"))?;
            let catalog = serde_json::to_vec(&json!({"models":[sanitized_model(model)]}))
                .map_err(io::Error::other)?;
            fs::write(root.join("models.json"), &catalog)?;
            verify_codex(manager, p, &profile.model, root, &catalog).await?;
            let bytes = run(
                codex_command(manager, p, &profile.model, root, &p.home),
                &prompt,
            )
            .await?;
            Ok((codex_text(&bytes)?, Value::Null))
        }
        "grok_cli" => {
            let p = harness(manager, Kind::Grok).ok_or(invalid("Grok CLI is unavailable"))?;
            if !grok_clean(&p.home) {
                return Err(invalid(
                    "Grok customizations must be disabled before tool-free Council use",
                ));
            }
            for parent in root.ancestors() {
                let candidate = parent.join(".grok");
                if candidate.exists() && candidate.canonicalize().ok() != p.home.canonicalize().ok()
                {
                    return Err(invalid(
                        "Council scratch directory has ancestor Grok customizations",
                    ));
                }
            }
            // Grok interprets .json files as typed ACP input, not ordinary text.
            let path = root.join("prompt.txt");
            fs::write(&path, prompt)?;
            let mut cmd = command(manager, &p.binary, root);
            cmd.args(["--no-auto-update", "--prompt-file"])
                .arg(path)
                .args([
                    "--model",
                    &profile.model,
                    "--tools",
                    "",
                    "--no-subagents",
                    "--disable-web-search",
                    "--permission-mode",
                    "dontAsk",
                    "--max-turns",
                    "1",
                    "--output-format",
                    "json",
                    "--verbatim",
                ]);
            let bytes = run(cmd, &[]).await?;
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid Grok CLI output"))?;
            if value["stopReason"] != "end_turn" {
                return Err(invalid("Grok CLI did not complete a text response"));
            }
            let text = value["text"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or(invalid("Grok CLI returned no text"))?;
            Ok((text.into(), value["usage"].clone()))
        }
        _ => Err(invalid("Unsupported Council CLI")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grok_defaults_are_not_executable_customizations() {
        assert!(grok_settings_safe(
            "[ui]\nyolo = true\n[[marketplace.sources]]\nname = 'official'\ngit = 'https://example.com/plugins'\n"
        ));
        assert!(!grok_settings_safe("[hooks]\ncommand = 'execute'"));
        assert!(!grok_settings_safe("[ui]\n\"hooks\" = {}"));
        assert!(!grok_settings_safe("[models]\ncommand = 'execute'"));
    }
    #[test]
    fn rejects_tools_and_incomplete_cli_output() {
        assert!(
            codex_text(
                b"{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\"}}\n"
            )
            .is_err()
        );
        assert!(codex_text(b"{\"type\":\"turn.completed\"}\n").is_err());
        assert_eq!(codex_text(b"{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hello\"}}\n{\"type\":\"turn.completed\"}\n").unwrap(), "hello");
    }
}
