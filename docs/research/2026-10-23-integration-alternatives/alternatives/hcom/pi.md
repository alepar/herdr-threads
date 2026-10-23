# hcom x Pi coding agent (pi-mono)

- hcom upstream: https://github.com/aannoo/hcom. Clone `alts/hcom`, HEAD `de33e31a0cdb` (2026-10-21, v0.7.31). MIT, 597 stars / 94 forks, latest release v0.7.31 published 2026-10-21T03:55Z (GitHub API, 2026-10-23).
- Pi upstream: https://github.com/badlogic/pi-mono. Clone `alts/pi-mono`, commit `a7b94a03f09f`, coding-agent 1.4.0 (see `primers/pi.md`).
- Citations: plain `path:line` is relative to the hcom clone root. `pi-mono:path:line` is relative to the pi-mono clone root. Every claim is tagged **V** (VERIFIED, read in source or docs) or **I** (INFERRED). Nothing in either clone was executed.

## Support status

**Native, first-class.** Pi has been supported since 0.7.21, released 2026-06-06 ("Added Pi Coding Agent, `hcom pi` (alias `pi-agent`): extension delivery, transcripts, resume, and fork", CHANGELOG.md:343) **V**. The README lists Pi as "automatic | `hcom pi`" (README.md:184) **V**. Later fixes worth knowing about:

| Release | Change | Source |
|---|---|---|
| 0.7.23 | Successive messages and queued wakes during delivery | CHANGELOG.md:294 |
| 0.7.26 | Identity kept across `/reload`, `/new`, `/resume`, `/fork` | CHANGELOG.md:215 |
| 0.7.27 | Per-launch hooks instead of a global install | CHANGELOG.md:172 |
| 0.7.29 | Mid-turn delivery "on the model's next request instead of after the turn" | CHANGELOG.md:99 |
| 0.7.29 | Lost messages when hcom marked them read before the agent accepted them | CHANGELOG.md:108 |

All tagged **V**.

The whole integration is three pieces:

1. A ~580-line in-process TypeScript Pi extension, `src/pi_plugin/hcom.ts`, embedded in the binary through `include_str!` (src/hooks/pi.rs:12) **V**.
2. An argv RPC surface on the hcom binary: `hcom pi-start|pi-status|pi-read|pi-beforetool|pi-stop` (src/integration_spec.rs:370-374). It is shared with Oh My Pi and Hermes as the "Pi family" (src/hooks/pi_family.rs:1-41, src/hooks/plugin_rpc.rs) **V**.
3. hcom's PTY proxy. For Pi it runs in "plugin mode" and never types into the terminal (details below) **V**.

## Injection mechanism

### Entry points

The extension uses Pi's own message API. There are no shell hooks, no stdout JSON and no keystrokes.

| Pi event (hcom.ts) | What hcom does | Pi delivery semantics | |
|---|---|---|---|
| `tool_result` (:560-562) | `deliverPending(ctx)` | busy → `sendUserMessage(text, {deliverAs:"steer"})`. Steering is polled after the tool batch, before the next model request (pi-mono:packages/agent/src/agent-loop.ts:304) | V |
| `turn_end` (:564-566) | `deliverPending(ctx)` | still streaming → steer. The steer poll at agent-loop.ts:304 runs right after `turn_end` (:296), so a steer queued here keeps the run going for one more request instead of letting it stop, unless a `finishTurn` decision of `end` returns first (agent-loop.ts:295-301) | V |
| `agent_settled` (:570-576) | if `ctx.isIdle()`: report `listening`, release an unadmitted delivery, then `deliverPending` | idle → `sendUserMessage(text)` with no options **starts a new run** (this is the wake). Pi defers a `prompt()` made while `agent_settled` handlers run until they finish (pi-mono:packages/coding-agent/src/core/agent-session.ts:2093-2095) | V |
| notify-port connect (:149-161, 227-230) | `deliverPending(currentCtx)` on any TCP connect | idle → new run; busy → steer | V |
| reconcile timer, every 5 s (:416-438) | ack retry, release, and when idle a pending-mail poll at most every 60 s (5 s if the notify server failed) (:176-178, 406-412) | as above | V |
| `input` (:489-518) | a bare `<hcom>` or `<hcom></hcom>` typed or RPC'd by a non-extension source → `{action:"transform", text: batch}`, or `{action:"handled"}` (swallowed) when nothing is pending or a delivery is outstanding | the typed wake becomes the delivery itself, with no extra turn | V |
| `before_agent_start` (:532-547) | once per session id: return `{message:{customType:"hcom-bootstrap", content: bootstrapText, display:false}}` | a hidden custom message added to the run. On the wire it is a `user` message (primer §2.3; pi-mono:packages/coding-agent/src/core/messages.ts:162-169) | V |

The core of `deliverPending` (src/pi_plugin/hcom.ts:283-298):

```ts
const batch = await readBatch(name);            // spawns `hcom pi-read --name <n>`
...
const text = formatMessagesForInjection(batch.messages, name);
const idle = ctx.isIdle();
delivery = { text, ackId: batch.ackId, hasMore: batch.hasMore, sentAt: Date.now(), admitted: false };
pi.sendUserMessage(text, idle ? undefined : { deliverAs: "steer" });
```

**Body source.** `readBatch` spawns `hcom pi-read --name <instance>` with a 5 s timeout, and SIGKILLs the child on expiry (hcom.ts:11, 43-80, 105-117) **V**. The Rust side, `handle_read`, returns the oldest unread messages: `events.id > last_event_id AND type='message'`, filtered by `should_deliver_to`, capped by `limit_delivery_messages`, with a `has_more` flag (src/hooks/plugin_rpc.rs:126-159; src/db/events.rs:119-143; src/hooks/common.rs:261-267) **V**. The plugin rejects a batch whose `event_id`s are not strictly increasing (hcom.ts:108-115) **V**.

### Payload format

`formatMessagesForInjection` (src/pi_plugin/hcom.ts:119-132) **V**:

```
<hcom>[request:review #812] luna -> soba: please look at PR 4</hcom>
<hcom>[3 new messages] | [inform #813] luna -> soba: ... | [thread:api #814] nova -> soba: ... | [new message #815] ...</hcom>
```

- The prefix is `[intent:thread #id]`, `[intent #id]`, `[thread:T #id]` or `[new message #id]`, followed by `from -> recipient: body` **V**.
- **This is a TS reimplementation, not the Rust `format_messages_json`.** It appends no config `hints`, no one-time tips and no `(+N others)` recipient count (compare src/hooks/common.rs:269-285, which loads hints for the hook paths). So Pi deliveries are leaner than hcom's Claude and Codex deliveries **V** (the difference is read from both code paths).
- No escaping. A body containing `</hcom>` or ` | ` makes the batch framing ambiguous (hcom.ts:128-131) **V**/**I**.
- The model receives this as a real `user`-role message "as if typed". `sendUserMessage` goes through `prompt(..., {source:"extension"})` (pi-mono:agent-session.ts:2429-2458) **V**. Inference: it renders in the TUI as a user turn, and a steer shows in the pending area as `Steering: <hcom>…` (pi-mono:packages/coding-agent/src/modes/interactive/interactive-mode.ts:4715-4724, V for the display code) **I**.

### Size bounds

- At most 50 messages per batch (`MAX_MESSAGES_PER_DELIVERY`, src/shared/constants.rs:19) **V**.
- 1 MiB per message (`MAX_MESSAGE_SIZE`, :22) **V**.
- No byte cap per batch, and no truncation in the plugin or in Pi (Pi passes `sendUserMessage` content through untouched; primer §2.3) **V**.
- Worst case: one user message of about 50 MiB (**I**, from the constants).

### Dedupe and ack

Ack happens on **admission**, not on send.

1. One outstanding `delivery` at a time. Wakes that arrive while a batch is in flight or awaiting admission set `deliveryPending` and are replayed once, collapsed through a microtask (hcom.ts:271-280, 315-333) **V**.
2. `message_end` with `role === "user"` whose text `includes(sent.text)` marks the batch admitted, then calls `flushAck` → `hcom pi-read --name N --ack --up-to <last event_id>` (hcom.ts:337-359, 523-530) **V**. The cursor only moves forward (`advance_instance_cursor`; test `ack_never_moves_cursor_backwards`, src/hooks/plugin_rpc.rs:132-144, 279-297) **V**.
3. If `has_more` is set, the next batch is fetched right after the ack (hcom.ts:352-353) **V**.
4. **Rejection recovery.** Pi's extension `sendUserMessage` returns void and routes a rejected prompt to `runner.emitError` (pi-mono:agent-session.ts:3457-3464) **V**. So the `try/catch` around `sendUserMessage` (hcom.ts:297-303) cannot catch the common failures: no model, missing auth, compaction in progress, "Agent is already processing" (pi-mono:agent-session.ts:2110-2114, 2137-2142, 2152-2170) **V**/**I**. hcom handles this with `releaseUnadmitted`. Once Pi is idle with nothing queued and 10 s have passed (`ADMIT_GRACE_MS`), it drops the outstanding record. It then defers the retry to the next wake or the next 60 s poll, so a persistently failing prompt does not loop tightly (hcom.ts:361-375) **V**.
5. **Guarantee: at least once.** Text the model saw is never withheld. A batch Pi admitted whose ack RPC failed stays outstanding, and reconcile retries the ack every 5 s (hcom.ts:416-422) **V**.
6. **Stale-process fence.** Read, ack and stop RPCs are refused unless this launch's `HCOM_PROCESS_ID` still owns the name (`require_attached`, src/hooks/plugin_rpc.rs:48-62; src/hooks/pi_family.rs:184-188) **V**. A `generation` counter discards async work that started before a session reset (hcom.ts:185-187, 445-463) **V**.

### When it does not inject

- `deliverPending` returns early unless the instance is bound and the current Pi session id matches the bound one (`isBoundSession`, hcom.ts:206-208, 269-270) **V**.
- The extension is inert unless `HCOM_LAUNCHED=1` (hcom.ts:220) **V**.
- The `hcom pi-*` dispatch is gated on `HCOM_PROCESS_ID` (`hook_gate_check`, src/hooks/pi_family.rs:176-178) **V**.

## Wake mechanism

**Mechanism.** The wake and the body are one act. When an idle Pi gets a notify-port connect, `deliverPending` calls `pi.sendUserMessage(batch)` with no `deliverAs`, and that starts a run inside the process (pi-mono:agent-session.ts:2453-2458 → `prompt()` → `_startRun`, :2172) **V**. No keystrokes, no wake marker, no extra turn.

**Who pings the port.**
- `hcom pi-start` registers the plugin's ephemeral `127.0.0.1:0` port as a `kind='plugin'` notify endpoint (hcom.ts:227-234; src/hooks/pi_family.rs:50-68, 143-145) **V**.
- `hcom send` calls `notify::wake_all`, which connect-and-closes **every** registered instance-loop port system-wide, not only the recipients' (src/commands/send.rs:482; src/notify/wake.rs:49-74) **V**.
- A status change into `listening` also wakes the instance's own loops (src/hooks/plugin_rpc.rs:113-115) **V**.

**PTY proxy in plugin mode.**
- Pi is still launched under `hcom pty` (`uses_pty_default: true`, src/integration_spec.rs:1928) **V**.
- Its delivery thread takes the plugin-mode branch for `Tool::Pi`, with `session_started = true` from the start, so it **never types** into Pi. It only refreshes title and liveness and re-registers endpoints, waking every 30 s (`IDLE_WAIT`) (src/delivery.rs:1669-1712, 1761-1768, 1832-1842, 1462) **V**.
- `get_input_box_text` returns `None` for Pi: no screen-scraped composer (src/pty/screen.rs:783) **V**.
- The active (typing) state machine still lists `Tool::Pi` among tools that get a bare `WAKE_TRIGGER` (src/delivery.rs:2068-2071), but that branch is in the `else` of the plugin-mode check (src/delivery.rs:1842-1843), so it is unreachable for Pi **V** (dead for Pi, read from control flow).
- All PTY gates are off except `block_on_approval` and `launch_requires_ready` (src/integration_spec.rs:1914-1923) **V**.
- The bodyless-wake `input` transform is therefore a fallback for text typed by someone else: a human typing `<hcom>`, `hcom term inject`, or a script. It is not the normal wake path (**I**, from the plugin-mode branch never typing for Pi).

**Idle/busy detection.**
- `ctx.isIdle()` at send time decides between a new run and a steer (hcom.ts:291, 298) **V**.
- Status for other agents comes from Pi events: `agent_start` → `active`, and `agent_settled` (not `agent_end`) → `listening` (hcom.ts:485-487, 570-576). A test pins the `agent_settled` choice: the source must register `agent_settled` and must not register `agent_end` (src/hooks/pi.rs:138-143) **V**.
- `tool_call` reports `active tool:<name>` through `hcom pi-beforetool` (hcom.ts:550-558) **V**.

**Launch readiness.**
- "Ready" for Pi is the plugin bind, not on-screen text. Pi's header differs between compact, expanded and quiet modes, and it is drawn before the key handlers are live (src/integration_spec.rs:1892-1898; src/delivery.rs:1106-1115) **V**.
- No `kind='plugin'` endpoint once the screen has produced no output for 10 s (`PLUGIN_BIND_GRACE`, replacing the 1.5 s settle threshold) → "screen settled but the hcom plugin never connected" (src/delivery.rs:1246-1290) **V**.

**Draft safety.**
- hcom never touches the editor. It does not call `ctx.ui.getEditorText()` (no use in hcom.ts) **V**.
- A human draft is therefore never clobbered, because no keystrokes are injected. But an idle wake starts a run **while the human is typing**. The draft stays in the editor and, once submitted, becomes a steer into the agent's run (primer §6) **I**.
- There is no "human is composing, hold the wake" gate (`block_on_user_activity: false`, src/integration_spec.rs:1919) **V**.

**Latency (I).** A send becomes visible in about a TCP connect plus one `hcom pi-read` process spawn plus a SQLite read, so sub-second. The fallbacks:
- a missed wake is picked up by the 60 s idle poll (5 s without a notify server);
- an unadmitted batch is retried after at least 10 s plus the next wake or poll.

**Failure modes.**
- `hcom` not on Pi's PATH: `spawn` errors (code 127), the bind fails and is only logged, and the PTY loop reports the missing plugin after 10 s (hcom.ts:71-74, 237-239; src/delivery.rs:1283-1284) **V**.
- Compaction in progress: `isIdle()` is false, so hcom steers, and `prompt()` throws for compaction before it checks streaming (pi-mono:agent-session.ts:2110-2114). The error goes to Pi's extension error display, and the batch is released after the grace period (**V** code path / **I** UI effect).
- Idle-check race: between `ctx.isIdle()` and Pi's streaming check there are awaits (input handlers). A human submit in that window makes Pi throw "Agent is already processing", and the batch is retried after the grace period (**I**).
- Escape or abort while a steer is queued: Pi's `restoreQueuedMessagesToEditor` moves all queued steering, including hcom's `<hcom>…` batch, **into the editor** and joins it with the human's draft (pi-mono:interactive-mode.ts:4734-4752; docs/usage.md:40) **V**. The batch is never admitted, so hcom releases it after 10 s and redelivers it through `sendUserMessage` on an idle Pi. That **restarts the run the human just aborted**, while the old copy still sits in the editor. If the human then submits the editor, the model sees the batch twice (**I**, from both code paths).
- Every `hcom send` anywhere wakes every Pi plugin, and each one spawns an `hcom pi-read` process, even when it has no mail (**V** wake_all + unconditional `onWake`; cost **I**).

## Setup / installation footprint

- `hcom pi [args]` launches `pi` under `hcom pty` with `-e <HCOM_DIR>/integrations/pi/<sha256>/hcom.ts` inserted before any `--` (src/hooks/pi.rs:15-21, 43-55; test :323-343) **V**. The file is published once per content digest and renamed into place, and an existing digest directory is reused (src/hooks/runtime.rs:441-461) **V**.
- **Nothing is written to `~/.pi`.** Per-run `-e` replaced the older ambient install in 0.7.27 (CHANGELOG.md:172) **V**.
- The launch removes legacy copies at `$PI_CODING_AGENT_DIR` (or `~/.pi/agent`)`/extensions/hcom.ts`, or `<HCOM_DIR parent>/.pi/extensions/hcom.ts`, but only files identified as hcom's: identical to the source or containing `customType: "hcom-bootstrap"` (src/hooks/pi.rs:23-82) **V**.
- Environment: hcom sets `HCOM_LAUNCHED=1` and `HCOM_PROCESS_ID` (src/launcher.rs:902, 2034, 1295) and reads `HCOM_PI_ARGS` for default arguments (src/integration_spec.rs:1926) **V**. `HCOM_PI=1` is a detection marker, documented as "Env set by the tool itself" (src/integration_spec.rs:30-31, 1906); no code in the hcom clone sets it, so who sets it is unknown. Markers plus `PI_CODING_AGENT` and `PI_CODING_AGENT_SESSION_DIR` are cleared for hcom-launched children (src/integration_spec.rs:32, 1907) **V**.
- Rejected arguments: `-p`/`--print` (src/tools/launch_arg_validation.rs:248-258) **V**.
- Resume and fork use `--session <id>` and `--fork <id>` (src/integration_spec.rs:1933-1942) **V**.
- hcom-side state lives in `<HCOM_DIR>/hcom.db` (default `~/.hcom`; tables `instances`, `events`, `notify_endpoints`; src/paths.rs:181, src/db/mod.rs:329-368) and the plugin logs to `<HCOM_DIR>/.tmp/logs/hcom.log` (hcom.ts:8-9) **V**.
- A plain `pi` without `hcom pi` gets no integration, because the extension needs `HCOM_LAUNCHED=1` (hcom.ts:220) **V**.

## Token cost

| Item | Cost | |
|---|---|---|
| Bootstrap | One hidden user-role custom message at the first run of each session id: identity, MESSAGES rules, CAPABILITIES (a long CLI cheat sheet), RULES, DELIVERY_AUTO, plus the active-instance list and user notes (src/bootstrap.rs:68-126, 207-216, 554-640). README states "~700 tokens" (README.md:231). Persisted in the session JSONL | V |
| Bootstrap repeats | `bootstrapInjectedForSession` is in memory and cleared by `resetBinding()` on every `session_start` (hcom.ts:460, 465-468). After `/reload` or `/resume` of the *same* session, the bootstrap is added again on top of the copy already in the transcript. It is also not re-added after a compaction that summarizes it away | I |
| Per message | `<hcom>` + `[intent:thread #id] from -> to: ` + body + `</hcom>`, about 15-25 tokens of framing per message plus `[N new messages] \| …` for a batch. No hints or tips (unlike hcom's Claude/Codex path) | I (framing V) |
| Wake | 0 extra tokens: the wake *is* the delivery. No content-free marker turn | V |
| Quiet turn | 0 model tokens: an empty batch returns before any send (hcom.ts:289) | V |
| Non-token overhead | Per tool call: one `hcom pi-beforetool` spawn (queued, not awaited) and one `hcom pi-read` spawn awaited inside the `tool_result` handler. Per turn: another `hcom pi-read` in `turn_end`. Per human prompt: an awaited `hcom pi-status` spawn before Pi processes the input (hcom.ts:516). Each has a 5 s timeout (hcom.ts:11). Pi awaits handler emission (`await emit(...)`, agent-loop.ts:296), so these spawns sit on the turn's critical path | V code / I latency |

## Limitations and bugs

1. **Every message wakes an idle Pi.** There is no priority or threshold, and `sendMessage(..., {triggerTurn:false})` / `deliverAs:"nextTurn"` (silent arrival) is never used (no such call in hcom.ts) **V**. This violates our delivery-contract rules 1-2 by design.
2. **No human-draft hold.** The editor is never consulted, so a wake can start the agent while the human is mid-sentence (**I**; `getEditorText` is available, primer §2.4).
3. **Abort is undone.** Escape returns hcom's steered batch to the editor, then hcom redelivers it and restarts the run. Two copies can follow (see Failure modes; **I**).
4. **Broadcast wake fan-out.** `wake_all` on every send makes every plugin on the host spawn `hcom pi-read`, which costs O(agents) processes per message (**V**/**I**).
5. **Process spawns on the hot path.** Every tool result and turn end waits for a child `hcom` process. A slow DB (busy_timeout 5000, overview) can add up to 5 s per tool call before the SIGKILL (**I**).
6. **Unescaped framing.** `</hcom>` and ` | ` inside a body are not escaped (**V**).
7. **No byte cap.** 50 × 1 MiB in one user message (**I**).
8. **Admission matching by substring.** `messageText(message).includes(sent.text)` (hcom.ts:526) can match a human message that quotes the batch, for example an editor-restored copy submitted after a release. Harmless in the common case, but it is a heuristic receipt (**V** code / **I** edge).
9. **Dead `try/catch`.** Pi's extension `sendUserMessage` never throws synchronously, so hcom.ts:299-302 cannot fire (pi-mono:agent-session.ts:3457-3464). The code comment acknowledges that rejection is only observable indirectly (hcom.ts:295-296) **V**.
10. **Unread scan cost.** `pi-read` scans every `message` event since the cursor and filters in Rust (src/db/events.rs:125-143). For an instance that receives little among heavy broadcast traffic, each read walks the backlog until an ack moves the cursor (**V**/**I**). Note that the cursor only advances on acks of this instance's own batches.
11. **PTY inject port still present.** Pi is PTY-wrapped, so the unauthenticated localhost inject server still exists. Anything local can type into Pi's terminal through `hcom term inject` or a raw TCP client (src/delivery.rs:1835; overview "Auth on local ports") **V**/**I**. The plugin notify port itself only triggers a DB read, so it cannot inject text (**V**).

## Clever ideas worth stealing

For a future herdr-threads Pi adapter (we have none today, primer §8):

1. **In-process extension, attached per launch with `-e <content-addressed path>`.** No `~/.pi` writes, cleanup only of files we own (marker check), and the bind doubles as authoritative launch readiness. This maps cleanly to our managed launch and to `PI_CODING_AGENT_DIR` for isolated tests.
2. **The wake is the delivery.** `sendUserMessage` (or better `sendMessage({customType, content, display}, {triggerTurn:true})`) when `isIdle()`, steer when busy. This is a `Channel`-class policy (delivery rule 6) with zero keystrokes and zero wake-marker tokens. We should add what hcom lacks: priority ≤ T decides `triggerTurn`; below threshold, use `triggerTurn:false` / `deliverAs:"nextTurn"`; and hold while `ctx.ui.getEditorText()` is non-empty.
3. **Ack on transcript admission (`message_end`)** with a grace release when Pi goes idle without admitting. It is a genuine "the model saw it" receipt that survives rejected prompts and dropped queues. Prefer matching on a custom message's `customType` + `details` id over substring matching.
4. **Steer from `tool_result` / `turn_end`.** Mid-turn delivery reaches the next model request (agent-loop.ts:304), which serves delivery rule 3. Alternative worth testing: `tool_result` content append, which keeps the item attached to a tool result instead of a fake user turn.
5. **Swallow an empty typed wake.** `input` → `{action:"handled"}` when nothing is pending, and `{action:"transform"}` replacing the marker with the body. This gives a keystroke fallback that never costs an empty turn.
6. **One outstanding batch, `has_more` chaining, generation counter, microtask-collapsed replay.** A small, robust state machine for event-driven delivery inside a reloadable runtime.
7. **Status from `agent_settled`, not `agent_end`.** `agent_end` is not final idle in Pi (primer §2.2). hcom has a test pinning this (src/hooks/pi.rs:138-143).

Things *not* to copy: broadcast wake fan-out, process spawns per tool call (a persistent socket from the extension to our daemon would avoid both), an unbounded batch size, and the abort-then-redeliver behaviour. To avoid the last one, an extension should treat an `agent_settled` with `aborted: true` (primer §2.2, types.ts:1032-1037) as "hold wakes until the human acts".

## Verification

Adversarial pass, 2026-10-23, against the hcom clone at `de33e31` and pi-mono at `a7b94a0`. Nothing was executed.

- Checked and confirmed: GitHub API (MIT, 597 stars, 94 forks, v0.7.31 published 2026-10-21T03:55:43Z); CHANGELOG lines 99/108/172/215/294/343 and the 0.7.21 date (2026-06-06); README:184, :231; the whole of `src/pi_plugin/hcom.ts` (577 lines) for every event, line range, constant (5 s timeout, 10 s grace, 5 s reconcile, 60 s/5 s poll), the framing, the substring admission match, and the absence of `getEditorText`, `triggerTurn` and `nextTurn`; `include_str!` (pi.rs:12); PI_HOOKS (integration_spec.rs:370-374); Pi spec gates, PTY default, resume/fork; `require_attached`, `hook_gate_check` (checks a non-empty `HCOM_PROCESS_ID`), `handle_read`, cursor-forward test; `limit_delivery_messages`, `MAX_MESSAGES_PER_DELIVERY=50`, `MAX_MESSAGE_SIZE=1_048_576`; the unread scan SQL; `wake_all` (all INSTANCE_LOOPS kinds, `plugin` included, inject excluded) called from send.rs:482; the plugin-mode branch, `IDLE_WAIT=30s`, screen.rs:783; per-run `-e` injection, content-addressed publish, legacy cleanup; `-p`/`--print` rejection; the bootstrap parts. In pi-mono: `prompt()` settled-deferral (2093), compaction throw (2111-2114), streaming throw (2138-2142), `_startRun` (2172), `sendUserMessage` → `prompt(source:"extension")` (2429-2458), extension binding swallowing errors into `runner.emitError` (3457-3464), `isIdle = !run && !compacting` (1484-1486), steering polls (agent-loop.ts:214, 304), custom message → `user` role (messages.ts:162-169), `restoreQueuedMessagesToEditor` (4734-4752) called on abort, usage.md:40, `AgentSettledEvent.aborted` (types.ts:1033-1037).
- Changed: the `turn_end` row now notes that a `finishTurn` `end` decision returns before the steer poll; "tests pin both choices" narrowed to the one test that pins `agent_settled` over `agent_end`; the readiness timeout reworded (10 s of no screen output without a plugin bind, not 10 s from launch); the environment bullet corrected (`HCOM_PI=1` is a detection marker hcom reads, not one it was shown to set; added the clear-for-child rule and `HCOM_PROCESS_ID` citation); the state-path bullet re-cited (`paths.rs:181`, `db/mod.rs` tables; the old citation only covered the log path); added the note that delivery.rs:2070 lists Pi for `WAKE_TRIGGER` in a branch Pi never reaches.
- Not refuted, still INFERRED: latency, the compaction UI effect, the idle-check race, the abort-then-redeliver duplicate, the bootstrap repeat after `/reload`, the per-send spawn cost, and the 5 s per-tool stall.
