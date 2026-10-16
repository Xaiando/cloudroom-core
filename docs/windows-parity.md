# Windows desktop parity

The original Cloudroom desktop is the functional reference. Cloud Bee keeps our native WPF design as the primary Windows GUI and ports supported functions into Rust Core's API.

## Source and architecture

- Original desktop: https://github.com/davidondrej/cloudroom-gui
- Windows desktop fork: https://github.com/Xaiando/cloudroom-gui, branch `windows-native`.
- Initial upstream baseline: `be71032c87e2621fdb317f57071a23bf2fb56603`.
- Windows Core fork: https://github.com/Xaiando/cloudroom-core, branch `windows-native`.

Keep the original desktop's React interface, Electron shell, plugins, local server, host daemon, and CLI together in its separate reference/preview repository. The Cloud Bee native client remains primary. Core remains one Rust package and one service. Preserve both projects' upstream licenses and attribution. Keep platform changes small enough to review and merge with upstream updates.

## Acceptance checklist

A feature is available only after its Windows workflow has been exercised. An upstream implementation or a successful compilation alone is not proof of Windows support.

| Area | Windows acceptance |
| --- | --- |
| Desktop | Original navigation, themes, settings, shortcuts, window controls, and clear startup errors |
| Projects and threads | Existing folders, Git worktrees, search, pin, rename, archive, and history after restart |
| Conversations | Markdown, code, file links, tools, attachments, queue controls, and interrupted requests |
| Plugins | Installation, enable/disable, settings, assets, and containment of plugin file paths |
| Automations | One-shot and recurring schedules, timezones, enable/disable, run now, and persisted run history |
| Agents | Installed harnesses, model selection, reasoning, authentication, and visible capability limits |
| Orchestration | Agent CLI, child threads, cross-thread controls, and clean process shutdown |
| Windows integration | Ordinary-user setup, paths with spaces and Unicode, native terminals, launcher, installer, and upgrade |
| Remote execution | Self-hosted connection, per-thread isolation, sync, Teleport, and explicit local-machine permissions |

Remote execution requires a configured host or cloud infrastructure. A local Windows process group or separate folder is not an isolated cloud sandbox. Do not label unsupported functionality as available or replace missing functionality with decorative controls.

## Improvements after parity

Prioritize improvements that can be demonstrated: reliable first-run discovery, actionable errors, safe restart and request recovery, persistent thread organization, clear account and model availability, and responsive navigation with many threads. Preserve the original workflow while measuring improvements with real Windows behavior and disposable fixtures.

Use separate preview data while validating the original desktop port. Do not silently replace or migrate the existing WPF client's history. The original Electron preview has its own launcher and data; Cloud Bee remains the primary application selected by the user.

## Run the original desktop preview

Double-click `Start-Cloudroom-Preview.cmd` after the original desktop has been built. For a new Core checkout:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install\original-desktop.ps1 -Action Setup
.\Start-Cloudroom-Preview.cmd
```

Setup checks out our Windows desktop fork, installs the pinned dependencies, and builds the original app. The preview uses `%LOCALAPPDATA%\CloudroomDesktopPreview`; it does not reuse the WPF client's Core data. The existing `Start-Cloudroom.cmd` remains available as a compatibility launcher; `Start-Cloud-Bee.cmd` is the primary native launcher.

The original app builds and type-checks on Windows, and its Plugins and Automations pages load. A local Node script automation has been created, run through the CLI and on a timer, checked for output and exit code, and deleted. Windows private-file ACL checks, plugin path containment, CLI argument forwarding, and local onboarding have dedicated checks. Full harness, terminal, child-thread, worktree, and remote parity still require acceptance tests; see the desktop fork's `gui/docs/windows.md` for validation and known suite limitations.

## Cloud Bee modes and Council

The WPF application remains our primary native Windows GUI, now branded Cloud Bee.
The [Cloud Bee implementation](cloud-bee.md) adds durable mode envelopes, separate
text-only API Chat, authorized Codex Work, reviewed handoffs and Council broadcast /
comparison. Original desktop Plugins, Skills and Automations are still outstanding
native features; the original Electron preview remains a separate reference app.
