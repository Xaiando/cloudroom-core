# Native Windows local runtime

This branch builds `cloudroom.exe` directly on Windows, with a [native desktop GUI](../integrations/windows/README.md). No WSL, Docker, PostgreSQL or administrator session is needed. It is an initial **single-user, localhost-only Codex runtime**. It does not install an SCM Windows Service.

Agents run under the same Windows identity as Core and have that account's permissions. The token and history are therefore readable by its agents. Job Objects manage process lifetime; they do not isolate files or credentials. Use a dedicated ordinary Windows account if you want to keep agent work separate from your personal account. The Linux installer and its two-account protection do not apply to this mode.

## Set up

Requires Windows 10/11, Git, Rust 1.89+ with the MSVC C++ build tools, and a native `codex.exe` with an existing login. The installed Codex desktop bundle's native CLI can be used. The fixture suite uses Python and Node.js; neither is required for basic Codex session execution.

Clone the Windows branch, then run these commands in PowerShell:

```powershell
git clone --branch windows-native https://github.com/Xaiando/cloudroom-core.git
cd cloudroom-core
.\install\windows.ps1 -Action Check
cargo build --locked --release
.\install\windows.ps1 -Action Configure
.\install\windows.ps1 -Action Start
.\install\windows.ps1 -Action Status
# After publishing the desktop executable (see desktop build guide):
.\install\windows.ps1 -Action Gui
```

Configure reads the model from your existing `.codex/config.toml`. Supply `-Model MODEL_ID` if it cannot find one. Use `-CodexBinary C:\path\codex.exe`, `-CodexHome C:\path\.codex`, `-Repository D:\your-project`, or `-DataDirectory D:\private-runtime` to override defaults. Configure refuses to overwrite existing settings. Edit its private JSON file to change settings, then restart Core.

By default, `%LOCALAPPDATA%\CloudroomCore` contains private `core.json`, logs, process metadata, an empty default `workspace`, and the `history` journal. Configure restricts that directory's ACL to your account and SYSTEM before saving a random token. Never put it in source control. Back up the history directory while Core is stopped.

`Start` launches a hidden background process for the current login. It does not install a boot service or create a Scheduled Task. `Run` instead runs in the foreground and supports Ctrl+C. `Stop` checks the saved process ID, start time and executable path before stopping it. An abrupt stop interrupts running work; restart recovers history without replaying uncertain prompts.

## Use the API

The API helper reads the token privately and keeps it out of command-line arguments:

```powershell
.\install\windows.ps1 -Action Api
.\install\windows.ps1 -Action Api -Method POST -ApiPath /v1/sessions -Body '{"request_id":"windows-demo","harness":"codex"}'
.\install\windows.ps1 -Action Api -ApiPath /v1/sessions/cr_windows-demo
# Wait for session.state = idle, then:
.\install\windows.ps1 -Action Api -Method POST -ApiPath /v1/sessions/cr_windows-demo/prompts -Body '{"request_id":"hello","text":"Create hello.txt containing Hello from Windows."}'
.\install\windows.ps1 -Action Api -ApiPath /v1/sessions/cr_windows-demo/events
.\install\windows.ps1 -Action Api -Method POST -ApiPath /v1/sessions/cr_windows-demo/close -Body '{"request_id":"close"}'
```

HTTP 202 accepts work asynchronously; inspect the receipt and saved events for completion. Core defaults to `http://127.0.0.1:9840`. Windows local mode refuses non-loopback listeners, even when the Linux non-loopback override is set.

## What is ported

- Authenticated HTTP API, Codex app-server protocol, model catalog, prompt queue and session lifecycle.
- Local journal, snapshots and restart recovery. Windows flushes each file before replacement; it does not use the Unix directory fsync operation.
- Direct project folders and repository paths, including paths containing spaces.
- Workloads start suspended, enter a non-inheritable Job Object, then resume. Closing a session or losing Core's Job handles ends descendants, including detached child processes.
- Disk capacity uses the Windows API. Linux CPU/process telemetry remains unavailable.

## Remaining Windows work

ConPTY terminals, other harnesses, the command guard, Git author hook, agent-local `room-cli` IPC, skills/settings sync, attachment upload, Teleport, SSH previews, Mac pairing, Linux virtual desktop automation, and Linux storage isolation are unavailable. The capability response marks these features false. Explicit requests to enable the command guard or author hook are rejected. Codex's own tools run without approval prompts, as in upstream; use this only for trusted tasks.

The native Windows GUI lives in `integrations/windows`. It connects through the same authenticated API, provides session and model selection, live conversations, queue and session controls, and reads the existing local connection settings. Its standalone build bundles .NET. `install\windows.ps1 -Action Gui` starts Core if needed and opens the GUI. Closing the window leaves sessions running.

## Validate

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked
python tests/windows_native.py
```

POSIX runtime/adapter/PTY integration fixtures remain Unix-only. Windows-specific tests cover native process ownership, detached descendants, authenticated API calls, prompt completion, project folders, history recovery and cleanup after a Core crash. They use a deterministic Codex fixture and do not call a model.
