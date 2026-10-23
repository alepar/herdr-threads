# Cotal x Pi coding agent (pi-mono)

- Upstream: https://github.com/Cotal-AI/Cotal (site https://cotal.ai/). Clone `alts/cotal`, HEAD `6426fb538e0b` (2026-10-23), read only, nothing executed.
- Harness: Pi coding agent (https://github.com/badlogic/pi-mono, now `earendil-works/pi`). Pi facts come from the harness primer `../../primers/pi.md`, which read Pi `1.4.0`. Cotal pins Pi `0.79.10`, so primer line numbers describe a newer Pi than the one Cotal targets.
- Citations are `path:line` relative to the Cotal clone root unless marked "(primer)". **V** = VERIFIED (read in source or docs), **I** = INFERRED.

## Support status

- **Native, alpha.** `@cotal-ai/pi` is "Cotal's first host-native framework adapter". It runs inside the Pi process and is "the one connector that can steer a live turn mid-flight" (docs/connect-pi.md:5-13; docs/connectors.md:24, 42-43) **V**.
- There are three surfaces from one bundled artifact, `dist/standalone.js` (docs/connect-pi.md:15-24) **V**:
  1. A managed seat: `cotal spawn --agent pi` launches `pi --extension <standalone.js>` in the manager's PTY (extensions/pi/src/connector.ts:84).
  2. An interactive Pi: the operator copies the artifact to `~/.pi/agent/extensions/cotal.js`.
  3. Pi SDK apps that use the default resource loader.
- **Version pin.** Docs say "This release pins Pi `0.79.10`", and the README says "exactly" that version (docs/connect-pi.md:26-27; extensions/pi/README.md:9-10) **V**. The runtime gate is only a feature probe. It throws if `pi.sendMessage`, `pi.registerMessageRenderer` or `pi.on` is missing (extensions/pi/src/extension.ts:133-135) **V**. The package's peer dependency is `@earendil-works/pi-tui >=0.79.10` (extensions/pi/package.json:39-42) **V**, so the code does not enforce "exactly". Whether it works against Pi 1.x (the primer's 1.4.0) is **unknown**.
- **Things that fail loud:** model variants, MCP sharing and raw launch options (extensions/pi/src/connector.ts:50-54) **V**.
- **Inert without identity.** The extension returns immediately unless `COTAL_NAME`, `COTAL_AGENT_FILE` or `COTAL_LINK` is set (extensions/pi/src/extension.ts:132; README.md:30-33) **V**.

## Injection mechanism

### Exact API

Every inbound batch goes through one call, `pi.sendMessage` with a custom message. This is the only place in the adapter that starts a Pi turn (extensions/pi/src/driver.ts:338-361) **V**:

```ts
this.host.sendMessage(
  { customType: "cotal-inbox", content: batch.content, display: true,
    details: { version: 1, batchId: batch.id, ids: [...batch.ids] } },
  { triggerTurn: true, deliverAs: "steer" },
);
```

Pi semantics, from the primer §2.3 (Pi `agent-session.ts:2344-2393`):

- If Pi is idle, `triggerTurn: true` starts a run immediately.
- If Pi is streaming, the message is queued as a steer. Pi polls the steer queue at loop start and after each complete tool batch, before the next model request.
- The model sees the message as a `user`-role message (primer: `messages.ts:162-169`).
- `details` is persisted but never sent to the model (primer: `docs/message-types.md:235`).

Cotal relies on this: the batch id and item ids ride in `details`, which costs no model tokens **V** (primer) / **I** (that this was the reason for putting them there). Docs state the design intent: "This removes an idle/streaming race while preserving structured batch details" (docs/connect-pi.md:31-32). One call shape covers both the idle and the busy case, so the adapter never has to pick a mode on a stale `isIdle()` reading **V**.

The TUI renders the message through a registered custom renderer that uses Pi's own ANSI- and grapheme-aware wrapper (extensions/pi/src/extension.ts:180; extensions/pi/src/wrap.ts:1-17) **V**.

### Code path

1. NATS/JetStream delivery reaches the in-process `MeshAgent` and is buffered in its local inbox (`buffer()`, extensions/connector-core/src/agent.ts:1133-1194). The buffer emits `incoming` (agent.ts:1193), `mention-wake` for a focus-mode @mention (agent.ts:1111), or `wake` (agent.ts:1733) **V**.
2. Those events are wired to `driver.onIncoming`, `onWake` and `onMentionWake` (extensions/pi/src/extension.ts:110-112). Each calls `pump()` **V**.
3. `pump()` (driver.ts:304-336) works through these steps **V**:
   - It returns if the driver is `held` or `shuttingDown`, or if any batch is unconfirmed. Only one batch is in flight at a time.
   - It discards tombstoned late duplicates and its own channel echoes (driver.ts:308-309).
   - It returns unless something is wakeable, a focus nudge is pending, a workflow "run turn" is pending, or `force` is set (driver.ts:314-317).
   - It selects up to `BATCH_LIMIT = 32` "automatic" items in FIFO order (driver.ts:7, 324; extensions/pi/src/inbox-turn.ts:49-57), formats them with `formatInjection`, appends any pending run-turn text (driver.ts:329), and dispatches.
4. Pi later fires lifecycle events. They are wired to the driver at extensions/pi/src/extension.ts:194-205: `agent_start`, `message_start`, `context`, `after_provider_response`, `tool_execution_start/end`, `session_before_compact` and `agent_end` **V**.

### When it fires

- **New traffic** calls `pump()` at once. When Pi is idle this starts a turn. When Pi is busy the batch becomes a steer that Pi admits after the current tool batch. A second batch waits until the first is confirmed. Confirmation also calls `pump()` mid-turn (driver.ts:204-207), so successive batches enter one long turn one request apart **V**.
- **Every `agent_start`** calls `pump(true)`, a forced pump, unless the driver is still `held` (driver.ts:165). That includes a turn the human starts. The force bypasses the "wakeable" filter, so ambient channel items that did not justify a wake still ride into the next turn, whoever started it. This is the equivalent of "fold pending into the next prompt" **V**.
- **Quiet-channel ambient** is never injected automatically. It stays pull-only until the model calls `cotal_inbox`, which this adapter restricts to `scope: "pull-only"` (extensions/pi/src/tools.ts:37-39, 90, 103-106; extensions/pi/README.md:63-64) **V**.

### Payload format and framing

`formatInjection` (extensions/connector-core/src/control.ts:120-127) produces a header line, one line per item and a fixed tail **V**:

```
📨 Cotal — N new message(s) from peers:
[DM from ada/reviewer] body...
[#general @you bob] body...
(If this asks for work, do it with your own tools first and verify the result, then reply with cotal_dm / cotal_send — never report an action you did not actually perform. cotal_roster shows who's here.)
```

- Item lines come from `formatItem` (extensions/connector-core/src/framing.ts:124-132).
- Peer-controlled fields (sender, role, channel, service) are neutralized against bracket and line-break forgery. In the body, every line break is replaced by `\n  `, so a peer can never start a line at column zero (framing.ts:64, 96-98; control.ts:100-111) **V**.
- Markers: `(history) ` for backfill, and `(redelivered, delivery N: an earlier delivery may already have run) ` driven by the broker's delivery count (framing.ts:125-127) **V**.
- The Pi adapter calls `formatInjection(items)` without `{metadata:true}` (driver.ts:327), so no `message_id` or `sender_id` appears in the frame **V**.
- **Focus-mode @mention.** The adapter sends a content-free nudge instead of the body: `Cotal: you were @mentioned on #<channel> by <who> while focused. The body is held; read it with cotal_inbox and reply with cotal_send if warranted.` (driver.ts:131-140). Up to 32 nudges are kept, and the oldest are dropped first **V**. The channel name and the sender label (`fromName/fromRole`) are interpolated raw here, not through `fmtChannel`/`peerLabel` (driver.ts:133-135) **V**. framing.ts:134-154 and packages/core/src/peer-label.ts:1-14 warn about exactly this class of bug, so this may be a framing hole **I**.

### Size bounds

- 32 items per batch (driver.ts:7) **V**.
- **No per-body or per-batch byte cap.** `fmtBody` only rewrites line breaks (framing.ts:96-98) and the driver sends `content` verbatim (driver.ts:346) **V**. Pi itself does not truncate `sendMessage` content (primer §2.3) **V**. The effective bound is the NATS `max_payload` per message, 1 MB by default in NATS **I**, so a batch could reach 32 × that. Compare herdr-threads, which truncates at 8 KiB with a `body <id>` continuation and caps pages at 32 items or 64 KiB.
- The local inbox holds at most `MAX_INBOX = 200` items. On overflow it evicts pull-only items first, then channel items. Directed mail is never acked on eviction, so the broker redelivers it (agent.ts:227, 1143-1191) **V**.

### Dedupe

- `MeshAgent` dedupes by message id against pending and handled sets. A durable redelivery of a pending id refreshes the ack handle and re-emits `incoming` (agent.ts:1022-1049) **V**.
- The Pi ledger `InboxTurn` keeps tombstones for confirmed ids that overflow had already removed, so a late copy cannot resurface. Tombstones are double-buffered with a cap of 4096 (extensions/pi/src/inbox-turn.ts:14, 31-39, 59-79) **V**.

### Acknowledgement: three-point proof, commit at a terminal boundary

The docs describe three proof points (docs/connect-pi.md:33-44; extensions/pi/README.md:38-45) **V**:

1. **`message_start`** with matching `details` proves that Pi dequeued the batch (driver.ts:168-181). A 5 s watchdog armed at dispatch holds the driver if this never fires (driver.ts:352-357) **V**.
2. **`context`** whose message list contains that `batchId` proves the batch entered a provider request (driver.ts:183-190) **V**.
3. **`after_provider_response`** with a 2xx status confirms the batch (driver.ts:192-198). On a transport without that hook (the docs name the Codex subscription), the batch is confirmed at the next clean terminal `agent_end` instead (driver.ts:272-275) **V**.

`agent_end` decides whether to commit (driver.ts:227-279) **V**:

- **Terminal** means not aborted, and a stop reason of `stop`, `toolUse`, or `length` with positive output.
- **On a terminal end**, `finalizeEnd` calls `inbox.commitConfirmed(ids)`. That runs `MeshAgent.drainInboxDeliveries`, an exact-key drain that acks only the confirmed ids even when pull-only items are interleaved ahead of them (driver.ts:371-389; inbox-turn.ts:59-67).
- **An unconfirmed batch at a terminal end** makes the driver hold with "Pi ended before confirming a queued Cotal steer" (driver.ts:379-382).
- **A non-terminal end** (error, unknown reason, or zero-output `length`) gets two automatic re-dispatches 50 ms apart. After that the driver holds with "needs intervention: start a new turn in this session, or replace the seat" (driver.ts:12-13, 252-269, 394-407).
- **An abort** never retries. It holds until the person continues (driver.ts:253-256). The smoke test asserts that "the next human turn carries held backlog" (extensions/pi/pi.smoke.ts:355-375), but only for a batch already provider-confirmed before the abort. For a batch aborted before confirmation, the next human turn's `pump` returns at driver.ts:306 without re-dispatching it; Pi's Escape returns queued messages to the editor (primer §6), and whether a queued custom steer survives an abort is **unknown**. If it is gone, the next terminal end holds with "Pi ended before confirming" and the seat stays held (see Failure modes) **I**.
- **Overflow compaction** with `willRetry` defers finalization (driver.ts:281-285, 373).

Workflow "run turns" are committed as surfaced at confirmation, or at the first tool start of the answer, so the model's own `cotal_yield` can find them (driver.ts:210-220, 363-369; docs/connect-pi.md:46-50) **V**.

## Wake mechanism

- **Mechanism.** The wake is the same `sendMessage({..}, {triggerTurn: true, deliverAs: "steer"})`. Called while Pi is idle, it starts a run in-process. There are no keystrokes, no PTY writes, no app-server and no re-launch **V** (driver.ts:343-351; primer §2.3). The sibling report `overview.md` ("No keystroke injection for messages") found no message-wake keystroke path for any Cotal harness; Cotal's docs do not state this as a rule **I**.
- **Who wakes, and on what.** `wakeable()` (driver.ts:66-70) returns true for DMs, anycasts and @mentions, and also for any channel message when attention is `open`. Own echoes never wake **V**. The default attention is `"open"` (extensions/connector-core/src/agent.ts:457) **V**. So by default, every non-quiet, non-historical channel message wakes an idle Pi seat. `dnd` and `focus` attention and per-channel `quiet` or `muted` narrow this (agent.ts:1063-1129) **V**. There is no numeric priority or per-seat threshold.
- **Idle/busy detection.** The driver does not gate on idle-versus-busy before sending, because the one call shape is correct in both states. It tracks its own state machine (`idle | dispatching | streaming | held | shuttingDown`, driver.ts:15) from `agent_start` and `agent_end`. `ctx.isIdle()` is used only to publish `idle` presence once Pi has really settled, polled every 10 ms (driver.ts:418-429). The adapter does not listen to `agent_settled` (grep of extensions/pi/src finds no `agent_settled`) **V**.
- **Draft safety.** No keystrokes are typed, so a human's composer text is never overwritten or submitted **I** (the extension never touches the editor). However, the adapter never calls `ctx.ui.getEditorText()` or `onTerminalInput`, and never watches `ui_prompt_start` (grep finds none of these in extensions/pi/src) **V**. A peer DM therefore starts a turn while a human is mid-draft. The draft stays in the editor, but if the human presses Enter while the peer turn runs, their prompt becomes a steer into that turn (primer §6: typing while busy becomes a steer) **I**.
- **Abort as a pause.** After a user abort the driver goes to `held` and sends nothing until the next `agent_start`, which in an interactive session is the human's next prompt. That turn then carries the backlog (driver.ts:253-256, 142-166) **V**. This is the Pi counterpart of the Claude native module's "pause after user Stop until the next person-origin prompt" (sibling `overview.md`, citing docs/connect-claude.md:357-362) **V**.
- **Latency.** The path is an in-process event from the NATS subscription to `pump` to `sendMessage`. No timers sit on the wake path, so the latency is broker delivery plus the provider's first token **I** (not measured). Mid-turn steers land at the next tool-batch boundary, never inside a running tool (primer §2.3) **V**.
- **Failure modes.**
  - *Dispatch throws.* `sendMessage` can throw, for example during compaction (the primer's `prompt()` throws then; whether `sendMessage` does is **unknown**). The driver then holds with "Pi rejected Cotal dispatch: ..." (driver.ts:358-360) **V**.
  - *Pi never starts the batch.* The 5 s start watchdog holds the driver (driver.ts:352-357) **V**.
  - *A hold needs an outside `agent_start`, and some holds survive even that.* `agent_start` lifts a hold only when a continuation is pending (aborted or non-terminal end) or when no batch is retained (driver.ts:150-160) **V**. A hold with a retained, unconfirmed batch and no pending continuation (start watchdog, dispatch rejected, "Pi ended before confirming") is not lifted by `agent_start` and no `pump` runs; it clears only if a later terminal `agent_end` confirms that batch (driver.ts:158, 165, 272-279) **V**. A rejected dispatch never reached Pi, so that seat stays held until restart **I**. In a headless managed seat nobody starts a turn, so the seat stays parked with a `waiting` presence that names the reason. The docs call restart the safe recovery (docs/connect-pi.md:52-67) **V**.
  - *A crashed process redelivers.* The process dies before a terminal boundary, nothing is acked, and JetStream redelivers. A managed crash reopens the exact Pi session, three times per two minutes (docs/connect-pi.md:74-79; README.md:47-49) **V**.

## Setup and installation footprint on the harness

- **Managed seat (`cotal spawn --agent pi`)** writes no Pi config. Its launch spec (extensions/pi/src/connector.ts:47-139) **V**:
  - Command: `pi --extension <dist/standalone.js>`, plus `--session-id <fresh UUID>`, `--session <id>` (exact reopen) or `--fork <id>` (resume). The fresh UUID gives an idle seat a recoverable session before its first turn.
  - Optional: `--append-system-prompt <tmp persona.md>`, `--model`, and a positional first prompt. A prompt starting with `-` or `@` is refused.
  - Env: `COTAL_*` identity and ACL variables, and an allowlist of provider API keys (connector.ts:28-36, 64-69).
  - Files: a 0600 launch-material file (creds, broker URL, control token) whose path the extension scrubs from the environment after reading it (extension.ts:149-158), and a lifecycle-keyed session-state file `<workspace>/.cotal/pi-sessions/<name>-<uid>.json`, written atomically 0600 (extension.ts:62-83).
  - An authenticated local control socket serves shutdown and session-id queries (extension.ts:115-126).
  - If the event plane is on, a write-ahead log under the workspace root (extensions/pi/src/events.ts:88-104; extensions/pi/src/connector.ts:71-76; extensions/connector-core/src/agui-wal-path.ts:69-75).
- **Interactive Pi** requires a manual `cp` of `standalone.js` into `~/.pi/agent/extensions/cotal.js`, a global user-config write. A Cotal identity must also be in the environment (docs/connect-pi.md:127-133) **V**. No `settings.json` or `mcp.json` edits were found **V** (connector.ts and extension.ts write neither).
- **Session tracking.** The adapter reads `PI_SESSION_ID` at load, because the first `session_start` may already have passed. It checks that id against `COTAL_PI_EXPECTED_SESSION` and throws on a mismatch, then follows `/new`, `/resume`, `/fork` and reload through `session_start` (extension.ts:140-148, 170-193) **V**. A process-global `Symbol.for("cotal.pi.runtimes")` map keeps the mesh, driver and control server alive across Pi's runtime teardown on reload, new, resume and fork. Only `session_shutdown{reason:"quit"}` stops the mesh (extension.ts:25, 38-47, 160-166, 210-224) **V**.

## Token cost

- **Per batch:** a header of about 45 chars, one line per item (`[DM from name/role] ` plus the body), and a fixed tail of 203 chars, counted from control.ts:123-125 **V**. That is roughly 60 tokens of framing per batch plus about 5-10 tokens per item, before bodies **I**. The tail repeats on every batch. herdr-threads (R17) puts no static text in its deliveries.
- **Per quiet turn:** zero injected text. A pump with nothing wakeable returns without sending (driver.ts:314-317), and a forced pump with nothing selectable returns at driver.ts:325 **V**. The standing cost is the tool surface, on every request (prompt-cached **I**):
  - About 24 `cotal_*` tools are registered through `pi.registerTool` (tool names in extensions/connector-core/src/tool-specs.ts:707-1859; some are gated by capability, tool-specs.ts:700-703) **V**.
  - Their literal description strings alone total about 9.9 k characters (the original author's count over tool-specs.ts, excluding computed descriptions and JSON schemas; not re-counted in verification) **I**.
  - `SEND_GUIDELINES` adds about 1.3 k characters of prompt guidelines on `cotal_send` (extensions/pi/src/tools.ts:44-54, 99; the two shared steers are extensions/connector-core/src/orientation.ts:28-41) **V**.
  - Estimate: about 3-5 k tokens of fixed overhead per request **I**.
- The batch is shown in the TUI (`display: true`) and persisted in the session JSONL, so it also counts toward compaction **I**.

## Limitations and bugs

1. **No holds or ack-wait renewal for in-flight Pi batches.** `MeshAgent.holdInFlight` exists so that a held durable delivery gets its JetStream ack wait renewed and is protected from the overflow valve. Its comment names the risk: "handing a long turn's role request to a second worker while the first still runs it" (agent.ts:1160-1164, 1385-1406, 1435-1450) **V**. The Pi adapter never calls it. The callers are connector-core's `inline-delivery.ts:57`, the OpenCode connector (extensions/connector-opencode/src/plugin.ts:1134, plugin2.ts:195) and tests (grep) **V**. So a Pi turn longer than the ack wait (default 60 s, agent.ts:571) leaves a carried anycast copy on the shared role queue unrenewed. The broker can then hand it to another worker, and duplicate work follows **I**. For DMs, the redelivery merely refreshes the pending entry (agent.ts:1039-1048) **V**.
2. **Wakes ignore the human composer.** There is no `getEditorText()` check, so a peer turn can start while the person is typing (see Wake) **V**/**I**.
3. **Default attention `open` wakes on all channel chatter** (driver.ts:66-70; agent.ts:457) **V**. Every message that wakes an idle seat costs a provider turn.
4. **No body truncation.** One large DM goes into context whole (framing.ts:96-98; driver.ts:346) **V**.
5. **A hold needs an outside turn to lift, and some never lift without a restart.** Beyond the two bounded retries, a hold waits for an `agent_start` the adapter cannot start itself; holds with a retained unconfirmed batch are not lifted even by that (driver.ts:150-165, 265-268) **V**. A headless seat parks with only a presence string for a signal **V**.
6. **The version pin contradicts the code.** Docs say "exactly 0.79.10", but the runtime only probes for features (extension.ts:133-135), and the primer's Pi is 1.4.0. Breakage in newer Pi would surface only at runtime **V**/**I**.
7. **The focus-nudge channel and sender labels are not neutralized** (driver.ts:133-135) **V**; whether that is exploitable is **I**, as noted under framing.
8. **Bursts are serialized.** One unconfirmed batch at a time (driver.ts:306), confirmed at `after_provider_response`. A burst of more than 32 items takes several provider requests **V**.
9. **Peer text bypasses the input chain.** `sendMessage` skips Pi's `input` event, so peer text bypasses other extensions' input transforms. Tool, provider and permission hooks still apply (docs/connect-pi.md:111-114) **V**. This is by design, but other Pi extensions cannot filter peer text at `input`.

## Clever ideas worth stealing

1. **A Pi adapter is a Channel-class policy with no keystrokes.** One in-process extension, one `sendMessage(..., {triggerTurn:true, deliverAs:"steer"})`, serves both idle wake and mid-turn delivery, race-free (driver.ts:343-351). For herdr-threads this maps onto the existing `Channel` wake class: content arrives in-turn, and rule 4 holds because a busy Pi gets a steer, not a nudge **I**. A content-free variant (our R18) would put the `wake up` marker in `content` and deliver the bodies at `before_agent_start` or as the steer **I**.
2. **Correlation ids in `details`.** Batch and item ids ride in a model-invisible field and come back in `message_start` and `context` events. That gives exact delivery proof at zero token cost (driver.ts:51-60, 168-190) **V**.
3. **Ack at "entered a provider request" (`context` event plus 2xx), not at "formatted".** This is a sharper definition of display than ours, and it is cheap in-process **V**.
4. **One batch in flight, and an exact-key drain.** Acks never become positional, even with pull-only items interleaved (inbox-turn.ts:46-67) **V**.
5. **Abort means hold until the person's next turn, and that turn carries the backlog** (driver.ts:253-256; pi.smoke.ts:355-375) **V**, for batches confirmed before the abort (see Acknowledgement). This matches the Claude-module pause rule. herdr-threads should adopt the same rule for any Pi adapter, and probably for the Claude mod as well (overview.md).
6. **Bounded self-continuation** on error and unknown stop reasons: two attempts, then a hold with a named reason (driver.ts:252-269) **V**.
7. **A deterministic session id at launch.** `--session-id <uuid>` plus an expected-id check against `PI_SESSION_ID` gives exact seat binding before the first turn, and a mismatch fails loud (connector.ts:85-99; extension.ts:140-148, 170-177) **V**. This fits our "never on heuristic evidence" rule in TRUST-POLICY.
8. **Process-global runtime map** keyed on `Symbol.for(...)`. It survives Pi's runtime teardown on reload, `/new`, `/resume` and `/fork` without reconnecting (extension.ts:25-47) **V**.
9. **Per-process attach with no config write** for managed launch: `pi --extension <path>` (connector.ts:84) **V**. Our launch could use the same approach for Pi.
10. **A forced pump at every `agent_start`**, including human-started turns. Non-waking backlog rides the next turn of any origin (driver.ts:165) **V**. This is the Pi analogue of our rule 3 (deliver every pending item in-turn).

Things to avoid copying: the per-batch instruction tail (it conflicts with our R17), no body cap, and wakes that ignore a human draft.

## Verification

Checked against the Cotal clone at `6426fb538e0b` (read only) and the Pi primer: every driver.ts, extension.ts, connector.ts, inbox-turn.ts, wrap.ts, package.json and README citation; connector-core control.ts, framing.ts, agent.ts (dedupe, overflow, attention default, ack wait, `requestWake`), tool-specs.ts tool count and gating; docs/connect-pi.md and docs/connectors.md line ranges; primer §2.3, §2.4 and §6. Most claims held. Changed:

- tools.ts citations pointed past the end of a 122-line file; now tools.ts:37-39, 90, 103-106 (pull-only inbox) and 44-54, 99 (`SEND_GUIDELINES`, about 1.3 k chars confirmed).
- events.ts:228-251 did not exist (153 lines); the WAL is opened at events.ts:88-104 via agui-wal-path.ts:69-75.
- Injection tail is 203 chars, not 197.
- `holdInFlight` is also called by the OpenCode connector, not only inline-delivery.ts.
- The focus nudge leaves the sender label raw as well as the channel (driver.ts:133-135, now **V**).
- The "no keystroke injection" and Claude-pause statements came from the sibling `overview.md`, not Cotal docs; re-attributed.
- Hold recovery was overstated: holds with a retained unconfirmed batch are not lifted by `agent_start`; the abort smoke test covers only an already-confirmed batch. Added the unconfirmed-abort gap (**I**).
- The 9.9 k-char tool-description count was not re-run; downgraded to **I**. Minor line fixes (inbox-turn.ts:49-57, pi.smoke.ts:355-375).

