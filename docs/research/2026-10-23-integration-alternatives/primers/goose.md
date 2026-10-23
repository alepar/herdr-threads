# Harness primer: Goose (id `goose`)

Shared basis for the alternative x harness reports. Scope: every Goose surface that can put text into the
model's context or wake an idle session, plus what herdr-threads does for Goose today.

## Provenance

- Upstream: <https://github.com/block/goose> (the clone's `origin`; the README's CI badges and install URL now
  point at <https://github.com/aaif-goose/goose>, README.md:12-13,38, so the project has moved to the AAIF org;
  INFERRED that block/goose redirects there).
- Docs site: <https://goose-docs.ai/> (`documentation/docusaurus.config.ts:23`). A doc at
  `documentation/docs/<p>.md` is served at `https://goose-docs.ai/docs/<p>`.
- **Two source trees were read, because the installed binary is 20 minor versions behind main:**
  - `goose@1.36` = tag `v1.36.0`, commit `f13f369a67db04b52f74d37a2f39f6459e4d10fe` (2026-05-27),
    `Cargo.toml` version 1.36.0. This matches the locally installed binary.
  - `goose@main` = commit `c3bfc5f7b0e230180ba3164ec1ee84fd951eaf0d` (2026-10-23), `Cargo.toml` version 1.56.0.
  - Both are shallow clones under
    `/tmp/claude-1001/-var-home-alepar-AleCode-herdr-threads/eb41f352-f521-473d-9f2c-a3cb4a8d6cb0/scratchpad/goose-src/`
    (`goose-1.36/`, `goose/`). Every `path:line` below is relative to the clone root and says which tree.
    Lines in herdr-threads are marked "(ht)".
- Installed locally: `/var/home/alepar/.local/bin/goose`, `goose --version` = `1.36.0`. Help text was observed
  with `--help` only, run under a scratch `HOME`/`XDG_*` so no real config was touched. Nothing was run that
  starts a session or talks to a provider.
- Tags: VERIFIED = read in source, docs or observed `--help` at the cited place. INFERRED = my reading, not
  stated. "unknown" = not determined.
- Version handling for the later reports: per AGENTS.md (ht) we assume newer versions keep older behaviour,
  so a mechanism is usable "from version X" once it appears. Each section says where 1.36 and main differ.

## 1. Architecture in one paragraph

Goose is a Rust agent (`crates/goose`) with several front ends. The **CLI** (`goose session`, `goose run`) runs
the agent **in-process** behind a `rustyline` line editor: there is no server between the terminal and the
agent, so nothing outside the process can submit a prompt into a running `goose session` except the terminal
itself. VERIFIED: `goose@1.36 crates/goose-cli/src/session/mod.rs:1086-1101` (CLI calls `self.agent.reply(..)`
directly), `crates/goose-cli/src/session/input.rs:5,152`. The **Desktop** app talks to a server (`goosed`
HTTP in 1.36, ACP in main). **ACP** (`goose acp` on stdio, `goose serve` on HTTP/WebSocket) exposes the same
agent to editors and other clients. Extensions are MCP servers (stdio, streamable HTTP) plus in-process
"platform" extensions; lifecycle **hooks** are shell commands shipped in Open Plugins-format plugins.
Conversation state lives in a SQLite database that every turn re-reads (section 8).

Agent loop (1.36): `Agent::reply` stores the user message, reloads the conversation from the session store,
then loops "provider call -> tool calls -> provider call" until the model stops or `max_turns` is hit.
VERIFIED: `goose@1.36 crates/goose/src/agents/agent.rs:1352-1520,1655-1690`. Main replaced the loop with a
state machine of operations (`steer`, `stop_hook`, `toolcalling`, `inference_preparation`, ...). VERIFIED:
`goose@main crates/goose/src/agents/state_machine_builder.rs:40-118`.

## 2. Front ends and process models

| Front end | Command | Agent location | External prompt entry | Version |
|---|---|---|---|---|
| Interactive CLI | `goose session [-r] [--session-id ID] [-n NAME]` | in-process | terminal keystrokes only | both |
| Headless CLI | `goose run -t TEXT \| -i FILE \| -i - [-r --session-id ID] [--output-format text\|json\|stream-json]` | in-process, exits after the run | argv/stdin of a new process | both |
| Terminal integration | `goose term init <shell>`, `goose term run PROMPT`, `goose term log CMD` | new headless process per `term run`, resuming the shell's session | a new process | both |
| ACP stdio | `goose acp [--with-builtin ..]` | in-process, one client | ACP `session/prompt` | both |
| ACP HTTP/WS | `goose serve [--host 127.0.0.1] [--port 3284]` | server, many connections | ACP over `/acp` | both |
| Desktop | Electron app | `goosed` HTTP (1.36), ACP (main) | `POST /sessions/{id}/reply` (1.36) | both |
| TUI | `goose tui` (npm `@aaif/goose`, spawns `goose acp`) | ACP child | n/a | 1.36; **deprecated and removed on main** |
| Gateway | `goose gateway start --bot-token T telegram` | server | Telegram messages | both |
| Scheduler | `goose schedule add --cron ... --recipe-source ...` | new session per run | cron | both |
| Roaming | iroh p2p ACP (`goose-roaming` crate) | server | remote ACP client | main only |

Evidence (VERIFIED):
- CLI help observed for `goose`, `session`, `run`, `acp`, `serve`, `term`, `term init`, `tui`, `plugin`,
  `plugin install`, `schedule`, `schedule add`, `gateway`, `gateway start`, `recipe`, `session list`,
  `session export`, `mcp`, `info` (goose 1.36.0). The table's flags are quoted from that output.
- `goose run` flags: `-t/--text`, `-i/--instructions` (`-` = stdin), `-s/--interactive` (continue interactively
  after the input), `-r/--resume`, `--session-id`, `--no-session`, `--max-turns`, `--system TEXT` (extra system
  instructions), `-q/--quiet`, `--output-format text|json|stream-json`, `--with-extension 'ENV=v cmd args'`,
  `--with-streamable-http-extension URL`, `--with-builtin NAME`, `--no-profile`, `--provider`, `--model`
  (observed help).
- `goose session` flags: `-r/--resume`, `--fork`, `--history`, `--session-id`, `-n/--name`, `--max-turns`,
  `--max-tool-repetitions`, `--with-extension`, `--with-streamable-http-extension`, `--with-builtin`,
  `--no-profile`, `--container` (observed help).
- TUI resolution and `GOOSE_BINARY` env: `goose@1.36 crates/goose-cli/src/commands/tui.rs:1-80`; TUI spawns
  `goose acp`: `goose@1.36 ui/text/src/tui.tsx:1363-1369`, `ui/text/README.md`. Deprecated:
  `goose@main ui/text/README.md:1-16`.
- `goosed` HTTP routes `/sessions/{id}/events` (SSE), `/sessions/{id}/reply`, `/sessions/{id}/cancel`:
  `goose@1.36 crates/goose-server/src/routes/session_events.rs:628-637`; auth header `X-Secret-Key`:
  `crates/goose-server/src/auth.rs:30,47`; a reply while another request is active is refused with
  "Session already has an active request. Cancel it first.": `session_events.rs:354`. `goose-server` is gone
  from `goose@main crates/` (directory listing).
- Gateway: incoming platform message -> `agent.reply(..)` on the paired session:
  `goose@1.36 crates/goose/src/gateway/handler.rs:63,356`.
- Roaming: `goose@main crates/goose-roaming/src/lib.rs:1-24` ("a remote ACP client ... can connect to and drive
  it through a relay"; an accepted peer "gets goose's full ACP surface"); doc
  <https://goose-docs.ai/docs/guides/roaming-agents>.

## 3. Hooks

Docs: <https://goose-docs.ai/docs/guides/context-engineering/hooks>
(`goose@main documentation/docs/guides/context-engineering/hooks.md`), blog
<https://goose-docs.ai/blog/2026/05/14/goose-hooks>. Modelled on the Open Plugins hooks spec
<https://open-plugins.com/agent-builders/components/hooks>. VERIFIED: `goose@1.36 crates/goose/src/hooks/mod.rs:1-24`.

### 3.1 Loading and shape (both versions)

- Hooks live in `<plugin-root>/hooks/hooks.json` of an enabled plugin; plugins are discovered from
  `<project>/.agents/plugins/<name>/` and the user plugin dir `~/.agents/plugins/<name>/` (or
  `$GOOSE_PATH_ROOT/.agents/plugins` in tests). VERIFIED: `goose@1.36 crates/goose/src/hooks/mod.rs:229-270`,
  `crates/goose/src/plugins/discovery.rs:41-75,96-98`, `crates/goose/src/plugins/mod.rs:33-39`,
  `crates/goose/src/config/paths.rs:8-35` (Plugins dir = `home/.agents/plugins`; `GOOSE_PATH_ROOT` override).
  Docs table: `hooks.md:19-38`.
- Disable with `disabledPlugins` in `~/.config/goose/settings.json` or `<project>/.config/goose/settings.json`
  (also `enabledPlugins`; local > project > user). VERIFIED: `goose@1.36 crates/goose/src/plugins/discovery.rs:22-30,65-90`,
  `hooks.md:481-497`.
- Schema: `{"hooks": {"<Event>": [{"matcher": "<regex>", "hooks": [{"type": "command", "command": "...",
  "timeout": 30}]}]}}`. Only `type: "command"`; unknown events and action types are ignored; the matcher is an
  unanchored regex (an invalid regex such as `*` skips the rule). Command runs as `sh -c`, `${PLUGIN_ROOT}`
  expanded and exported, payload JSON on stdin, default timeout 30 s, `kill_on_drop`. VERIFIED:
  `goose@1.36 crates/goose/src/hooks/mod.rs:41-42,110-151,426-531`; `hooks.md:97-152`.
- Hooks are loaded once per `HookManager::load` (agent construction). INFERRED that adding a plugin needs a new
  session or process; not checked.
- Main adds `on_failure: allow|block` (PreToolUse only). VERIFIED: `hooks.md:127,280-317`.

### 3.2 Events

| Event | 1.36 | main | Can block? | Output reaches model? |
|---|---|---|---|---|
| `SessionStart` | emitted (CLI) | emitted | no | main: a `{"banner": "..."}` on stdout is shown to the user and stored in a session-start message (see 3.4) |
| `SessionEnd` | emitted (CLI) | emitted | no | no |
| `UserPromptSubmit` | emitted before each reply | emitted at turn start and per drained steer message | no | no |
| `PreToolUse` | emitted, blocking | emitted, blocking | **yes** | **deny reason is returned to the model as the tool error** |
| `PreToolUseResult` | absent | emitted (observation) | no | no |
| `PostToolUse` / `PostToolUseFailure` | emitted | emitted | no | no |
| `BeforeReadFile`, `BeforeShellExecution` | emitted (observation) | emitted | no | no |
| `AfterFileEdit`, `AfterShellExecution` | emitted after success | emitted | no | no |
| `Stop` | emitted **fire-and-forget** after the reply stream | emitted **blocking** | main: **yes** | main: **the block reason is injected as a hidden user message and the turn continues** |
| `SubagentStart` / `SubagentStop` | parsed, never emitted | removed from the enum | - | - |

Evidence (VERIFIED):
- 1.36 enum and emit sites: `goose@1.36 crates/goose/src/hooks/mod.rs:48-63`; CLI SessionStart/End
  `crates/goose-cli/src/session/mod.rs:482,488,1054,1061`; UserPromptSubmit
  `crates/goose/src/agents/agent.rs:1392-1404` (plain `emit`, result discarded); PreToolUse
  `agent.rs:880-911`; Post*/After* `agent.rs:420-460`; Stop `agent.rs:2286` (`self.emit_hook(Stop, ..)`, a
  non-blocking `emit`).
- main enum: `goose@main crates/goose/src/hooks/mod.rs:54-67`; Stop: `crates/goose/src/agents/state_machine/ops_stop_hook.rs:80-119`;
  UserPromptSubmit: `crates/goose/src/agents/state_machine/session.rs:277-325`, `ops_steer.rs:89-95`;
  PreToolUse/Result: `ops_toolcalling.rs:100-135`; Subagent events not emitted: `hooks.md:512`.
- "Most events are observation-only ... **`PreToolUse` and `Stop` can block** ... a block decision from those
  events is ignored": `hooks.md:244`.

### 3.3 Payload (stdin)

`{event, session_id, matcher_context, tool_name?, tool_input?, tool_output?, message?, working_dir?}` in 1.36;
main adds `tool_call_id`, `last_assistant_message` (Stop), `decision`, `policy_evaluated`, `blocked_by`,
`reason`, `cause` (PreToolUseResult). Note `tool_output` is declared but the 1.36 Post* sites do not set it.
VERIFIED: `goose@1.36 crates/goose/src/hooks/mod.rs:153-215`, `agent.rs:436-439`; `goose@main hooks/mod.rs:216-258`;
`hooks.md:160-213`.

No `transcript_path`, no pane/tty, no parent pid in the payload. The only identity is `session_id`
(VERIFIED, same lines). INFERRED: a hook command inherits goose's process environment (tokio `Command` does not
clear env, `hooks/mod.rs:507-516` sets only `PLUGIN_ROOT`), so `HERDR_PANE_ID` from the pane would be visible
to it.

### 3.4 Decision protocol and what can reach the model

- Deny: exit 2 (reason from stderr) or stdout `{"decision":"block","reason":"..."}`; in 1.36 anything else is
  allow; in main, exit 0 with empty stdout or `{"decision":"allow"}` is allow and anything else is a "hook
  failure" (fail-open unless `on_failure: block`). VERIFIED: `goose@1.36 hooks/mod.rs:338-424`;
  `goose@main hooks/mod.rs:897-950`, `hooks.md:261-278`.
- **There is no `additionalContext` / `systemMessage` / context-injection field on any event in either version.**
  Grepping both `hooks/mod.rs` finds only `decision`, `reason` and (main) `banner` keys. VERIFIED.
- Ways hook output nevertheless reaches the model:
  1. **PreToolUse deny reason** -> tool result error text
     ``Tool call denied by policy hook `<plugin>`: <reason>. Do not retry; this is a policy denial, not a transient failure.``
     VERIFIED: `goose@1.36 agent.rs:895-910`, `hooks.md:253-257`. Only at a tool call, it destroys that tool
     call, and the wording tells the model it was a policy denial. INFERRED: unusable for delivery in practice.
  2. **Stop block (main only)**: the reason becomes a user message with visibility (user=false, agent=true):
     ``Stop hook `<plugin>` blocked ending this turn:\n\n<reason>\n\nAddress this policy hook denial before trying to stop again.``
     plus an inline notification to the user; the turn continues. Consecutive blocks are capped (default 8,
     `GOOSE_STOP_HOOK_BLOCK_CAP`), after which goose ends the turn with a warning. A Stop hook is always
     fail-open. VERIFIED: `goose@main ops_stop_hook.rs:19-45,95-117`, `crates/goose/src/agents/agent.rs:56`,
     `hooks.md:259,315`. This is a Claude-style "block and continue" turn-end delivery path, but the framing
     ("policy hook denial") is fixed and not ours, and it fires only when a turn is ending, never on an idle
     session (INFERRED from `ends_turn` gate, `ops_stop_hook.rs:86-89`).
  3. **SessionStart banner (main only)**: stdout `{"banner": "..."}` is collected, shown before the first prompt,
     and written into a session-start message. VERIFIED: `goose@main hooks/mod.rs:606-680,873-885`,
     `agents/agent.rs:334-353`. Whether that stored message is agent-visible: unknown (not traced into
     `session_start_message`).

## 4. Plugins

Docs: <https://goose-docs.ai/docs/guides/context-engineering/plugins>. A plugin bundles skills (`skills/*/SKILL.md`)
and hooks (`hooks/hooks.json`). Formats: Open Plugins (`plugin.json`, `.plugin/plugin.json`,
`.goose-plugin/plugin.json`) and Gemini extensions (`gemini-extension.json`). `goose plugin install <git-url>
[--auto-update]`, `goose plugin update`. VERIFIED: `goose@main documentation/docs/guides/context-engineering/plugins.md:9-24,195-206`;
`goose@1.36 crates/goose/src/plugins/formats/{open_plugins.rs,gemini.rs}`, `open_plugins.rs:21`
(component markers `hooks/hooks.json`, `commands`, `agents`); observed `goose plugin --help`.
There is no in-process plugin code API (unlike Pi/OpenCode): plugin = files + shell hooks. VERIFIED by the
component list above.

## 5. MOIM / "Top Of Mind": per-call context injection from a file

This is the most useful injection surface Goose has for us.

- Goose builds a "MOIM" block (`<info-msg>` in 1.36, a turn-context block in main) with time, working dir,
  token usage and parts contributed by **platform** extensions, and adds it to the model request as a user
  message. VERIFIED: `goose@1.36 crates/goose/src/agents/moim.rs:12-52`,
  `crates/goose/src/agents/extension_manager.rs:1889-1966`.
- The platform extension `tom` ("Top Of Mind"), **enabled by default**, contributes
  `GOOSE_MOIM_MESSAGE_TEXT` (literal) and the contents of the file at `GOOSE_MOIM_MESSAGE_FILE` (`~` expanded),
  re-read on every collection, each capped at 64 KiB with UTF-8-safe truncation. VERIFIED:
  `goose@1.36 crates/goose/src/agents/platform_extensions/tom.rs:13-15,63-110`,
  `crates/goose/src/agents/platform_extensions/mod.rs:178-190` (`default_enabled: true`). In 1.36 the vars are read
  with `std::env::var` (process env only); in main with `Config::global().get_param` (env or config file).
  VERIFIED: diff of `tom.rs` between the trees. Docs:
  <https://goose-docs.ai/docs/guides/context-engineering/using-persistent-instructions> ("re-read and injected
  fresh with every interaction ... Changes take effect immediately", lines 9-29).
- **When it is injected:**
  - 1.36: `inject_moim` runs before **every provider call** in the loop, i.e. at turn start and again after
    each batch of tool results; the injected message is inserted before the last assistant message, merged
    into the adjacent user message, and is **ephemeral** (only the provider request carries it; the stored
    conversation does not). VERIFIED: `goose@1.36 agent.rs:1671-1682` (only `conversation_with_moim.messages()`
    goes to the provider), `moim.rs:26-49`.
  - main: parts are collected at every inference preparation; a new turn-context message (agent-only, marked
    turn-context) is appended **only if its text differs from the last one in this turn**, and is persisted so
    "later requests in the turn reuse the same bytes" (prompt caching). The system prompt tells the model
    "Blocks from earlier turns stay in the conversation history; only the most recent block is current".
    VERIFIED: `goose@main crates/goose/src/agents/state_machine/inference_preparation.rs:64-84`,
    `crates/goose/src/agents/moim.rs:1-30,90-106`, `crates/goose/src/agents/extension_manager/lease.rs:577-591`.
  - Skipped entirely when the model's context limit is < 32,000 tokens. VERIFIED: `goose@1.36 extension_manager.rs:1894-1901`,
    `goose@main moim.rs` (`MIN_CONTEXT_FOR_MOIM`).
- Only platform (in-process) extensions contribute; external MCP servers cannot (the `McpClient` default
  `get_moim` returns `None`, and main's lease filters `is_platform()`). VERIFIED: `goose@1.36 mcp_client.rs:135-137`,
  `extension_manager.rs:1934-1952`; `goose@main mcp_client.rs:139-141`, `lease.rs:583`.
- Semantics for delivery (INFERRED): a file the herdr-threads daemon rewrites with the pending items gives
  **in-turn, mid-turn (after each tool batch) delivery with no model tool call**, the analogue of Claude's
  post-tool hook context. There is no ACK or de-dup from goose: the file content is re-sent on every call
  (1.36) or every change (main) until we clear it, so the daemon must decide when an item counts as displayed
  (for example, clear after the next `PostToolUse`/`Stop` hook fires for that session). The env var must be in
  goose's environment at launch (1.36), which suits a managed launch, not an already-running session. It does
  not wake an idle session.

## 6. MCP client support

Goose is an MCP client for extensions (stdio, streamable HTTP; SSE removed). Relevant capabilities:

| Capability | 1.36 | main |
|---|---|---|
| tools | yes | yes |
| `notifications/tools/list_changed` | not handled (rmcp default no-op) | handled: bumps the tools version so the cached list is refetched |
| resources | model-pull only, via platform tools `list_resources` / `read_resource`; no `resources/subscribe`, no `resources/updated` handling | same (no subscribe handler found) |
| prompts | `/prompts`, `/prompt NAME k=v` in the CLI | same |
| sampling (`sampling/createMessage`) | **supported**: runs a one-off completion with goose's provider, outside the session conversation | capability no longer advertised (`create_message` gone from `GooseClient`) |
| elicitation | supported (form and URL); CLI shows a prompt, 300 s timeout | supported |
| roots | working dir | working dir |
| `notifications/message` (logging), `notifications/progress` | forwarded **to the UI only** (CLI log lines / spinners, `stream-json` `Notification` events) | same |

Evidence (VERIFIED): `goose@1.36 crates/goose/src/agents/mcp_client.rs:245-440` (handlers `list_roots`,
`on_progress`, `on_logging_message`, `create_message`, `create_elicitation`; `get_info` enables roots,
sampling, elicitation; protocol `2025-03-26`); `goose@main mcp_client.rs:336-370` (`on_tool_list_changed` at 358;
`get_info` enables roots, extensions, elicitation, no sampling), `extension_manager/mod.rs:135`;
`goose@1.36 crates/goose/src/agents/platform_extensions/ext_manager.rs:65-66,319-345,427`;
CLI rendering `goose@1.36 crates/goose-cli/src/session/mod.rs:1226,1757-1858`.

**Server-pushed notifications never reach the model and are only seen during a tool call.** The extension
manager subscribes to a client's notifications per tool dispatch, and the agent forwards them as
`AgentEvent::McpNotification` for display. A notification sent while no tool of that server is running goes to
no subscriber. VERIFIED: `goose@1.36 crates/goose/src/agents/extension_manager.rs:1663` (subscribe inside
`dispatch_tool_call`), `mcp_client.rs:253-284` (`try_send` to current handlers), `agent.rs:1885-1918`.
INFERRED consequence: no MCP-channel wake (nothing like Claude's channels) and no MCP push into context.

Stdio MCP extensions get `AGENT_SESSION_ID=<goose session id>` in their env (both versions). VERIFIED:
`goose@1.36 extension_manager.rs:952-954`, `goose@main extension_manager/mod.rs:867`. A long-poll MCP tool
(blocking `wait_for_messages`) is possible as for any MCP host; tool timeout is per extension (`timeout`
in the extension config; the CLI accepts `url... timeout=N`, `goose@1.36 crates/goose-cli/src/cli.rs:174`; the default value is unknown).

## 7. ACP server (editor/SDK mode)

Docs: <https://goose-docs.ai/docs/guides/goose-cli-commands> (`acp`, `serve` sections,
`goose-cli-commands.md:702-760` on main), <https://goose-docs.ai/docs/gdk/acp>.

- Standard ACP: `session/new`, `session/load` (resume by id), `session/prompt`, `session/cancel`, streamed
  `session/update`. VERIFIED: `goose@main crates/goose/src/acp/server.rs:2271-2382,2426-2443`;
  `goose@1.36 crates/goose/src/acp/server.rs:2522,2775`.
- One active run per session: a second `session/prompt` while busy fails with
  ``session already has active run `<id>`; use _goose/unstable/session/steer`` (main; the guard spans
  connections). VERIFIED: `goose@main acp/server.rs:1884-1912,2291-2300`. 1.36: unknown.
- **Steering (main only)**: `_goose/unstable/session/steer {sessionId, prompt: ContentBlock[], expectedRunId}`
  queues user input into the **running** turn; the steer queue is drained between turns or right after a tool
  result (`ends_turn || last role == Tool`), each drained message fires `UserPromptSubmit`, and the client gets
  the same `messageId` back on the streamed `UserMessageChunk` with `_meta.goose.steer` (pickup correlation).
  VERIFIED: `goose@main crates/goose-sdk-types/src/custom_requests.rs:220-242`,
  `acp/server.rs:2384-2424`, `agents/state_machine/ops_steer.rs:60-99`, `agents/agent.rs:355-389`. This is a
  real mid-turn injection API with an acknowledgement id, but only for sessions hosted by an ACP server, not
  for a `goose session` in a terminal (the CLI never calls `steer`: `goose@main` grep of `.steer(` finds only
  ACP and live-voice callers).
- **System prompt edits (main only)**: `_goose/unstable/session/system-prompt/set {sessionId, mode: set|append,
  key?, text}`. VERIFIED: `goose@main custom_requests.rs:200-218`. 1.36 has only the in-process
  `Agent::extend_system_prompt` (`goose@1.36 agent.rs:2291`).
- **Busy/idle observability**: the server sends a `SessionInfoUpdate` carrying the active run id in `_meta` at
  run start and `None` at run end. VERIFIED: `goose@main acp/server.rs:1981-1992,2318,2367`.
- Auth: `goose serve` requires `GOOSE_SERVER__SECRET_KEY` unless `--dangerously-unauthenticated` (main docs,
  `goose-cli-commands.md:731-760`). In 1.36 the `/acp` routes are mounted without the secret layer (the secret
  only guards the MCP-app proxy), so INFERRED unauthenticated on loopback:
  `goose@1.36 crates/goose/src/acp/transport/mod.rs:116-136`. The installed 1.36 `serve --help` shows no
  `--dangerously-unauthenticated` flag (observed).
- Keyring: unattended ACP needs `GOOSE_DISABLE_KEYRING=1` plus provider env keys (`goose-cli-commands.md:719-728`).

## 8. Sessions, resume and the shared store

- Store: SQLite `sessions.db` (WAL) under the data dir (`Paths::data_dir()`; `GOOSE_PATH_ROOT` relocates all
  dirs). VERIFIED: `goose@1.36 crates/goose/src/session/session_manager.rs:24,53,640-660`,
  `crates/goose/src/config/paths.rs:8`.
- Each reply **re-reads the conversation from the store** after adding the user message. VERIFIED:
  `goose@1.36 agent.rs:1475-1480` (`session_manager.get_session(&id, true)` then `conversation`).
  INFERRED: a message another process writes into `sessions.db` between turns is in the next turn's context;
  there is no session lock (grep finds none in `session/` or `goose-cli/src`), so two writers can interleave.
- Goose itself does this: `goose term log CMD` (run from a shell `DEBUG` trap / preexec) appends a user-only
  message to the shell's session; `goose term run PROMPT` folds those messages into
  `<shell_history>...</shell_history>` and runs **headless with `resume: true`** on that session id
  (`AGENT_SESSION_ID`). VERIFIED: `goose@1.36 crates/goose-cli/src/commands/term.rs:40-55,230-317`;
  docs <https://goose-docs.ai/docs/guides/terminal-integration>.
- Resume/fork: `goose session -r [--session-id ID | -n NAME] [--fork] [--history]`, `goose run -r ...`;
  `goose session list --format json [-w DIR] [-l N]`, `goose session export --format json|yaml|markdown`
  (observed help).
- Headless re-launch as a wake (INFERRED): `goose run -r --session-id ID -t "<wake>"` would run one turn on an
  existing session in a **new process**; while a `goose session` holds that session in a pane this creates a
  second, unsynchronised agent on the same conversation. Only safe for sessions nobody has open.

## 9. Interactive CLI input (keystroke injection)

- Line editor: `rustyline` 18, Emacs mode by default (vi selectable), prompt string `"> "`, newline key
  Ctrl+J (`GOOSE_CLI_NEWLINE_KEY`), Enter submits. On an **empty** line a dim hint is drawn:
  `Enter to send · Ctrl+J newline` (after an interrupt: `Interrupted, what should goose work on instead?`;
  after one Ctrl+C on empty: `Press Ctrl+C again to exit, or type new instructions to continue`). VERIFIED:
  `goose@1.36 crates/goose-cli/src/session/input.rs:84-91,128-158`,
  `crates/goose-cli/src/session/completion.rs:422-450`, `mod.rs:546-553`, `Cargo.lock` (rustyline 18.0.0);
  main unchanged in substance (`goose@main input.rs:116`, `crates/goose-cli/Cargo.toml:50`).
- Bracketed paste: not configured by goose; rustyline's default enables it (INFERRED from rustyline docs,
  <https://docs.rs/rustyline/18.0.0/rustyline/config/struct.Builder.html#method.bracketed_paste>; not verified
  in source). A pasted multi-line block therefore lands as one buffer; Enter still has to be sent separately.
- Ctrl+C with text in the buffer **kills the whole buffer** (a human draft would be lost); Ctrl+C on an empty
  line arms "exit", a second one exits the session. Empty Enter is ignored (`Retry`); typing `exit`/`quit` ends
  the session. VERIFIED: `input.rs:56-81,154-173`.
- Optional external editor: `GOOSE_PROMPT_EDITOR` (+ `GOOSE_PROMPT_EDITOR_ALWAYS`) opens `$EDITOR` for every
  prompt instead of the inline line (a keystroke-injection target that is no longer the composer). VERIFIED:
  `input.rs:93-126`.
- While busy: the CLI does not read the terminal (no queueing; "Message queuing is not available in the goose
  CLI", in-session-actions.md:166-168); only a `tokio::signal::ctrl_c` task listens and cancels the run.
  VERIFIED: `goose@1.36 mod.rs:1086-1092`. INFERRED: bytes typed (or injected) while busy sit in the tty buffer,
  are echoed by the cooked-mode tty into the output, and are read by rustyline as the next prompt's input,
  so a keystroke wake sent during a turn becomes the next user message after the turn, not a mid-turn steer.
- Interactive prompts inside a turn: tool approvals in `GOOSE_MODE=approve|smart_approve` (default `auto`) and
  MCP elicitation forms are drawn as interactive selects; injected keys during them would answer them.
  VERIFIED: default `goose@1.36 crates/goose/src/config/goose_mode.rs:24-34`; elicitation UI
  `crates/goose-cli/src/session/elicitation.rs`. INFERRED hazard.
- Slash commands (1.36): `/exit` `/quit` `/help` `/?` `/t [theme]` `/r` (full tool output) `/prompts`
  `/prompt NAME` `/extension CMD` `/builtin NAMES` `/mode M` `/plan` `/endplan` `/clear` `/recipe [FILE]`
  `/compact` (`/summarize` deprecated) `/edit [TEXT]` `/skills [NAMES]`; main adds `/model`, `/new` and drops
  `/plan`. Unknown `/x` is sent as a normal message. VERIFIED: `goose@1.36 input.rs:190-300`,
  `goose@main input.rs:224-341`. Docs: <https://goose-docs.ai/docs/guides/sessions/in-session-actions>.
- Desktop (not a terminal) queues messages typed while busy and treats "stop/wait/..." keywords as
  interrupts (docs `in-session-actions.md:114-245`); irrelevant to Herdr panes but shows the UX goose chose.

## 10. Idle/busy observability

- CLI: no machine-readable state. The pane shows `"> "` plus the dim hint when idle, spinner / "thinking"
  messages while busy (`crates/goose-cli/src/session/thinking.rs`). VERIFIED for the strings; INFERRED that a
  screen reader can classify idle by the `> ` row with the hint. Herdr itself has no goose detection: the
  installed `herdr` binary contains no "goose" string (`strings` check). INFERRED.
- Hooks give edges: `UserPromptSubmit` (turn start), `Stop` (turn end; 1.36 fire-and-forget after the stream,
  main blocking), `SessionStart`/`SessionEnd`. INFERRED: a hook-maintained "busy until Stop" flag is feasible
  in both versions.
- Headless: `goose run --output-format stream-json` streams events, including MCP notifications
  (observed help; `mod.rs:1785-1840`).
- ACP: active-run `SessionInfoUpdate` (section 7).

## 11. Identity and session tracking

- Session id: in every hook payload (`session_id`), in stdio MCP extension env (`AGENT_SESSION_ID`), in the
  shell tool env **on main only** (`goose@main crates/goose/src/agents/platform_extensions/developer/shell.rs:612-632`
  sets it, or removes an inherited stale one). In 1.36 the developer shell tool sets only `PATH`
  (`goose@1.36 .../developer/shell.rs:623,651`), so a `herdr-threads` CLI run by the model cannot learn the
  goose session id from env on 1.36, except an `AGENT_SESSION_ID` inherited from `goose term init`, which names
  the shell's term session, not necessarily this one. VERIFIED (grep of both trees).
- Pane identity: INFERRED that goose passes its environment (including `HERDR_PANE_ID`) to hooks, stdio
  extensions (merged envs) and the shell tool, since none of those clear the env.
- Session names (`-n`) and working dirs are queryable through `goose session list --format json`.

## 12. What herdr-threads does for Goose today

Nothing. VERIFIED:
- The adapter registry has Claude, Codex, Hermes and agy only (`src/harness/registry.rs:1482-1490` (ht)); there
  is no `src/harness/goose*` and no `integrations/goose` (`ls src/harness integrations` (ht)).
- `grep -ri goose` over the repository's `*.rs, *.md, *.js, *.toml, *.tsv, *.txt` finds no match.
- The composer reader knows only Claude and Codex layouts (`src/harness/composer.rs:1-24` (ht)), so a Native
  wake into a goose pane has no draft-safety classification today.
- Adapter vocabulary a goose adapter would have to fill (`src/harness/adapter.rs:449-470` (ht)):
  `DeliveryMechanism::{PushChannel, HookPull}`, `IdleWake::{Native, Channel}`, `wake_text`.

Mapping notes for the later reports (all INFERRED):
- Turn-start/in-turn delivery: hooks cannot add context (section 3.4), so `HookPull` as built for Claude/agy does
  not transfer. The candidates are the `tom` MOIM file (in-turn after every tool batch, no tool call, needs the
  env var at launch, no ACK), the main-only Stop block (turn end, fixed "policy denial" framing, capped), and
  `inbox` round trips through the shell tool.
- Idle wake: no `Channel` equivalent for the terminal CLI (MCP notifications are not surfaced; steer exists only
  under ACP). `Native` typing of the content-free wake marker plus Enter into the `> ` prompt is the only way to
  wake a `goose session` pane; it needs a goose composer reading (empty = `> ` row with the dim hint), must never
  send Ctrl+C to clear a draft (it deletes it, and twice exits), and must avoid approval/elicitation prompts.
- ACP-hosted goose (`goose serve`, roaming) has the cleanest primitives of any harness here: `session/prompt`
  for idle wake, `steer` for mid-turn delivery with pickup ids, active-run updates for busy/idle. None of that
  applies to a goose CLI running in a Herdr pane.
