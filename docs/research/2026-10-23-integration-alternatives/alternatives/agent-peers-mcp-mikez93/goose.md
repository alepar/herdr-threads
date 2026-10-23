# agent-peers-mcp (mikez93) x Goose (Block)

- Upstream: https://github.com/mikez93/agent-peers-mcp, read at `2c63379d65ad44845815faa21a0c5c78355002fa`. Paths
  without a prefix are relative to that clone's root.
- Goose evidence comes from the harness primer (`../../primers/goose.md`) and the two Goose clones it names
  (`goose@1.36` = tag v1.36.0, the locally installed version; `goose@main` = c3bfc5f7b). Goose paths are relative to
  those clone roots: https://github.com/block/goose.
- Tags: VERIFIED = read in source or docs at the cited place. INFERRED = my reading. "unknown" = not determined.
  No code from the clone was run.

## Support status

**Not supported.** VERIFIED:

- A case-insensitive grep for `goose` over the whole clone has two hits, and both are the random-name noun list:
  `shared/names.ts:20` (`"goose","eel","mantis",...`) and its copy in
  `docs/superpowers/plans/2026-04-13-agent-peers-mcp-implementation.md:346`.
- The broker schema has a closed set of peer types:
  `peer_type TEXT NOT NULL CHECK(peer_type IN ('claude', 'codex', 'hermes', 'droid'))` (`broker.ts:145`, again at
  `broker.ts:307,454`). A `goose` peer type cannot be registered without a schema migration.
- The server's runtime selector knows only these values:
  `AGENT_PEERS_RUNTIME === "hermes" ? "hermes" : ... === "droid" ? "droid" : "codex"` (`codex-server.ts:103-107`).
- There is no Goose launcher, wrapper, hook script or install prompt. `bin/` holds only `claude-peer-rewake`,
  `codex-peer`, `droid-peer` and `droidpeer`. The README covers Claude, Codex and Hermes installs
  (`README.md:63-180`).

What follows is an assessment of how the project's **generic** mechanism would behave if a user wired it into Goose
by hand. Everything in it is INFERRED unless a line cites Goose or clone source.

### The generic mechanism and how it maps onto Goose

The only harness-agnostic piece is the durable polling stdio MCP server `codex-server.ts`. Its header calls it
"Durable polling MCP stdio server shared by Codex CLI, Hermes Agent, and Factory Droid" (`codex-server.ts:1-4`).
The Hermes and Droid adapters are 47- and 11-line shims: they set `AGENT_PEERS_RUNTIME` and `AGENT_PEERS_ENABLED`,
then `import("./codex-server.ts")` (`hermes-server.ts:22-45`, `droid-server.ts:5-9`). VERIFIED. Exception: with
`AGENT_PEERS_HERMES_V2=1` the Hermes shim starts `shared/hermes-conversation-runtime.ts` instead
(`hermes-server.ts:39-42`).

A Goose user could therefore add a stdio extension to `~/.config/goose/config.yaml`, in the shape the Goose docs
give (`goose@1.36 documentation/docs/getting-started/using-extensions.md:320-330`):

```yaml
extensions:
  agent-peers:
    type: stdio
    cmd: bun
    args: [/path/agent-peers-mcp/hermes-server.ts]   # or codex-server.ts with AGENT_PEERS_ENABLED=1
    envs: { AGENT_PEERS_ENABLED: "1", PEER_NAME: "...", AGENT_PEERS_DISABLE_TAB_TITLE: "1" }
    enabled: true
    timeout: 300
```

The session would then register with the broker as a `codex` or `hermes` peer: an impostor peer type, shown that
way in `list_peers` and in every sender line. INFERRED.

The fit is reasonable for **injection on tool call** and absent for **wake**. Details follow.

## Injection mechanism

### Exact mechanism

Delivery piggybacks on MCP tool results. Every tool on the server is wrapped in `withPiggyback`
(`codex-server.ts:479-547`, called from the `CallToolRequestSchema` handler at `codex-server.ts:693-812`). The wrapper prepends a `PEER INBOX` text block (the server instructions call it `[PEER INBOX]`; the literal banner
has no brackets, `shared/piggyback.ts:38`) to the tool's own result text:
`const finalText = inbox + toolText;` (`codex-server.ts:533-534`). VERIFIED.

The tools are `list_peers`, `send_message`, `set_summary`, `check_messages`, `wait_for_peer_messages` and
`rename_peer` (`codex-server.ts:229-291`). VERIFIED.

In Goose, a stdio MCP tool result reaches the model as an ordinary tool response. VERIFIED by the primer:
tools are supported in both versions, `primers/goose.md` §6. So the path works unchanged. INFERRED.

### Code path (VERIFIED, `codex-server.ts`)

1. **Background poll.** A timer polls the broker every 1 s while active and every 3 s when idle (the active window
   is 120 s): `POLL_INTERVAL_MS = 1000`, `POLL_IDLE_MS = 3000` (lines 94-100); scheduled at lines 1037-1046.
   - The broker returns *all* unacked, unleased rows, `ORDER BY id ASC`, with no LIMIT (`broker.ts:901-910`).
   - The broker leases each returned row for 30 s (`LEASE_DURATION_MS`, `broker.ts:27,880`). VERIFIED.
   - `applyLeasedToQueue` writes the leased rows to a durable on-disk inbox
     `~/.agent-peers-codex/<peer>.json` (or `-hermes`), plus a bodyless `<peer>.metadata.json`
     (`shared/codex-inbox.ts:57,110,214-216,275-285`). This step does not ack.
2. **Preview push.** It sends a "signal-only" MCP `notifications/message` with no body (lines 409-442;
   `formatInboxPreview`, `shared/piggyback.ts:25-31`).
   - In Goose, MCP logging notifications go **to the UI only**. They are also delivered only to a subscriber that
     exists *during a tool call to that server*. VERIFIED by the primer, §6: `goose@1.36 extension_manager.rs:1663`,
     `mcp_client.rs:253-284`.
   - So in a Goose CLI the preview is dropped while the model is idle or busy with other tools. INFERRED.
   - It might appear as a log line during an agent-peers tool call. INFERRED. Either way it never enters model
     context.
3. **Tool call.** On any agent-peers tool call, `acquireInboxBatch` runs under a lock (lines 550-666):
   1. It confirms the ids drawn into the *previous* fully built response: prune from disk, then enqueue broker acks.
   2. It flushes acks.
   3. It runs an inline broker poll.
   4. It reads the unread queue and filters out ids that are drawn or confirmed (`delivery.isBlocked`).
   5. It draws the rest into this call.
4. `formatInboxBlock(fresh)` is prepended to the result. On `extra.signal` abort the draws are rolled back;
   otherwise they are promoted to confirm-eligible (lines 533-546).

### Payload format (VERIFIED, `shared/piggyback.ts:33-62`)

```
PEER INBOX — N unread message(s) from your colleagues.
Read each one. Then:
  - If it's a question you can answer now: reply briefly via send_message(to_id="<from_name>", ...).
  ... (4 more instruction lines) ...
Do NOT auto-acknowledge. "Got it" / "on it" is noise.

--- message 1 of N ---
from: <name> (<peer_type>, cwd=<cwd>)
their current work: <summary>
sent_at: <iso>
message_id: <int>
text:
<body>
reply_action: send_message(to_id="<name>", message="...")

PEER INBOX
```

### When it fires in Goose

- **Only when the model calls one of the six agent-peers tools.**
- Neither version of Goose has a hook that can add context. The primer (§3.4) finds no `additionalContext` field;
  a hook's output reaches the model only through a PreToolUse deny reason or, on main only, a Stop block reason.
- MCP extensions cannot contribute to Goose's per-call MOIM block. Only platform extensions can
  (`goose@1.36 mcp_client.rs:135-137`, primer §5).
- So the project's Goose path has nothing equivalent to Claude's channel push. Delivery is as late as the model's
  next voluntary agent-peers call. INFERRED from the above.

The only thing that makes the model call at turn start is the server `instructions` text. It contains
"RULE: Call `check_messages` as the FIRST thing you do every time the user sends you a message"
(`codex-server.ts:173-225`). Goose does put MCP server instructions into the system prompt: `get_extensions_info`
reads `ext.get_instructions()` (`goose@1.36 crates/goose/src/agents/extension_manager.rs:1070-1081`), and the
template renders `{{extension.instructions}}` (`goose@1.36 crates/goose/src/prompts/system.md:23-24`). VERIFIED.
So the rule reaches a Goose model. Whether a given model obeys it on every turn is behavioural and unknown.

The text says "The current Codex CLI does NOT surface..." when the runtime is `codex`, and a Hermes sentence
otherwise (line 177). Under Goose it would name the wrong harness. VERIFIED that the text is runtime-keyed; the
effect is INFERRED.

### Size bounds

- **Per body:** none found in `shared/piggyback.ts` or `broker.ts`. INFERRED as absent; treat it as unknown.
- **Per page:** none. The block contains every fresh unread message (`broker.ts:901-910`, no LIMIT).
- **Backlog cap:** 500 undelivered messages, enforced only for a *stale* durable peer (one whose `last_seen` is past
  the cutoff); sends to a live peer are not capped (`broker.ts:55,836-843,857-861`; the Hermes-v2 conversation
  mailbox has its own 500 cap at `broker.ts:805-809`). VERIFIED.
- **Goose's own bound:** a tool result text above 200,000 characters is written to a temp file. The model then sees
  only "The response returned from the tool call was larger (N characters) and is stored in the file ..."
  (`goose@1.36 crates/goose/src/agents/large_response_handler.rs:6,20-30`, applied at `agent.rs:428`). VERIFIED.
  - Interaction: the server would still treat that call's messages as presented and confirm them on the next call.
    The model receives them only if it opens the file. INFERRED.

### Framing and per-delivery cost

- There is a fixed header of about 480 bytes per delivery block (rendered from the literal at `piggyback.ts:40-47`).
- Each message adds about 150-250 bytes of framing (`from` line with cwd, summary, `sent_at`, id, `reply_action`).
  The size depends on path and summary length.
- Static behaviour instructions are repeated on every delivery. INFERRED: this is the opposite of our R17 rule.

### De-duplication and acknowledgement (VERIFIED)

- The `DeliveryState` machine (`shared/delivery-state.ts`; `codex-server.ts:302-311,550-666`) moves each message
  through draw, then promote when the response is fully built, then confirm *when the next tool call arrives*.
- The arrival-generation barrier stops a call that was parked in `wait_for_peer_messages` from confirming a later
  response (`codex-server.ts:487-496`).
- Broker acks are sent only at confirm time, so delivery is at-least-once.
- Goose fits this machine well:
  - It sends MCP `notifications/cancelled` on both its tool timeout and user cancellation
    (`goose@1.36 crates/goose/src/agents/mcp_client.rs:536-557`, `send_cancel_message`). VERIFIED.
  - The server passes `extra.signal` into `withPiggyback` (`codex-server.ts:780`) and rolls the draw back if it is
    aborted when the response is built (`codex-server.ts:535-541`). VERIFIED. That the MCP SDK aborts the signal on
    `notifications/cancelled` is INFERRED.
  - A Ctrl+C'd turn therefore re-offers the mail instead of losing it. INFERRED.
- One gap: confirmation needs a *second* agent-peers call.
  - A Goose model that calls `check_messages` once and never touches the server again leaves those ids
    "presented, unconfirmed". The broker leases keep expiring and re-offering, and the triage refreshes the tokens
    (`codex-server.ts:356-372`).
  - Nothing is re-shown, because `isBlocked` covers presented ids. But nothing is acked until the next call, or ever
    if the session ends. If the MCP process restarts, the in-memory state is gone and those messages are shown
    again. INFERRED from the in-memory `DeliveryState`.
- There is no semantic ACK: the colleague prompt discourages replies (`piggyback.ts:46`).

## Wake mechanism

**None for Goose.** VERIFIED that no Goose wake exists in the code (see Support status). The project's wakers are all
harness-specific:

- Codex: `turn/start` / `turn/steer` over a managed app-server (`shared/wake-daemon.ts`).
- Droid: ACP or the native relay (`shared/droid-*.ts`).
- Claude: the `asyncRewake` Stop hook (`bin/claude-peer-rewake:1-30`).
- Hermes v2: host RPCs (`shared/hermes-conversation-wake.ts`).

Wake registration runs only `if (RUNTIME_IS_CODEX)` or `RUNTIME_IS_DROID` (`codex-server.ts:1001-1015`). Under a
Goose host with runtime `codex`:

- `isWakeLaunchEnv` is false without the launcher's env.
- `registerWakeableSessionIfEnabled` would do nothing useful.
- `desktopWakeEligible` is a pure env check (true for runtime `codex` without wake env,
  `shared/codex-desktop-wake.ts:13-16`); the registration then looks for a Codex app-server socket on the parent pid
  via `/usr/sbin/lsof` (`shared/codex-desktop-wake.ts:37`), and a Goose parent has none.

So `isWakeableSession` stays false. INFERRED from `codex-server.ts:882-905,937-942,1180-1260`.

Would each existing waker port to Goose? All INFERRED, using the primer:

- **Claude-style `asyncRewake` Stop hook.**
  - Goose hooks run with a 30 s default timeout and `kill_on_drop`. They have no background-rewake mode in either
    version (primer §3.1).
  - In 1.36, Stop is fire-and-forget. On main it blocks, but it fires only when a turn ends, never on an idle
    session, and it is capped at 8 consecutive blocks (primer §3.4, `goose@main ops_stop_hook.rs:19-45,86-89`).
  - The rewake pattern does not transfer.
  - The nearest Goose analogue is main's Stop block. It could keep a *finishing* turn alive when mail has arrived:
    poll the bodyless metadata file, then block with "call check_messages". That is turn-end extension, not idle
    wake, and its framing is Goose's fixed "policy hook denial" text.
- **Codex-style app-server wake.**
  - The Goose equivalent is ACP: `goose acp` / `goose serve`, with `session/prompt` to wake and, on main only,
    `_goose/unstable/session/steer` for mid-turn input (primer §7).
  - The project's `droidpeer` ACP client (`shared/droid-acp-client.ts`) shows how such a launcher could be built.
  - It would apply only to Goose sessions hosted by that launcher, not to a `goose session` in a terminal pane,
    which runs the agent in-process with no external prompt entry (primer §1).
- **Long-poll `wait_for_peer_messages`.** This is the only "wake-like" behaviour that works under Goose as shipped:
  - It blocks inside a tool call, polling every 500 ms, until fresh mail arrives or the timeout expires
    (`shared/wait-for-peer-messages.ts:60-80`; `codex-server.ts:124,318-341`).
  - Defaults: 300 s for runtime `codex`, 60 s for `hermes` (`codex-server.ts:121-123`).
  - Goose's default extension timeout is also 300 s (`goose@1.36 crates/goose/src/config/extensions.rs:11`). A
    codex-runtime wait at the 300 s default would therefore race Goose's tool timeout. Goose cancels at 300 s; the
    server rolls back and the model sees a timeout error instead of "No peer messages arrived" (INFERRED). Raise
    `timeout` in the extension config, or use the hermes runtime (60 s), to avoid it.
  - Either way, this keeps a turn alive. It does not wake an idle session.

### Idle/busy detection, draft safety, latency, failure modes

- **Idle/busy detection:** none for Goose. There is no pane or tty reading and no hook edge.
- **Draft safety:** not applicable, because nothing types into the composer.
- **Latency:**
  - A busy model sees mail at its next voluntary agent-peers call, so latency is unbounded.
  - An idle model sees it only after a human sends the next prompt *and* the model obeys the
    check-first rule. INFERRED.
- **Failure mode:** mail sits silently while the session is idle. The sender's `send_message` success means only
  "queued", as the overview notes, so the sender cannot tell.

## Setup and installation footprint on Goose

Nothing is written for Goose by the project, which ships no Goose install path (VERIFIED). A manual setup would
touch:

- `~/.config/goose/config.yaml`: one `extensions.<name>` stdio entry with `envs` (INFERRED, from the Goose doc
  above). The project's Codex prompt shows its preferred footprint, one config block and nothing else
  (`README.md:75,140-147`).
- Shared state that the server creates on its own:
  - `~/.agent-peers.db` (broker, 0600) and `~/.agent-peers-secret`;
  - `~/.agent-peers-codex/` or `~/.agent-peers-hermes/` (inbox and metadata, 0600/0700);
  - the clone with `node_modules` (`docs/ARCHITECTURE.md:42-72,144-149`, per the overview).
- Broker ownership is through launchd. A client runs `launchctl kickstart gui/<uid>/com.mike.agent-peers-broker`
  and self-spawns the broker only with `AGENT_PEERS_SPAWN_BROKER=1` (`shared/ensure-broker.ts:11-14,38,57`).
  VERIFIED. On Linux (Herdr's platform here) the user must set that variable or run the broker by hand.
- Terminal side effect: the server writes OSC 0/1/2 title sequences to `/dev/tty` every second
  (`shared/tab-title.ts:43-61`) unless `AGENT_PEERS_DISABLE_TAB_TITLE=1` is set (`tab-title.ts:53`). VERIFIED.
  - As a Goose stdio child it shares the pane's controlling tty. INFERRED.
  - It would therefore fight Herdr's or the shell's title. It would not corrupt rustyline's line, because OSC
    sequences are not printed. INFERRED.

## Token cost

- **Per session (fixed):** the server `instructions` go into Goose's system prompt (VERIFIED above).
  - That is `COLLEAGUE_PROTOCOL` (`shared/colleague-prompt.ts`, 6,382 bytes) plus the delivery section
    (`codex-server.ts:175-225`, about 2.5 KB): roughly 9 KB, about 2,200 tokens. INFERRED estimate at 4 bytes per
    token.
  - Six tool schemas add a few hundred more.
  - Goose re-sends the system prompt on every provider call. Prompt caching makes it cheap but not free. INFERRED.
- **Per message:** the body, plus about 200 bytes of per-message framing, plus a 430-byte header per delivery block.
- **Per quiet turn:**
  - If the model obeys "call `check_messages` first", every user turn costs one tool call and one model round trip.
    The tool schema, call and result total roughly 50-100 tokens, plus the latency of an extra inference step.
  - If it does not obey, the cost is zero and mail goes undelivered.
  - Inline polling adds no tokens.

## Limitations and bugs visible for Goose

1. **No Goose identity.** The `peer_type` CHECK constraint (`broker.ts:145`) forces a Goose session to masquerade as
   `codex` or `hermes`. The runtime-keyed instructions then name the wrong harness (`codex-server.ts:177`).
2. **No wake at all** for a Goose pane. Mail to an idle Goose sits until a human types.
3. **Delivery depends on model discipline.** It relies on "call check_messages first" in the system prompt. Goose
   offers no hook context injection to make delivery deterministic (primer §3.4).
4. **The preview notification is wasted under Goose.** It is shown only in the UI and only during that server's
   tool calls (primer §6). The server still sends one per message.
5. **The wait timeout collides with Goose's tool timeout.** Both default to 300 s for runtime `codex`
   (`codex-server.ts:121-123`; `goose@1.36 config/extensions.rs:11`). INFERRED race.
6. **Goose's 200,000-character spill to a file** silently turns a large inbox block into a pointer. The server
   still counts it as presented. INFERRED interaction.
7. **The orphan check is Linux-unsafe.** `parentProcessWasLost` treats only `ppid === 1` as orphaned
   (`shared/process-lifecycle.ts:1-3`). On Linux with a user-level subreaper (systemd `--user`, many terminal
   multiplexers) an orphaned MCP reparents to the subreaper instead. The server would keep heartbeating as a live
   peer after Goose dies. INFERRED, Linux-specific. The project targets macOS; Linux is "untested", per the
   overview.
8. **macOS-centric broker startup** (launchd). Codex Desktop discovery uses `lsof` and is irrelevant here.
9. **Unbounded page size.** All unread mail is sent in one block with no per-body cap (`broker.ts:901-910`).
   VERIFIED for the broker query and `formatInboxBlock`.

## Clever ideas worth stealing (for a herdr-threads Goose adapter)

All INFERRED.

- **Confirm on the next call, with cancel rollback.** "Shown" is defined as "the response was fully built and the
  model called us again". Goose's `notifications/cancelled` on timeout and Ctrl+C (`goose@1.36 mcp_client.rs:536-557`)
  makes the rollback half reliable under Goose. If we ever deliver through an MCP tool result for Goose, this is a
  cheap, honest display criterion that does not need a separate ACK call.
- **A bodyless metadata file as the waker and hint source.** Applied to Goose:
  - Point Goose's `tom` platform extension at a herdr-threads-maintained file: `GOOSE_MOIM_MESSAGE_FILE`, re-read
    before every provider call and capped at 64 KiB (primer §5).
  - The file would hold a one-line "pending: N items, ids ..." hint, or the full bodies.
  - That gives mid-turn delivery after every tool batch with no tool call, which beats this project's
    tool-piggyback, which Goose cannot improve on.
  - Its confirm rule could mirror theirs: clear the file once a later `PostToolUse` or `Stop` hook fires for that
    session.
- **A sender-filtered long-poll that never consumes non-matching mail** (`codex-server.ts:318-341`). This is useful
  for a Goose "stand by for X" tool. Set its timeout below Goose's extension timeout.
- **A disable flag file for hosts where env cannot be injected** (`hermes-server.ts:6-9,24-32`). Goose 1.36 reads
  `tom` env only from the process environment, so a similar file-based switch would help a managed launch.
- **Do not copy:**
  - the per-delivery instruction header;
  - the closed `peer_type` enum (a new harness means a schema migration);
  - "obey the system prompt and call check_messages first" as the main delivery trigger.

## Verification

Adversarial pass, 2026-10-23, against the clone at `2c63379d6`, the primer and the two Goose trees it names.

- Checked and held: the `goose` grep hits; `peer_type` CHECK at `broker.ts:145,307,454`; runtime selector
  `codex-server.ts:103-107`; `bin/` contents; shim sizes; the six tool names; `finalText = inbox + toolText`;
  poll constants and scheduler; broker query `ORDER BY id ASC`, no LIMIT; preview push and `formatInboxPreview`;
  `formatInboxBlock` layout; instructions text and the runtime-keyed sentence (line 177); arrival-generation barrier;
  sender-filtered wait and its 500 ms loop; wait defaults 300 s / 60 s; wake registration gates (`1001-1015`);
  launchd kickstart and `AGENT_PEERS_SPAWN_BROKER`; OSC 0/1/2 writes, 1 s keepalive, disable flag;
  `parentProcessWasLost` (`ppid === 1`); `colleague-prompt.ts` size (6,382 bytes, file size). Goose: extension YAML
  shape, `get_extensions_info` / `get_instructions` / `system.md`, 200,000-char spill (`agent.rs:428`), default
  timeout 300, `send_cancel_message` on timeout and cancel, MCP `get_moim` returns `None`, notifications subscribed
  inside dispatch (`extension_manager.rs:1663`), main's Stop-hook framing and `ends_turn` gate.
- Changed: `withPiggyback` citation (was 694-830, the dispatch handler range); banner has no brackets; lease is
  set by the broker and is VERIFIED, not "per the overview"; backlog cap applies only to stale durable peers
  (was stated as unconditional); header size about 480 bytes, not 430, and citation 40-47; Hermes-v2 exception
  noted; cancel/rollback split into VERIFIED server side and INFERRED SDK side; `desktopWakeEligible` described
  correctly (env check, then `lsof` discovery); unlabeled inferences tagged.

