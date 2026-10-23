# open-cross-session (ocs) x Pi coding agent

- Alternative: <https://github.com/leeguooooo/open-cross-session>, clone `scratchpad/alts/open-cross-session-ocs`, HEAD `1a3cee23a` (= `v0.9.4`). Paths marked `ocs:` are relative to that clone root.
- Harness: <https://github.com/badlogic/pi-mono>, clone `scratchpad/alts/pi-mono`, HEAD `a7b94a0`. Paths marked `pi:` are relative to that clone root.
- Read only. Nothing was executed or installed. VERIFIED = read in source or docs at the cited line. INFERRED = my reading, not stated.
- Context: `overview.md` (this directory) and `../../primers/pi.md`.

## Support status

**Native, TUI mode only.** VERIFIED.

- ocs ships a Pi extension as a string constant inside its binary (`ocs:src/pi-extension.ts:13-224`) and installs it as a global extension. Inside each interactive Pi process the extension opens a per-session Unix socket (a named pipe on Windows) and turns every authenticated "wake" frame into `pi.sendMessage(..., {deliverAs:"followUp", triggerTurn:true})` (`ocs:src/pi-extension.ts:170-179`).
- It runs only in TUI mode: `if (ctx.mode !== "tui") return;` (`ocs:src/pi-extension.ts:98`). Pi in RPC, JSON or print mode never registers and is unreachable. VERIFIED.
- The overview cites `src/pi-extension.ts:150-200`. The `sendMessage` call itself is at lines 170-179.
- It is advertised in `ocs:README.md:242` ("Run `ocs skill install`, then restart Pi... queues inbound messages as follow-ups, so a busy turn is not interrupted"), `ocs:DESIGN.md:28` and `ocs:docs/wake-protocol.md:24-26,199`.
- The capability matrix reports Pi as `wake:"available"`, `busyDelivery:"queued"`, `deliveryReceipt:"none"` (`ocs:src/capabilities.ts:6-13`). VERIFIED.
- Tests: `ocs:test/pi.test.ts:139-200` loads the generated extension into a **fake** Pi host (`ocs:test/fixtures/pi-extension-host.ts:12-30`, whose `sendMessage` only records the call). There is no test against a real Pi. Whether Pi behaves as the comments claim was therefore not exercised by ocs's suite (VERIFIED from the fixture). I checked it against Pi's source below.

## Injection mechanism

### Path, end to end

1. **Sender CLI** (`ocs send <ch> "... @pi-<8hex>"`, `ocs dm pi-<8hex> ...` or `--reply-to <seq>` whose author is a Pi):
   - The message is appended to the channel log first.
   - Then `splitWakeMentions` sends `pi-<uuid>` addresses to the Pi branch (`ocs:src/wake.ts:186-200`), and `resolveDmTarget` resolves short `pi-<8hex>` addresses by prefix against live registrations (`ocs:src/roster.ts:724-770`).
   - Send path: `ocs:src/cli.ts:573-605`. DM path: `ocs:src/deliver.ts:485-505`. Delivery notices back to a Pi sender: `ocs:src/deliver.ts:370-383`. VERIFIED.
2. **Discovery**: `listPiSessions` reads `$OCS_HOME/pi-sessions/<uuid>.<pid>.json` (`ocs:src/pi-sessions.ts:170-186`). A registration is accepted only if all of these hold (`ocs:src/pi-sessions.ts:102-164`):
   - the file is a regular file, not a symlink, at most 16 KiB, mode `& 0o077 == 0` and owned by the current uid;
   - the key set is exactly the expected one and `extension_version === 1`;
   - the filename uuid and pid match the content;
   - the socket path lies under `$OCS_HOME/pi-inbox`;
   - the token is 64 hex characters;
   - the pid is alive (`kill(pid,0)`);
   - the socket is a real 0600 socket owned by the uid.

   VERIFIED. On Windows the mode checks and the socket stat are skipped; the pipe name must instead match `ocs-pi-<sha256(uuid)[:24]>-<pid>-<32hex>` (`ocs:src/pi-sessions.ts:25,35-40,117,152`).
3. **Transport**: one connection per wake. The CLI writes a single JSON line `{"v":1,"type":"wake","session_id","token","note"}` and waits up to 2 s for a single JSON line ack (`ocs:src/pi-sessions.ts:205-291`, timeout `PI_WAKE_TIMEOUT_MS = 2_000` at :19). VERIFIED.
4. **Receiver (inside Pi)**: the extension's server checks:
   - frame size ≤ 16 KiB (`MAX_FRAME_BYTES`, `ocs:src/pi-extension.ts:22,144-148`);
   - one newline-terminated frame per connection, a 2 s socket timeout, then the connection is closed (`:137-151`);
   - `v`, `type`, `session_id`, `token` and a non-empty `note` (`:158-169`).

   Then it injects:

   ```js
   pi.sendMessage(
     { customType: "ocs", content: frame.note, display: true,
       details: { transport: WINDOWS ? "local-pipe" : "local-uds" } },
     { deliverAs: "followUp", triggerTurn: true },
   );
   reply(socket, { v: 1, ok: true, session_id: current.sessionId, delivery: "queued" });
   ```
   (`ocs:src/pi-extension.ts:170-180`, VERIFIED)
5. **In Pi**: `sendMessage` maps to `AgentSession.sendCustomMessage` (`pi:packages/coding-agent/src/core/agent-session.ts:3448-3455`). It is fire-and-forget: the returned promise's errors go to `runner.emitError`, not back to the caller. The branches are (`pi:.../agent-session.ts:2357-2393`, VERIFIED):
   - busy (`isStreaming`), `followUp`: `this.agent.followUp(appMessage)`;
   - idle, `triggerTurn`: `_startRun({messageType:"custom", message})`, which starts a new run immediately. If `agent_settled` handlers are running, the start is deferred until they finish (:2379-2381).

   The custom message is persisted in the session JSONL and reaches the model as a plain **`user`-role** text message (`pi:packages/coding-agent/src/core/messages.ts:162-169`). `details` is never sent to the model (primer §2.3). VERIFIED.

### Payload and framing

The content is the shared ocs "wake note" (`ocs:src/wake.ts:117-172`). The Pi branch passes `implicitReceiver: true`, so the `--as` flag is dropped from the generated commands (`ocs:src/cli.ts:591-594`, `ocs:src/deliver.ts:491-494`). VERIFIED. Shape (English catalog, `ocs:src/i18n.ts:315-327,512`):

```
[ocs wake] <sender> mentioned you in #<channel> (seq <N>[, reply to seq <M>])

<body>

Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.
Reply: ocs send <channel> "<your reply>" --reply-to <N>     (or: ocs dm <target> "<your reply>")
Thread: ocs read <channel>
```

- Unlike the Claude carrier, there is **no `<cross-session-message>` wrapper**. Pi receives the bare note. The only provenance the model gets is the `[ocs wake]` text header, inside a `user`-role message that looks like the user typed it. In the TUI it is rendered as a custom message (`display:true`), not in the editor. VERIFIED for wrapper absence (`ocs:src/wake.ts:90`, a Chinese comment saying Codex queue / Pi / cmux have no wrapper; paraphrased). How the TUI renders it is INFERRED.
- Neutralization still applies to the body: `<cross-session-message` becomes `‹…`, and lines that start with `Reply:`, `Thread:`, `[ocs wake]` and similar markers get a `> ` prefix (`ocs:src/wake.ts:85-98`). VERIFIED.

### When it fires

At send time, as a push, from the sender's CLI process. There is no turn-start hook, no `tool_result` append, no `before_agent_start` or `context` injection, and nothing on a quiet turn. VERIFIED: the extension subscribes only to `session_start`, `session_info_changed` and `session_shutdown` (`ocs:src/pi-extension.ts:208-223`).

### Size bounds

- Body inline up to 4096 B. Above that, the first 512 B plus `… (N bytes total; full text: ocs read <ch>)`. The skeleton is capped at 1024 B, the note at 5120 B (`ocs:src/wake.ts:39-46,145-171`). VERIFIED.
- Transport cap: 16 KiB, checked on both sides (`ocs:src/pi-sessions.ts:20,209-212`; `ocs:src/pi-extension.ts:22,144,165`). VERIFIED.

### Dedup and acknowledgement

- **Ack = "the extension called sendMessage without a synchronous throw."** The `queued` reply is sent right after the call (`ocs:src/pi-extension.ts:171-180`). Since Pi's `sendMessage` is fire-and-forget (`pi:.../agent-session.ts:3448-3455`), an asynchronous failure in `_startRun` (for example `_prepareRun` throwing, `pi:.../agent-session.ts:1873-1886`) is reported to Pi's extension error channel, while the sender has already printed `wake(pi): queued`. VERIFIED that the code is ordered this way. That it can actually occur is INFERRED.
- ocs itself says acceptance is not a read receipt (`ocs:src/pi-sessions.ts:9-10`, `ocs:README.md:249` "Pi acceptance means its extension queued the message"). VERIFIED.
- Outcome classes:
  - `unavailable`: no connect, or nothing written yet;
  - `failed`: the extension rejected the frame;
  - `unknown-outcome`: the frame was written but no valid ack arrived.

  `unknown` is never retried, and the CLI exits 3 with "do NOT resend" (`ocs:src/pi-sessions.ts:200-204,225-290`; `ocs:src/i18n.ts:365-374`). VERIFIED.
- Dedup: none at the receiver. Every valid frame becomes one custom message. Protection against duplicates comes only from the sender's never-retry rule. VERIFIED.
- Delivering the note does not advance the receiver's `ocs read` cursor (no cursor code in the extension). INFERRED: the same message still shows as unread in `ocs inbox` until the Pi model runs `ocs read`.

## Wake mechanism

- **Mechanism**: the injection is the wake. `triggerTurn:true` on an idle session calls `_startRun`, so a turn starts with no keystrokes, no Herdr/tmux involvement and no screen scraping (`pi:.../agent-session.ts:2378-2383`). VERIFIED.
- **Idle/busy detection**: ocs does none. It delegates to Pi's own `isStreaming` branch. Busy means the message goes on the follow-up queue, which Pi drains only when the agent would otherwise stop (primer §2.3, `pi:packages/agent/src/agent-loop.ts:310-317`). So a busy Pi is not steered mid-turn; the loop picks the follow-up up at the point where the run would otherwise stop and continues the **same** run with it (`continue` at `agent-loop.ts:316`), not a separate run. VERIFIED from Pi source. The overview's "busy queues after the current turn" is more precisely "after the run's natural stop point" (after all tool calls and steering of that run).
- **Burst behaviour**: Pi's `followUpMode` default is `one-at-a-time` (`pi:packages/agent/src/agent.ts:256`; the coding agent passes the user setting, default `one-at-a-time`, `pi:packages/coding-agent/src/core/settings-defaults.ts:8`, `sdk.ts:434`; a user may set `all`). Under the default, N wakes that arrive while Pi is busy become N follow-up model requests in sequence, each with its own full note skeleton. There is no coalescing, unlike ocs's Codex backlog (`overview.md`, "Codex busy handling"). VERIFIED for the queue default. The per-message request cost is INFERRED.
- **Compaction**: compacting is not `isStreaming`, so `triggerTurn` takes the `_startRun` branch. Whether that waits, throws or races with compaction is unknown (not traced). Any throw is lost as described under "Dedup and acknowledgement".
- **Draft safety**: the extension never reads or writes the editor (no `ctx.ui.getEditorText`/`setEditorText` call in `ocs:src/pi-extension.ts`). VERIFIED. So a human's draft is not clobbered (INFERRED: custom messages render in the chat, not the editor). But an idle wake starts a run while the human may be mid-draft. When the human then presses Enter, the draft becomes a **steer** into the agent's ocs-triggered run (primer §6, `interactive-mode.ts:3333-3338`). That is contention, not a clobber. Pi exposes `getEditorText()`, which ocs could have used to defer the wake (INFERRED).
- **Self-wake**: `ocs send` skips a Pi mention whose target equals the sender (`ocs:src/cli.ts:574-578`, test `ocs:test/pi-cli.test.ts:110-121`). VERIFIED. I found no equivalent guard in the `ocs dm` path (`ocs:src/deliver.ts:485-505`). INFERRED from the absence.
- **Latency**: one local UDS round trip from the sender's CLI, synchronous with `ocs send`, plus Pi's run start. ocs does no polling. INFERRED: sub-second. Bounded by the 2 s timeout.
- **notify-when-idle** (wake me when the target goes idle) is Claude-only: the target must resolve to a live Claude session (`ocs:src/cli.ts:906`). Pi's `agent_settled` event is not used. VERIFIED.
- **Priority/threshold**: none. Every mention, DM and reply-to wakes, unless the sender passes `--no-wake` (`overview.md`). VERIFIED in the overview.
- **Failure modes** (VERIFIED unless marked):
  - Pi not restarted after install, or not in TUI mode: no registration, `wake(pi): ... is not a live registered Pi TUI` and exit 2 (`ocs:src/i18n.ts:370-371,530-531`). The message remains stored only.
  - Pi crashed (SIGKILL): the registration file stays behind. The reader rejects it because the pid is dead, or because connecting to the stale socket fails (`unavailable`). If the pid is reused, the 256-bit token check rejects the frame (`ocs:src/pi-extension.ts:131,162`).
  - `/new`, `/resume`, `/fork` or reload: `session_shutdown` then `session_start` rebinds to the **new** Pi session id (`ocs:src/pi-extension.ts:96-99,208-222`). The address `pi-<old uuid>` becomes unreachable, and messages to it are stored only. There is no forwarding and "no auto-nudge" when a session comes online (`overview.md`). INFERRED: identity is tied to the Pi session, not to the pane or agent.
  - The same session open in two Pi processes: refused as ambiguous (`ocs:src/roster.ts:477,749-762`; `ocs:src/i18n.ts:373-374`).
  - The registration keeps `extension_version === 1` as an exact-match requirement (`ocs:src/pi-sessions.ts:129`). A future v2 extension and an older `ocs` reader would not see each other until both are updated (INFERRED).
  - LAN: a remote DM is delivered only to a live registered Pi; otherwise `not-found` and nothing is stored (`ocs:src/lan-daemon.ts:252-259`).

## Setup and installation footprint on the harness

VERIFIED (`ocs:src/pi-extension.ts:226-284`, `ocs:src/cli.ts:1409-1427,1730-1731`, `ocs:README.md:128-146`):

- `ocs skill install`, the curl installer, or `ocs doctor --fix` writes:
  - `$PI_CODING_AGENT_DIR/extensions/ocs.ts` (default `~/.pi/agent/extensions/ocs.ts`): a **global** extension that every interactive Pi process loads;
  - `$PI_CODING_AGENT_DIR/skills/ocs/SKILL.md`: the ocs skill (12,748 B, same bytes as `ocs:skills/ocs/SKILL.md`; `ocs:src/cli.ts:1492-1665`).

  Writes are atomic (tmp file plus rename, mode 0600, `ocs:src/pi-extension.ts:244-264`), and the write is skipped when the file content is already identical (which preserves a `skills add` symlink). The installer may also run the pinned `skills` CLI through `npx` with telemetry disabled (`ocs:README.md:128-129`).
- `ocs doctor` byte-compares the installed extension against the embedded source (`piExtensionCurrent`, `ocs:src/pi-extension.ts:278-284`) and counts live sessions.
- Runtime files, per Pi process:
  - `~/.ocs/pi-sessions/<uuid>.<pid>.json` (0600), registration with a socket path and a 64-hex token;
  - `~/.ocs/pi-inbox/pi-<sha256(uuid)[:24]>-<pid>.sock` (0600);
  - directories 0700 (`ocs:src/pi-extension.ts:101-113,196`).

  The extension also exports `OCS_PI_SESSION_ID` into `process.env` for self-identification (`:200`) and sets a footer status `ocs: pi-<uuid>` (`:201`).
- A restart of open Pi sessions is required after install or upgrade (`ocs:README.md:140`, `ocs:src/i18n.ts:437`).
- The extension imports no Pi package, so the same file works across Pi distributions (`ocs:src/pi-extension.ts:1-3`).
- It does not use `pi -e` (per-process load), `registerFlag` or `registerMcpServer`.

## Token cost

- **Per message**: the note skeleton is roughly 250-300 B on top of the body: header ≈ 60-90 B, the fixed reply-policy line 84 B (`ocs:src/i18n.ts:512`), `Reply:` ≈ 50-70 B, `Thread:` ≈ 25-40 B. INFERRED from the format strings, not measured. Each note also costs a full model request: an idle wake starts a run, and a busy follow-up adds one request per message under `one-at-a-time`. The body is inline (≤ 4096 B), so a short message needs no extra `ocs read` round trip.
- **Per quiet turn**: zero. The extension registers no per-turn events, so nothing is injected (VERIFIED).
- **Static**: the 12,748 B skill. How much of it Pi loads into context per session (description only, or the full body) was not checked against Pi's skill loader. Unknown.

## Limitations and bugs I can see

1. **The ack comes before the outcome**: the `queued` reply follows a fire-and-forget `sendMessage`. A failure when the run starts is invisible to the sender (VERIFIED ordering, see above).
2. **No coalescing when busy**: a burst becomes N follow-up runs with N skeletons (VERIFIED queue default; cost INFERRED).
3. **No draft or human-presence check**: an idle wake can start a run under a human who is typing, and their Enter then steers that run (INFERRED from Pi semantics).
4. **No in-turn delivery**: a busy Pi sees the message only at the run's natural stop point, never after a tool batch (VERIFIED: `followUp`, not `steer`; no `tool_result` handler). That avoids disrupting the run, but long runs mean high latency.
5. **No provenance wrapper**: the note arrives as a `user`-role message. The model can only tell it is a peer message from the `[ocs wake]` text (VERIFIED).
6. **Identity churn**: the address is the Pi session uuid, so `/new`, `/resume` and `/fork` drop reachability for the old address, and messages to it are stored only with no catch-up wake (VERIFIED/INFERRED as above).
7. **TUI only**: RPC, JSON and print Pi sessions are unsupported (VERIFIED).
8. **Global extension**: it loads in every interactive Pi, including ones that do not use ocs, and opens a socket in each (VERIFIED by install path; the effect is INFERRED).
9. **Possible self-wake via `ocs dm`**: the explicit self guard exists only on the `send`/mention path (INFERRED from absence).
10. **Read state diverges from delivery state**: a delivered note leaves the message unread in `ocs inbox` (INFERRED).
11. **The real-host path is untested**: tests use a fake Pi host (VERIFIED).

## Would ocs's generic mechanism work on Pi without the extension?

Not applicable: Pi has a native carrier. For completeness: without the extension, a Pi agent can still run `ocs send/read` through its `bash` tool, but it cannot be woken. ocs's cmux keystroke fallback is wired only for Claude and Codex TUIs (`overview.md`). Typing into a Pi TUI while it is busy would become a steer (primer §6). INFERRED.

## Clever ideas worth stealing (for a herdr-threads Pi adapter)

1. **Extension-owned per-session socket with a token-bearing registry.** These pieces map directly onto a herdr-threads Pi "mod channel": a daemon-to-extension push with no keystrokes, the `Channel` wake class from AGENTS.md, rule 6.
   - a 0600 UDS under a 0700 dir;
   - a 256-bit random token per runtime;
   - a registration file with strict reader validation (uid, mode, exact key set, pid alive, socket path confined to the inbox dir, filename matches content);
   - start in `session_start` and cleanup in `session_shutdown`, guarded by token and pid (`ocs:src/pi-extension.ts:74-79,96-206`; `ocs:src/pi-sessions.ts:102-164`).
2. **`followUp` + `triggerTurn` in one call**: idle wakes, busy never steers. For herdr-threads, the better split (INFERRED) follows the delivery contract:
   - `p <= T` and idle: `sendMessage(triggerTurn)`;
   - busy: append in-turn via `tool_result` or `steer` (rule 3);
   - otherwise: `deliverAs:"nextTurn"`, or `before_agent_start` (silent arrival, rule 2).
3. **Outcome classes `unavailable` / `failed` / `unknown-outcome`, with the rule that a written frame is never retried** (`ocs:src/pi-sessions.ts:200-204,242-249`). This is a clean way to state at-most-once wake semantics.
4. **Self-identification through an env var exported by the extension** (`OCS_PI_SESSION_ID`, `ocs:src/pi-extension.ts:200`). Children of the Pi process, including user `!` commands, inherit it. Pi's own `PI_SESSION_ID` is set only for LLM-called bash (`pi:packages/coding-agent/src/core/tools/bash.ts:193-203`). For herdr-threads seat attribution, a provenance value would be needed (TRUST-POLICY).
5. **Embedded extension source, byte-compare for doctor, atomic write, no package import** for cross-distribution compatibility (`ocs:src/pi-extension.ts:1-3,244-284`).
6. **Footer status line showing the session's address** (`ctx.ui.setStatus("ocs", ...)`, `ocs:src/pi-extension.ts:201`): a cheap human-visible "this pane is reachable" signal.

Things to do better than ocs:
- Send the ack only after `sendCustomMessage` resolves, or report asynchronous errors.
- Check `ctx.isIdle()` and an empty `ctx.ui.getEditorText()` before an idle wake.
- Coalesce a burst into one message.
- Use `pi -e` at managed launch instead of a global extension.
- Bind the seat across `/new`, `/resume` and `/fork` with `session_start.reason` and `previousSessionFile` (primer §2.1, §5).

## Verification

Adversarial pass against ocs `1a3cee2` and pi-mono `a7b94a0`, read only.

- Checked: `pi-extension.ts` (whole file: TUI guard, frame checks, `sendMessage` call and ack order, event subscriptions, install paths, atomic write, byte-compare); `pi-sessions.ts` (registration validation, timeout, outcome classes); `wake.ts` (size constants, neutralization, Pi routing); `cli.ts` send path, self-skip, `notify-when-idle` Claude-only check, doctor, exit codes 2/3 (`cli.ts:394-398`); `deliver.ts` notice and DM paths (no Pi self guard on DM confirmed); `roster.ts` short-id and ambiguity resolution; `lan-daemon.ts:259`; `capabilities.ts`; i18n strings; README/DESIGN/wake-protocol lines; the fake-host fixture and self-wake test. Pi: `sendCustomMessage` branches, fire-and-forget `sendMessage` binding, `_startRun` throw path, custom-to-`user` conversion, follow-up drain, `followUpMode` default, `PI_SESSION_ID` scoping, Enter-while-streaming steer, `ExtensionMode`, `getEditorText`/`isIdle`, `session_start` reasons.
- Changed: skill size 12,970 B to 12,748 B (computed from the `SKILL_MD` template, equal to `skills/ocs/SKILL.md`); reply-policy line 86 B to 84 B; busy follow-ups continue the same run rather than starting an extra run; added the coding-agent `followUpMode` default citation and the user-settable `all`; noted Windows skips the mode/socket checks; marked the `wake.ts:90` quote as a paraphrase of a Chinese comment.
- Not checked: compaction interaction (still unknown); real-Pi rendering of `display:true` custom messages.
