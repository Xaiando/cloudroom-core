[CmdletBinding()]
param(
    [string]$CoreBinary,
    [string]$DesktopBinary,
    [string]$Destination
)
$ErrorActionPreference = 'Stop'
$sourceRoot = Split-Path -Parent $PSScriptRoot
if (!$CoreBinary) { $CoreBinary = Join-Path $sourceRoot 'target\release\cloudroom.exe' }
if (!$DesktopBinary) { $DesktopBinary = Join-Path $sourceRoot 'integrations\windows\publish\Cloudroom.Desktop.exe' }
if (!$Destination) { $Destination = Join-Path $sourceRoot 'private\releases\Cloud-Bee-Windows-alpha' }
if (!(Test-Path -LiteralPath $CoreBinary) -or !(Test-Path -LiteralPath $DesktopBinary)) { throw 'Build release Core and publish the standalone desktop app first.' }
if ((Test-Path -LiteralPath $Destination) -or (Test-Path -LiteralPath ($Destination + '.zip'))) { throw 'Choose a new Destination; packages are never overwritten.' }
[void][System.IO.Directory]::CreateDirectory($Destination)
[void][System.IO.Directory]::CreateDirectory((Join-Path $Destination 'install'))
Copy-Item -LiteralPath $CoreBinary -Destination (Join-Path $Destination 'cloudroom.exe')
Copy-Item -LiteralPath $DesktopBinary -Destination (Join-Path $Destination 'Cloudroom.Desktop.exe')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'LICENSE') -Destination (Join-Path $Destination 'LICENSE')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'integrations\windows\THIRD-PARTY-NOTICES.txt') -Destination (Join-Path $Destination 'THIRD-PARTY-NOTICES.txt')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'install\windows.ps1') -Destination (Join-Path $Destination 'install\windows.ps1')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'install\create-shortcut.ps1') -Destination (Join-Path $Destination 'install\create-shortcut.ps1')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'Start-Cloudroom.cmd') -Destination (Join-Path $Destination 'Start-Cloudroom.cmd')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'Start-Cloud-Bee.cmd') -Destination (Join-Path $Destination 'Start-Cloud-Bee.cmd')
$guide = @'
# Cloud Bee for Windows (alpha)

This portable package includes the native Core runtime and desktop GUI.
Requires Windows 10/11 x64 and a signed-in native codex.exe. The launcher finds
Codex on PATH, in its desktop bundle, npm packages, or Microsoft Store installs.
No .NET installation is required for the bundled desktop app.

Double-click Start-Cloud-Bee.cmd. First use creates private runtime settings
under %LOCALAPPDATA%\CloudroomCore, then starts Core and the GUI. If Codex
cannot be found or its model is not configured, use PowerShell:

  .\install\windows.ps1 -Action Configure -CodexBinary C:\path\codex.exe -Model MODEL_ID
  .\install\windows.ps1 -Action Gui

To add a desktop shortcut, run from this extracted folder:

  powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install\create-shortcut.ps1

Keep the extracted folder in place; recreate the shortcut after moving it.

Projects: click + Project in the sidebar to choose a name, colour and existing
local workspace folder. Select a project and click its + to create multiple
independent Chat/Work or Council threads. Organize selected can rename, pin,
archive or move a chat. The project's ... opens settings. Archived projects can
be shown and restored. Folder changes affect new threads; existing chats keep
their original workspace and permissions. Choosing a folder grants no agent access.

Closing the GUI leaves agents running. Stop Core with:

  .\install\windows.ps1 -Action Stop

Use Chat connections to configure OpenAI, xAI, Anthropic, Alibaba Qwen, or a local
compatible server. API billing is separate from consumer subscriptions. Ordinary
Chat is text-only; unsupported modes stay unavailable. Work supports native Codex
with reviewed context and explicit workspace authorization.

Council has GPT/Grok/Claude/Qwen/DeepSeek model selectors, shared messages,
dependency microtasks and live progress. GPT, Grok and Claude can use existing
CLI logins without API profiles. Claude requires Claude Code 2.1.299+ and
claude auth login with a subscription account. Qwen supports a local compatible
server such as Ollama, or an Alibaba API profile. Save connections and click
Refresh models in Council. Account access and limits apply. CLI text requests
disable execution tools; Codex verifies its tool policy locally before sending.
Grok custom hooks, MCP and plugins must be disabled. Claude managed policy
requires a separate verified integration. Choose a local folder and shared access when creating
one. API Chat participants may discuss/read approved files but cannot write them.
Choose isolated Work access on a clean Git project, then Run isolated Work on a
microtask. Native Codex workers run in separate detached Git worktrees. Git for
Windows is required. Hive Traffic Control queues conflicting shared integration,
shows resource ownership separately from task progress, and provides inspect,
pause, cancel, retry, approve, reject and recovery controls. V1 integrates one
existing tracked UTF-8 file up to 256 KiB per task, after revision checks and human
review. Whole branches, new/deleted files, automatic project tests and additional
native coding providers are pending. Unfinished worktrees are preserved.

On restart, HTC ownership enters recovery and native workers are not replayed.
Inspect disk/worktree/history before explicitly releasing recovery. Leases and
worktrees coordinate Core-mediated writes; same-user native processes can bypass
application locks. This is not OS account isolation.

The sidebar includes Automations and Plugins. Schedule native PowerShell, Node or
Python scripts, or target an authorized Work conversation. Local schedules require
Core running and Windows awake. Custom instructions and Keep Awake are native
plugin ports; other original marketplace packages require individual Windows ports.

Docs, Memory and Tasks are also bundled native ports. Open Docs / Memory / Tasks
in the sidebar and install/enable the desired type. Documents support private text
editing, revisions, search and archive/restore. Memories enter new messages only
when selected for context. Tasks track status and prepare drafts for review.
Organize selected saves conversation names, pins, collections and archive state.
Full original Cloudroom parity is still in progress; see the source feature checklist.

Conversations and Council answers render native Markdown with tables, lists and
basic code coloring. Workspace files browses and previews UTF-8 text inside the
selected conversation's workspace. File links open native previews, including
source-line references. Edit / Preview opens the native text editor; Compare,
Save file and Reload disk provide revision checks and reviewed conflict handling.
Recovery copies and unsaved drafts are retained privately. Interrupted in-place
writes may require manually loading a recovery draft; restart never replays writes.
Attachments, file creation/deletion and fuller editor integrations are pending.

DeepSeek: in Chat connections choose OpenCode -> DeepSeek Flash Free (limited
time), supply an OpenCode Console API key, then select DeepSeek in Chat or
Council. The pinned free model never falls back to a paid one. The separate
DeepSeek direct API option requires a DeepSeek key and is paid. Check current
access/terms: https://opencode.ai/v2/docs/console/models/
Create a key at https://opencode.ai/console/ under Keys -> Add Service Account
-> Add API Key, with Inference only permissions. Paste it into Cloud Bee and
Save connection, then Refresh models in Council. Never share the key in chat.
Availability limitation: a valid Console key can still receive HTTP 403
FreeTierError because OpenCode restricts its free tier to OpenCode. Live checks
on 2026-10-16 also failed through official headless OpenCode CLI 1.19.3/2.0.31.
Free DeepSeek inference from Cloud Bee is not verified as available. No paid
model is substituted; creating another key does not fix this access restriction.

If a Codex app update removes the saved CLI path, the launcher rediscovers the
installed executable while preserving other runtime settings.

This is a single-user, localhost-only alpha. Windows account isolation is not
provided. Managed Work requests workspace-write/untrusted approvals; unsupported
approval requests are denied. Existing legacy sessions keep their original
permissions. Back up Core data while stopped. Never share core.json, bee/profiles.json,
account tokens, conversations or desktop outbox files.

Source and full setup guide: https://github.com/Xaiando/cloudroom-core
'@
[System.IO.File]::WriteAllText((Join-Path $Destination 'README.md'), $guide)
Compress-Archive -LiteralPath $Destination -DestinationPath ($Destination + '.zip')
Write-Output ($Destination + '.zip')
