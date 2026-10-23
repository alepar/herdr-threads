# cross-agent-teams-mcp (jtianling), "xats": overview

- Upstream: https://github.com/jtianling/cross-agent-teams-mcp (npm `cross-agent-teams-mcp`)
- Clone read: `--depth 1` at `17df5cf7bd9b66f00460eff05080b0e0495b6ec8` (2026-10-20 13:42 +0800)
- All paths below are relative to the clone root. Code was read, never run.
- Tags: VERIFIED = read in source/docs; INFERRED = my reading, not stated in source.

## What it is

A local Node/TypeScript MCP daemon (Streamable HTTP, default `127.0.0.1:9100/mcp`). Agents in
Claude Code, Codex, opencode, Kimi Code or any other MCP client register into `(device, team, name)`
seats, then send 1:1 messages, team broadcasts and role broadcasts. The daemon wakes recipients
through a per-harness transport: a Claude Code channel, the Codex app-server, opencode's HTTP API,
Kimi's HTTP API, or a tmux paste as the fallback. VERIFIED: `README.md:5`, `README.md:61`,
`src/lib/delivery-spec.ts:3-49`, `src/lib/agent-type.ts:1`.

The project calls itself an alternative to Claude Code's agent teams: a team can mix harnesses, agents
are long-lived and started by hand, and a team can span machines. VERIFIED: `README.md:7-15`.

## Architecture

| Aspect | Finding | Tag |
|---|---|---|
| Process model | One long-lived daemon (`cross-agent-teams-mcp daemon --port`), started by the user ("keep it alive"). Each agent connects to it as an ordinary MCP client over Streamable HTTP. Claude Code also spawns a stdio "channel proxy" child (`cross-agent-teams-channel`). | VERIFIED `README.md:76-93`, `README.md:113-137`, `package.json` bin |
| Storage | SQLite through better-sqlite3 in WAL mode, default `~/.cross-agent-teams-mcp/data.db`. Tables: `events`, `agents` (with per-agent `last_processed_event_id` cursor), `messages`, `message_delivery_status`, `codex_pane_bindings`, `codex_pane_pre_registrations`. | VERIFIED `src/storage/db.ts:8`, `src/storage/schema.ts:4-81`, `README.md:92` |
| Transport, agent to daemon | MCP Streamable HTTP tools (`register_agent`, `send_message`, `send_message_by_id`, `broadcast`, `broadcast_to_role`, `get_inbox`, `get_delivery_status`, `poke`, `reconnect`, `bind_channel`, ...). There is also a loopback-only REST "lifeboat" (`/api/send`, `/api/inbox`, `/api/agents`, `DELETE /api/agents/:id`, `/api/runtime/opencode/*`). | VERIFIED `src/mcp/tools.ts`, `README.md:269-323` |
| Transport, daemon to Claude proxy | The proxy calls `register_agent` as `__channel_proxy__` and then `subscribe_channel_wake`. The daemon pushes `notifications/channel_wake` over the MCP session's SSE stream (30 s heartbeat). The proxy re-emits each one as `notifications/claude/channel`. | VERIFIED `plugins/cross-agent-teams-channel/src/daemon-client.ts:117-125`, `src/daemon/channel-wake-send.ts:28-36`, `src/daemon/sse-fanout.ts:8`, `plugins/cross-agent-teams-channel/src/proxy.ts:15-27` |
| Server/daemon | Yes, a single central daemon. There is no peer-to-peer mode. | VERIFIED |
| Cross-machine | Yes. One daemon bound to a non-loopback `--host`, which requires `--token`. Remote agents must declare `device` (rejected with `device_required_from_remote` otherwise). Addressing uses `name:device`. A single shared bearer token covers everyone: no per-agent auth, no TLS. tmux, Codex app-server and opencode wakes only work on the daemon's own host (pane ownership checks skip non-local devices). | VERIFIED `README.md:359-442`; the claim that only Claude channel wakes reach other devices is INFERRED from `src/mcp/poke.ts:361-364` and the fact that delivery URLs are loopback |

## Identity and session model

- A seat is the row `agents(device, team, name)` → `agent_id` UUID. Re-registering the same tuple
  reuses the agent_id. The team defaults to the basename of the cwd. VERIFIED `README.md:337`,
  `README.md:427`, `CHANGELOG.md` 0.8.0.
- Registration is model-driven: the user tells the agent "Register me to xats as alice", and the agent
  calls `register_agent` with `agent_type`, `ui_pid: $PPID` and harness coordinates. For Claude, the
  proxy's startup channel message tells the model not to auto-register and to ask the user first.
  VERIFIED `README.md:327-337`, `plugins/cross-agent-teams-channel/src/cli.ts:108-120`.
- Delivery coordinates are stored per row as a `DeliverySpec`: `claude-channel{channel_session_id}`,
  `codex-appserver{thread_id, ws_url}`, `opencode-server{session_id, base_url, runtime_generation}`,
  `kimi-server{session_id, base_url}` or `none`. A row may also carry `tmux_pane_id`, `runtime_ui_pid`
  and `runtime_tty`. VERIFIED `src/lib/delivery-spec.ts:3-49`, `src/mcp/poke.ts:90-100`.
- Claude: the daemon auto-binds the proxy's per-startup `channel_session_id` to the agent through
  `ui_pid`. Recovery after a restart or a context clear goes through `reconnect({ui_pid})`, or
  `reconnect({identity_key})` when a launcher exported `XATS_IDENTITY_KEY`. VERIFIED
  `plugins/cross-agent-teams-channel/src/cli.ts:24-29,100-119`.
- Codex: the launcher mints a UUID, pre-registers `$TMUX_PANE` → uuid (120 s TTL) and execs
  `codex --remote ws://... -c xats.agent_id="<uuid>"`. The daemon matches the uuid in the foreground
  process argv to bind the pane. The register call carries `thread_id: $CODEX_THREAD_ID`. VERIFIED
  `README.md:187-217`.
- This identity binding is the system's most complex part: generation-fenced conditional writes,
  CAS on prior state, recovery nonces carried in recovery pokes, and "same-thread seat" inheritance.
  The 0.8.x CHANGELOG lists repeated production incidents of pane and key theft between Codex panes.
  VERIFIED `CHANGELOG.md` 0.8.0 and 0.8.1 entries.
- A `register_agent` from a second MCP session triggers a takeover that force-closes the earlier
  session. VERIFIED `README.md:267`.

## Injection mechanism (general)

- **Wake hint only, never the body.** Every transport injects a short hint:
  `新邮件 from <sender>@<team> (<agent_id>) → <name>@<team>, 请调 get_inbox 查看`, capped at 200 characters
  by dropping segments. The model then pulls bodies with the `get_inbox` MCP tool. VERIFIED
  `src/mcp/tools.ts:196`, `src/mcp/tools.ts:367-393`.
- **Hooks:** none. There are no UserPromptSubmit, PostToolUse or Stop hooks, and no in-turn delivery
  of bodies. INFERRED from the file inventory (no hook config or scripts in the repo).
- `get_inbox` with no `since_event_id` returns mail past the server-side cursor and advances the cursor
  in the same transaction. Pagination with `limit` advances only to the last returned row. An explicit
  `since_event_id` is a read-only peek. VERIFIED `src/mcp/tools.ts:2924-2945`.
- The manual `poke` tool sends arbitrary text up to 8192 bytes, same team only, never to oneself.
  VERIFIED `src/mcp/poke.ts:102,271-293`.

## Wake mechanism (general)

`dispatchPoke` routes on the effective agent type. VERIFIED `src/mcp/transport-dispatch.ts:101-311`.

- **Claude Code:** a `notifications/claude/channel` push through the stdio proxy. The proxy declares
  `experimental: {'claude/channel': {}}`, and Claude is launched with
  `--dangerously-load-development-channels server:cross-agent-teams-channel`. If no subscriber is
  attached, a bound tmux pane is used instead. VERIFIED `transport-dispatch.ts:157-203`,
  `proxy.ts:3-8`, `README.md:139-145`.
- **Codex:** a JSON-RPC client on the app-server WebSocket runs `initialize` and then
  `thread/resume{threadId}`. With no in-progress turn it calls `turn/start`; with an active turn it
  calls `turn/steer{expectedTurnId}`, so a message reaches a busy agent mid-turn. It waits up to 5 s for
  `turn/started` and then up to 30 s for an `item/started` whose `mcpToolCall` is `get_inbox`; only that
  counts as success (`codex_wake_unconfirmed` otherwise). The socket stays open until `turn/completed`,
  for at most 10 minutes. On other failures it falls back to a tmux paste. VERIFIED
  `src/mcp/codex-appserver-dispatch.ts:117-245`, `transport-dispatch.ts:205-235`.
- **opencode:** `POST {base_url}/session/{id}/prompt_async` with a text part, which starts a fresh agent
  turn. A `runtime_generation` fence returns `runtime_recovering` during recovery. The code does no busy
  check before the POST. VERIFIED `src/mcp/opencode-server-dispatch.ts:43-61`,
  `transport-dispatch.ts:237-269`, `README.md:220-255`. What opencode does with a prompt that arrives
  mid-turn (queue or interleave) is unknown.
- **Kimi Code:** `POST {base_url}/api/v1/sessions/{id}/prompts`, with a busy gate first. A busy session
  (`main_turn_active`, `tui_recent_write`) gives `kimi_session_busy` and a retry. A pending approval
  gives `kimi_pending_interaction`, which is never retried. VERIFIED
  `src/mcp/kimi-server-dispatch.ts:19-25,142-180`, `src/mcp/auto-poke-fanout.ts:137-157`.
- **tmux fallback** (custom agents, and every transport above when it fails):
  1. Verify that the pane's host process belongs to the target.
  2. Run the quiet guard: capture the last 8 lines, wait 2 s (`POKE_QUIET_MS`), capture again; any
     change fails the guard.
  3. Codex only: require the `›` composer prefix in the tail (an allowlist).
  4. `load-buffer`, then `paste-buffer -d`, then wait 400 ms, then send `Enter`.
  5. Re-check ownership at each step. If ownership changes after the paste, abort before `Enter`
     (`ownership_lost`).

  The guard detects a moving screen, not a non-empty composer, so a typed draft that is not changing
  would get the hint pasted into it and submitted with Enter (INFERRED). VERIFIED
  `src/mcp/poke.ts:173-245`, `src/mcp/poke-guard.ts:3-31`, `src/mcp/codex-prompt-readiness.ts:21-29`.
- **Retries:** a `guard_failed` (tmux) or `kimi_session_busy` result is retried at 30 s, 180 s and
  600 s. Retries stop as soon as the recipient's cursor passes the message (`alreadyReadFn`). VERIFIED
  `src/mcp/auto-poke-fanout.ts:129-178`, `src/mcp/tools.ts:195`.
- **Idle vs busy:** each harness gets a different answer.
  - Codex: the app-server turn status.
  - Kimi: a session-state API.
  - tmux: screen quiescence.
  - Claude channel and opencode: no check.

  There is no priority or threshold, so every send wakes the recipient unless `auto_poke:false`.
  VERIFIED (the absence of priority is INFERRED from the tool schemas read).

## Delivery guarantees

- **Durability:** the mailbox row is written before any wake attempt, and the wake result never undoes
  it. Messages are deleted after 30 days whether read or not. Since the Unreleased changes, agent rows
  idle for 30 days are deleted too, and broadcasts reach only members seen in the last 7 days.
  VERIFIED `src/mcp/tools.ts:178`, `src/mcp/tools.ts:2940`, `CHANGELOG.md` Unreleased.
- **Ack:** `send_message` blocks for up to `await_ack_s` (default 10, max 30) and returns
  `ack:{status:"read"|"not_yet"}`. "read" means the recipient's own `get_inbox` returned the message.
  The read receipt is the cursor (an implicit ACK when displayed), not an explicit acknowledgement.
  VERIFIED `src/mcp/tools.ts:166-178`.
- **Unread watchdog:** a `need_reply` message still unread after 15 minutes triggers a daemon-originated
  alert poke to the sender. Transient alert failures are retried for 10 minutes. VERIFIED
  `src/mcp/unread-watchdog.ts:6,31`, `src/mcp/tools.ts:177`.
- **Ordering:** the inbox is read in ascending `event_id` order past the cursor. VERIFIED
  `src/mcp/tools.ts:2933-2935`; the ascending order itself is INFERRED from "past the cursor" semantics.
- **Delivery status:** `get_delivery_status` separates `wake_status` (the transport accepted the wake)
  from `read` (the cursor passed the message). VERIFIED `src/mcp/tools.ts:2950-2960`.
- **Threads:** there is no thread or conversation object, only 1:1 mail plus team and role broadcast.
  VERIFIED (tool list).

## Human participation

There is no first-class human seat. A human talks to their own agent, which calls the tools. The REST
lifeboat can `send` as an already registered `(team, name)`, so a script or a person with curl can
impersonate a registered agent on loopback (with the token when one is set). VERIFIED `README.md:263-319`.
A human approval pending in Kimi blocks wakes (`kimi_pending_interaction`). VERIFIED (see above).

## License and maturity

- License: MIT. VERIFIED `LICENSE`, GitHub API.
- Maturity: 22 stars, 2 forks, 0 open issues. Created 2026-04-21, last push 2026-10-20. npm version
  `0.8.6` (`package.json:3`), with a CHANGELOG through 0.8.x. VERIFIED (GitHub API fetched 2026-10-23).
- Very active: about 260 test files with vitest, openspec change records, and a `lab/` of reproduction
  scripts for identity races. Docs are mixed Chinese and English. VERIFIED by the file listing.
  The code itself is about 20 k lines of TypeScript under `src/` and the plugin.

## Harness support

| Harness | Status | Mechanism | Tag |
|---|---|---|---|
| claude | native | MCP HTTP tools plus a stdio channel proxy that emits `notifications/claude/channel` (needs `--dangerously-load-development-channels`); tmux paste fallback | VERIFIED |
| codex | native | MCP HTTP tools plus a Codex app-server WebSocket: `thread/resume`, then `turn/start` or `turn/steer`, confirmed by observing a `get_inbox` call; requires a `codex --remote` launcher and a resident `codex app-server`; tmux fallback | VERIFIED |
| hermes | none | No mention in the repo. As a generic MCP client inside tmux it could register as `custom` and receive tmux pastes | INFERRED |
| agy | none | No mention in the repo. Same generic `custom` plus tmux path, if it can load a Streamable HTTP MCP server | INFERRED |
| goose | generic | No mention in the repo. Generic MCP `custom` registration plus tmux paste wake | INFERRED |
| pi | unknown | No mention in the repo. pi has no built-in MCP client as far as I know, so even the generic path would need an extension; unverified | unknown |
| opencode | native | MCP HTTP tools plus `POST /session/{id}/prompt_async` (launcher `free-xats-opencode` exports `OPENCODE_XATS_BASE_URL`); runtime-generation recovery REST; tmux fallback without the launcher | VERIFIED |

Kimi Code also has native support (`kimi-server` delivery). Cursor and other agents are "custom"
(tmux only). VERIFIED `README.md:257-259`, `src/lib/agent-type.ts:1`.

## Notes for comparison with herdr-threads (INFERRED)

- The Codex `turn/steer` path delivers into a busy turn without typing into the terminal. It is a
  harness-native in-turn channel that herdr-threads does not use for Codex today.
- Confirming a wake by observing a `get_inbox` `mcpToolCall` item, rather than trusting that the
  transport accepted the request, is a stronger delivery signal than a keystroke acknowledgement.
- Their weaknesses relative to our contract:
  - There are no priorities or thresholds, so every send wakes the recipient.
  - Bodies always need a separate `get_inbox` round trip.
  - The tmux guard can paste into a static human draft.
  - The Claude channel and opencode wakes have no busy check.
  - Identity binding needed a long series of race fixes because the seat is inferred from
    pane, pid and argv evidence.
