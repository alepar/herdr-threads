# Harness primer: Claude Code (`claude`)

This primer lists every Claude Code surface that can put text into the model's context or wake an idle session. The later alternative x harness reports use it as their shared reference. The primer is about the harness itself. Section 9 says briefly what herdr-threads uses today.

- Observed locally: `claude --version` printed `2.1.309 (Claude Code)` (`/var/home/alepar/.local/bin/claude`). The mod API types came from the bundled `plugin-authoring` skill for build 2.1.308 (`/tmp/claude-1001/bundled-skills/2.1.308/.../plugin-authoring/types/claude-code.d.ts`, 22,324 lines, written by the engine when the skill loaded) and from its `reference.md`.
- Docs were read on 2026-10-23 from `https://code.claude.com/docs/en/*` (index: `https://code.claude.com/docs/llms.txt`).
- Tags: **VERIFIED** means read in docs, CLI help, or the local types or source. **INFERRED** means reasoned from those and not observed. **unknown** means not found.
- No Claude config was written. Only `--help` and `--version` were run.

---

## 0. Summary: which surfaces inject and which wake

| Surface | Injects into context | Wakes an idle session | Mid-turn delivery | Notes |
|---|---|---|---|---|
| Settings/plugin **hooks** (`SessionStart`, `UserPromptSubmit`, `PreToolUse`/`PostToolUse` `additionalContext`, `Stop` block) | yes (system reminder) | **no**, except `asyncRewake` exit 2 | yes, beside tool results | 10,000-char cap per field |
| **Async hook with `asyncRewake: true`** | yes (stderr/stdout as system reminder) | **yes**: "wakes Claude immediately even when the session is idle" | n/a | `timeout` is enforced (default 600 s) |
| **Mods** (function hooks, `hooks/hooks.json` `modules`) | `tool.call` result `context`, `prompt.submit` `context`, `$.session.append` hidden user rows, `prompt.compose` system-prompt sections | **yes**: `$.prompt.submit` queues a turn that starts once idle | yes: `tool.call` `context`, `session.append` rows enter the "loop's next top" | herdr-threads' current path |
| **Native cross-session messaging** (`SendMessage`/`ListAgents`, per-session UDS inbox `CLAUDE_CODE_MESSAGING_SOCKET`) | yes, plain text with sender name and reply address | **yes**: "When the receiving session is idle, Claude Code starts a new turn with the message" | yes: "between tool calls during an active turn" | Requires v2.1.224 or later. Inbound controls: accept/hold/refuse |
| **Channels** (MCP server with `claude/channel` capability, `notifications/claude/channel`) | yes, `<channel source=... k="v">body</channel>` | yes (INFERRED from docs; see §4) | queued; "delivered together on the next turn" if busy | Research preview, allowlist, `--channels` flag, claude.ai or Console auth |
| **Plugin monitors** / Monitor tool | yes: each stdout line becomes a notification | Monitor: "Claude interjects when an event arrives" (wake on idle INFERRED) | yes | Interactive only. Monitor-tool watches have a 5 min default and 30 min max deadline. A plugin monitor runs for the whole session |
| **Cron / `/loop` / `ScheduleWakeup`** | stored prompt | yes: fires "between your turns" while idle | no (waits for turn end) | Session-scoped. Recurring tasks expire after 7 days |
| **Keystroke injection** into the TUI (tmux/Herdr typing) | yes, as a user prompt | yes when idle | when busy, typed Enter is **queued** and passed "as soon as those tool calls finish, within the same turn" | Pastes over 800 chars or 3 lines are collapsed and marked as pasted |
| **Headless / SDK** (`-p`, `--input-format stream-json`, Agent SDK streaming input) | user messages from the host | host-driven | queued messages "process sequentially" | Host owns the process. Not usable for an existing TUI pane |
| **Resume** (`--resume <id>`, `--continue`, `claude --bg --resume`, `claude attach`) | new prompt into an old transcript | relaunch | n/a | Background sessions are run by a supervisor (`claude daemon`) |
| **MCP tools/resources** | only when the model calls a tool or the user @-mentions a resource | no | tool result only | `list_changed` refreshes lists. Elicitation is interactive only. Sampling: unknown |

---

## 1. Settings hooks (`settings.json` `hooks`, plugin `hooks/hooks.json`)

Source: `https://code.claude.com/docs/en/hooks` (VERIFIED unless marked).

### 1.1 Events (VERIFIED, from the docs' event table)

`SessionStart`, `Setup`, `UserPromptSubmit`, `UserPromptExpansion`, `PreToolUse`, `PermissionRequest`, `PermissionDenied`, `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`, `Notification`, `MessageDisplay`, `SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`, `Stop`, `StopFailure`, `TeammateIdle`, `InstructionsLoaded`, `ConfigChange`, `CwdChanged`, `DirectoryAdded`, `FileChanged`, `WorktreeCreate`, `WorktreeRemove`, `PreCompact`, `PostCompact`, `PreModelSwitch`, `PostModelSwitch`, `Elicitation`, `ElicitationResult`, `SessionEnd`.

Every event receives these common input fields: `session_id`, `prompt_id`, `transcript_path`, `cwd`, `scratchpad_dir`, `permission_mode`, `effort`, `hook_event_name`. Inside a subagent the input also carries `agent_id` and `agent_type`. (VERIFIED)

Hook types: `command` (with `args` for exec form, plus `async`, `asyncRewake`, `shell`, `onFailure`), `http` (POST JSON), `mcp_tool`, `prompt` (single-turn model judgement) and `agent` (experimental). `SessionStart` supports only `command` and `mcp_tool`. (VERIFIED)

### 1.2 What reaches the model (VERIFIED)

- **`hookSpecificOutput.additionalContext`** is wrapped in a system reminder at the point where the event fired: at conversation start for `SessionStart`, beside the tool result for tool events, at the end of the turn for `Stop`. "Claude sees it on the next request but it isn't a chat message."
- **Plain exit-0 stdout** becomes context only on `UserPromptSubmit`, `UserPromptExpansion`, `SessionStart` and `PostModelSwitch`. On `UserPromptSubmit`, both stdout and `additionalContext` are "injected as a system reminder that starts with the hook's name", with no visible transcript entry.
- **`decision:"block"` + `reason`**: on `PreToolUse`, Claude receives `reason` as the tool error. On `PostToolUse`, `reason` is added next to the result. On `UserPromptSubmit`, `reason` is shown to the user and **not** added to context.
- **Exit 2 stderr** is the blocking reason. On `PreToolUse`, Claude receives it prefixed with the hook name.
- **`systemMessage`** is a warning shown to the user. On an async hook, it is delivered to Claude on the next turn (see §1.4).
- **`PostToolUse.updatedToolOutput`** replaces what Claude sees. It must match the tool's output shape; for built-in tools a mismatch is ignored.
- **`PreToolUse.updatedInput`** replaces the whole tool input, and permission rules are evaluated against the rewritten input. `permissionDecision` is `allow|deny|ask|defer`, with precedence deny > defer > ask > allow.
- **`SessionStart`** outputs: `additionalContext`, `initialUserMessage` (becomes the first turn, **only in `-p` mode**), `watchPaths`, `sessionTitle`, `reloadSkills`. Its `source` is one of `startup|resume|clear|compact|fork`. Resume and fork add `seconds_since_last_response`, `context_tokens` and `prompt_cache_likely_expired` (v2.1.251+).
- **Size cap**: `additionalContext`, `systemMessage`, `initialUserMessage` and plain stdout are each capped at **10,000 characters** per field per hook. Past the cap, the full text is saved to a file and Claude gets the path plus a 2,000-char preview. "Claude Code doesn't ask Claude to read the file." The cap cannot be raised.

### 1.3 Stop hook and "block and continue" (VERIFIED)

- `Stop` input adds `stop_hook_active`, `last_assistant_message`, `background_tasks` (each with `id`, `type`, `status`, `description`, ...) and `session_crons` (CronCreate/ScheduleWakeup/`/loop` entries). These let a hook tell "done" from "paused waiting on background work".
- Exit 2 or `decision:"block"` keeps Claude going. `reason` (or `additionalContext`) is fed back to it.
- Loop cap: "after stop hooks have continued the turn eight times in a row, Claude Code overrides the next block and ends the turn". The count resets on each tool call. The cap is raised with `CLAUDE_CODE_STOP_HOOK_BLOCK_CAP`.
- A Stop-hook loop is therefore an **end-of-turn** injection point. It can append pending mail to a turn that is ending, but it cannot wake a session that is already idle.

### 1.4 Async hooks and `asyncRewake`: a wake primitive (VERIFIED)

- A `command` hook with `"async": true` runs in the background. When it exits, its `additionalContext` and `systemMessage` are delivered "on the next conversation turn". "If the session is idle, the response waits until the next user interaction."
- The exception: "**an `asyncRewake` hook that exits with code 2 wakes Claude immediately even when the session is idle.**" The field table says `asyncRewake`: "runs in the background and wakes Claude on exit code 2. The hook's stderr, or stdout if stderr is empty, is shown to Claude as a system reminder."
- "Claude Code still enforces `timeout` on a hook you run with `asyncRewake`". The default is 600 s for `command`, and the docs state no maximum. The docs also do not say which events accept `async`.
- In `-p` mode, async hooks still running at teardown are killed with outcome `cancelled`.
- INFERRED design pattern: a `SessionStart` (or `Stop`) hook with `asyncRewake: true` could block until mail arrives, then exit 2 with the framed batch on stderr. The result is a hook-only, version-tolerant idle wake that needs no TUI typing. Constraints:
  - each wake consumes the hook, so a later event has to re-arm it (`Stop` re-fires after every turn);
  - the timeout bounds each wait;
  - it is unknown whether a wake landing mid-turn is folded in or held, and whether the user's draft is affected.

  None of this is captured here.

### 1.5 Idle/busy signals from hooks (VERIFIED)

- `Notification` `notification_type` values:
  - `idle_prompt` fires "about 60 seconds" after Claude finishes responding, only if the user hasn't typed since and no background agent runs.
  - `permission_prompt` fires about 6 s after a permission prompt appears; typing defers it.
  - Also `agent_needs_input`, `agent_completed`, `elicitation_dialog`, `plugin_notification`, and others.
  - Notification hooks cannot block, and their `systemMessage` is discarded.
- `Stop` marks the end of a turn. `UserPromptSubmit` marks the start of one. Together they give a busy/idle edge stream (INFERRED).
- `TeammateIdle` (agent teams only): exit 2 keeps the teammate working.
- `FileChanged` (with `watchPaths`) has no decision control. Whether it can inject context or wake a session is unknown.

### 1.6 Environment exported to hooks (VERIFIED)

`CLAUDE_PROJECT_DIR`, `CLAUDE_ENV_FILE` (on `CwdChanged`/`FileChanged`), `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN` (see §3). herdr-threads' own captures already show the two messaging variables in the SessionStart and PreToolUse hook environment: `docs/evidence/claude-286-hook-capture/payloads/01-sessionstart-startup.env-names.txt:7-8` and `docs/evidence/claude-compact-capture/payloads/03-sessionstart-resume.env-names.txt:7-8` (VERIFIED, repo).

---

## 2. Mods (function hooks): the engine API

Docs: `https://code.claude.com/docs/en/plugins/mods/overview.md`, `.../mods/api.md`, `.../mods/events.md`, `.../mods/reference.md`. Local authority: the bundled `types/claude-code.d.ts` for build 2.1.308. (VERIFIED)

### 2.1 Shape and availability (VERIFIED)

- A plugin folder with `.claude-plugin/plugin.json` and `hooks/hooks.json` `{ "modules": ["./register.js"] }`. The module exports `register(on, options)`, and every hook is `($, e, next)`. The module runs in a sandbox with no DOM and no Node; everything is reached through `$` (`$.process.spawn`, `$.fs`, `$.http.fetch`, `$.clock`, `$.store`, `$.state`, ...).
- "Mods are on by default. In the terminal, use Claude Code v2.1.287 or later" (overview). Hooks run in `-p` and the Agent SDK as well, but nothing is drawn there.
- A mod is loaded with `--plugin-dir`, with `CLAUDE_CODE_PLUGIN_DIRS` (process env or the user `settings.json` `env` block, "never a project's settings"), or by installing it from a marketplace. Interactive sessions watch the folder and hot-reload it. `register` and `session.start` run again on reload; `$.state` and `$.store` survive (reference.md §Developing one).
- Disabled by `disableAllHooks`, `--safe-mode`, `--bare`, managed `allowManagedModsOnly`, and managed `disableSideloadFlags` (the last one refuses `CLAUDE_CODE_PLUGIN_DIRS`; herdr-threads' setup checks it, see §9).

### 2.2 Injection points (VERIFIED, types)

- **`tool.call`**: `await next(e)` resolves to a `ToolCallResult`. Returning `{ ...result, context: [...] }` adds "What the model reads after the tool's result and the user never sees ... One reminder, as a PostToolUse hook's is". Each entry can be any length; past 100,000 chars (200,000 together) the model reads a head plus a path. Refusing the call instead (`{ deny }`) gives an error result.
- **`prompt.submit`**: a hook may attach `context: string[]` ("What the model reads beside the prompt and the user never sees"). The input carries `origin` (closed set: `composer`, `bridge`, `sdk`, `task-notification`, `scheduled-trigger`, `peer`, `peer-send-message`, `projects-relay`, `channel{server}`, `coordinator`, `observer`, `auto-continuation`, `unclassified`, `slack-ping`, `plugin{name, asUser?}`), `turnId` (set when the prompt was "typed over that turn, or delivered into it"), `wait`, and `isMeta`.
- **`$.session.append({ message: { type: 'user', content: [{type:'text', text}] } })`**: "REALLY appends: a user-role row the person does not see as typed (stored `isMeta`, the model reads it) ... The row is in the list at once and in a running turn's requests from the loop's next top." It starts **no turn**. `type: 'system'` makes a notice the model never reads. The row carries `origin: { kind: 'plugin', name }`, door `note`.
- **`prompt.compose` / `prompt.section`**: rewrite or add system-prompt sections (`scope: 'session'`). **`prompt.context`**: first-message context blocks.
- **`session.receive`**: "Fires when a delivery reaches the session, before it is queued: a relay's event, a peer's message, a Remote Control prompt, room events". It can rewrite `text` or `{ consumed }` the delivery. **`session.send`** is its dual, covering the `SendMessage` tool and `$.session.send`.
- **`session.append`** hook: every stored row passes through it (prompt, response, tool result, `hook-context`, `delivery`, `attachment`, `compaction`, ...) and `content` can be rewritten.

### 2.3 Wake points (VERIFIED)

- **`$.prompt.submit({ text, asUser? })`**: "queues a prompt that starts a turn of its own once the session is idle (never folded into a running turn; the call resolves as that turn starts, not when it ends), so background work can wake a quiet session" (reference.md §Work that outlives a dispatch). The model reads it as "The <name> plugin sent a message: ..." unless `asUser: true`.
- After a user interrupt (Esc/Stop), `$.prompt.submit` resolves `{ drop: 'the session was stopped and nobody has sent input since' }`, and a queued one is dropped with `'the prompt was removed from the queue before it ran'`. Retry from `turn.complete` (mods/api.md, v2.1.304+ for most calls).
- **`$.session.send({ to: { sessionId } | { agentId } | <address>, text })`** makes "the same delivery the SendMessage tool makes" to another session, subagent or teammate. It resolves `{ isDelivered, reason }` once queued.
- `$.clock.every/after` timers and `$.process.spawn` streams keep running between turns. A timer callback "doesn't start" a turn.

### 2.4 Idle/busy and draft observability (VERIFIED)

- `turn.start` / `turn.complete` (with `agentId` for subagents, `reason`/`isAborted`), `turn.step` (each model request) and `turn.abort`.
- `$.prompt.read()` returns `{ text, cursor }` of the composer (empty under `-p` or SDK). `prompt.edit` fires on every edit. `$.prompt.fill` writes a draft. `prompt.suggest` controls the dim suggestion; core refuses it "while the box holds text or a turn runs".
- `$.session.id()`, `$.session.turns()`, `$.session.messages()`, `$.session.usage()`, `session.measure`.
- UI without a turn: `$.ui.log` (dim transcript line the model doesn't read), `$.ui.status`, `$.ui.toast`, `$.ui.notify`.

---

## 3. Native cross-session messaging (built-in competitor to our transport)

Doc: `https://code.claude.com/docs/en/cross-session-messaging.md` (VERIFIED).

- Requires v2.1.224 or later on macOS/Linux/WSL2 (v2.1.234 on native Windows). It is "on with nothing to enable". The model uses the tools `ListAgents` and `SendMessage` (`summary` preview of at most 200 chars, `notify_when_idle`). The user uses `/list-agents` (`/peers`) and `@name` mentions (v2.1.232+).
- **Delivery semantics**:
  - "The receiving Claude reads the message between tool calls during an active turn, so a running tool is never interrupted. When the receiving session is idle, Claude Code starts a new turn with the message."
  - The message is plain text with the sender's name and a reply address. `@` mentions in it are not expanded, and `/commands` in it do not run.
  - The UI shows a dim preview `› Message from @api-worker: … (ctrl+o to expand)`, and the model reads the full text.
  - The receiver is told the message is from another session, can't approve permission prompts, and must not change config.
- **Transport**: same machine uses a per-session Unix domain socket (named pipe on Windows), "never through Anthropic servers". Other machines and cloud go via Remote Control through Anthropic servers.
  - The path is exported as **`CLAUDE_CODE_MESSAGING_SOCKET`** to hooks and Bash. `/status` shows `Peer address uds:<path>`. The fallback directory is `/tmp/cc-socks-<uid>`.
  - A per-session **`CLAUDE_CODE_MESSAGING_TOKEN`** can be sent as the first line `{"type":"auth","token":"..."}` (optional on Linux/macOS).
  - "Open the connection only when the message you're posting is ready", because connections idle for 30 s without a complete line are closed. The message line's JSON schema is **not documented** (unknown).
- **Inbound controls**: `crossSessionInbound` is `accept|hold|refuse`. Without a setting, a prompting receiver accepts unless the sender is bypass-permissions, and a bypass receiver holds unless the sender also bypasses. Held messages raise an approval dialog with a `dialogExpiry` of 5 min. At most 100 are held.
  - "**Own-child messages**: when no `crossSessionInbound` value applies, Claude Code delivers a message it verifies came from the session's own child processes, such as a hook or Bash command posting back to its own session's socket". On Linux this is verified by process evidence, otherwise by the token.
- **Limits**: about 1M chars per message. Burst refusal at the sender. "drops identical repeats arriving within a short window, and queues at most 50 accepted messages". Plain text only. No ACK beyond "queued" and an optional reply. `notify_when_idle` gives one notice when the target next goes idle (12 h expiry, v2.1.236+).
- `claude -p` sessions bind the inbox too. `--bare` does not.
- Relevance (INFERRED):
  - A helper process that is a child of the Claude session (a hook, or a `watch` spawned by a mod) could post a framed batch into the session's own socket. It would get native "idle → new turn, busy → between tool calls" delivery without typing into the pane and without the mod's `$.prompt.submit` round trip.
  - Messages would appear as peer/own-child deliveries, with the platform's framing, de-dup and 50-message queue limits.
  - The wire schema has to be captured first. This is not documented and not tested here.

---

## 4. Channels (MCP push)

Docs: `https://code.claude.com/docs/en/channels.md` and `channels-reference.md` (VERIFIED).

- An MCP stdio server declares `capabilities.experimental['claude/channel']: {}` and emits `notifications/claude/channel` with `{ content, meta }`.
  - The model sees `<channel source="<server>" k="v">content</channel>`. Meta keys must be identifiers; others are silently dropped.
  - Server `instructions` are delivered as context at connect, cut at 4,096 chars (MCP doc).
  - Two-way channels add an ordinary reply tool.
- **Timing**: "Events queue into the session and are processed in order. If several notifications arrive while Claude is busy, they're delivered together on the next turn." The overview says channels let "Claude react to things that happen while you're not at the terminal", and the webhook example shows Claude starting to respond to an event that arrives while idle. Idle wake is therefore INFERRED from docs plus example, not stated in so many words.
- **No acknowledgement**: "Claude Code doesn't acknowledge notifications ... If the session hasn't loaded your server as a channel ... drops the events silently".
- **Gating**:
  - research preview;
  - the session must be started with `--channels plugin:<name>@<marketplace>` (allowlisted, default `claude-plugins-official`) or `--dangerously-load-development-channels server:<name>|plugin:...` (interactive confirmation dialog; ignored under `-p`/SDK);
  - claude.ai or Console API-key auth only (not Bedrock/Vertex/Foundry);
  - Team/Enterprise need `channelsEnabled`, and `allowedChannelPlugins` can replace the allowlist;
  - neither flag appears in `claude --help` (confirmed locally: absent from the 2.1.309 help);
  - the v2 MCP runtime does not register channel servers on protocol revision 2026-07-28.
- Permission relay: `claude/channel/permission` capability, `notifications/claude/channel/permission_request` (5-letter `request_id`) and verdict `notifications/claude/channel/permission`.
- INFERRED: a channel suits an operator, but it is a poor fit for a plugin that must work unattended in every pane. It needs per-launch flags, an allowlist or a dev-flag dialog, and has no ACK.

---

## 5. MCP client support (VERIFIED, `https://code.claude.com/docs/en/mcp`)

- Tools are offered alongside built-ins (deferred under tool search). The output warning is at 10k tokens and the default cap is 25k (`MAX_MCP_OUTPUT_TOKENS`). Text results over 50k chars go to a file. Tool timeout is `MCP_TOOL_TIMEOUT`, default about 28 h. The idle timeout is 30 min for stdio (`CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT`), and progress notifications count as activity.
- Calls still running after 2 min move to a background task. The MCP tasks extension needs v2.1.304+, and when a task ends "the result goes to Claude".
- `list_changed` refreshes tools, prompts and resources in interactive sessions (only tools under `-p`/SDK).
- Resources reach the model only via user `@server:uri` mentions or auto-provided list/read tools. Prompts appear as slash commands.
- `roots/list` is answered. Elicitation (`form`, `url`) is "served only in interactive terminal sessions".
- Sampling: not documented (unknown).
- `logging` and `resources/updated` are not said to reach the model. The docs say channels are "the documented route" for server events.
- INFERRED: a **long-poll MCP tool** (it blocks until mail, and progress keeps it alive) works only while the model is inside a call. It cannot wake an idle session, and it holds the turn open.

---

## 6. Headless, SDK, background and server modes

- **`-p/--print`** (VERIFIED, help and `https://code.claude.com/docs/en/headless`): `--output-format text|json|stream-json`, `--input-format stream-json` ("realtime streaming input"), `--replay-user-messages`, `--include-hook-events`, `--resume <id|name>`, `--continue`, `--session-id <uuid>`, `--fork-session`, `--no-session-persistence`, `--permission-prompts host|none`.
  - In `-p` mode, `SessionStart` `initialUserMessage` starts the first turn.
  - A `-p` run waits for background work after its turn: background commands, subagents, Monitor watches and pending `/loop` wakeups (10 min ceiling, `CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`).
- **Agent SDK streaming input** (`https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode`): a long-lived process with "Queued messages: send multiple messages that process sequentially, with ability to interrupt". Hooks and MCP are available. This is a host-owned session; it cannot attach to an existing TUI pane.
- **Background sessions** (VERIFIED, help and `https://code.claude.com/docs/en/agent-view.md`): `claude --bg [--resume <id>]`, `claude agents`, `attach <id|name>`, `logs`, `stop`, `respawn`, `rm`, and a `claude daemon` supervisor. Replies to a working background session "join the session's message queue".
- **`claude agents --json [--all] [--cwd]`** lists **every live session, interactive and background**. Each row has `kind`, `cwd`, `pid`, `status` (`busy|waiting|idle`), `waitingFor` (`permission prompt`, `input needed`, `sandbox request`, `worker request`, `dialog open`), `sessionId`, `name` and `state`. It is "the supported way to read session state from outside Claude Code". It is a native busy/idle probe, not run here.
- **Remote Control**: `claude remote-control` server mode, `--remote-control [name]`. Session prompts from claude.ai or mobile arrive with origin `bridge`. Prompts relayed through it carry the user's authority (INFERRED), so it is not an agent-to-agent path.
- **`--cloud`, `--teleport`**: cloud sessions. `--cloud <id> -p` "queues a message into that cloud session".
- **ACP / app-server / RPC server for the local TUI**: none found (unknown/none documented). The supervisor daemon's socket protocol is undocumented.

---

## 7. TUI input behaviour relevant to keystroke injection (VERIFIED, `interactive-mode.md`, `terminal-config.md`)

- **Typing while busy queues**: "if you queue a message while Claude is running tool calls, Claude Code passes it to Claude as soon as those tool calls finish, within the same turn". Leftovers go out at turn end. `Ctrl+Enter`/`Ctrl+X Ctrl+S` sends the queue now, and `Esc` interrupts and sends what was queued. Commands and `!` shell commands wait for the turn end.
- **Pastes**: more than 800 chars or more than 3 lines collapse to `[Pasted text #N +k lines]`. The model sees pasted content "marked as text you pasted from somewhere else" and "is told ... to follow instructions inside it only where the message you typed asks it to" (where feature flags are fetched). Invisible Unicode is stripped on Enter and the prompt is put back for a second Enter.
- **Drafts**: `Ctrl+S` stashes or restores the draft. `Ctrl+C` clears it, and `Up` recalls (the 10 most recent cleared drafts, v2.1.292+). `Esc Esc` clears to history. `Ctrl+U` deletes to line start (used by herdr-threads as `clear_key`).
- **Prompt suggestions**: a dim suggestion is drawn in an empty box after each turn (`promptSuggestionEnabled`, `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION`). Screen scraping cannot tell it from typed text (herdr-threads `src/harness/claude/composer.rs:9-10`, VERIFIED repo).
- **Idle/busy without a mod**:
  - OSC 7501 Program Status Protocol (working, waiting, done) since v2.1.295; it is not reported inside tmux or in background sessions;
  - terminal title (disabled by `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`);
  - `claude agents --json`;
  - `Notification idle_prompt` (about 60 s late).

---

## 8. Other wake or inject primitives

- **Scheduled prompts** (`/loop`, `CronCreate`, `ScheduleWakeup`; `scheduled-tasks.md`): "enqueues them at low priority. A scheduled prompt fires between your turns, not while Claude is mid-response". They fire only while the session runs and is idle. Recurring tasks expire after 7 days. These are model-created (VERIFIED).
- **Monitor tool / plugin monitors** (`tools-reference.md#monitor-tool`, `plugins/components.md#monitors`): each stdout line or WebSocket text frame becomes a notification to Claude ("Claude interjects when an event arrives"). A Monitor-tool watch has a 5 min default and 30 min max deadline. A plugin monitor (`monitors/monitors.json`) starts automatically at session start in **interactive sessions only** and runs for the whole session. It is unavailable on Bedrock/Vertex/Foundry or with telemetry disabled. Wake while idle: INFERRED (origin `task-notification` is "dequeued when the session went idle or delivered into a running turn", types `PromptOrigin`).
  - INFERRED: a plugin monitor running `herdr-threads watch --lines` would be a second push path that needs no mod API. Its gaps: no ACK of model receipt, no draft guard, one notification per line.
- **Agent teams**: file mailboxes at `~/.claude/teams/{team}/inboxes/{agent}.json`, and the `TeammateIdle` hook (VERIFIED, `agent-teams.md`). This is in-team only.
- **`/goal`**: a session-scoped prompt-based Stop hook that keeps working toward a condition (VERIFIED, hooks doc preface).

---

## 9. What herdr-threads does today for Claude (brief)

- **Settings hooks** (`integrations/claude/README.md:3-10`; `src/harness/claude.rs`):
  - `SessionStart` (startup/clear/resume, plus compact on the 2.1.287 recipe) returns check-in `additionalContext`, capped at `MAX_HOOK_OUTPUT = 4096` (`src/harness/claude.rs:37`), below Claude's 10,000-char cap.
  - Bash `PreToolUse` rewrites subagent `herdr-threads` calls with `--agent-id` via `updatedInput`.
  - `SubagentStart` gives child guidance, and `SubagentStop` is quiet.
  - The README still describes exact-version refusal. The code now admits versions optimistically (`src/harness/claude.rs:213-219`, `check_version` → `admit` at `:317`), so that README text looks stale (INFERRED).
- **Delivery mod** (`integrations/claude/mod/README.md`; `integrations/claude/mod/hooks/register.js`): installed into `<state>/claude-mod/herdr-threads/` and loaded through `env.CLAUDE_CODE_PLUGIN_DIRS` in user settings.
  - It spawns `herdr-threads watch --harness claude --session <id>` via `$.process.spawn` (`register.js:1014`) and tracks busy state from `turn.start`/`turn.complete`, main loop only.
  - **Busy**: it appends the batch to the next answered main `tool.call` result as `context` (`register.js:834`, hook at `:1092`).
  - **Idle**: `$.session.append` hidden user rows (`:989`), then `$.prompt.submit('wake up')` (`:12`, `:609`) under plugin attribution.
  - Lazy (non-waking) rows are appended at once.
  - Gates: 120 s draft hold via `$.prompt.read` (`:986`, `DRAFT_WAIT_MS`), post-abort hold, one wake in flight, assumed-busy at load (`ASSUMED_BUSY_IDLE_MS = 5000`).
  - ACK via `herdr-threads watch ack --via context|append|submit`.
  - Declared as `DeliveryMechanism::PushChannel` / `IdleWake::Channel` (`src/harness/claude/delivery.rs`).
- **Fallback without the mod** (Claude older than 2.1.287, managed `disableSideloadFlags`, `--hooks-only`): hooks plus Herdr native typing wake.
  - The composer is read from Herdr detection text. Text in it is always `Unsafe`, because it may be a prompt suggestion (`src/harness/claude/composer.rs`). So setup offers to disable `promptSuggestionEnabled`.
  - `clear_key` is `ctrl+u`. Composer stash is declared supported only on the 2.1.287 recipe.
- **Not used today** (grep of `src/`, `integrations/`, `docs/`; VERIFIED absent): `asyncRewake`, `UserPromptSubmit`/`PostToolUse`/`Stop` hooks for Claude, channels (`notifications/claude/channel`), native cross-session messaging (the `CLAUDE_CODE_MESSAGING_SOCKET` name appears only in captured env-name lists), `claude agents --json`, OSC 7501, plugin monitors, and the mod `session.receive` event.

## 10. Open questions for the comparison reports

1. Wire format of a line posted to `CLAUDE_CODE_MESSAGING_SOCKET`, and whether own-child posts are framed as peer messages (unknown; capture needed).
2. Whether an `asyncRewake` exit-2 wake that arrives mid-turn is folded in or held, how it interacts with a typed draft, and the effective maximum `timeout` (unknown).
3. Whether channel notifications start a turn on an idle session in interactive mode (INFERRED yes; not captured).
4. Whether plugin monitor lines start a turn while idle, and their framing (INFERRED; not captured).
5. Whether `claude agents --json` `status` for interactive sessions is cheap enough to poll as a busy/idle oracle, and whether running it starts the supervisor (unknown).
