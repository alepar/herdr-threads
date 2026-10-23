# cross-agent-teams-mcp (jtianling, "xats") x OpenAI Codex CLI

- Upstream: https://github.com/jtianling/cross-agent-teams-mcp. The clone was read at
  `17df5cf7bd9b66f00460eff05080b0e0495b6ec8` (npm `0.8.6`). Paths are relative to the clone root. The code was read, never run.
- Codex source cross-checks use the primer's sparse clone of `openai/codex` at `rust-v0.163.0`. Those paths are written `codex:codex-rs/...`.
  The generated 0.163.0 app-server schema is at `.../scratchpad/codexprimer/schema/`.
- Tags:
  - **VERIFIED**: read in source or docs.
  - **INFERRED**: my reasoning from what I read; not observed running.
  - **unknown**: not found.

## 1. Support status

**Native, with a dedicated wake transport.** VERIFIED.

- Codex agents connect to the xats daemon as ordinary MCP clients over Streamable HTTP. The daemon
  wakes them through Codex's own app-server JSON-RPC WebSocket (`delivery.kind = 'codex-appserver'`):
  `initialize`, then `thread/resume`, then `turn/start` (idle) or `turn/steer` (busy). See
  `src/mcp/codex-appserver-dispatch.ts:117-245` and `src/mcp/transport-dispatch.ts:205-235`.
- A tmux paste is the fallback.
- README: "Wake-ups go through Codex's own app-server WebSocket transport — there is no channel proxy
  involved" (`README.md:149`).

Prerequisites (VERIFIED, `README.md:147-216`, `README.agent.md:56-74`):

1. A **resident** `codex app-server --listen ws://127.0.0.1:8799`, and the TUI launched as
   `codex --remote ws://127.0.0.1:8799 -C "$PWD"`. Without `--remote`, the thread lives in a process
   the daemon cannot reach. The README's minimum config (MCP entry only, no app-server delivery) is
   "mailbox only, no push wake" (`README.md:151`, `:167`).
2. An MCP server entry the **app-server** can see. Under `--remote`, MCP servers are loaded by the
   app-server, not by the TUI (`README.md:173`).
3. A tmux pane plus the `free-xats-codex` / `xats-codex` launcher, for the pane binding that the
   tmux fallback and identity recovery rely on (`README.md:187-216`).
4. Codex 0.124.0 or later, which exports `CODEX_THREAD_ID` to MCP tool processes
   (`README.agent.md` section 2.2). The docs say the setup was verified on Codex 0.144.x (`README.md:173`).
   I did not observe it against 0.163.0. Notes on version drift are in section 6.

## 2. Injection mechanism

### 2.1 What reaches the model

There are two pieces. VERIFIED.

1. **A wake hint, injected as a user turn (or a steer) through the app-server.** It never carries the body.
   - The hint text comes from `buildAutoPokeHint` (`src/mcp/tools.ts:367-397`):

     ```ts
     const render = (who: string, segment: string): string =>
       `新邮件 from ${who}${segment}, 请调 get_inbox 查看`
     ```

     It renders as `新邮件 from alice@proj (<36-char agent uuid>) → bob@proj, 请调 get_inbox 查看`
     ("new mail from ..., please call get_inbox to view").
   - It is capped at `HINT_MAX_CHARS = 200` (`tools.ts:367`). To fit, it drops first the `→ target`
     segment and then the sender name (`tools.ts:390-396`).
   - The `→ recipient` segment exists so that a pane that gets a misrouted wake can tell
     (`tools.ts:196`).
2. **The bodies, pulled by the model** through the `get_inbox` MCP tool (`src/mcp/tools.ts:2925-2948`,
   `src/mcp/get-inbox.ts:30-79`).
   - The result is `JSON.stringify` of `{messages:[...], has_more, last_event_id}`
     (`tools.ts:101-103`).
   - Each message carries 14 fields: `id, event_id, from_team, to_team, from_agent_id, from_name,
     from_device, from_role, to_agent_id, to_role, subject, body, need_reply, sent_at`
     (`get-inbox.ts:4-19`).

There are no Codex hooks of any kind: no `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PostToolUse` or `Stop`. There is no `additionalContext` and no `thread/inject_items`. VERIFIED: no
hook config exists in the repo, and the only app-server methods used anywhere in `src/` are
`initialize`, `thread/resume`, `turn/start`, `turn/steer`, `thread/loaded/list` and `thread/read`
(grep).

### 2.2 Code path for one `send_message` to a Codex recipient

All steps VERIFIED.

1. `SendMessageService.send` writes the mailbox row and the event in one transaction **before** any
   wake (`src/mcp/send-message.ts:180`, `:219-251`).
2. It **awaits** `runFanoutWithRetry` (`send-message.ts:197-206`, `src/mcp/fanout-with-retry.ts:27`).
   That calls `fanoutAutoPoke` (`src/mcp/auto-poke-fanout.ts:94-127`), which calls `createAutoPokeImpl`
   (`tools.ts:399-453`), which builds the hint and calls `poke()` (`src/mcp/poke.ts:271-293`).
   `poke()` reaches `dispatchPoke`, which routes on the effective agent type to `dispatchCodex`
   (`transport-dispatch.ts:101-112`, `:205-235`).
3. `dispatchCodexAppserverPoke` (`codex-appserver-dispatch.ts:81-255`) then does the following. It
   opens a **new short-lived WebSocket for every wake** (a design decision: "每次 poke 建立短连接",
   `openspec/changes/archive/2026-04-21-add-codex-appserver-delivery/design.md:30-42`).
   - It sends the bearer token from `process.env[auth_token_ref]` when one is configured
     (`codex-appserver-rpc.ts:108-121`; `dispatch.ts:88-96`).
   - `initialize {clientInfo:{name:'cross-agent-teams-mcp'}, capabilities:{experimentalApi:true}}`,
     then the `initialized` notification (`:117-136`).
   - `thread/resume {threadId, persistExtendedHistory:false}` (`:138-141`).
   - A busy check on the resume result: it scans `result.thread.turns` from the end for
     `status === 'inProgress'` (`:265-278`).
   - `turn/start` when there is no active turn. Otherwise `turn/steer` with `expectedTurnId` (`:150-162`):

     ```ts
     const turnMethod = activeTurnId === undefined ? 'turn/start' : 'turn/steer'
     ... input: [{ type: 'text', text: input.content, text_elements: [] }],
     ...(activeTurnId === undefined ? {} : { expectedTurnId: activeTurnId }),
     ```

   - On `turn/start`, it waits up to 5 s for a matching `turn/started` (`:180-204`).
   - In both cases it then waits up to **30 s** for an `item/started` notification whose item is
     `{type:'mcpToolCall', tool:'get_inbox'}` in that thread and turn (`:206-228`, `:302-317`). Only
     that counts as success.
   - On success it keeps the socket open until `turn/completed`, for at most 10 minutes, and then
     closes it (`:230-239`). The reason for holding the socket is not documented (unknown). The
     likely reason is that disconnecting the client that started a turn could disturb it (INFERRED).

### 2.3 When it fires

- It fires **on every send** with `auto_poke` left at its default of `true`, to every recipient
  (`tools.ts:182`, `send-message.ts:182`). There is no priority and no threshold. VERIFIED.
- **Idle recipient:** `turn/start` opens a new turn whose first user message is the hint. VERIFIED code;
  that Codex then samples is INFERRED from the primer (§2.2).
- **Busy recipient:** `turn/steer` appends the hint to the running turn. The model sees it at the next
  sampling request. The primer (§2.2, §5) also says this ends a model `sleep` early. VERIFIED code;
  the sampling-boundary timing is INFERRED from the primer.
- **Turn with a pending approval** (`active{waitingOnApproval}`): the turn counts as `inProgress`, so it
  is steered. The model sees the hint only after the human answers. INFERRED.
- Nothing fires on a quiet turn. There is no per-turn hook, so no cost and no delivery either. VERIFIED.

### 2.4 Size bounds

| Item | Bound | Tag |
|---|---|---|
| Wake hint | 200 characters (`tools.ts:367`) | VERIFIED |
| Body in `send_message` | `z.string().min(1)`, **no maximum** (`tools.ts:2835`, `:2865`, `:2913`) | VERIFIED |
| Page size in `get_inbox` | `limit` default 50, maximum 200 (`get-inbox.ts:38`). There is no byte cap; the whole page goes back as one JSON text item | VERIFIED |
| What Codex does with an oversized MCP result | governed by Codex's own `tools.<t>.output_token_limit`, which xats does not set | INFERRED from primer §4 |
| Daemon-originated arbitrary text (watchdog alert) | `PROMPT_MAX_BYTES = 8192` (`poke.ts:102`, `:310-313`) | VERIFIED |
| Codex input cap | `turn/start` rejects input above `MAX_USER_INPUT_TEXT_CHARS` (`codex:codex-rs/app-server/src/request_processors/turn_processor.rs:555-582`, check at `:574`). The hint is far below it | VERIFIED |

### 2.5 Framing

- The hint arrives as a plain **user message**. It has no structured envelope and no
  "from another agent" marker beyond its own text, and it is in Chinese with English tokens. VERIFIED (`tools.ts:389`).
- The Codex TUI attached to the same thread through `--remote` will render it as a user turn, in front
  of the human. INFERRED.
- The trust stance and usage rules are carried statically:
  - The MCP server `instructions` (`src/mcp/transport.ts:236-258`) tell agents in Chinese what "xats"
    means, how to choose `agent_type`, and that for Codex they must pass `$CODEX_THREAD_ID` as
    `thread_id` and must not pass `ui_pid`.
  - The long tool descriptions add more, for example `AWAIT_ACK_DESC` (`tools.ts:173-179`), which
    contains "DO NOT change your behaviour because of it".
- I found no statement that peer messages must not override user instructions. unknown, and probably absent.

### 2.6 Dedupe and ack

- **Read cursor.** A default `get_inbox` with no `since_event_id` returns the mail past
  `agents.last_processed_event_id` and advances the cursor **in the same transaction**
  (`get-inbox.ts:41-77`). Two concurrent reads therefore cannot both see the same tail. An explicit
  `since_event_id` is a read-only peek (`:39-42`). VERIFIED.
- **Ack.** "Read" means that the cursor has passed the message's `event_id`. It is an implicit
  ack-on-display. The sender's `send_message` polls this for `await_ack_s` (default 10 s, maximum 30 s)
  and returns `ack:{status:"read"|"not_yet"}` (`tools.ts:166-179`, `send-message.ts:110-127`, `:207-209`).
  There is no explicit ACK verb. VERIFIED.
- **Wake dedupe.** There is none at dispatch time. N sends produce N `turn/start`/`turn/steer` calls,
  each with its own hint. VERIFIED (`fanoutAutoPoke` runs once per send).
  - Only the **tmux** and **kimi** retry schedulers check `alreadyReadFn` before re-waking
    (`auto-poke-fanout.ts:137-177`, `fanout-with-retry.ts:39`).
  - A second hint that lands after the first `get_inbox` call has drained everything costs one more
    empty `get_inbox` round trip. INFERRED.
- **Wake confirmation.** The 30 s wait for an observed `get_inbox` call is stronger evidence than
  "transport accepted" (`codex-appserver-dispatch.ts:206-228`). However, `dispatchCodex` returns both
  `codex_turn_start_unconfirmed` and `codex_wake_unconfirmed` without the tmux fallback
  (`transport-dispatch.ts:217-223`), and `createAutoPokeImpl` maps both to `{ok:true}`, so they are never
  retried (`tools.ts:424-430`):

  ```ts
  if (err === 'codex_turn_start_unconfirmed' || err === 'codex_wake_unconfirmed') {
    // The app-server accepted the turn input. Confirmation uncertainty must
    // not trigger tmux fallback or retry duplicate delivery.
    return { ok: true }
  }
  ```

  In the auto path, therefore, confirmation only adds latency for the sender. It does not change the
  outcome. VERIFIED.

## 3. Wake mechanism

### 3.1 Exact mechanism

The wake is an app-server `turn/start`, or a `turn/steer`, issued over a WebSocket to the resident
app-server that holds the thread. VERIFIED (section 2.2).

- The delivery coordinates are `{thread_id, ws_url, auth_token_ref?}` (`src/lib/delivery-spec.ts:12-17`).
  They are validated as a UUID and a `ws://`/`wss://` URL (`delivery-spec.ts:232-266`).
- `ws_url` is picked at registration time (`src/mcp/register-codex-self.ts:188-194`, `:221-283`,
  `:344-370`). The candidate list is the first of:
  1. an explicit `ws_url`;
  2. the legacy `CROSS_AGENT_TEAMS_CODEX_WS_URL` environment variable;
  3. the URLs in `CROSS_AGENT_TEAMS_CODEX_WS_URLS`;
  4. the default `ws://127.0.0.1:8799`.

  Every candidate in that list is then probed with `initialize` and `thread/resume{threadId}`
  (`:344-352`), not only the multi-URL form.

  Registration fails if more than one endpoint answers (`codex_endpoint_ambiguous`). VERIFIED.

### 3.2 Idle and busy detection

The client side reads `thread.turns[].status === 'inProgress'` from the `thread/resume` response
(`codex-appserver-dispatch.ts:265-278`). It does not use `thread/status/changed`, and it does not
subscribe for long. VERIFIED.

Two things about 0.163.0 matter here:

- **`turn/start` already starts or steers on the server side.** `turn_start_inner` calls
  `thread.start_or_steer_turn(...)` (`codex:codex-rs/app-server/src/request_processors/turn_processor.rs:662`,
  with `TurnInputSubmission::Steered` handled at `:689`). `start_or_steer_turn` lives at
  `codex:codex-rs/core/src/codex_thread.rs:417-423`. So on current Codex the client-side branch
  matters only for the confirmation logic. VERIFIED (Codex source).
  - If the thread turns busy between `thread/resume` and `turn/start`, the request becomes a steer and
    no `turn/started` arrives.
  - xats then returns `codex_turn_start_unconfirmed` after 5 s, which the auto path treats as success.
    The input was still delivered. INFERRED from the two sources together.
- **Full-history hydration is deprecated for paginated threads.** The message is: "use
  `excludeTurns: true`, then page with `thread/turns/list`"
  (`codex:codex-rs/app-server/src/request_processors/thread_processor.rs:42`).
  - xats resumes without `excludeTurns`, so every wake hydrates and transfers the thread's full history
    over the socket just to find the last turn's status. That is a cost on the daemon side, not
    tokens. INFERRED.
  - If a future Codex stops populating `turns`, `extractActiveTurnId` will always return undefined.
    xats will then always send `turn/start`, which still steers on the server. INFERRED.
- `persistExtendedHistory` does not appear in the 0.163.0 `ThreadResumeParams` schema
  (`codexprimer/schema/v2/ThreadResumeParams.json`). It is apparently ignored. INFERRED.

### 3.3 Draft safety

- On the app-server path, nothing is typed into the TUI. The turn starts on the server, and the TUI is
  just another subscriber, so a human's composer draft is not overwritten. INFERRED (the primer's §3
  reasoning applies equally to `turn/start`).
- There is still a UX hazard: if a human is mid-draft and an injected turn starts, their Enter during
  that turn becomes a steer of the injected turn, not a new turn (primer §7). INFERRED.
- The **tmux fallback** is much weaker. For ordinary mail wakes it relies only on the quiet guard.
  - `dispatchTmux` passes no `requireReady` predicate (`transport-dispatch.ts:134-141`).
  - The guard (`src/mcp/poke-guard.ts:26-32`) captures the pane, waits 2 s, captures again and requires
    the two captures to be identical. `capture-pane -S -8` captures the visible screen plus 8 history
    lines (`src/daemon/tmux-cli.ts:24`; the reading of `-S -8` is INFERRED from tmux semantics).
  - A motionless human draft in the composer passes, and the hint is then pasted into it and submitted
    (`poke.ts:212-234`). VERIFIED code; the clobbering itself is INFERRED.
  - The Codex composer allowlist (`›` prefix, `src/mcp/codex-prompt-readiness.ts:21-29`) is applied
    **only** to the identity seeding and recovery pastes (`codex-seeding-poke.ts:416`,
    `codex-recovery-poke.ts:636`). Mail wakes do not get it. This corrects the overview, which implied
    that it applies to every Codex tmux poke. VERIFIED.
  - Even where it is applied, the allowlist matches any captured line that starts with `›`. If past
    user messages in the scrollback also render with `›`, a menu screen could pass. unknown: I have not
    checked Codex's history rendering.
  - The fallback paste does use bracketed paste (`paste-buffer -p`, `tmux-cli.ts:48`) and a 400 ms
    settle before `Enter` (`poke.ts:103`, `:225`, `:234`). That sits above Codex's 120 ms paste-burst
    window (primer §7). VERIFIED.
  - It also re-checks pane ownership and the foreground Codex carrier (`ps -t <tty>`, a live
    `codex --remote` command, `pgid == tpgid`) before the paste and again before Enter
    (`poke.ts:201-233`, `:419-435`). On this path the argv uuid is **not** checked ("The stored uuid is
    unavailable on this path; the command-level match suffices", `poke.ts:410-412`), and the carrier
    check is skipped when the row has no `runtime_ui_pid` (`poke.ts:426`). VERIFIED.

### 3.4 Latency

- App-server idle wake: one WebSocket connect plus three RPCs. That is sub-second on loopback, then the
  model starts a turn. INFERRED.
- Busy steer: the hint is visible at the next sampling step. INFERRED (primer §2.2).
- **The sender blocks.** The fan-out is awaited inside `send_message` (`send-message.ts:197-206`). The
  dispatcher in turn waits up to 5 s for `turn/started` and 30 s for the `get_inbox` call. A single send
  can therefore hold the sender's tool call for up to about 35 s, plus the `await_ack_s` poll (default
  10 s, `send-message.ts:207-209`). VERIFIED code; actual durations are INFERRED.
  - The normal case is short: the recipient calls `get_inbox` within a few seconds.
  - The bad cases are long: the recipient is busy in a long tool call, is waiting on approval, or
    decides not to call `get_inbox`.
  - The 30 s ceiling on `await_ack_s` exists because of the sender's harness tool timeout
    (`tools.ts:159-166`). The **dispatch wait is not counted against that ceiling**, so about 45 s in
    total is possible. That is close to Codex's 60 s default `tool_timeout_sec` (primer §4) when the
    sender is itself a Codex agent. INFERRED.
- Tmux fallback: at least 2 s of guard plus 0.8 s of settle. On `guard_failed`, retries follow at 30 s,
  180 s and 600 s (`auto-poke-fanout.ts:158-177`, `src/mcp/poke-retry.ts:1-2`). VERIFIED.

### 3.5 Failure modes

1. **App-server unreachable, or resume or start fails**, with a bound pane: it falls back to a tmux paste
   (`transport-dispatch.ts:217-224`). With no pane, the error maps to `guard_failed` and is never
   retried, because retries require `paneId` (`tools.ts:449-451`, `auto-poke-fanout.ts:158`). The mail
   then waits for a voluntary `get_inbox` or the 15-minute unread alert to the sender. VERIFIED.
   - **The docs and spec contradict the code.** They state that a Codex app-server failure "will not
     auto-fallback to tmux":
     - `docs/configs/codex-cli.md:287`;
     - `openspec/specs/agent-delivery/spec.md:314`, `:351-356`;
     - the original design decision 5,
       `openspec/changes/archive/2026-04-21-add-codex-appserver-delivery/design.md:84-95`.
   - The code and test do fall back: `tests/transport-dispatch.test.ts:157`, "falls back to tmux when
     codex dispatcher fails".
   - The CHANGELOG names this fallback as the misdelivery path behind a fix: a row left holding a
     competing session's pane after a CAS drift would receive the tmux fallback "into someone else's
     terminal" (`CHANGELOG.md:32`). VERIFIED that the CHANGELOG describes it; whether it was observed
     in production or only identified is not stated there.
2. **The socket drops after the turn was accepted.** A close while waiting for confirmation is reported
   as `codex_connect_failed`, not as unconfirmed (`codex-appserver-dispatch.ts:215-221`; test
   `tests/codex-appserver-dispatch.test.ts:237`). `dispatchCodex` then falls back to tmux. The recipient
   gets the hint twice: once as the app-server turn and once as a pasted prompt. VERIFIED code path;
   the double delivery is INFERRED.
3. **The TUI is gone but the app-server is alive.** `dispatchCodex` has no liveness check. Closing the
   MCP session releases only in-memory connection state (`src/mcp/transport.ts:150-173`,
   `register-agent.ts:585-597`); the `codex-appserver` delivery row stays.
   - `thread/resume` reloads or rejoins the thread on the app-server, and `turn/start` runs a **headless
     turn**. The model calls `get_inbox`, which advances the cursor, so the sender sees
     `ack: read`. It may even reply, with no human or TUI attached. INFERRED from the code plus primer
     §2.1-2.2 (`thread/resume` loads a stored thread).
   - This makes "read" receipts unreliable for dead panes.
4. **Restarting the app-server** (for example `start-xats`) ends every `--remote` conversation on the
   machine. The TUIs show `Connection reset without closing handshake`, and the user must
   `codex resume <id>` by hand (`README.agent.md:329-334`). VERIFIED (their own measurement).
5. **Watchdog alerts to a Codex sender are mislabelled.** The unread alert tells the model
   `这不是新邮件, 无需调 get_inbox` ("not new mail, no need to call get_inbox",
   `src/mcp/unread-watchdog.ts:183-188`). The Codex dispatcher, however, reports success only after it
   sees a `get_inbox` call.
   - `pokeAsDaemon` returns the raw dispatcher result (`poke.ts:306-317`), so each alert to a Codex
     sender blocks the watchdog loop for about 30 s. It is then logged as
     `alert undelivered ... error=codex_wake_unconfirmed`, although it was delivered. It is not retried,
     because the error is not in `TRANSIENT_ALERT_FAILURES` (`unread-watchdog.ts:65-69`, `:264-289`).
   - The sweep is sequential (`for (const row of due)`), so several Codex senders delay one another.
   - VERIFIED code; runtime effect INFERRED.
6. **Approval-gated `get_inbox`.** `xats-codex`, unlike `free-xats-codex`, runs without
   `--dangerously-bypass-approvals-and-sandbox` (`README.agent.md:280-281`). Whether Codex asks for
   approval of the MCP `get_inbox` call, and whether `item/started` is emitted before the approval, is
   unknown. Either way confirmation can time out, which is harmless because it maps to `ok`.
7. **Thread rotation.** `/clear` and `/new` create a new thread id, while the delivery row still
   points at the old thread. Wakes then go to the old thread, possibly as headless turns (see 3).
   Recovery is model-driven: `reconnect({launch_id: $XATS_CODEX_LAUNCH_ID, thread_id: $CODEX_THREAD_ID})`.
   Forks and subagent threads may not use the inherited id to recover the parent's identity
   (`docs/launchers/free-xats-codex.md:55-59`). VERIFIED doc; the stale-row effect is INFERRED.

### 3.6 Identity, as it affects delivery

- Registration is model-driven. The model reads `$CODEX_THREAD_ID` and passes it as `thread_id`.
  `ui_pid` must not be passed (`transport.ts:250`, `docs/configs/codex-cli.md:50-67`). VERIFIED.
- Under `--remote`, the agent's tool shells run inside the **shared app-server's** environment. So
  `$TMUX_PANE` belongs to whichever pane started the app-server.
  - xats says so explicitly in `src/mcp/codex-recovery-poke.ts:112-116` ("its tools run in a shared
    app-server, where $TMUX_PANE is some other pane's id") and in `CHANGELOG.md:20`.
  - This is the same daemon-environment leak that herdr-threads works around with `--no-daemon`
    (primer §2.1, §9). VERIFIED.
- Their fix has three parts:
  - The **launcher** announces `pane → uuid` before `exec codex` (120 s TTL) and puts
    `-c xats.agent_id="<uuid>"` in argv. The daemon finds the pane's foreground Codex process by
    `ps -t <tty>` and matches that argv (`README.md:191-216`, `src/mcp/auto-bind-codex-pane.ts:183-215`).
  - When that is ambiguous (two panes launched close together), the daemon **pastes a one-time nonce
    into the pane through tmux**, and the model quotes it back in `register_agent({recovery_nonce})`
    (`src/mcp/codex-seeding-poke.ts:100-116`, `CHANGELOG.md:22`).
  - The pane-attested nonce is the only correlation the daemon creates itself. Their experiments found
    that `CODEX_THREAD_ID` is the only per-session value a model can read (`CHANGELOG.md:22`).
    VERIFIED.

## 4. Setup and installation footprint on the harness

All VERIFIED unless marked.

- **Codex config.** `[mcp_servers.cross-agent-teams-mcp] type="streamable-http" url="http://127.0.0.1:9100/mcp"`,
  optional `bearer_token_env_var`, and a top-level `experimental_use_rmcp_client = true`
  (`README.md:155-165`, `README.agent.md` section 2.2).
  - It is written by `npx mcpsmgr@latest add jtianling/cross-agent-teams-mcp -a codex --global` or by
    hand. It goes to the global `~/.codex/config.toml`, or to a trusted project's `.codex/config.toml`
    (`README.agent.md` sections 2.2 and 3.2).
  - **The docs disagree about which home.** `docs/configs/codex-cli.md:6-9`, `:18-21` and
    `docs/launchers/free-xats-codex.md:5-7` say the CLI runtime uses a separate `~/.codex-cli`.
    `README.agent.md:60-63` and `:317` say it clears `CODEX_HOME` and uses `~/.codex`. The spec
    `openspec/specs/codex-runtime-isolation/spec.md:8` says the CLI "MUST" use an independent
    `CODEX_HOME`.
  - Whether `experimental_use_rmcp_client` still matters on 0.163.0 is unknown. It does not appear in
    the primer's sparse source clone.
- **No hooks, no `hooks.json`, no AGENTS.md or skill edits**, and no hook trust to grant.
- **Shell.** `~/.xats.sh`, sourced from `~/.zshrc`, written only with the user's consent
  (`README.agent.md:42-43`, `:91-...`). It defines:
  - `start-xats` / `stop-xats`;
  - `_xats-codex`, which auto-starts a disowned `codex app-server --listen ws://127.0.0.1:8799` when the
    port is closed, logs to `~/.config/xats/codex-cli-app-server.log`, and runs `pre-register-codex-pane`;
  - the launch itself: `codex --remote ws://127.0.0.1:8799 -C "$PWD" -c xats.agent_id="\"<uuid>\"" -c
    shell_environment_policy.set.XATS_CODEX_LAUNCH_ID="\"<uuid>\""`
    (`README.agent.md:238-281`, `docs/launchers/free-xats-codex.md:28-48`).
  - `free-xats-codex` adds `--dangerously-bypass-approvals-and-sandbox` (`README.agent.md:280`).
- **Resident processes.** The xats daemon on 9100 and the Codex app-server on 8799. The app-server is
  unauthenticated on loopback by default; `--ws-auth capability-token` is optional
  (`docs/configs/codex-cli.md:113-129`). Any local process can drive any loaded Codex thread through
  it. That is acceptable under a same-user cooperative model, but it is a wide surface. INFERRED.
- **Daemon state.** `~/.cross-agent-teams-mcp/data.db` (default `--db`, `README.md:92`), the token in `~/.config/xats/token`
  (`README.agent.md` section 2.1 notes).

## 5. Token cost

- **Per message, idle recipient:**
  - one new turn, with the full context re-sent (cached), whose input is the hint: about 100 characters,
    roughly 40 to 60 tokens with the 36-character uuid;
  - one `get_inbox` MCP call (small);
  - the result: about 300 to 400 bytes of JSON overhead per message plus the body, roughly 100 tokens of
    overhead per message;
  - plus whatever the model does next.
  - The turn itself dominates. INFERRED estimate.
- **Per message, busy recipient:** about 50 tokens of steer hint mid-turn, plus a `get_inbox` call that
  the model is invited to make immediately, plus the possible distraction from the current task.
  - Each additional send triggers another steer, because nothing coalesces.
  - After the first drain, later hints produce empty `get_inbox` calls (`{"messages":[],...}`, about 50
    bytes).
  - INFERRED.
- **Per quiet turn:** zero injected tokens. No hooks run. VERIFIED.
- **Static, per session:**
  - MCP `instructions`: about 4 KB of source text (`transport.ts:236-258`).
  - Descriptions for 18 tools: 17 registered in `tools.ts` plus `echo` (`transport.ts:262`) (`tools.ts`, for example `SEND_MESSAGE_DESC` at `:181-201`, the
    `register_agent` detection procedure at `:1869-...`).
  - Measured as string literals: about 11.7 KB in the shared `*_DESC` constants plus about 23.5 KB in
    the registration region. The second figure is an upper bound, because it includes non-description
    strings.
  - That is roughly 8,000 to 10,000 tokens per Codex session if Codex sends all MCP tool schemas up
    front. INFERRED; whether 0.163.0 defers MCP tool schemas is unknown.
  - Several tools are launcher-only or channel-proxy-only (`pre_register_codex_pane`,
    `subscribe_channel_wake`, `reserve/commit_opencode_runtime`, `bind_channel`), so a Codex agent pays
    for them without using them.

## 6. Limitations and bugs I can see

| # | Issue | Evidence | Tag |
|---|---|---|---|
| 1 | No priority or threshold: every default send wakes (or steers) the recipient. Only `auto_poke:false` opts out | `tools.ts:182`, `send-message.ts:182-195` | VERIFIED |
| 2 | Bodies always cost a second, model-initiated round trip (`get_inbox`). There is no in-turn body delivery through hooks | §2.1 | VERIFIED |
| 3 | The sender's `send_message` blocks on the wake confirmation (up to about 35 s) plus the ack poll (10 s by default). The confirmation result is then discarded (treated as ok) | `send-message.ts:197-209`, `codex-appserver-dispatch.ts:188`, `:213`, `tools.ts:426-430` | VERIFIED code, durations INFERRED |
| 4 | A socket drop after the turn was accepted falls back to tmux, so the recipient gets a double wake | `codex-appserver-dispatch.ts:215-221`, `transport-dispatch.ts:217-224` | VERIFIED path, effect INFERRED |
| 5 | A dead TUI with a live app-server gets a headless turn: the cursor advances, so the read receipt lies | `transport-dispatch.ts:205-235` (no liveness check), `transport.ts:150-173` | INFERRED |
| 6 | Watchdog alerts to Codex senders always come back `codex_wake_unconfirmed`: logged as undelivered, about 30 s per alert, sequential | `unread-watchdog.ts:183-188`, `:237-289`; `poke.ts:306-317` | VERIFIED code, effect INFERRED |
| 7 | The tmux fallback for mail wakes has no composer-readiness or draft check; a static draft gets the hint pasted in and submitted | `transport-dispatch.ts:134-141`, `poke-guard.ts:26-32` | VERIFIED code, effect INFERRED |
| 8 | Doc and spec drift: "no tmux fallback" (docs and spec) contradicts the code. The design said `turn/start` only, with no steer, but the code steers. `openspec/specs/codex-appserver-transport/spec.md:38-47` still describes `turn/start` plus close-on-success, with no confirmation wait. The `~/.codex` vs `~/.codex-cli` disagreement is in §4 | cited above | VERIFIED |
| 9 | Every wake hydrates the full thread history (`thread/resume` without `excludeTurns`), which 0.163.0 deprecates for paginated threads | `codex-appserver-dispatch.ts:138-141`, `codex:.../thread_processor.rs:42` | VERIFIED |
| 10 | It depends on `codex --remote` and an externally managed app-server. Restarting it kills every conversation, and MCP config resolves in the app-server, not the TUI | `README.md:173`, `README.agent.md:329-334` | VERIFIED |
| 11 | Each wake opens a new socket and can hold it for 10 minutes; a burst of sends leaves many subscribers on one thread | `codex-appserver-dispatch.ts:230-239` | VERIFIED code, impact INFERRED |
| 12 | The notification buffer is capped at 100 entries, and the oldest are shifted out (`codex-appserver-rpc.ts:188-189`). Streaming deltas from a busy steer target could evict a buffered `get_inbox` `item/started` that arrived before the waiter was armed. Waiters are armed right after the RPC response, so the risk is small | `codex-appserver-rpc.ts:172-201` | INFERRED |
| 13 | Identity is the most fragile part: argv uuid matching, a 120 s pre-registration TTL, nonce pastes, CAS and generation fences. The CHANGELOG documents production pane and key theft between Codex panes | `CHANGELOG.md:20-43`, `auto-bind-codex-pane.ts` | VERIFIED |
| 14 | The hint is a user-attributed message in Chinese, with no machine-parsable header and no trust note | `tools.ts:389` | VERIFIED |

## 7. Clever ideas worth stealing

All of these are INFERRED as recommendations; the mechanisms they rest on are cited above.

1. **App-server `turn/start` / `turn/steer` instead of keystrokes.**
   - It never touches the composer, so draft clobbering is impossible on this path.
   - It reaches a busy turn natively (a mid-turn steer), which herdr-threads gets today only at a
     `PreToolUse ^Bash$` boundary.
   - On 0.163.0, `turn/start` alone already does start-or-steer on the server (`turn_processor.rs:662`),
     so one call covers both cases.
   - The catch is the same as ours: it needs a connection to the server that holds the thread (the
     shared daemon or `--remote`). That conflicts with herdr-threads' `--no-daemon` advice (primer §2.1,
     §10). For idle-only wakes, the durable `thread/queue/add` (primer §3) is the closer fit to our
     contract: idle-gated on the server, never a steer, and it survives the session.
2. **Confirm a wake by observing the model act.** xats watches for `item/started{mcpToolCall,
   tool:get_inbox}` in the turn it started. For herdr-threads, the equivalent is to watch for
   `herdr-threads inbox` / hook activity, or `item/started{commandExecution}`, after a wake. That turns
   "Herdr typed it" into "the model read it". It would be a better signal for overdue warnings (`wake=`)
   than `agent_status`. Do not block the sender on it, which is xats's mistake (#3).
3. **Pane-attested nonce for identity under a shared daemon.** The daemon writes a one-time token into
   one pane, and the model quotes it back. That is a daemon-created correlation, not a heuristic.
   Their own analysis says it is the only per-pane fact a `--remote` model can carry
   (`CHANGELOG.md:22`). It is compatible with TRUST-POLICY's "never bind on heuristic evidence". The
   provenance would need a defined value.
4. **Launcher argv marker plus a foreground-carrier proof.** `-c xats.agent_id="<uuid>"` in argv is
   matched when a stored uuid is available (`auto-bind-codex-pane.ts:154-176`); before paste and before
   Enter the poke path re-checks synchronously, without the uuid, that the bound pid is the tty's
   foreground live `codex --remote` process (`pgid == tpgid`, `poke.ts:405-435`). It is a cheap guard against SIGSTOP-ed or
   backgrounded harnesses and PID reuse when typing into panes.
5. **`-c shell_environment_policy.set.XATS_CODEX_LAUNCH_ID=...`** injects a per-launch identifier into
   tool shells even when they run in a shared app-server, because the config override is per-session.
   It survives `/clear` (`docs/launchers/free-xats-codex.md:55-59`). That could give herdr-threads a
   pane-scoped marker without `--no-daemon`. It needs verifying on 0.163.0.
6. **A distinct error for "pasted, Enter not sent" (`ownership_lost`)** versus "nothing written"
   (`pane_reassigned`), plus `pokeWroteContent()` to decide whether a pasted token still counts as
   delivered (`poke.ts:226-269`). It is a precise vocabulary for keystroke-injection outcomes.
7. **A Codex composer allowlist keyed on the fixed `›` glyph, not the randomised placeholder**
   (`codex-prompt-readiness.ts:1-29`). herdr-threads already reads composer grammar
   (`src/harness/codex/composer.rs`); their note that the placeholder text is randomised is worth
   checking against our `Empty` classifier.

What not to copy:

- **Wake on every send.** It breaks our delivery-contract rules 1-2 and 4: a busy steer is a nudge
  while busy.
- **A user-role hint in prose.** It breaks rule (c), content-free wake prompts.
- **Blocking the sender on recipient behaviour.**

## Verification

Adversarial pass against the clone at `17df5cf` (package.json `0.8.6`) and the primer's Codex clone
(`83086188`, `rust-v0.163.0`). Checked: the whole dispatch path (`codex-appserver-dispatch.ts`,
`transport-dispatch.ts`, `tools.ts` hint and auto-poke mapping, `auto-poke-fanout.ts`,
`fanout-with-retry.ts`, `send-message.ts`, `get-inbox.ts`), the tmux primitive and guards (`poke.ts`,
`poke-guard.ts`, `tmux-cli.ts`, `codex-prompt-readiness.ts`, seeding/recovery call sites), the
watchdog, the RPC client buffer, delivery-spec validation, endpoint probing, the cited README,
README.agent.md, docs/configs, launcher doc, openspec and CHANGELOG lines, the two cited tests, and
the Codex source lines (`turn_processor.rs:574`, `:662`, `:689`; `codex_thread.rs:417`;
`thread_processor.rs:42`; `persistExtendedHistory` absent from `ThreadResumeParams.json`). Most claims
and line numbers held.

Changed:
- The no-tmux-fallback on unconfirmed wakes is enforced in `dispatchCodex`
  (`transport-dispatch.ts:217-223`), not only by `createAutoPokeImpl`.
- Endpoint probing applies to every candidate list, not only `CROSS_AGENT_TEAMS_CODEX_WS_URLS`.
- The pre-paste/pre-Enter carrier re-check does not match the argv uuid and is skipped without a
  `runtime_ui_pid` (§3.3, §7.4).
- The CHANGELOG:32 misdelivery is described there as the hazard a fix closes; "real" was not supported.
- Forks cannot recover the parent identity via `reconnect` (§3.5.7).
- Added citations: `poke-retry.ts:1-2` for retry delays, `README.md:92` for the db path; the
  instructions range is `:236-258`; tool count is 18 including `echo`; instructions size about 4 KB.
- `README.md:167` "mailbox only" refers to the minimum config, reworded in §1.

