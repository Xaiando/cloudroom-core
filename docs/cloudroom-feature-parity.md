# Cloudroom functions in Cloud Bee

Cloud Bee's scope is the original Cloudroom desktop's functions, in our native
Windows GUI. This is a delivery checklist, **not a claim of complete parity**.
The original React/Electron preview remains a separate reference application.
There is one Rust package and one Core service; the WPF client calls its API.

Reference: [original desktop](https://github.com/davidondrej/cloudroom-gui),
upstream baseline `be71032c87e2621fdb317f57071a23bf2fb56603`. The bundled plugin
registry contains 29 builtins and 6 opt-in official plugins. New upstream
functions must be added here when the reference is updated. Existing optional
marketplace packages require individual compatibility checks.

**Available** means the stated Windows workflow is implemented and exercised.
**Partial** describes a real subset with remaining behavior listed. **Pending**
means it must not be shown as a working native function. These states apply to
Cloud Bee, not to the separate original-desktop preview or Linux Core.

## Desktop and core workflows

| Function | State | Implemented behavior / remaining acceptance |
| --- | --- | --- |
| Native launcher, GUI and reconnect | Available | Ordinary-user launcher, connection errors, hidden Core, durable request retry and conversation drafts |
| Chat / Work modes | Partial | Tool-free API Chat, reviewed Codex Work handoffs, independent modes; richer interactive approvals and other Work providers pending |
| Council of AI | Partial | Broadcast and response comparison, saved membership; cross-provider council and autonomous deliberation pending |
| Conversation history | Available | Local journal, reconnect, replay without duplicate messages, interrupted execution recovery |
| Conversation organization | Available | Saved names, pins, collections, search by title/model/folder/collection, sidebar archive/restore; archive leaves running work alone |
| Full transcript search | Pending | Search indexed message and tool content with stable result navigation |
| Projects and environments | Partial | Separate managed workspace folders; arbitrary existing project registration, Git checkout/worktree creation and environment lifecycle pending |
| Conversation presentation | Partial | Native selectable Markdown, tables, lists, basic code coloring, heading/file links and tool output; richer media/diagram rendering pending |
| Attachments and file work | Partial | Bounded native workspace browser and UTF-8 previews; editing, uploads/downloads and image/PDF context pending |
| Interactive terminals | Pending | Native PTY lifecycle, resize/input/output and replay; detached descendants must remain owned |
| Agent controls | Partial | Queue/edit/cancel, interrupt/resume/close and managed permission policy; interaction UI, native subagent orchestration and broader CLI pending |
| Provider and model selection | Partial | Native Codex models/reasoning and separate Chat API profiles; original provider families and account management pending |
| Settings, theme and accessibility | Partial | Native window controls, connection and provider settings; theme editor, full shortcuts and accessibility acceptance pending |
| Update and installation lifecycle | Partial | Portable package and launchers; installer, signed updates, rollback and migration UX pending |
| Remote hosts, sync and Teleport | Pending | SSH connection/enrollment, credential handling, remote workspaces, sync conflicts and explicit remote execution permissions |
| Cloud execution | Pending | Requires configured infrastructure and credentials; a local folder or Windows Job is not cloud isolation |
| Voice / browser / computer tools | Pending | Original tool and voice workflows need explicit native integrations and permission checks |
| General plugin SDK and marketplace | Pending | No execution of arbitrary original TypeScript server/host code or React contributions yet |

## Every bundled original plugin

Source definitions:
[builtin registry](https://github.com/davidondrej/cloudroom-gui/blob/main/gui/apps/server/src/services/plugins/builtin-registry.ts)
and [plugin packages](https://github.com/davidondrej/cloudroom-gui/tree/main/gui/plugins).
Plugin IDs are preserved where a native port exists. Ports must not report
unsupported original SDK functions as available.

| Original plugin ID | State | Native behavior / remaining work |
| --- | --- | --- |
| bb-guide | Pending | Original guide, app discovery and agent help |
| accounts | Partial | Existing Codex CLI login and separate Chat API profiles; original account/login/subscription UI pending |
| ask-user-question | Pending | Interactive agent questions and durable replies; unsupported approvals currently fail closed |
| automations | Partial | Script/authorized-agent scheduling, timezones, run history, cancel and lifecycle; remote hosts, fresh worktrees, original retry policy and SQLite import pending |
| connect | Partial | Authenticated local Core connection; original remote host connection workflow pending |
| environment-project-checkout | Pending | Existing repositories, checkout ownership and project configuration |
| environment-git-worktree | Pending | Create/reuse/remove worktrees, dirty state and branch lifecycle |
| environment-personal-workspace | Partial | Managed local folders; original personal environment lifecycle pending |
| custom-instructions | Available | Saved settings, enable/disable, immutable context snapshot for new Bee messages and agent automation runs |
| plugin-api-tester | Pending | Original plugin API conformance tools |
| inline-vis | Pending | Inline interactive visual artifacts with resource and navigation boundaries |
| monaco-editor | Partial | Native read-only source previews and basic token coloring; editing, diff, save conflicts and original editor contributions pending |
| pdf-preview | Pending | Local PDF viewing and attachments |
| provider-codex | Partial | Native app-server Work and model/reasoning selection; full original provider contribution and interactions pending |
| provider-claude-code | Pending | Native runtime, login, streaming and permission integration |
| provider-pi | Pending | Native provider runtime and model/account integration |
| provider-usage | Partial | Last Chat response tokens; full provider quota, reset and usage views pending |
| time-in-cloudroom | Pending | Activity tracking and reporting |
| provider-acp | Pending | Agent Client Protocol runtime, models, tools and interaction lifecycle |
| computer-use | Pending | Native authorized desktop/browser tools and cancellation |
| keep-awake | Available | Native power request, observable status, release on disable/uninstall/Core exit |
| machine-ssh | Pending | Machine discovery, enrollment, SSH credentials and remote lifecycle |
| plugin-api-docs | Pending | Compatible native/plugin API documentation and agent discovery |
| provider-retry | Pending | Explicit provider retry/fallback policy with duplicate-execution protection |
| push-notifications | Pending | Notification registration, delivery and user controls |
| secrets | Pending | Windows protected credential store, explicit secret scopes and injection controls |
| drafts | Partial | Durable conversation drafts and unsent library drafts; original cross-host/plugin draft integration pending |
| scheduled-send | Pending | Durable delayed messages, edit/cancel and restart without duplicate send |
| side-chat | Pending | Side conversation lifecycle, context and parent navigation |
| workflows | Pending | Original saved workflow definition, execution and recovery |
| environment-modal-sandbox | Pending | Optional Modal infrastructure, credentials and remote lifecycle |
| github | Pending | Original repository/PR/issue integrations and explicit publication controls |
| simple-notes (Docs) | Partial | Native private Markdown editing, text import/export, revisions, search and archive/restore; vaults, file watching, folders, attachments and sync pending |
| memory | Partial | Native global/workspace memories, kinds/tags, explicit message context and immutable snapshots; original agent CLI/catalog, importance and richer retrieval pending |
| tasks | Partial | Native workspace lists, original statuses, revisions and reviewed message preparation; projects, labels, comments, attachments, PRs and delegated agent lifecycle pending |
| theme-preview | Pending | Original theme preview and editing workflow |

## Delivery sequence and acceptance

1. Native content and organization: Docs, Memory, Tasks, rename/pin/search/archive.
   The implemented subset is described in [native-library.md](native-library.md).
2. Conversation and project work: Markdown, files/attachments, native terminal,
   existing repositories and Git worktrees. Preserve process ownership and file
   permissions through failures and restarts. Native Markdown, file links and
   read-only UTF-8 previews are delivered; see [native-files.md](native-files.md).
3. Agent integrations: interactive questions/approvals, Claude/Pi/ACP, native
   child agents and CLI parity, usage, secrets, provider retry and scheduled send.
4. Remaining desktop plugins: side chats, workflows, notifications, inline
   visualizations, editor/PDF, themes, guide and plugin developer tools.
5. Remote workflows: SSH, enrollment, sync, Teleport, optional cloud environments
   and GitHub. Test with explicitly configured disposable hosts/accounts.
6. Marketplace compatibility: define/version the API contract, adapt original
   app/host/server contributions and test each package's real workflow. Preserve
   one Core service; any interpreter workers must be owned child processes.

Each completed item needs a working API, native GUI workflow where applicable,
persisted recovery behavior, actual Windows acceptance and updated capability
reporting. A menu entry or copied package is not acceptance. Existing user
history, credentials and running work must be preserved. Tests use disposable
state and deterministic providers; only the owner applies production SQL
migrations. Publication retains upstream attribution and excludes private data.
