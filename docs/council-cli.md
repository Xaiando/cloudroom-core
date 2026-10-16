# Council with existing CLI sign-ins on Windows

Council discovers Codex, Grok and Claude Code CLI models without creating API profiles. Open
**Council of AI**, enable GPT, Grok or Claude, choose a model labelled **CLI login**, and
send a message. Each enabled participant receives the message. Coordination,
microtask progress, history, shared folder access and HTC remain in the same GUI.

Codex needs its normal CLI login and `models_cache.json` in the configured
`CODEX_HOME`. Running Codex normally refreshes that catalog. Grok needs its normal
`grok login` credentials in the configured account's `.grok` directory. A cached
login/catalog does not guarantee access to every model: the selected CLI checks
account access when sending. CLI account limits and billing still apply. Cloud
Bee does not copy OAuth credentials into API profiles or switch to a paid API
after a CLI failure. Existing Chat API connections remain available separately.

Claude requires native Claude Code 2.1.299 or newer, installed under the configured
account's `.local/bin` or `AppData/Roaming/npm`. Run `claude auth login` with your
subscription account, then **Refresh models**. Council offers the `sonnet`, `opus`
and `haiku` aliases; the CLI resolves them and checks account access when sending.
An API-key-only Claude CLI login does not qualify for this subscription transport.

Qwen can use a local OpenAI-compatible server. In **Chat connections**, choose
the local server provider, enter its completion endpoint (for example
`http://127.0.0.1:11434/v1/chat/completions` for Ollama) and an already installed
model such as `qwen2.5:7b`. Save, then **Refresh models** in Council and choose it
in the Qwen / Local selector. Alibaba Qwen API profiles also work. Qwen Code's
legacy OAuth free tier ended on April 15, 2026; see its
[authentication guide](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/).

For Gemini, open **Chat connections → Google Gemini API → Gemini**, enter a
Google AI Studio API key, keep `gemini-2.5-pro` (or another exact Gemini model ID)
selected, and save the connection. Then click **Refresh models** in Council.
Normal Work CLI integrations are separate from these Council text transports.

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
- Claude uses its normal subscription login with safe mode, empty settings
  sources, disabled memory and file mentions, no built-in tools and an empty
  strict MCP configuration. Hooks, plugins and slash commands are disabled.
  Council rejects managed policy files or registry settings, unsupported CLI
  versions, nonempty tool registries and tool-use output. It does not use bare
  mode, because that mode ignores Claude's OAuth login.
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
cleanup, Claude subscription-login enforcement and the Grok customization guard.
