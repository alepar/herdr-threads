# Harness primer: Pi coding agent (id `pi`)

Shared basis for the alternative x harness reports. Scope: every Pi surface that can put text into the
model's context or wake an idle session, plus what herdr-threads does for Pi today.

## Provenance

- Upstream: <https://github.com/badlogic/pi-mono> (cloned; it now presents itself as
  <https://github.com/earendil-works/pi>, README.md:11,76). npm package `@earendil-works/pi-coding-agent`.
- Clone read: `scratchpad/alts/pi-mono`, commit `a7b94a03f09f20f4937fa1a76339014c60d261ad` (2026-10-23),
  `packages/coding-agent/package.json:3` version `1.4.0`. All `path:line` below are relative to the clone root
  unless they start with `src/`, `tests/` or `docs/research/` of herdr-threads (marked "(ht)").
- Docs site: <https://pi.dev>; the same docs are in `packages/coding-agent/docs/*.md` and are cited there.
- Not installed locally: `which pi` finds nothing, so no CLI help was observed. CLI facts come from
  `packages/coding-agent/docs/cli.md` (which says `pi --help` is authoritative, cli.md:5).
- Tags: VERIFIED = read in source or docs at the cited line. INFERRED = my reading, not stated.

## 1. Architecture in one paragraph

Pi is a single Node process (TUI, print, JSON or RPC mode) wrapping an `AgentSession`. Extensions are
TypeScript modules loaded **in-process** (via jiti) with full process permissions, and they get a
first-class API to send messages into the session and start turns. There is no hook-subprocess model like
Claude Code's: what Claude needs a shell hook plus stdout JSON for, Pi does through event handlers inside
the process. VERIFIED: `packages/coding-agent/docs/how-pi-works.md:43-45`,
`packages/coding-agent/docs/extensions.md:3-5,13-38`.

Agent loop: a run is a sequence of turns; one turn is one model request plus the tool calls from its
response. VERIFIED: `how-pi-works.md:7-13`, `extensions.md:62-92`. The low-level loop is
`packages/agent/src/agent-loop.ts:174-325`.

## 2. Extension surfaces (the "hooks")

### 2.1 Loading

- Locations: `~/.pi/agent/extensions/*.ts|js` or `<dir>/index.ts`, project `.pi/extensions/` (project ones
  only after project trust), configured paths in settings, Pi packages (`pi install npm:...|git:...`).
  VERIFIED: `extensions.md:15-48`, `docs/cli.md:250-267`, `how-pi-works.md:47-49`.
- **Per-process, no config write**: `pi -e <path>` / `--extension <path>` (repeatable) loads an extension
  for that process only; `-ne` disables discovered ones but explicit `-e` still loads. VERIFIED:
  `docs/cli.md:194-197,217`. Relevant to managed launch: an integration can be attached at launch without
  touching `~/.pi`. Extensions may also register CLI flags (`pi.registerFlag`, `getFlag`) VERIFIED:
  `src/core/extensions/types.ts:1690-1712` (clone path `packages/coding-agent/src/...`).
- `PI_CODING_AGENT_DIR` overrides the config dir (default `~/.pi/agent`). VERIFIED:
  `docs/environment-variables.md:81`. Useful for isolated tests.
- Rule: don't start sockets/watchers/timers in the factory; start them in `session_start`, release in
  `session_shutdown` (idempotent). Reload and `/new`, `/resume`, `/fork` replace the runtime. VERIFIED:
  `extensions.md:50,58-60,127-145`.

(All `src/core/...` and `docs/...` paths in this section are under `packages/coding-agent/`.)

### 2.2 Events (`pi.on(name, handler)`)

VERIFIED from `extensions.md:111-163` and `packages/coding-agent/src/core/extensions/types.ts`:

| Event | Fires | Can affect model context? | Notes |
|---|---|---|---|
| `session_start` | startup, reload, new, resume, fork (`reason`) | no (but can call `sendMessage`) | types.ts:755-761 |
| `session_shutdown` | quit, reload, new, resume, fork | no | types.ts:824-829 |
| `session_before_switch` / `_fork` / `_compact` | before those ops | can cancel | types.ts:770-795 |
| `input` | raw user input before expansion; `source` = `interactive`/`rpc`/`extension`; `streamingBehavior` set when busy | `transform` text, or `handled` (swallow) | types.ts:1167-1187 |
| `before_agent_start` | before a run; `messageType` `user` or `custom` | return `message` (a custom message added to the run) or `systemPrompt` (replace), or mutate `systemPromptOptions` | types.ts:932-957,1496-1500; extensions.md:216. **Closest analogue of UserPromptSubmit additionalContext.** |
| `agent_start` / `agent_end` | per low-level run | no | `agent_end` is NOT final idle (extensions.md:145) |
| `turn_start` | each turn | no | types.ts:1057-1062 |
| `context` | every provider request | chained transform of the message list, request-local | extensions.md:155,209,228; **ephemeral per-request injection, not persisted** |
| `context_with_system` | every provider request | full transcript incl. system msg | extensions.md:228 |
| `before_provider_headers` / `before_provider_request` / `after_provider_response` / `provider_stream_event` | provider stages | payload rewrite | extensions.md:94-102,222-224 |
| `tool_call` | before each tool | block / mutate input | types.ts:1454-1463 |
| `tool_result` | after each tool | chained replace of `content` (model-facing) | types.ts:1478-1489. **Analogue of PostToolUse additionalContext: append text to a tool result mid-turn.** |
| `tool_execution_start/update/end` | tool lifecycle | no | |
| `message_start/update/end` | message lifecycle | `message_end` can replace a finalized message | extensions.md:218 |
| `turn_end` | after a response + its tools | return `entries` (custom, custom_message, context_edit, compaction) and `continue: true` for one more request | types.ts:1015-1025,1064-1072; extensions.md:230 |
| `agent_before_settle` | when the agent would stop | same as `turn_end`: append entries, `continue: true` | types.ts:1027-1030. **Analogue of a Stop hook "block and continue".** Loops if unguarded (extensions.md:230). |
| `agent_settled` | no automatic work remains (`aborted` flag) | notification only | types.ts:1032-1037; **the true "now idle" signal** (extensions.md:145, `docs/json.md:48,56`) |
| `ui_prompt_start` / `ui_prompt_end` | an extension dialog blocks on the user | no | types.ts:1039-1055; a "blocked/awaiting human" signal |
| `user_bash` | `!cmd` from the user | can replace execution | extensions.md:240 |
| `cache_warming_decision` | idle prompt-cache refresh | `warm`/`stop` | extensions.md:232-234 |
| `mcp_servers_change`, `resources_discover`, `project_trust`, `model_select`, `thinking_level_select`, `session_info_changed`, `session_compact*` | misc | no | extensions.md:318; types.ts |

Handler composition: notification, in-place mutation, chained transformation, gate, actionable boundary,
last-override. Handlers run in load order; errors are reported and Pi continues, except a failing
`tool_call` handler blocks the tool. VERIFIED: `extensions.md:195-215,373-377`.

### 2.3 Actions: putting text in context and starting turns

This is the key surface. VERIFIED: `types.ts:1734-1751`, implementation
`packages/coding-agent/src/core/agent-session.ts:2344-2459`.

- `pi.sendMessage({customType, content, display, details}, {triggerTurn?, deliverAs?})`:
  a **custom message** (role `custom`). On the wire to the model it becomes a `user` message
  (`packages/coding-agent/src/core/messages.ts:162-169`); `display` controls only TUI rendering, `details`
  is never sent (`docs/message-types.md:235`). Persisted as a `CustomMessageEntry` in the session JSONL.
  Behaviour (agent-session.ts:2344-2393):
  - `deliverAs: "nextTurn"`: held and injected alongside the next run's starting message
    (agent-session.ts:2370-2371,1959-1961). Does not wake.
  - streaming and `triggerTurn !== false`: queued as **steer** (default) or **followUp** into the agent's
    queues (2372-2377).
  - idle and `triggerTurn: true`: **starts a run immediately** (2378-2383). This is a wake with no
    keystrokes. If called while `agent_settled` handlers run, it is deferred until they finish (2379-2381).
  - streaming and `triggerTurn: false`: appended after the current turn's tool results, no new turn
    (2384-2389).
  - idle, no trigger: appended to history silently (2390-2391); the model sees it on the next request.
- `pi.sendUserMessage(content, {deliverAs?: "steer"|"followUp", expandPromptTemplates?})`: a real user
  message "as if typed"; always triggers a turn when idle; **throws** if busy and `deliverAs` is omitted
  (agent-session.ts:2421-2459 delegating to `prompt()` 2092-2175, the throw at 2137-2142; example
  `packages/coding-agent/examples/extensions/send-user-message.ts:67-75`). `prompt()` also throws during
  compaction (2110-2114).
- `pi.appendEntry(customType, data)`: persisted, **not** sent to the model (types.ts:1750-1751).
- `before_agent_start` result `message`, `turn_end`/`agent_before_settle` `entries` (`custom_message`),
  `context` transforms, `tool_result` content replacement: see 2.2.

Delivery timing (VERIFIED):
- Steering is polled at loop start, after a long `prepareNextTurn`, and after each complete tool batch,
  before the next model request (`packages/agent/src/agent-loop.ts:184-185,210-215,304`). It does **not**
  interrupt a running tool or skip remaining tool calls of the same assistant message (INFERRED from the
  absence of polling inside `executeToolCalls`, agent-loop.ts:276-281).
- Follow-ups are polled only when the agent would otherwise stop (agent-loop.ts:310-317).
- Queue modes `steeringMode` / `followUpMode`: `one-at-a-time` (default) or `all`
  (`docs/settings.md:31-32`, `packages/agent/src/agent.ts:255-256`; `docs/rpc-commands.md:329-363`).
  With the default, N queued injections are delivered one per turn. (The `get_state` example at
  rpc-commands.md:174 shows `"all"`; that is an example value, not the default.)
- "Pi counts as busy while a run starts ... so messages sent meanwhile go into the starting run"
  (`extensions.md:123`).

Bounding/framing/dedup: Pi applies **no** truncation, framing, de-duplication or acknowledgement to
`sendMessage`/`sendUserMessage` content (VERIFIED by reading agent-session.ts:2357-2459: content is passed
through). All of that is the extension's job. Tool results have their own guidance ("truncate large
model-facing results", extensions.md:257) and MCP text results are middle-truncated over 20 KB
(`docs/mcp.md:232`).

### 2.4 Idle/busy observability from inside

VERIFIED (`types.ts:325-365`, agent-session.ts:1478-1486):
- `ctx.isIdle()`: no active run and not compacting. `isStreaming` covers a run *and* post-run continuation.
- `ctx.hasPendingMessages()`, `ctx.signal` (defined only while streaming), `ctx.abort()`.
- Events: `agent_start`, `agent_settled` (idle), `ui_prompt_start/end` (waiting on a human dialog),
  `session_compact*`.
- **Composer draft**: `ctx.ui.getEditorText()`, `setEditorText()`, `pasteToEditor()` (types.ts:230-237),
  and `ctx.ui.onTerminalInput(handler)` (types.ts:163) to observe raw keystrokes. An extension can
  therefore refuse to wake while the human has a draft, without screen scraping. INFERRED that this is
  reliable in TUI mode; in RPC mode `getEditorText()` returns `""` (`docs/rpc-extension-ui.md:19`).
- `ctx.mode` (`tui`/`rpc`/`json`/`print`) and `ctx.hasUI` (extensions.md:363-366).
- `pi.events`: an in-process bus between extensions (extensions.md:186).

### 2.5 Worked upstream example: external wake via file watch

`packages/coding-agent/examples/extensions/file-trigger.ts:14-41` watches `/tmp/agent-trigger.txt` from
`session_start` and calls `pi.sendMessage({customType:"file-trigger", content, display:true},
{triggerTurn:true})`, then truncates the file. VERIFIED. This is the upstream-blessed pattern for "an
external system sends the agent a message": in-process watcher + `sendMessage(triggerTurn)`. It has no
idle/draft check, no dedup beyond truncating the file, and a race between read and truncate (INFERRED).

`examples/extensions/notify.ts:189-194` uses `agent_settled` for "ready for input" (VERIFIED).

## 3. MCP

VERIFIED from `docs/mcp.md` and `packages/coding-agent/src/extensions/mcp/runtime.ts`,
`packages/mcp/src/client.ts`:

- Built-in client (a built-in extension, `builtin:mcp`), stdio and streamable HTTP; legacy SSE rejected
  (mcp.md:3,62-64,77-82). Config `~/.pi/agent/mcp.json`, project `.pi/mcp.json` after trust (mcp.md:32);
  `pi mcp add|remove|list|login|logout` (cli.md:318-333). Extensions can add servers per session with
  `pi.registerMcpServer` without writing config (extensions.md:309-318, mcp.md:258-262).
- Tools: named `mcp__<server>__<tool>`; exposure `codemode` (default: callable only from codemode
  scripts, **not declared to the model**), `deferred` (via `tool_search`), `direct`, `hidden`
  (mcp.md:77,189-230). An integration that wants the model to see its tools must set `direct`.
- Resources: exposed only as **pull** tools `list_mcp_resources`, `list_mcp_resource_templates`,
  `read_mcp_resource` (mcp.md:236-248).
- Client capabilities sent in `initialize`: only `roots` (client.ts:176-180,217-218; runtime.ts:360-365).
  **No sampling, no elicitation.** VERIFIED.
- Notifications handled: `notifications/message` (appended to `~/.pi/agent/mcp.log`, never to the model;
  runtime.ts:367, mcp.md:98) and `notifications/tools/list_changed` (refresh tools; runtime.ts:380-382).
  `resources/updated`, `resources/list_changed`, prompts: not handled (grep of `packages/coding-agent/src`
  and `packages/mcp/src` finds no handler or `prompts/*` call). VERIFIED by absence in this commit.
- Consequence: **an MCP server cannot push text into Pi's context or wake it.** MCP is pull-only; only an
  extension can push. Progress notifications only reset the request timeout (mcp.md:68).
- A long-poll MCP tool would work as a blocking "wait for mail" tool while the model is in a turn
  (INFERRED; default request timeout 60 s, progress notifications reset it, mcp.md:68).
- Replaceable: an extension registering `/mcp` (e.g. `pi-mcp-adapter`) replaces built-in MCP
  (mcp.md:264-268). SDK sessions do not load built-in MCP (mcp.md:272).

## 4. Headless, RPC, SDK, sessions

- Modes: interactive TUI; `-p/--print` one-shot; `--mode json` one-shot JSONL events; `--mode rpc`
  long-lived JSONL over stdin/stdout. Non-TTY stdio selects print. VERIFIED: `docs/cli.md:21-51`,
  `docs/cli-integration.md:1-25`.
- **RPC** (`docs/rpc.md`, `docs/rpc-commands.md`): commands `prompt` (with `streamingBehavior`
  `steer`/`followUp`, required when busy, rpc-commands.md:7-44), `steer`, `follow_up`, `abort`,
  `clear_queue`, `new_session`, `get_state` (`isStreaming`, `isCompacting`, `pendingMessageCount`,
  `sessionFile`, `sessionId`; 155-186), `get_messages`, `switch_session`, `fork`, `clone`, `bash`,
  `compact`, modes, `get_commands`. Events: the JSON stream incl. `queue_update`, `agent_settled`
  (`docs/json.md:31-60,117-126`). Extension UI subprotocol including fire-and-forget `set_editor_text`
  (`docs/rpc-extension-ui.md:8,159-170`). RPC is the process owner's channel: it replaces the TUI; there
  is **no** way to attach RPC to an already running interactive Pi. VERIFIED: no server/socket in
  `packages/coding-agent/src` other than the OAuth callback (`src/extensions/mcp/oauth.ts:461-464`).
- **SDK**: `createAgentSession()` → `session.prompt/steer/followUp/subscribe` in-process
  (`docs/sdk.md:8-86`). Same in-process-only caveat.
- **ACP**: not present in this commit (no `acp` match in `packages/coding-agent/src` or docs). VERIFIED by
  absence.
- **Sessions**: JSONL tree files under `~/.pi/agent/sessions/` grouped by cwd (`docs/sessions.md`,
  `how-pi-works.md:21-27`). `-c/--continue`, `-r/--resume`, `--session <path|id>`, **`--session-id <id>`
  (open exact id or create it)**, `--fork`, `--session-dir`, `--no-session`, `-n/--name`
  (`docs/cli.md:78-107`). `--session-id` lets a launcher choose the session id up front (INFERRED useful
  for deterministic seat binding). No session-file locking found (`grep -i lock` in
  `src/core/session-manager.ts` finds nothing relevant), so two processes on one session is unknown/unsafe.
- Headless "wake" by re-launch: `pi -p --session <id> "msg"` or `--continue` would append a turn to a
  session (INFERRED), but not while an interactive Pi owns that session (unknown behaviour, no lock).

## 5. Identity and environment

VERIFIED `docs/environment-variables.md:11-49`, `packages/coding-agent/src/core/tools/bash.ts:194-203`:
- Process markers inherited by children: `AI_AGENT=pi`, `PI_CODING_AGENT=true`.
- LLM-called `bash`/`powershell` tools get `PI_SESSION_ID`, `PI_SESSION_FILE` (unset for ephemeral),
  `PI_PROVIDER`, `PI_MODEL`, `PI_REASONING_LEVEL`, resolved per command; **not** for user `!`/`!!`
  commands. So a CLI like `herdr-threads` called by the model can attribute the call to a Pi session id.
- Inside an extension: `ctx.sessionManager.getSessionId()`, session file, `session_start.reason` and
  `previousSessionFile` for `/new`/`/resume`/`/fork` continuity (types.ts:755-761).

## 6. TUI input (keystroke injection)

VERIFIED unless marked:
- Enter submits; Shift+Enter / Ctrl+J newline; **Alt+Enter queues a follow-up**; Alt+Up dequeues;
  Escape aborts and returns queued messages to the editor (`docs/usage.md:11-40`,
  `docs/keybindings.md:84-97,120-130,160-167`).
- Typing + Enter **while busy becomes a steer**, not a rejected input (`docs/usage.md:35,40`;
  `src/modes/interactive/interactive-mode.ts:3333-3338`). During compaction input is queued as steer
  (3321-3328). So a typed wake marker never gets lost, but injected keystrokes into a busy Pi steer the
  current task (INFERRED: matters for delivery-contract rule 4).
- Bracketed paste is enabled (`packages/tui/src/terminal.ts:211-212`); pastes > 10 lines or > 1000 chars
  collapse to a `[paste #N ...]` marker in the editor and are expanded on submit
  (`packages/tui/src/components/editor.ts:1298-1316,1092-1093`).
- Keyboard protocols: Kitty protocol negotiation, xterm modifyOtherKeys / CSI-u for tmux
  (`docs/tmux.md:1-55`; `packages/tui/src/terminal.ts:255-262`). INFERRED: a plain `\r` from Herdr is
  still Enter.
- Draft clobber: injected text is inserted at the cursor of whatever draft exists, then Enter submits the
  combined text (INFERRED from editor insertion behaviour; not tested). An in-process extension avoids
  this via `getEditorText()` (2.4).
- Fullscreen (alt screen, default) vs `--tui-mode regular` (`docs/cli.md:233-234`, `docs/usage.md:86`):
  affects screen-scraping of the composer (INFERRED).

## 7. External idle/busy observability

- **OSC 7501 Program Status**: Pi reports `working`/`blocked`/`done`/`error`/`idle` to terminals that
  answer the support query; never prompt content; tmux/screen do not forward it; `PI_PROGRAM_STATUS=1`
  forces it (`docs/terminal-setup.md:221-233`, `packages/tui/src/program-status.ts:2-52`,
  `src/modes/interactive/program-status-reporter.ts:12`). Unknown whether Herdr answers the query or
  consumes the reports.
- Herdr pane status: herdr-threads test fixtures feed a Herdr pane with agent kind `"pi"`
  (ht `src/host/native.rs:5990`, ht `tests/harness/launch.rs:1567-1570`), so Herdr apparently detects
  Pi panes (INFERRED from fixtures; not verified against Herdr source).
- Window title via `ctx.ui.setTitle` (types.ts:210-211); the `titlebar-spinner.ts` /
  `working-indicator.ts` examples (listing in `packages/coding-agent/examples/extensions/`).

## 8. What herdr-threads does for Pi today

VERIFIED (ht = this repository):
- No Pi adapter: `builtins()` registers claude, codex, hermes, agy only (ht
  `src/harness/registry.rs:1482-1490`); no `integrations/pi`, no `src/harness/pi*`.
- A Herdr pane reporting kind `pi` maps to no registered parser, so the host returns
  `HostUiState::Unknown` with no composer read (ht `src/host/native.rs:2029-2047`; test
  `blocked_and_unrecognized_panes_make_no_composer_read`, ht `src/host/native.rs:5984-5995`).
- Launch treats a `pi` pane as "not a live bound agent" (ht `tests/harness/launch.rs:1567-1580`).
- Prior research noted only Pi's `session_start` reasons and the repo-identity change (ht
  `docs/research/harness-adapters-2026-10-03/other-harnesses-and-prior-art.md:9,25,34`).
- Practical consequence (INFERRED): a Pi agent can run the `herdr-threads` CLI through its `bash` tool and
  read its inbox, but gets no in-turn delivery and no wake.

## 9. Mapping to herdr-threads' delivery contract (INFERRED, for later reports)

| Need | Pi mechanism | Policy class |
|---|---|---|
| Idle wake, content-free or with content | extension watcher + `pi.sendMessage(..., {triggerTurn:true})` when `ctx.isIdle()` and editor empty | `Channel`-like (harness wakes itself, like the Claude mod), no keystrokes |
| In-turn delivery after a tool batch | `sendMessage` while streaming (default steer) or `tool_result` content append | rule 3 |
| Turn-start delivery | `before_agent_start` returning `message` | rule 3 |
| Silent arrival for next turn | `sendMessage` without trigger (idle) or `deliverAs:"nextTurn"` | rule 2 (no wake) |
| Busy detection, never nudge busy | `isIdle()`, `agent_start`/`agent_settled`, `ui_prompt_*` | rule 4 |
| Human draft protection | `ctx.ui.getEditorText()` / `onTerminalInput` | rule 4/6 |
| Attach without config writes | `pi -e <ext>` at managed launch; `PI_CODING_AGENT_DIR` for tests | setup |

Caveats to carry into reports: custom messages reach the model as plain `user` role (no system/tool
framing, messages.ts:162-169); queue mode `one-at-a-time` spreads bursts over turns; `prompt()` and
`sendUserMessage` throw during compaction; MCP cannot push; no out-of-process control channel for an
interactive Pi other than keystrokes or an extension-owned socket.
