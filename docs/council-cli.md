# Council with existing CLI sign-ins on Windows

Council discovers Codex and Grok CLI models without creating API profiles. Open
**Council of AI**, enable GPT or Grok, choose a model labelled **CLI login**, and
send a message. Each enabled participant receives the message. Coordination,
microtask progress, history, shared folder access and HTC remain in the same GUI.

Codex needs its normal CLI login and `models_cache.json` in the configured
`CODEX_HOME`. Running Codex normally refreshes that catalog. Grok needs its normal
`grok login` credentials in the configured account's `.grok` directory. A cached
login/catalog does not guarantee access to every model: the selected CLI checks
account access when sending. CLI account limits and billing still apply. Cloud
Bee does not copy OAuth credentials into API profiles or switch to a paid API
after a CLI failure. Existing Chat API connections remain available separately.

Claude, Qwen and DeepSeek Council participants currently use their API
connections. Native Council transports for their CLI accounts are not implemented.
The model selectors show this requirement rather than presenting an empty list
as though it were usable. Normal Work CLI integrations are separate.

Council CLI participants are text transports, not unrestricted coding agents:

- Each call starts a new, disposable conversation in an empty private directory.
  Core supplies bounded conversation text and mediates the explicitly selected
  folder reads. The CLI is never started in the shared workspace.
- Codex runs with `--ignore-user-config`, `--ephemeral`, a read-only sandbox,
  disabled execution/search/apps/hooks/MCP and a sanitized public model catalog.
  Before the first call for each CLI binary/catalog combination, Core tests the
  same policy against a temporary loopback mock provider using an empty login
  directory. The probe must send an empty tool registry and complete a text
  response. It uses no account tokens and makes no model-provider request.
- Grok runs headless with an empty `--tools` allowlist, no subagents or web tools,
  `dontAsk` permissions and one model turn. Because Grok does not provide a global
  customization bypass, Council refuses accounts with hooks, MCP, plugins,
  managed configuration or customization references in `config.toml`.
- CLI children inherit only selected Windows startup variables, not Core's token,
  saved environment credentials or API keys. Each call owns a kill-on-close
  Windows Job Object, has a 120-second deadline and a 2 MiB output limit.
- CLI changes that invalidate the Codex policy check fail closed. Text output
  containing unsupported native tool events is rejected. There is no automatic
  provider fallback. CLI-managed account/history files remain under that CLI's
  control; Grok may save its own native conversation history.

Native Work still requires the separate microtask authorization and uses HTC's
isolated worktrees and review flow. Selecting a CLI model grants no additional
shared folder access.

Validation: `tests/windows_council_cli.py` uses disposable fixture CLIs and no
real accounts. It covers selection without API profiles, backend identity,
coordination, rejected tool registries/events, cancellation with descendant
cleanup and the Grok customization guard.
