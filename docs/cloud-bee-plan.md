# Cloud Bee: first Chat / Work slice

Keep the WPF desktop, one Rust service, and the existing native session journal.
The original Cloudroom remains the feature reference; Cloud Bee is the Windows
application name. Existing Core API clients and saved agent sessions remain compatible.

Add a durable conversation envelope with independent Chat and Work histories,
selected mode, Council group identity, immutable requests, structured handoffs,
workspace, provider/model identity, and linked native execution session. Switching
modes never interrupts work or makes an inference request. Council initially means
explicit broadcast and comparison, not autonomous delegation or consensus.

Chat uses a dedicated OpenAI-compatible HTTP adapter with no tool dispatcher,
no tools supplied, and tool_choice=none. Reject tool-call responses. OpenAI and
xAI require separate API credentials; Codex/Grok subscription login is not reused.
Local endpoints must be loopback. Unconfigured or incompatible Chat is unavailable.

Managed Work initially supports Codex: explicit user authorization, registered
workspace selection, workspace-write and untrusted approval policy. Approval
requests that Core cannot interactively authorize fail closed. No approval bypass,
automatic escalation, or claim of Windows account isolation. Other existing
harnesses remain available through their original sessions; they are not advertised
as managed Work until their permission contracts can be verified.

Use compact, user-reviewed structured handoffs, never credentials or automatically
copied tool output. Return to Chat with an explicit result handoff. Retain native
histories separately. Persist intent before dispatch; unknown outcomes after
restart require inspection, never automatic replay. Test with disposable native
Codex and HTTP fixtures, then build, capture WPF screens, document limitations,
and publish only to the fork's windows-native branch.
