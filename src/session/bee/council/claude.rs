//! Claude subscription login stays with Claude Code; Council has no execution tools.
use super::*;

pub(super) fn binary(manager: &Manager) -> Option<PathBuf> {
    let home = &manager.config.account_home;
    [
        home.join(".local/bin/claude.exe"),
        home.join("AppData/Roaming/npm/claude.cmd"),
        home.join(".local/bin/claude.cmd"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
pub(super) fn models(manager: &Manager) -> Vec<Profile> {
    if binary(manager).is_none() {
        return Vec::new();
    }
    ["sonnet", "opus", "haiku"]
        .into_iter()
        .map(|model| Profile {
            harness: "claude".into(),
            model: model.into(),
            provider: "claude_cli".into(),
            endpoint: String::new(),
            api_key: String::new(),
        })
        .collect()
}
fn managed_policy() -> bool {
    use windows_sys::Win32::{Foundation::ERROR_FILE_NOT_FOUND, System::Registry::*};
    let key = "SOFTWARE\\Policies\\ClaudeCode\0"
        .encode_utf16()
        .collect::<Vec<_>>();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut handle = std::ptr::null_mut();
        let result = unsafe { RegOpenKeyExW(hive, key.as_ptr(), 0, KEY_READ, &mut handle) };
        if !handle.is_null() {
            unsafe {
                RegCloseKey(handle);
            }
        }
        if result != ERROR_FILE_NOT_FOUND {
            return true;
        }
    }
    std::env::var_os("ProgramData").is_some_and(|root| {
        let root = PathBuf::from(root).join("ClaudeCode");
        [
            "managed-settings.json",
            "managed-settings.d",
            "managed-mcp.json",
        ]
        .iter()
        .any(|p| root.join(p).exists())
    })
}
pub(super) async fn complete(
    manager: &Manager,
    profile: &Profile,
    root: &FsPath,
    prompt: &[u8],
) -> Result<(String, Value)> {
    let binary = binary(manager).ok_or(invalid(
        "Install Claude Code, sign in and refresh Council models",
    ))?;
    if managed_policy() {
        return Err(invalid(
            "Managed Claude policy requires a separate verified Council integration",
        ));
    }
    let mut version = cli::command(manager, &binary, root);
    version.env("CLAUDE_CODE_SAFE_MODE", "1").arg("--version");
    let version = cli::run(version, &[]).await?;
    if !supported_version(&version) {
        return Err(invalid(
            "Update Claude Code to 2.1.299 or newer for tool-free Council support",
        ));
    }
    let mut status = cli::command(manager, &binary, root);
    status
        .env("CLAUDE_CODE_SAFE_MODE", "1")
        .args(["--setting-sources", "", "auth", "status"]);
    let bytes = cli::run(status, &[])
        .await
        .map_err(|_| invalid("Sign in with Claude Code: claude auth login"))?;
    let status: Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid("Claude login status unavailable"))?;
    if status["loggedIn"] != true
        || !matches!(
            status["authMethod"].as_str(),
            Some("claude.ai" | "oauth_token")
        )
    {
        return Err(invalid(
            "Council requires the Claude Code subscription login; run claude auth login",
        ));
    }
    let mut cmd = cli::command(manager, &binary, root);
    // --bare deliberately ignores OAuth, so use the normal login with explicit
    // settings isolation. Hooks, skills, MCP and built-in tools remain disabled.
    cmd.env("DISABLE_AUTOUPDATER", "1")
        .env("CLAUDE_CODE_SAFE_MODE", "1")
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
        .env("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS", "1")
        .env("CLAUDE_CODE_DISABLE_FILE_MENTIONS", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .args([
            "--print",
            "--model",
            &profile.model,
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--disallowedTools",
            "mcp__*",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--setting-sources",
            "",
            "--settings",
            "{\"disableAllHooks\":true,\"enableWorkflows\":false,\"fastMode\":false}",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--permission-mode",
            "dontAsk",
            "--no-chrome",
        ]);
    parse(&cli::run(cmd, prompt).await?)
}
fn supported_version(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let parts = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>();
    parts.is_ok_and(|p| p.len() == 3 && (p[0], p[1], p[2]) >= (2, 1, 299))
}
fn parse(bytes: &[u8]) -> Result<(String, Value)> {
    let mut initialized = false;
    let mut answer = None;
    for line in bytes.split(|b| *b == b'\n').filter(|b| !b.is_empty()) {
        let event: Value =
            serde_json::from_slice(line).map_err(|_| invalid("Invalid Claude CLI output"))?;
        match event["type"].as_str() {
            Some("system") if event["subtype"] == "ui_invalidate" => {}
            Some("system") if event["subtype"] == "init" => {
                if !event["tools"].as_array().is_some_and(Vec::is_empty) {
                    return Err(invalid("Claude Council requires an empty tool registry"));
                }
                initialized = true;
            }
            Some("assistant") if initialized => {
                if event["message"]["content"]
                    .as_array()
                    .is_none_or(|content| {
                        content
                            .iter()
                            .any(|c| !matches!(c["type"].as_str(), Some("text" | "thinking")))
                    })
                {
                    return Err(invalid(
                        "Claude Council rejected a tool or unsupported content",
                    ));
                }
            }
            Some("rate_limit_event") if initialized => {}
            Some("result")
                if initialized && event["subtype"] == "success" && event["is_error"] == false =>
            {
                let text = event["result"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or(invalid("Claude returned no response text"))?;
                answer = Some((text.into(), event["usage"].clone()));
            }
            _ => {
                return Err(invalid(
                    "Claude Council returned an unsupported event or failed request",
                ));
            }
        }
    }
    answer.ok_or(invalid("Claude Council did not complete a text response"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_empty_registry_and_completed_text() {
        assert!(supported_version(b"2.1.301 (Claude Code)"));
        assert!(!supported_version(b"2.1.285 (Claude Code)"));
        assert!(!supported_version(b"unknown"));
        assert!(
            parse(b"{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"Bash\"]}\n").is_err()
        );
        assert!(parse(b"{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"hello\"}\n").is_err());
        assert_eq!(parse(b"{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[]}\n{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"hello\"}\n").unwrap().0,"hello");
        assert_eq!(parse(b"{\"type\":\"system\",\"subtype\":\"ui_invalidate\"}\n{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[]}\n{\"type\":\"rate_limit_event\"}\n{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"hello\"}\n").unwrap().0,"hello");
        assert!(parse(b"{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[]}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"}]}}\n").is_err());
    }
}
