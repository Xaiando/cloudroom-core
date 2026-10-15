# Cloudroom Desktop for Windows (alpha)

A native WPF client for the Windows Core runtime. It uses Core's authenticated HTTP API; it does not run another service, embed a browser, or duplicate session storage.

## Use

With the runtime configured and the desktop executable published:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install\windows.ps1 -Action Gui
```

Or double-click `install\Start-Cloudroom.cmd`. The execution-policy override applies only to this launch. The launcher finds native Codex on PATH, in its desktop bundle, in installed npm packages, or in Microsoft Store installations.

The launcher starts Core if necessary and opens the desktop window. Closing the window leaves Core and its sessions running. Use `-Action Stop` to stop the backend.

The GUI reads `%LOCALAPPDATA%\CloudroomCore\core.json` by default. Connection settings can load another settings file or accept a loopback address and token in memory. Tokens are never passed on the command line, saved by the GUI, or included in screenshots and test reports. The HTTP client disables redirects and proxies.

Create a session with the selected model and reasoning level, optionally in a new project folder. Send messages, watch live output, expand command output, browse saved sessions, pause/resume the queue, interrupt a running turn, close a session, or open its folder. Enter sends; Shift+Enter adds a line. Closing a session preserves history.

If a send loses its response, retry the unchanged draft to reuse its original request ID and payload. The app does not automatically resend uncertain work. Switching sessions retains drafts in memory. Session titles derive from the first prompt and are rebuilt from history when opened.

Output is displayed as selectable plain text. Rich Markdown, file attachments, integrated terminals, and system-tray/boot integration are not included in this alpha. The backend's same-account, local-only security limits still apply.

## Build a standalone executable

Install the [.NET 10 SDK](https://dotnet.microsoft.com/download/dotnet/10.0) on Windows, then from the repository root:

```powershell
dotnet publish integrations/windows/Cloudroom.Desktop.csproj -c Release -r win-x64 --self-contained true -o integrations/windows/publish -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -warnaserror
```

The output is `integrations/windows/publish/Cloudroom.Desktop.exe`. Its runtime is bundled; users of that build do not need to install .NET. Use `win-arm64` to publish for ARM64 Windows. Building requires the SDK; having only a .NET runtime is insufficient.

The manifest uses the current user's permissions and per-monitor DPI awareness. Runtime configuration, build outputs and test artifacts stay out of Git.

## Validate

```powershell
cargo build --locked
python tests/windows_gui.py
```

The smoke test opens and exercises the actual WPF window against a disposable deterministic Codex fixture, never a real inference account. It validates connection restrictions, project creation, prompt delivery, live output, duplicate-free polling, queue pause/resume, session close, and transcript replay. A test proxy deliberately drops an accepted prompt response, then verifies that a retry preserves the original ID and payload even if the effort picker changes. It renders `welcome.png` and `conversation.png` under ignored `private/gui-qa` for visual inspection.

After publishing the desktop app and building release Core, `install/package-windows.ps1` creates an ignored portable ZIP containing both executables, a double-click launcher, instructions and the upstream license. It refuses to overwrite an existing package. Windows CI exercises the GUI and uploads that ZIP as a build artifact.
