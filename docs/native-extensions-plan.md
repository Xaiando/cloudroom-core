# Native Automations and Plugins

Keep WPF and one Rust Core service. Port the original Automations, Custom
instructions and Keep Awake features first, as requested. The original source
reference is `davidondrej/cloudroom-gui`, baseline
`be71032c87e2621fdb317f57071a23bf2fb56603`, particularly `gui/plugins/automations`,
`gui/plugins/custom-instructions` and `gui/plugins/keep-awake`.

Persist schedules, settings, authorization, immutable execution snapshots and run
history in private local Core storage. Claim a run before execution. Never replay
uncertain runs after restart. Prevent overlapping runs; pause affects future runs,
and cancellation stops owned script descendants. Local schedules require Core
running and Windows awake. Provide numeric five-field cron with IANA timezones,
one-shot timestamps and intervals, with explicit missed-run behavior.

Scripts use native PowerShell, Node or Python, selected workspace, bounded output
and timeout. Agent automations target an already-authorized Cloud Bee Work session;
Chat never gains tools. The editor explicitly authorizes repeated execution.

Custom instructions snapshot onto new prompts; edits cannot change queued requests
or retries. Disabling removes future additions. Keep Awake uses Windows power
requests, released on disable or Core exit. Plugin state/settings persist; catalogue
and installed views distinguish these native ports from unsupported React/Node SDK
plugins. Do not claim general original-plugin binary compatibility.

Verify with disposable Core/script/provider fixtures and actual WPF screens, then
package, publish to our fork and activate without interrupting active tasks.
