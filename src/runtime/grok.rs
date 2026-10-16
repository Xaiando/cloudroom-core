//! Grok Build over ACP. `grok agent stdio` keeps its own sessions under `~/.grok/sessions`.
use super::{Handle, Resume, command as child_command, cursor};
use crate::config::{Config, HarnessConfig};
use serde_json::json;
use std::io;
use tokio::process::Command;

pub(super) const GROK: cursor::Flavor = cursor::Flavor {
    harness: "grok",
    name: "Grok",
    native_path: |home, _| home.join("sessions"),
    valid_id: cursor::valid_id,
    capture: false,
    notices: &[],
    // Setup notifications are not conversation history.
    replay: false,
    explain_stops: true,
};

/// Efforts the signed-in Grok catalog offers. A model may advertise a subset.
pub const REASONING_LEVELS: &[&str] = &["low", "medium", "high", "xhigh"];

/// Known models must use one of their advertised efforts. An unlisted model is left for Grok to reject.
pub fn accepts(model: &str, reasoning: &str) -> bool {
    match models().into_iter().find(|entry| entry.model == model) {
        Some(entry) => entry
            .reasoning_levels
            .iter()
            .any(|level| level == reasoning),
        None => REASONING_LEVELS.contains(&reasoning),
    }
}

pub fn models() -> Vec<super::Model> {
    let full = ["low", "medium", "high", "xhigh"];
    let standard = ["low", "medium", "high"];
    [
        ("grok-4.7", full.as_slice()),
        ("grok-4.7-build-fast", full.as_slice()),
        ("grok-4.6", full.as_slice()),
        ("grok-4.5", standard.as_slice()),
    ]
    .into_iter()
    .map(|(model, reasoning_levels)| super::Model {
        model: model.into(),
        reasoning_levels: reasoning_levels
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
    if profile.home.canonicalize()? != config.account_home.join(".grok").canonicalize()? {
        return Err(invalid(
            "Grok home must belong to the configured agent account".into(),
        ));
    }
    let mut command = child_command(&profile.binary, config);
    command.args(["--no-auto-update", "agent", "--always-approve"]);
    if profile.model != "default" {
        command.arg("--model").arg(&profile.model);
    }
    command.arg("stdio");
    // The Windows runtime shares the user account. A key in the environment is this user's Grok login.
    if let Ok(key) = std::env::var("XAI_API_KEY")
        && !key.is_empty()
        && !key.contains('\0')
    {
        command.env("XAI_API_KEY", key);
    }
    Ok(command)
}

pub(super) async fn start(handle: &Handle) -> io::Result<String> {
    let signed_in = handle.profile.home.join("auth.json").is_file()
        || std::env::var("XAI_API_KEY").is_ok_and(|key| !key.is_empty());
    if !signed_in {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Grok is not signed in. Run `grok login` on this Windows account, or set XAI_API_KEY.",
        ));
    }
    cursor::initialize(handle, "Grok").await?;
    handle
        .call(
            "authenticate",
            json!({"methodId":"cached_token","_meta":{"headless":true}}),
        )
        .await
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Grok rejected the saved login: {error}. Run `grok login` and try again."),
            )
        })?;
    let mut params = json!({"cwd":handle.repository,"mcpServers":[],"_meta":{"yoloMode":true}});
    let method = if let Some(saved) = &handle.resume {
        params["sessionId"] = json!(saved.id);
        "session/load"
    } else {
        "session/new"
    };
    let (id, session) = cursor::attach(handle, "Grok", method, params).await?;
    let requested = handle.profile.model.as_str();
    if requested != "default" && session["models"]["currentModelId"] != requested {
        set(handle, &id, "model", requested)
            .await
            .map_err(|error| {
                invalid(format!(
                    "Grok model {requested} is unavailable for this login: {error}"
                ))
            })?;
    }
    if let Some(reasoning) = handle.reasoning.as_deref() {
        set(handle, &id, "reasoning_effort", reasoning).await?;
    }
    Ok(id)
}

async fn set(handle: &Handle, id: &str, option: &str, value: &str) -> io::Result<()> {
    let result = handle
        .call(
            "session/set_config_option",
            json!({"sessionId":id,"configId":option,"value":value}),
        )
        .await?;
    if cursor::option_value(&result, option) != Some(value) {
        return Err(io::Error::other(format!(
            "Grok did not confirm {option} {value}"
        )));
    }
    Ok(())
}

pub(super) fn validate(
    profile: &HarnessConfig,
    saved: &Resume,
    _identity: super::files::Identity,
) -> io::Result<()> {
    if !cursor::valid_id(&saved.id) || saved.path != profile.home.join("sessions") {
        return Err(io::Error::other(
            "Grok native path does not match its session",
        ));
    }
    if !profile.home.join("sessions").is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Grok session store is missing",
        ));
    }
    Ok(())
}
