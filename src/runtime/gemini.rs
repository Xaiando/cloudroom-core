//! Gemini CLI over ACP. `gemini --acp` keeps its own sessions under `~/.gemini/tmp`.
use super::{Handle, Resume, command as child_command, cursor};
use crate::config::{Config, HarnessConfig};
use serde_json::{Value, json};
use std::{io, path::Path};
use tokio::process::Command;

pub(super) const GEMINI: cursor::Flavor = cursor::Flavor {
    harness: "gemini",
    name: "Gemini",
    native_path: |home, _| home.join("tmp"),
    valid_id: cursor::valid_id,
    capture: false,
    notices: &[],
    replay: false,
    explain_stops: true,
};

pub const REASONING_LEVELS: &[&str] = &["low", "medium", "high"];

pub fn models() -> Vec<super::Model> {
    [
        "gemini-2.5-pro",
        "gemini-2.5-flash",
        "gemini-2.5-flash-lite",
        "gemini-3-pro-preview",
        "gemini-3-flash-preview",
    ]
    .into_iter()
    .map(|model| super::Model {
        model: model.into(),
        reasoning_levels: REASONING_LEVELS
            .iter()
            .map(|level| (*level).to_owned())
            .collect(),
    })
    .collect()
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub(super) fn command(config: &Config, profile: &HarnessConfig) -> io::Result<Command> {
    let home = config.account_home.join(".gemini");
    if profile.home.canonicalize()? != home.canonicalize()? {
        return Err(invalid(
            "Gemini home must belong to the configured agent account".into(),
        ));
    }
    let mut command = hosted(config, &profile.binary)?;
    command.arg("--acp").arg("--yolo");
    if profile.model != "default" {
        command.arg("--model").arg(&profile.model);
    }
    for name in [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_GENAI_USE_VERTEXAI",
        "GOOGLE_CLOUD_PROJECT",
        "GOOGLE_CLOUD_LOCATION",
    ] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
            && !value.contains('\0')
        {
            command.env(name, value);
        }
    }
    Ok(command)
}

/// npm publishes Gemini CLI as a `.cmd` shim. ACP needs the Node entry so stdin stays a JSON-RPC pipe.
fn node_binary() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            for name in ["node.exe", "node"] {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    std::path::PathBuf::from("node")
}

fn hosted(config: &Config, binary: &Path) -> io::Result<Command> {
    if binary.extension().and_then(|ext| ext.to_str()) == Some("js") {
        let mut command = child_command(&node_binary(), config);
        command.arg(binary);
        return Ok(command);
    }
    #[cfg(windows)]
    if matches!(
        binary.extension().and_then(|ext| ext.to_str()),
        Some("cmd" | "bat")
    ) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let mut command = child_command(&Path::new(&root).join("System32").join("cmd.exe"), config);
        command.arg("/d").arg("/c").arg(binary);
        return Ok(command);
    }
    Ok(child_command(binary, config))
}

pub(super) async fn start(handle: &Handle) -> io::Result<String> {
    let (id, mut session) = cursor::open(handle, "Gemini").await.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{error}. Sign in with `gemini`, or set GEMINI_API_KEY or GOOGLE_API_KEY for this account."
            ),
        )
    })?;
    let requested = handle.profile.model.as_str();
    if requested != "default" && cursor::option_value(&session, "model") != Some(requested) {
        let offered = choices(&session, "model");
        if offered.contains(&requested) {
            session = set(handle, &id, "model", requested).await?;
        } else if !offered.is_empty() {
            return Err(invalid(format!(
                "Gemini model {requested} is unavailable. Offered: {}",
                offered.join(", ")
            )));
        }
    }
    let offered = choices(&session, "reasoning_effort");
    let effort_option = if offered.is_empty() {
        "effort"
    } else {
        "reasoning_effort"
    };
    let offered = if offered.is_empty() {
        choices(&session, "effort")
    } else {
        offered
    };
    if let Some(reasoning) = handle.reasoning.as_deref()
        && offered.contains(&reasoning)
        && cursor::option_value(&session, effort_option) != Some(reasoning)
    {
        set(handle, &id, effort_option, reasoning).await?;
    }
    Ok(id)
}

fn choices<'a>(session: &'a Value, option: &str) -> Vec<&'a str> {
    session["configOptions"]
        .as_array()
        .and_then(|options| options.iter().find(|entry| entry["id"] == option))
        .and_then(|option| option["options"].as_array())
        .map(|options| {
            options
                .iter()
                .filter_map(|entry| entry["value"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

async fn set(handle: &Handle, id: &str, option: &str, value: &str) -> io::Result<Value> {
    let result = handle
        .call(
            "session/set_config_option",
            json!({"sessionId":id,"configId":option,"value":value}),
        )
        .await?;
    if cursor::option_value(&result, option) != Some(value) {
        return Err(io::Error::other(format!(
            "Gemini did not confirm {option} {value}"
        )));
    }
    Ok(result)
}

pub(super) fn validate(
    profile: &HarnessConfig,
    saved: &Resume,
    _identity: super::files::Identity,
) -> io::Result<()> {
    if !cursor::valid_id(&saved.id) || saved.path != profile.home.join("tmp") {
        return Err(io::Error::other(
            "Gemini native path does not match its session",
        ));
    }
    if !profile.home.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Gemini home is missing",
        ));
    }
    Ok(())
}
