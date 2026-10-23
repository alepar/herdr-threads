# open-cross-session (ocs) x OpenAI Codex CLI

- Upstream: https://github.com/leeguooooo/open-cross-session. The clone at `scratchpad/alts/open-cross-session-ocs` is at HEAD `1a3cee23a` (tag `v0.9.4`, 2026-10-22). It was read only, never executed.
- Paths are relative to the clone root unless prefixed. `codex-rs/...` paths refer to the Codex `rust-v0.163.0` sparse clone described in `primers/codex.md`.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from them. **unknown** means not found.
- Most source comments in ocs are Chinese. The quotes below are my translations unless shown as code.

## 1. Support status

**Native, with three carriers and one fallback.** VERIFIED: `src/deliver.ts:100-252`, `skills/ocs/SKILL.md:111-126`, `DESIGN.md:26-27`.

| Codex host | Idle target | Busy (mid-turn) target | Provenance |
|---|---|---|---|
| Terminal TUI (`codex`, including inside tmux/cmux/Herdr) | `codex queue --thread <uuid> --message <note>` | app-server `turn/steer` over the daemon control socket. If steer is not possible, the wake goes into an ocs-side backlog and one merged wake is sent after the turn | plain user message (ocs states this, `src/deliver.ts:141-150`) |
| ChatGPT Desktop-hosted task | private Desktop IPC `thread-follower-start-turn` carrying a `send_message_to_thread` tool output wrapped in `<codex_delegation>` | backlog plus one merged wake (steer is skipped on purpose to keep the envelope) | native cross-task envelope |
| Either, when the above fail definitely | cmux keystrokes (`cmux send` + `send-key enter`) into one uniquely matched idle surface | skipped while the screen shows `to interrupt` | none (typed text) |

The project's own validation notes say a pure terminal TUI in tmux ran the queued turn on Codex v0.153.4 (`src/codex-queue.ts:8-9`). They also say `turn/steer` into a daemon-hosted TUI worked on codex 0.160.1 on macOS (`src/codex-steer.ts:6-9`, `CHANGELOG.md:91`). VERIFIED that these claims are in the repo. I did not reproduce them.

Linux note: terminal-TUI support depends on `lsof` and `ps` (`src/codex-queue.ts:166-172`, `:367-372`). On Windows the queue path is not used (`CHANGELOG.md:157`), so only Desktop IPC applies there. VERIFIED.

## 2. Injection mechanism

### 2.1 Entry point and ordering

Each `ocs send`/`ocs dm` is a short-lived CLI process. It appends the message to `~/.ocs/channels/<channel>.jsonl` (`$OCS_HOME` overrides `~/.ocs`, `src/store.ts:51-62`) and then runs the delivery ladder `deliverToCodexTaskImpl` (`src/deliver.ts:120-252`). There are no hooks, no MCP and no resident daemon for this path (overview; grep of `src/` for hook event names finds none). VERIFIED.

The order of rungs (`src/deliver.ts`):

1. **Self-wake guard** (`:132-137`): if the sender's `CODEX_THREAD_ID` equals the target, ocs skips delivery.
2. **Liveness** (`:138-139`): `psTable()` + `codexThreadLivePid()`. ocs finds the rollout file whose name contains the thread UUID under `$CODEX_HOME/sessions/YYYY/MM/DD/`, runs `lsof -F pftn -a -p <pids>` over every process whose ancestry contains a binary named exactly `codex` or the ChatGPT.app bundle, and keeps only REG fds on that rollout (`src/codex-queue.ts:145-191`, `:397-408`).
3. **Host classification** (`:147`): ocs walks the parent chain of the holder pid to find the first `.app` bundle. `ChatGPT` means Desktop-hosted (`src/codex-queue.ts:325-356`).
4. **Busy branch** (`:151-187`): see §3.2. It ends in steer, deferral, or neither.
5. **`codex queue`** (`:188-208`): for a live, non-Desktop target.
6. **Desktop IPC** (`:209-222`): `wakeCodexTask`.
7. **cmux fallback** (`:223-226`): only when the IPC reason is `unavailable|not-open|no-source` (`:84`).
8. **Queue as the last resort for a Desktop-hosted task** (`:229-248`): this gives up the envelope.
9. Otherwise ocs reports `codexFailed`. The message stays stored and the exit code is 2.

Any `unknown-outcome` stops the ladder at once and is never replayed (`:169-173`, `:202-206`, `:218-222`). VERIFIED.

### 2.2 The three wire payloads

**(a) `codex queue` (idle terminal TUI).** VERIFIED `src/codex-queue.ts:243-247`:

```ts
const proc = spawnSync("codex", ["queue", "--thread", threadId, "--message", input.prompt], {
  encoding: "utf8", timeout: CODEX_QUEUE_TIMEOUT_MS /* 20_000 */, env });
```

The note is passed as one argv element. On the Codex side this becomes `thread/queue/add {threadId, input:[{type:text}], clientUserMessageId: uuidv7}` (primer §3, `codex-rs/tui/src/session_queue_commands.rs:84-127`). The item persists in SQLite and is dispatched as a user turn with `turn_trigger:"queue"` when the thread is idle. After the spawn, ocs checks with `kill(pid,0)` that the same holder pid is still alive. If the holder exited or changed, the result is `unknown-outcome` (`src/codex-queue.ts:263-270`, `:291-303`). ocs reads the message id from stdout with `/Queued message ([0-9a-f-]{36})/` (`:209`, `:268`). VERIFIED.

**(b) `turn/steer` (busy terminal TUI).** VERIFIED `src/codex-steer.ts:269-330`. ocs connects to `$CODEX_HOME/app-server-control/app-server-control.sock`, resolving symlinks by hand. It requires that the socket and its directory are owned by the current uid and that the directory is not group- or world-writable (`:52-71`). It runs a hand-written WebSocket client over the UDS (HTTP Upgrade, masked frames, ping/pong; `:74-254`), then:

```ts
conn.request("initialize", { clientInfo: { name: "ocs", title: null, version: "1" },
                             capabilities: { experimentalApi: true } }, connectTimeout);
conn.send({ method: "initialized" });
conn.request("turn/steer", { threadId, expectedTurnId,
  input: [{ type: "text", text: input.prompt, text_elements: [] }] }, 10_000);
```

`expectedTurnId` comes from the rollout tail (§3.1). Codex checks it as a precondition. It rejects an empty id, a mismatched turn ("expected active turn id ... but found ..."), the case with no active turn, and review or compact turns (`codex-rs/app-server/src/request_processors/turn_processor.rs:1062-1130`, VERIFIED). ocs classifies the outcomes this way: a connect or initialize failure is `unavailable`, a JSON-RPC error is `rejected` (definitely not inserted), and a timeout after the frame was written is `unknown-outcome` (`src/codex-steer.ts:280-326`). The socket name `app-server-control.sock` matches Codex 0.163 test fixtures (`codex-rs/tui/src/startup_preflight_tests.rs:122`, `codex-rs/app-server-daemon/src/lib.rs:1406-1408`). A success reply without a string `turnId` is also classed `unknown-outcome` (`src/codex-steer.ts:321-325`). This corrects the primer's inferred `server.sock`. VERIFIED.

**(c) Desktop IPC (Desktop-hosted task).** VERIFIED `src/codex-ipc.ts:13-17`, `:180-187`, `:333-365`. The transport is length-prefixed JSON on `$CODEX_HOME/ipc/ipc.sock`. The client first discovers the renderer that owns the thread, then sends:

```ts
"thread-follower-start-turn", { conversationId: target, turnStart: { request: {
  threadId: target, input: [], clientUserMessageId,
  toolOutput: { name: "send_message_to_thread", namespace: "codex_app",
                output: codexDelegationEnvelope(sourceThreadId, prompt) } },
  context: { inheritThreadSettings: true, threadStartKind: "user" } } }
```

The envelope is XML-escaped `<codex_delegation><source_thread_id>…</source_thread_id><input>…</input></codex_delegation>`. It needs a *second* open task under the same renderer to act as the source. ocs probes candidates concurrently with a 500 ms deadline and refuses if the source and target owners differ (`src/wake.ts:437-571`). This is a private protocol, and the file header warns that it "may break with host upgrades" (`src/codex-ipc.ts:10-11`).

### 2.3 When it fires

Injection is push-only, at send time, from the sender's process. VERIFIED (no hooks). In Codex terms:

- Idle target: the note becomes the user message of a new turn (queue or IPC).
- Busy target with steer: the note joins the active turn as user input at the next sampling step. That is real mid-turn delivery with no tool-call dependency (`CHANGELOG.md:88`). INFERRED from `turn/steer` semantics (primer §2.2).
- Busy target without steer: no injection until the turn ends, then one merged wake (§3.3).
- Target offline: nothing is pushed. The message stays in the log and is seen only when the agent runs `ocs inbox` or `ocs read` (`README.md:349-351`, overview). VERIFIED.

### 2.4 Payload format and framing

The note is built by `wakeNote` (`src/wake.ts:131-172`, i18n strings `src/i18n.ts:315-327`, `:512`). For Codex, `receiver` is `codex-<8hex>` and `implicitReceiver:true`, because Codex exports `CODEX_THREAD_ID` to tool shells and ocs can identify the agent from it, so no `--as` is needed (`src/deliver.ts:158-162`, `:192-196`). Shape:

```
[ocs wake] <sender> mentioned you in #<channel> (seq <N>[, reply to seq <M>][, plus K earlier unread from seq F — read the thread first][, <ago>])

<body>

Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.
Reply: ocs send <channel> "<your reply>" --reply-to <N>      (or: ocs dm <target> "<your reply>")
Thread: ocs read <channel>
```

- There is **no wrapper tag on the Codex terminal path**. Claude gets `<cross-session-message from=… from-name=…>`, and Desktop gets `<codex_delegation>`, but queue and steer deliver the bare note as a user message. ocs acknowledges this in source: the queue "leaves an ordinary `UserMessage` — it presents another agent's message as if the user typed it" (`src/deliver.ts:141-145`). That is why Desktop-hosted tasks prefer IPC and skip steer (`:150`). VERIFIED.
- **Neutralization** applies on every carrier. `<cross-session-message` becomes `‹cross-session-message`, and body lines that start with `Reply:`, `Thread:`, `[ocs wake]` and similar markers get a `> ` prefix, so the body cannot forge the skeleton (`src/wake.ts:85-98`). VERIFIED.

### 2.5 Size bounds

- A body of 4096 UTF-8 bytes or less (counted after neutralization) is inlined verbatim. A larger body is cut to its first 512 bytes on a character boundary, followed by `… (<total> bytes total; full text: ocs read <ch>)`. The skeleton is at most 1024 bytes, and the ladder drops `ago` first, then the sender. The whole note is at most 5120 bytes (`src/wake.ts:40-46`, `:145-171`). VERIFIED.
- Codex itself limits steer and queue input to `MAX_USER_INPUT_TEXT_CHARS` (primer §3; `turn_processor.rs:1068` `validate_v2_input_limit`), far above 5120. VERIFIED.
- A merged wake carries **only the latest unread body**. Earlier messages are only counted in the header (`src/codex-defer.ts:310-339`). VERIFIED.

### 2.6 Dedup and acknowledgement

- **There is no ACK protocol.** "Read" means that the per-consumer cursor was advanced by `ocs read`. For Codex the consumer names are the thread id, `identityCursorConsumer("codex:<tid>")`, or any `ocs rename` name, and the highest of them wins (`src/codex-defer.ts:294-301`). An inlined wake body does **not** advance the cursor: "a wake whose body was inlined does not mean the agent processed it" (`src/codex-defer.ts:25-26`). INFERRED: the same message therefore still shows as unread in `ocs inbox` until the agent runs `ocs read`.
- **Wake dedup is about the wake, not the message.** Three mechanisms:
  - The backlog record deduplicates by seq (`src/codex-defer.ts:258`).
  - The merged wake drops seqs already at or below the receiver's cursor, and sends nothing if every seq has been read (`:316-319`, `:427-431`).
  - An `unknown-outcome` is never retried, at any rung. A batch the watcher has taken out "counts as processed" even if its outcome is unknown (`:418-424`).
- **Codex's own dedup handle is not used.** `codex queue` generates its own `clientUserMessageId` (primer §3), so ocs has no idempotency key across retries. It compensates by never retrying. INFERRED.

## 3. Wake mechanism

### 3.1 Idle and busy detection

The busy check is a **rollout-tail parse**, not an app-server status subscription. VERIFIED `src/codex-defer.ts:164-206`:

```ts
const LIFECYCLE_RE = /"type":"event_msg","payload":\{"type":"(task_started|task_complete|turn_aborted)"(?:,"turn_id":"([^"]+)")?/g;
// ... read the rollout backwards in 256 KiB chunks, take the last match:
if (last !== null) return last[1] === "task_started" ? (last[2] ?? "") : null;
```

- `task_started` as the last lifecycle event means busy, and its `turn_id` becomes `expectedTurnId`. An empty string means busy but not steerable, which applies to old formats.
- If no rollout exists, the file cannot be read, or no event is found, the target counts as **idle**. This fails open: the wake is delivered at once rather than held.
- The regex depends on the exact JSON key order. A local rollout from 2026-10-03 (Codex version not recorded) contains this exact prefix: 15 `task_started` and 15 `task_complete` events, each with `turn_id` directly after `type`. VERIFIED locally by counting the prefix only. Whether 0.163 still serializes it this way: unknown, because the protocol crate is not in the sparse clone.

Liveness is a separate check (§2.1 step 2), and it fails closed: if no holder is found, there is no queue.

### 3.2 Busy target: steer first, then the backlog

VERIFIED `src/deliver.ts:151-187`. The steer is attempted only when all of the following hold:

- the target is live,
- `defer !== false` (the watcher's own delivery passes `defer:false`, `src/cli.ts:1896-1903`),
- the wake is not a raw ocs notice,
- `activeTurn` is a non-empty id,
- **no backlog already exists** for that (thread, channel),
- the target is not Desktop-hosted.

The `!backlog` condition is deliberate. Once messages are waiting for a merged wake, a newer message must not overtake them by steering into the turn. INFERRED intent; the condition itself is VERIFIED at `:154`.

If the steer is `rejected` or `unavailable`, or was not attempted (no turn id, existing backlog, Desktop-hosted), `deferCodexWakeIfBusy` runs (`src/codex-defer.ts:233-270`). Under a lock file it does the following:

1. It merges the message into an existing record, or opens a new one if the target is busy.
2. It writes `~/.ocs/codex-wakes/<thread>.<channel>.json` atomically (tmp file plus rename, mode 0600).
3. If no watcher heartbeat is fresher than 30 s, it writes the heartbeat **before** spawning a detached `ocs _codex-wake-watch <thread> <channel>`.

The sender reports `deferred` with exit code 0.

### 3.3 The merged-wake watcher

VERIFIED `src/codex-defer.ts:374-447`. Each poll interval (default 2 s, `OCS_IDLE_POLL_MS`), the watcher:

- touches its heartbeat,
- exits and deletes the record after 3 consecutive not-live probes ("target-gone"),
- keeps waiting while the target is busy, unless the backlog is older than 6 h (`CODEX_WAKE_MAX_DEFER_MS`),
- otherwise takes the whole batch under the lock, filters it by the read cursor, builds one note (latest body, `plus K earlier unread from seq F — read the thread first`, `ago` from the stored timestamp), and delivers it through the same ladder with `defer:false`. In practice that means `codex queue`, because the target is idle now.

Every decision is appended to `~/.ocs/codex-wakes/history.jsonl` for diagnosis.

### 3.4 Idle target: how the wake actually happens

- **`codex queue`**: Codex's queue engine calls `wake_if_loaded` and then `dispatch_if_idle`, which runs `start_turn_if_idle(turn_trigger:"queue")`. A cross-process write is picked up by the SQLite `change_version` watcher every 10 s (primer §3, `codex-rs/ext/queue/src/service.rs:89-245`, `:405-483`). VERIFIED in Codex source through the primer.
- **Desktop IPC**: the renderer starts the turn itself. The reply carries a turn id (`result.result.turn.id`); without one ocs reports `unknown-outcome` (`src/codex-ipc.ts:362-369`). VERIFIED.
- **cmux**: ocs reads the last 8 lines of the screen. If they contain `to interrupt`, the result is busy and ocs does nothing. Otherwise it runs `cmux send --surface <ref> <note>` and then `cmux send-key enter` (`src/roster.ts:253-279`). The surface is matched by a title regex `codex-<8hex>` and must be unique and run a live `codex` process (`cmux top --all --processes --json`, filtered on process name `codex`; `:185-245`). VERIFIED.

### 3.5 Draft safety

- **queue, steer, IPC**: none of them touches the composer. A human's half-typed draft is never submitted or merged. Codex's queue starts its turn beside the TUI state (primer §3). INFERRED from both sources. This is the main advantage of these carriers over keystroke injection.
- However, a turn **starts while a human is typing**. The queued note becomes the active turn, and whatever the human submits next becomes a steer of that turn, because Enter during a turn steers (primer §7). ocs has no "human is composing" gate. INFERRED.
- **cmux**: no draft check at all. `cmux send` types the note after any existing composer text, so the two are submitted together (`src/roster.ts:262-271`). The note is sent as a plain `cmux send` followed by a separate Enter key event. Whether cmux sends bracketed paste is unknown. If it does not, Codex's 120 ms paste-burst Enter suppression (primer §7) could turn the Enter into a newline. INFERRED.

### 3.6 Latency

| Path | Latency | Basis |
|---|---|---|
| Sender overhead before any carrier runs | one `ps -A`, a directory walk of `$CODEX_HOME/sessions` (the comment mentions more than 2500 files), one `lsof` (about 349 ms measured), plus a backwards read of the rollout | `src/codex-queue.ts:151-152`, `:272-276`; VERIFIED (measured figures are the author's) |
| `turn/steer` | immediate, at the next sampling step; 3 s connect budget, 10 s answer budget | `src/codex-steer.ts:28-30` VERIFIED; arrival point INFERRED |
| `codex queue` to a daemon-hosted thread | about immediate (same server) | primer §3, INFERRED |
| `codex queue` to a `--no-daemon` TUI | about 10-20 s (SQLite poll plus dispatcher loop), or never; the primer says this is contested | primer §3, INFERRED |
| Merged wake | turn end + at most 2 s poll + queue latency | `src/codex-defer.ts:42` VERIFIED |
| Desktop IPC | about 1 s worst case for owner discovery on a closed task; then immediate | `CHANGELOG.md:211` VERIFIED |

### 3.7 Failure modes

1. **The `--no-daemon` launch that herdr-threads advises (`integrations/codex/README.md:236` in this repo) disables steer.** `turn/steer` goes to the shared daemon. A thread loaded only in a `--no-daemon` TUI's embedded server is not in the daemon's thread manager, so the result is `thread not found`: `turn_steer_inner` calls `load_thread`, which maps a `get_thread` failure to `invalid_request("thread not found: …")` (`codex-rs/app-server/src/request_processors/turn_processor.rs:1044-1050`, `:377-392`, VERIFIED; that the thread is absent from the daemon's manager is INFERRED). ocs then classifies `rejected` and falls back to the backlog, which is correct but slower. If no daemon socket exists, the result is `unavailable` and also falls back to the backlog. VERIFIED (ocs side).
2. **An interrupted turn blocks the queue wake.** After a human presses Ctrl-C, the rollout's last event is `turn_aborted`, so ocs sees the target as idle and runs `codex queue`, which returns ok. But Codex's queue engine does not dispatch after `Interrupted` (`service.rs:472-483`, `:549-566`, primer §3). The message sits in Codex's queue until the next completed turn or an explicit `thread/queue/start`, while ocs has reported `queued`. VERIFIED in both sources; the combined effect is INFERRED.
3. **The daemon can keep a closed TUI "live".** With the shared daemon, the rollout holder is `codex app-server --managed-daemon` (`src/codex-steer.ts:7-8`). The daemon keeps a thread loaded for a 30-minute grace period after its last subscriber leaves (primer §2.3). If the daemon keeps the rollout fd open during that time, ocs counts a TUI that has exited as live, queues to it, and the daemon may run the turn **headless** with nobody watching. INFERRED; untested.
4. **Race between the busy check and the queue add.** A turn can start between the rollout read and the `codex queue` call. Codex then holds the item and replays it as its own turn after the current one, which is the very behaviour #41 tried to avoid. The window is small and the effect harmless. INFERRED.
5. **Per-channel merging, not per-recipient.** The backlog key is `<thread>.<channel>` (`src/codex-defer.ts:74-76`). A busy Codex that receives DMs on three channels gets three separate wakes, and so three new turns, after its turn ends. VERIFIED (key); the consequence is INFERRED.
6. **A busy detection that fails open triggers the problem it guards against.** If the rollout tail format changes (key order, renamed events), every target looks idle. Messages then go to `codex queue` during turns and come back as N replayed turns. No error is reported. INFERRED from `src/codex-defer.ts:169-171`.
7. **Liveness heuristics.** The holder must have `codex` as the exact basename somewhere in its ancestry (`src/codex-queue.ts:397-408`). A wrapper with a different name, such as a renamed binary or a `node` shim that keeps the fd itself, would read as dead, and the message would be stored only, with exit 2. INFERRED. Ephemeral `/side` threads have no durable queue (primer §3), so `codex queue` to them fails and the result is `failed`. INFERRED.
8. **Short-id collisions.** `codex-<8hex>` is the UUIDv7 prefix and is shared by tasks started within about a minute of each other (`CHANGELOG.md:58`). Ambiguous addresses fail before the write (`CHANGELOG.md:203`). VERIFIED.

## 4. Setup and installation footprint on Codex

- **The only Codex-side file is a skill.** `ocs skill install` (also run by the installer and by `doctor --fix`) writes the embedded `SKILL.md` atomically to `~/.codex/skills/ocs/SKILL.md`, mode 0600 (`src/cli.ts:1675-1741`). It does not consider `CODEX_HOME` and always uses `homedir()/.codex` (`:1679`). The curl installer may instead register it through the pinned `skills` CLI (`README.md:128`, `CHANGELOG.md:229`). VERIFIED.
- **No `hooks.json`, no `config.toml` edits, no MCP server registration, no `notify`, no trust prompts.** Grep of `src/` for writes under `.codex` finds only the skill path. VERIFIED.
- **Runtime reads** under `$CODEX_HOME`:
  - `sessions/**/rollout-*.jsonl`, for liveness and busy state,
  - `app-server-control/app-server-control.sock`, for steer,
  - `ipc/ipc.sock`, for Desktop.

  ocs writes only under `~/.ocs`: `codex-wakes/`, the channel logs and the cursors. VERIFIED.
- **Prerequisites**: the `codex` binary on PATH (checked by an executable-bit test only, not a spawn, `src/codex-queue.ts:56-75`), `lsof` and `ps`. Steer also needs a running shared daemon (`CHANGELOG.md:88`). `ocs doctor` runs `codex queue --help` and checks for `--thread` (`src/codex-queue.ts:81-88`, `src/cli.ts:1376-1380`). This is a capability probe, not a version gate. VERIFIED.

## 5. Token cost

- **Per message, idle target**: one new user turn whose input is the note. The skeleton is about 250-350 bytes:
  - header of about 60 bytes,
  - policy line of 84 bytes,
  - `Reply:` line of about 60 bytes,
  - `Thread:` line of about 25 bytes.

  On top of that comes the body, up to 4096 bytes. INFERRED estimate from `src/wake.ts:131-171` and `src/i18n.ts:315-327,512`. The model can act without a round trip because the body is inline. A reply is one `ocs send` tool call.
- **Per message, busy target with steer**: the same note is appended to the running turn, with no extra turn. INFERRED.
- **Per message, busy target with backlog**: N messages produce one extra turn carrying the latest body. The header asks the agent to read the thread first, which costs one `ocs read` tool call plus the full unread history in its output. INFERRED.
- **Per quiet turn: zero.** There are no hooks, no turn-start digest and no polling inside the model. VERIFIED (no hooks).
- **Static**: the skill is 12,748 bytes (`skills/ocs/SKILL.md`). Codex shows skill name and description per session and loads the body on demand (primer §8, INFERRED for Codex's loading policy).
- Every wake wakes. ocs has no priority or threshold model. Only `--no-wake` keeps a message silent (`docs/wake-protocol.md:93-95`). VERIFIED.

## 6. Limitations and bugs visible in the source

1. **Provenance loss on the terminal path.** Queue and steer deliver another agent's text as a plain user message with no wrapper tag. The header `[ocs wake] X mentioned you` is the only attribution, and the author calls this a compromise (`src/deliver.ts:141-150`, `:227-228`). VERIFIED. Herdr-threads has the same property for any typed text, but it delivers bodies as hook `additionalContext` (developer context), not as user input.
2. **Steer requires the shared daemon**, which conflicts with herdr-threads' `--no-daemon` advice. See §3.7.1.
3. **The interrupted-turn hole.** See §3.7.2. The sender gets `queued` (exit 0) for a wake that will not fire.
4. **A merged wake shows one body.** The earlier messages need `ocs read`, so a merged wake is not self-contained. VERIFIED.
5. **The per-channel backlog** can still cause several post-turn turns. See §3.7.5.
6. **`--peek` still triggers the reminder.** A peek does not advance the cursor, so peeked messages still trigger the merged reminder. This is a deliberate choice: "prefer one extra reminder over swallowing a message" (`docs/wake-protocol.md:122`). VERIFIED.
7. **The cmux fallback has no draft protection**, and its `to interrupt` busy check matches screen text. VERIFIED (`src/roster.ts:262`).
8. **The skill path ignores `CODEX_HOME`** (`src/cli.ts:1679`). A Codex with a non-default home, such as an `aisw` profile, does not see the skill, while the runtime paths do honour `CODEX_HOME` (`src/codex-steer.ts:36-46`, `src/codex-sessions.ts:51-59`). VERIFIED inconsistency.
9. **Per-send cost** is spawns: ps, lsof and `codex queue` (a Codex CLI process start that connects to the store). With many recipients a send costs seconds, not milliseconds. INFERRED from the timeouts and the author's measured figures.
10. **Codex is not covered by `notify-when-idle`.** `src/idle.ts` has no Codex reference, so ocs cannot tell a sender when a Codex target goes idle. VERIFIED (grep, 0 hits).
11. **No offline catch-up.** A Codex that restarts later gets no wake for what it missed. It must run `ocs inbox` on its own (`README.md:349-351`). VERIFIED.

## 7. Clever ideas worth stealing

1. **`turn/steer` with `expectedTurnId` taken from the rollout tail.** The busy Codex gets the message inside its current turn through the harness's own steer path. It needs no hook, no tool call and no keystroke, and the host's precondition prevents a stale-turn misfire. For herdr-threads this covers busy Codex sessions that never call Bash, which our `PreToolUse ^Bash$` digest misses. The cost is the shared-daemon requirement. A hybrid is possible: steer when the thread is daemon-hosted, hooks otherwise. INFERRED applicability.
2. **Merge the backlog before it reaches the host queue (#41).** The author found that Codex replays every queued input as its own turn after the current one ("12 → 129" in the report, `CHANGELOG.md:86`). The rule is to gate before enqueueing, because "once written to the host queue it cannot be withdrawn" (`src/codex-defer.ts:7`). If herdr-threads ever adopts `codex queue` or `thread/queue/add` for wakes, it must enqueue at most one content-free `wake up` per idle edge. Our wake text is already content-free (R18), so one queued item is enough.
3. **Prove liveness by the rollout fd holder, with an ancestry identity check.** This fails closed against `codex queue`'s "exit 0 even for a dead thread". The basename-exact ancestry check rejects indexers such as `codex-issue-runner` (issue #35, `src/codex-queue.ts:385-396`). Cheap and harness-native.
4. **Re-check the holder after the write.** `kill(pid,0)` after `codex queue` narrows the "target died during delivery" window and reports it as `unknown`, not `ok` (`src/codex-queue.ts:263-270`).
5. **A strict three-valued outcome (`ok` / `failed` / `unknown`) with "unknown never replays" on every rung,** and exit codes 0/2/3 for the sender. It is a clean contract for at-most-once wakes.
6. **Respect provenance per host.** ocs prefers the carrier that preserves a native "message from another agent" envelope even when a simpler path exists (`src/deliver.ts:141-145`). Codex's `send_message_to_thread` / `<codex_delegation>` tool-output shape shows that Codex has a native representation for cross-thread input. INFERRED: if an app-server-level equivalent appears, it would beat plain user text for rule (e) framing.
7. **Busy detection from the rollout without a connection.** Parsing the last `task_started` / `task_complete` / `turn_aborted` event works for `--no-daemon` TUIs, where no app-server socket is reachable. It could cross-check Herdr's `agent_status` for Codex. Its fragility (key order, fail-open) argues for using it as a second signal only.
8. **Socket trust checks.** Before handing input to a control socket, ocs resolves symlinks by hand and requires that the socket and its directory are owned by the uid and that the directory is not writable by others (`src/codex-steer.ts:52-71`). This is directly reusable if herdr-threads talks to the Codex daemon.
9. **A heartbeat-before-spawn singleton watcher.** One detached waiter per key, a 30 s stale heartbeat for takeover, batches taken out under a lock, and a JSONL decision history (`src/codex-defer.ts:141-161`, `:264-268`, `:358-367`). It is a small, crash-tolerant pattern for "deliver when the target goes idle" without a resident daemon.

## 8. Relevance to herdr-threads (INFERRED)

- **Idle wake.** ocs's `codex queue` path is the server-side, draft-safe alternative to our Herdr `agent.prompt` typing plus composer read. Its known holes are interrupted turns and the 10-20 s latency or reachability under `--no-daemon`. ocs falls into both. Our composer-`Empty` gate refuses wakes over a draft. The queue never clobbers a draft, but it starts a turn under a human who is typing.
- **Busy delivery.** ocs relies on steer and otherwise defers to the turn end. Herdr-threads delivers at turn start and at Bash tool boundaries through hooks. Neither covers a busy `--no-daemon` Codex that makes no Bash calls. A `PostToolUse` hook with matcher `*` or an async hook (primer §11.4) closes our gap without a daemon dependency.
- **Body inline vs digest.** ocs inlines bodies up to 4 KiB in the wake. Herdr-threads' Codex hook sends a digest and the model runs `inbox`, which costs an extra tool call. ocs's approach saves that round trip, but it puts peer text in the *user* role.

## Verification

Checked against the clone at `1a3cee23a` (`git describe` = `v0.9.4`, commit 2026-10-22; GitHub `pushed_at` 2026-10-22T07:20:12Z) and the Codex `rust-v0.163.0` sparse clone. Re-read every cited range in `src/deliver.ts`, `src/codex-queue.ts`, `src/codex-steer.ts`, `src/codex-defer.ts`, `src/codex-ipc.ts`, `src/wake.ts`, `src/i18n.ts`, `src/roster.ts`, `src/cli.ts`, `src/codex-sessions.ts`, `CHANGELOG.md`, `README.md`, `DESIGN.md`, `docs/wake-protocol.md`, `skills/ocs/SKILL.md` (12,748 bytes confirmed), and the Codex `turn_processor.rs`, `ext/queue/src/service.rs`, `session_queue_commands.rs`, `startup_preflight_tests.rs`. Grep confirmed: no hook event names or MCP in `src/`, 0 Codex hits in `src/idle.ts`, the skill path hard-codes `homedir()/.codex` (`src/cli.ts:1679`). The ladder order, steer gating, outcome classes, size limits, neutralization, backlog/watcher constants (2 s, 30 s, 6 h, 3 probes) and the IPC payload all match the source.

Changed:
- Channel log path is `~/.ocs/channels/<channel>.jsonl`, not `~/.ocs/<channel>.jsonl`.
- §3.7.1: the `--no-daemon` advice is herdr-threads' (`integrations/codex/README.md:236`), not Herdr's. The turn processor's `load_thread` → "thread not found" is now VERIFIED (`turn_processor.rs:377-392`, called at `:1044-1050`), replacing the apps/mcp processor proxy citations.
- Policy line is 84 bytes, not 85.
- Desktop IPC turn-id citation corrected to `src/codex-ipc.ts:362-369`; cmux liveness filter citation widened to `src/roster.ts:185-245`.
- Added the daemon fixture `app-server-daemon/src/lib.rs:1406-1408` for the socket name, and the steer "no turnId → unknown-outcome" case.
- §3.2: deferral also runs when the steer was never attempted.

Not re-checked: the local rollout key-order count (§3.1; reads real user Codex state) and primer-only claims (30-minute grace, 120 ms paste burst, §11.4 async hooks), which remain as cited.
