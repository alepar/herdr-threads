# open-cross-session (ocs) x Antigravity CLI (agy)

- Upstream: https://github.com/leeguooooo/open-cross-session
- Clone read: `scratchpad/alts/open-cross-session-ocs`, HEAD `1a3cee23a` (= `v0.9.4`). Not executed. All paths are relative to the clone root.
- Harness primer: `../../primers/agy.md`. Overview: `./overview.md`.
- Tags: VERIFIED = read in ocs source/docs (or the agy docs or herdr-threads fixtures the primer cites). INFERRED = my reasoning, not observed. unknown = no source answers it.

## Support status

**Generic: ocs never delivers into a live agy session.** It ships an opt-in *headless receiver*. The receiver is a long-lived ocs worker process. For every message addressed to `agy`, it re-launches `agy -p <prompt> --output-format json [--conversation <id>]` in a configured workspace and posts the final answer back to the channel. VERIFIED: `docs/agy.md:3-6` ("It starts a dedicated headless conversation, rather than injecting into an existing TUI"), `src/agy-worker.ts:131-149`, `DESIGN.md:31`, `README.md:207-221`. It was added in 0.8.3: `CHANGELOG.md:45-48`.

What this means in practice:
- `agy` is a **single reserved address per `OCS_HOME`** (`docs/agy.md:40`, `src/names.ts:34`, `src/agy-worker.ts:257-261`). It is not a per-session address. A user's interactive agy TUIs are invisible to ocs: there is no `agy-<id>` roster entry and no agy session discovery (VERIFIED: `src/roster.ts:333,351-352`; the only agy roster entry is the worker).
- The receiver is a **delegate** (task in, result out), not a peer seat in a conversation the human is driving. One agy conversation is kept per ocs channel (`docs/agy.md:41-43`).
- The rest of this report covers that receiver. The last part of "Wake mechanism" covers the one generic path that could reach a live agy TUI (the cmux fallback).

## Injection mechanism

### Path, end to end (VERIFIED)

1. **Sender side.** `ocs dm agy "…"` or an `@agy` mention in `ocs send` (`src/cli.ts:631-635`) commits the message to the channel log. Then `deliverDmImpl` takes the `resolved.kind === "agy"` branch (`src/deliver.ts:425-447`):
   - It refuses a self-wake (`OCS_NAME === "agy"` or `from === "agy"`, `:432-436`).
   - It builds the standard ocs wake note with `receiver:"agy", implicitReceiver:true` (`:440`).
   - It calls `wakeAgy({channel, seq, note, replyTarget?})`.
2. **Transport.** `requestAgy` (`src/agy.ts:99-149`):
   - It finds the worker through `~/.ocs/agy/worker.json`. That record must be owner-only, non-symlink, have a heartbeat newer than 15 s, a live pid and a matching socket path (`src/agy.ts:32-56`).
   - It writes one JSON line `{channel, seq, note, replyTarget?, token}` to a per-runtime UDS `~/.ocs/agy/worker-<token16>.sock` (Windows: a named pipe whose owner SID and server pid are verified on the same handle, `src/agy.ts:83-97`).
   - It waits up to 4 s for `{ok:true, job}` (`AGY_WAKE_TIMEOUT_MS`, `src/agy.ts:10`).
3. **Worker accept.** The worker's server (`src/agy-worker.ts:172-203`) checks the 64-hex token with `timingSafeEqual` (`:187-188`) and validates the job (`validAgyJob`, `src/agy.ts:63-71`). `AgyQueue.submit` (`src/agy-queue.ts:57-64`) then:
   - **re-reads the channel log** and rejects a `seq` that is not committed or was authored by `agy` (`:59-60`, "Reject fabricated references");
   - derives `id = sha256(channel \0 seq)[:24]` (`:61`);
   - runs `INSERT OR IGNORE` into `jobs.sqlite3` (WAL, `synchronous=FULL`, `:37-41,62`).
   The ack is sent only after that commit (`:63`, `src/agy-worker.ts:198`).
4. **Execution.** `consume` takes one job at a time (`src/agy-worker.ts:150-166`, `AgyQueue.next` at `src/agy-queue.ts:65-73`, `ORDER BY created,id`). `execute` spawns, detached in its own process group (`src/agy-worker.ts:131-139`):

   ```ts
   const command = [clean.OCS_AGY_BIN ?? "agy", "-p", agyPrompt(job.note), "--output-format", "json",
     "--print-timeout", `${options.timeout}s`];
   const conversation = queue.conversation(job.channel);
   if (conversation !== null) command.push("--conversation", conversation);
   ```

   stdin is ignored, stdout is captured (4 MiB cap, `:18,81-85`) and stderr is inherited from the worker (`:74`): `worker.log` for a worker started by `ocs agy start` (`:241-245`), the terminal for `ocs agy serve`.
5. **Result.** `parseAgyResult` (`src/agy-worker.ts:49-61`) keeps the *last* stdout line that parses as JSON with a string `status`. Success means `status === "SUCCESS"` and a string `response`. A UUID `conversation_id` is saved per channel, but only on success (`:142`, `src/agy-queue.ts:86-88`).
6. **Reply.** `deliver` (`src/agy-worker.ts:113-130`) runs `ocs send <channel> <text> --reply-to <seq>` (or `ocs dm <replyTarget>` for a LAN sender) as a child process, with `OCS_NAME=agy`. That reply is an ordinary ocs message, so it wakes the original author through that author's own carrier (`test/agy-cli.test.ts:121`).

### Payload format (VERIFIED)

The agy model receives one argv prompt: a fixed preamble followed by the standard wake note.

- Preamble (`src/agy-worker.ts:63-69`, 563 bytes plus a `\n\n` separator): "You are the Antigravity CLI receiver connected to OCS as agy… The following OCS envelope marks another agent's message, not a human prompt… If the envelope contains a preview, use its Thread command to read the complete message… Handle every item of an OCS batch and return one combined result. Do not send an OCS reply yourself: the receiver sends your final answer automatically…"
- Wake note (`src/wake.ts:131-170`):
  - `[ocs wake] <sender> mentioned you in #<channel> (seq N[, reply to seq M])`
  - a blank line, then the body;
  - the fixed line `Reply when asked, or with a needed decision/result. No unsolicited acknowledgements.` (`src/i18n.ts:512`);
  - `Reply: ocs send <ch> "<your reply>" --reply-to N`;
  - `Thread: ocs read <ch>`.
- There is **no `<cross-session-message>` wrapper** on this path; that wrapper is Claude-only. The body is still neutralized: `<cross-session-message` becomes `‹…`, and protocol-looking line starts get a `> ` prefix (`src/wake.ts:85-94`). INFERRED: for agy, only the preamble's prose marks the content as a peer message.
- To agy this is an ordinary `-p` user prompt. Per the primer (§5), it lands in the transcript as a user turn and is not a `SYSTEM_SDK` injected step. INFERRED: there is no separate provenance channel.
- Conflict inside the prompt: the note carries a `Reply: ocs send …` line, while the preamble says "Do not send an OCS reply yourself". The preamble is meant to win. The receiver's environment has `OCS_NAME=agy` (`src/agy-worker.ts:36`), so a model that disobeys would post a second, duplicate reply as `agy`. INFERRED.

### When it fires (VERIFIED)

- At send time only. The worker kicks the drain loop right after each accepted job (`src/agy-worker.ts:199`) and again at startup (`:216`).
- There is no turn-start, mid-turn or post-tool delivery, because agy runs one headless turn per job. Nothing reaches a running agy turn: a second job waits until the first process exits (`docs/agy.md:42-43`; test "mentions queue behind an active turn and only start a second turn after it ends", `test/agy-cli.test.ts:100-108`).
- ocs registers no agy hooks (`SessionStart`, `PreInvocation`, and so on), no plugin and no MCP server. VERIFIED: grep finds no `hooks.json` or `~/.gemini` writes in `src/`. The only agy integration files are `src/agy*.ts`.

### Size bounds (VERIFIED)

| Bound | Value | Where |
|---|---|---|
| Body inlined verbatim | ≤ 4096 B, else the first 512 B plus `… (N bytes total; full text: ocs read <ch>)` | `src/wake.ts:40-46,147-150` |
| Whole note | ≤ 5120 B (`AGY_NOTE_MAX_BYTES`) | `src/agy.ts:12`, `src/agy.ts:68` |
| Wire frame (after JSON escaping) | ≤ 16 KiB | `src/agy.ts:13,103`, test `test/agy.test.ts:137` |
| agy stdout | 4 MiB, then the process group is killed ("output-limit") | `src/agy-worker.ts:18,81-85` |
| Turn time | `--timeout` 1..21600 s, default 3600; passed as `--print-timeout` plus a 5 s hard kill | `src/agy-worker.ts:40-47,132-138` |
| Reply posted back | 90 000 B (Windows 12 000); above that, a preview of 80 000 B (10 000) plus `Full result: <path>` written mode 0600 | `src/agy-worker.ts:116-121` |
| Error text echoed | 1500 chars, URLs redacted | `src/agy-worker.ts:143-147` |

A truncated body depends on the agy model running `ocs read <ch>` through `run_command`. `docs/agy.md:49-51` states that "headless tools that cannot obtain approval may be denied", so in any permission mode that requires review the receiver may act on a 512-byte preview only (which mode headless agy uses by default is unknown). INFERRED; ocs does not add an allow rule (`docs/agy.md:28-29`: "never… changes agy permissions automatically").

### Dedupe and acknowledgement (VERIFIED)

- **Idempotent wake.** The job id is a hash of channel and seq, inserted with `INSERT OR IGNORE`, so a repeated wake for the same message reuses the job and never re-runs it (`src/agy-queue.ts:61-63`; `docs/agy.md:56-57`; test "duplicate suppression", `test/agy-cli.test.ts:74`).
- **Receiver ack** = "committed to the local execution queue". It is not "read" or "done" (`docs/agy.md:48-49`). The sender prints `wake(agy): accepted into durable task queue (<job>); completion is reported separately` (`src/i18n-agy.ts:7`).
- **Unknown outcome.** If the frame was written but no ack arrived within 4 s, the result is `unknown-outcome` and ocs exits 3. It is never replayed (`src/agy.ts:116-117`, test `test/agy.test.ts:96`).
- **Completion** is reported as the reply message itself. `capabilities.completionReceipt` is always `false` (`src/capabilities.ts:9-14`), but in practice the auto-reply is the completion signal.
- **At-most-once execution, at-most-once reply** (an interrupted job is never retried, so it is not exactly-once).
  - `recover()` at startup marks `running` jobs `interrupted`, posts the interruption notice instead of re-executing, and marks `sending` replies `unknown` without resending (`src/agy-queue.ts:51-56`, `docs/agy.md:59-61`).
  - `beginReply` is a compare-and-set from `pending` to `sending` (`src/agy-queue.ts:77-79`).
  - Exit codes 2/3 from the reply child are recorded as `unknown` and never retried (`src/agy-worker.ts:127-129`).
- **Batching** is explicit only: `ocs dm agy --batch tasks.json` becomes one message, one agy invocation and one result (`CHANGELOG.md:38`, `test/agy-cli.test.ts:131`). Messages that arrive while a job runs are *not* coalesced; each one is its own job and its own `agy -p` run. INFERRED from `consume` (`src/agy-worker.ts:150-166`), which takes jobs one by one.

## Wake mechanism

### Exact mechanism (VERIFIED)

"Wake" here means a **fresh process launch**. No idle agy session is involved:
- the first message on a channel runs `agy -p` with no `--conversation`;
- later messages on that channel run `agy -p --conversation <saved uuid>`, which resumes that channel's private conversation (`src/agy-worker.ts:134-135`; test "per-channel resume", `test/agy-cli.test.ts:74`).

The worker must already be running: `ocs agy start --cwd <ws>` spawns a detached `ocs _agy-worker` (`src/agy-worker.ts:236-255`), or `ocs agy serve` runs it in the foreground for a supervisor (`docs/agy.md:27-28`).

### Idle/busy detection (VERIFIED)

- The worker publishes `status: "idle" | "working"` with a 2 s heartbeat in `worker.json` (`src/agy-worker.ts:111,154,165,214`), and `ocs who` shows it.
- Busy is handled by **serial queueing**: `busyDelivery: "serial"` (`src/capabilities.ts:10`). A job waits behind the running one, and nothing interrupts a turn.
- There is no notion of a human at the agy session, because there is no TUI.

### Draft safety

Not applicable: no keystrokes, no composer. The receiver's conversations belong to the worker. If a human opened the same conversation id in a TUI (`agy --conversation <id>`), the two processes would share one conversation store concurrently; ocs neither guards nor documents this. INFERRED risk; agy's behaviour under concurrent writers is unknown.

### Latency (INFERRED)

- The ack is under 4 s.
- Execution start is immediate when the worker is idle. When it is busy, a job waits for every earlier job on **any** channel, because there is one global serial queue (`ORDER BY created,id`, `src/agy-queue.ts:68`), up to the 1 h default timeout each.
- Each job pays the full agy process start-up, login-cache load and conversation resume before the first model call. Not measured.

### Failure modes (VERIFIED unless marked)

- **Worker down.** `wake(agy): unavailable … message remains in the channel` (`src/i18n-agy.ts:8`), and the sender exits 2. **There is no catch-up**: "starting the receiver does not automatically execute messages it never acknowledged" (`docs/agy.md:65-67`).
- **Worker crash mid-turn.** The job is reported as interrupted ("partial effects are unknown. No automatic retry was performed.", `src/i18n-agy.ts:15`). The agy child is in its own process group and is killed on shutdown (`src/agy-worker.ts:21-29,206`).
- **Timeout or output cap.** The process group gets SIGKILL and a failure reply is posted (`src/agy-worker.ts:79-80,143`).
- **Malformed output.** `agy returned no valid final JSON result` (`src/i18n-agy.ts:17`).
- **Untested result shape.** The expected shape (`status`, `response`, `conversation_id`) is tested only against a fake agy (`test/fixtures/agy-cli.ts:7-8`). Whether genuine `agy -p --output-format json` emits exactly these fields is not established by ocs's tests, and the primer (§5) documents stream-json events but not the plain json shape. unknown.
- **Name collision.** If a Claude or other session is already named `agy`, the receiver refuses to start, and a send to `agy` fails as ambiguous (`src/agy-worker.ts:257-261`, `src/deliver.ts:427-431`, test `test/agy.test.ts:123`).

### Generic path to a live agy TUI: cmux keystrokes (VERIFIED code, INFERRED applicability)

- `ocs dm surface:<n> "…"` addresses any cmux terminal surface (`src/roster.ts:689-690`; `README.md:244`). So a user running interactive agy inside cmux *could* be reached this way.
- `wakeCmuxSurface` (`src/roster.ts:251-275`) reads the last 8 screen lines and treats the target as busy only if the text contains `to interrupt` (`:263`). It then runs `cmux send <note>` and `send-key enter`.
- Against agy this check is wrong. agy's busy footer reads `esc to cancel` (primer §8, `tests/fixtures/agy/composer/busy.read-detection.txt` in herdr-threads), so ocs would type a 5 KB note plus Enter into:
  - a **running** agy turn;
  - a **permission menu**, where Enter confirms the highlighted option and could approve a command (primer §8);
  - a composer holding a human draft, which the note would be appended to and submitted with.
  INFERRED. The multi-line note's newlines would also go through `cmux send`; whether agy treats a pasted `\n` as submit or as a newline depends on bracketed paste support, which is unknown (primer §8).
- ocs does not document cmux for agy. This is a hazard rather than a feature.

## Setup and installation footprint on the harness (VERIFIED)

- **Nothing is written to agy's config**: no `~/.gemini/config/hooks.json`, no plugin, no `mcp_config.json`, no permission rules (`docs/agy.md:28-29`). The user must log in to agy interactively once (`docs/agy.md:14-17`).
- ocs state lives under `$OCS_HOME/agy/` (mode 0700): `worker.json`, `worker-<token16>.sock`, `jobs.sqlite3` (tables `lease`, `jobs`, `conversations`), `worker.log` and `<job>.txt` for long replies (`docs/agy.md:54-57`, `src/agy-queue.ts:17-24,37-41`, `src/agy-worker.ts:91-96,118`).
- Environment scrubbing: the worker removes the Claude, Codex, Pi, Grok and Hermes session variables before spawning agy, so the receiver does not impersonate the agent that started it (`src/agy-worker.ts:31-38`). **It does not remove `HERDR_*`, nor agy's own `ANTIGRAVITY_CONVERSATION_ID`** (no `ANTIGRAVITY`/`HERDR` string anywhere in `src/`), so a worker started from inside an agy session passes that conversation id to every headless child. INFERRED interaction: if herdr-threads' global agy plugin (`~/.gemini/config/plugins/herdr-threads/`) is installed and the worker was started from a Herdr pane, every headless `agy -p` run would fire herdr-threads' `SessionStart`/`PreInvocation` hooks with that pane's `HERDR_PANE_ID`. Whether our witness classifies such a run as top-level depends on whether headless runs create a `presence/<id>.lock`, which is unknown.
- agy's headless mode reportedly loads no workspace `.agents/` config without `--add-dir` (primer §5, third-party). ocs passes only `cwd`, so workspace hooks, rules and plugins probably do not apply to the receiver. INFERRED.

## Token cost

- **Per message.** One full agy headless turn:
  - a 563-byte preamble, plus the note (header about 80 B, body up to 4096 B, about 200 B of reply-policy, Reply and Thread lines);
  - **plus agy's own system prompt and the whole resumed channel conversation**, re-read at every job, because each job is a new process resuming `--conversation <id>`. Whether agy's provider-side prompt caching softens this across process restarts is unknown.
  - For a busy channel this grows without bound until agy compacts. ocs never starts a fresh conversation on its own; the mapping lives until `jobs.sqlite3` is deleted (`src/agy-queue.ts:83-88`). INFERRED.
  - The reply then costs the *sender* a wake note (≤ 5120 B, or a preview plus a file path, `src/agy-worker.ts:116-121`).
- **Per quiet turn: zero.** There are no hooks and no polling inside agy. The worker's 2 s heartbeat is a file write, not a model call (`src/agy-worker.ts:214`).
- Every `@agy` mention runs a full turn. ocs has no priority or threshold model (overview), so cheap FYI messages still cost a whole agy run unless the sender passes `--no-wake`. INFERRED.

## Limitations and bugs I can see

1. **No live-session integration.** A human's interactive agy cannot take part. agy is a single shared remote worker, not one seat per session (VERIFIED, `docs/agy.md:6`).
2. **One global serial queue** across all channels. A one-hour job on one channel blocks every other channel's agy task (INFERRED from `src/agy-queue.ts:65-73`).
3. **One workspace per receiver** (`--cwd`). The model is told to "use any workspace explicitly specified by the requester" (`src/agy-worker.ts:66`), so cross-project work relies on the model `cd`-ing into the path the requester names (VERIFIED prompt; INFERRED effect).
4. **Permission dead-ends.** Headless `-p` in a review-requiring mode (e.g. `request-review`; whether it is the headless default is unknown) cannot obtain approval for `run_command`, so the receiver may be unable to run the very `ocs read` needed for a truncated body, or the project's tests. ocs documents this only as a diagnostic in `worker.log` (`docs/agy.md:49-52`). VERIFIED documented, INFERRED impact.
5. **Offline messages are never caught up** (`docs/agy.md:65-67`). Persistence without redelivery means the sender must notice exit 2 and resend after `ocs agy start`.
6. **Contradictory reply instructions** in one prompt (preamble "do not reply" versus note `Reply:` line). See Payload. INFERRED risk of duplicate replies.
7. **The conversation id is saved only on success** (`src/agy-worker.ts:141-142`). A failed or timed-out first job leaves no mapping, so the next job starts a new conversation and the context of the failed attempt is lost (INFERRED). A failed *later* job keeps the old id, so whatever agy persisted of its partial turn is resumed silently by the next job (INFERRED).
8. **Fake-only verification** of the agy JSON result shape (`test/fixtures/agy-cli.ts`). Nothing in the repo shows a genuine agy capture. unknown whether `conversation_id`/`status:"SUCCESS"` match agy ≥ 1.3.
9. **Argv prompt.** The whole note is a command-line argument. That is fine at 5 KB on Unix. Windows needed the long-reply file workaround, and that applies to the reply argv, not the prompt (`docs/agy.md:63-65`). VERIFIED.
10. **cmux fallback is unsafe for agy** (see the cmux section above): the busy pattern is wrong, and there is no permission-menu or draft check.

## Clever ideas worth stealing

- **Job id = hash(channel, seq) with `INSERT OR IGNORE`, and submit-time validation against the log** (`src/agy-queue.ts:57-64`). A wake is idempotent by construction, and a forged or stale reference is rejected before any model call. herdr-threads' delivery ledger already de-duplicates offers. The point to borrow is the cheap rule "an executor job keyed on the message id is never run twice" for any future headless or delegate path.
- **An explicit crash-recovery split between "execution" and "reply"** (`state` vs `reply_state` with a `sending` CAS, `src/agy-queue.ts:37-41,51-56,77-82`). A crash never re-runs side effects and never double-posts, and the user gets an honest "partial effects unknown" notice. This is a clean model for herdr-threads' handoff and launch flows, where a restarted daemon must not repeat a spawn.
- **A delegate mode for agy at all.** Headless `agy -p --conversation <id>`, one conversation per thread and the answer auto-posted as a reply, gives a token-cheap "ask an agy" service with no TUI, no composer parsing and no wake keystrokes. herdr-threads could offer this as an *additional* seat kind (a headless responder bound to a thread) while keeping the hook-driven live seat. Its rough edges (permissions, global serial queue, unbounded conversation growth) are visible above. INFERRED.
- **Rendezvous hygiene.** The `worker.json` record is trusted only when owner-only, non-symlink, a heartbeat fresher than 15 s, a live pid and a socket path derived from the token all hold, and the per-runtime 64-hex token is compared with `timingSafeEqual` (`src/agy.ts:32-56`, `src/agy-worker.ts:187-188`). This is a compact recipe for any local helper herdr-threads exposes beyond the daemon socket.
- **Environment scrubbing of foreign session identities** before spawning a harness child (`src/agy-worker.ts:31-38`). herdr-threads' launch path should make sure that a child harness never inherits a parent's `CLAUDE_CODE_SESSION_ID`, `CODEX_THREAD_ID`, `ANTIGRAVITY_CONVERSATION_ID` and the like, which would cause mis-attribution. ocs's own list omits `ANTIGRAVITY_CONVERSATION_ID`, so copy the idea, not the list. Worth an audit against TRUST-POLICY. INFERRED.
- What *not* to steal: cmux's `to interrupt`-only busy test. herdr-threads' `AgyComposer` already refuses any non-empty, busy, permission, overlay or subagent screen (primer §10), and that is the right bar.

## Verification

Adversarial pass against the clone at `1a3cee23a` (v0.9.4) and the agy primer.

Checked and confirmed: every cited line range in `src/agy.ts`, `src/agy-queue.ts`, `src/agy-worker.ts`, `src/deliver.ts:425-449`, `src/cli.ts:631-635`, `src/wake.ts` (4096/512/5120 bounds, neutralizer `:85-94`, `wakeNote` `:131-170`), `src/i18n.ts:512`, `src/i18n-agy.ts`, `src/capabilities.ts`, `src/names.ts:34`, `src/roster.ts` (`:253-275` cmux busy test `to interrupt` at `:263`, roster `:333,351-352`, `surface:` `:689-690`), `docs/agy.md` line refs, `README.md:207-221,244`, `DESIGN.md:31`, `CHANGELOG.md:38,45-48`, the test names at `test/agy-cli.test.ts:74,100,121,131` and `test/agy.test.ts:96,123,137`, and the fake-only result fixture `test/fixtures/agy-cli.ts:7-8`. Confirmed no `hooks.json`/`.gemini` writes in `src/`, and that no herdr-threads agy composer fixture contains `to interrupt` (busy footer is `esc to cancel`, `busy.read-detection.txt:14`).

Changed:
- "Exactly-once execution" corrected to at-most-once (interrupted jobs are never retried, `src/agy-queue.ts:51-56`).
- stderr: inherited from the worker, so `worker.log` only for `ocs agy start`, not `serve` (`src/agy-worker.ts:74,241-245`).
- Preamble size clarified: 563 B text plus a 2-byte separator.
- "agy's default `request-review` mode": the default is not established by the primer or ocs; marked unknown.
- Env scrubbing: added that ocs also leaves `ANTIGRAVITY_CONVERSATION_ID` in place (no such string in `src/`).
- Resumed partial turn after a failed later job: marked INFERRED.
- "added recently" pinned to 0.8.3.
