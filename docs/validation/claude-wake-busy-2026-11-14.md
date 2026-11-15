# Claude wake submitted while busy: investigation

Date: 2026-11-14. Branch `investigate/claude-wake-busy`. No product code changed.

Question (the user's): "wake prompt sometimes gets submitted when agent is already busy (ie wakeup
is not necessary)". Confirm or disprove.

## Verdict

**Partly confirmed.** It happens, rarely (5 of 1,679 mod wakes, 0.3%), and only in one shape: the
mod submits `wake up` in the instant between the engine accepting another prompt and that prompt's
`turn.start`. The wake then queues behind that turn, and its own turn finds an empty inbox. No wake
was submitted while the model was generating or a tool was in flight.

## Evidence

Source: Claude Code session transcripts (read-only), main chain only, 121 sessions that received at
least one mod wake, Claude Code 2.1.302 to 2.1.325, 2026-10-18 to 2026-11-15 UTC. The transcript
records each `$.prompt.submit('wake up')` as a `queue-operation` `enqueue`, and the start of its
turn as a user row with `origin.kind = plugin`. The gap between the two is how long the engine made
the wake wait. `claude-wake-busy-2026-11-14/classify_wakes.py` produces the rows;
`ctx.py` prints the transcript around one of them.

| Category | Count | Contract |
|---|---|---|
| Wake started its own turn within 1 s of the submit (engine idle) | 1,674 | ok |
| of these, within 0.5 s after a turn ended (mail arrived after the turn's last tool call) | 69 | ok |
| of these, while a subagent of the session was running, main loop idle | 120 | ok (see below) |
| Wake queued 37 to 131 s behind a turn that had just been accepted | 5 | violation |
| Wake submitted while generating or with a tool call in flight | 0 | |
| Wake with no hidden rows appended in the second before it (bare wake) | 0 | |
| Native typed `herdr-threads: wake up` (hooks-only stack) | 9, all idle | ok |

19 of the 1,674 have no `turn_duration` row before them (a `No response requested.` turn, or a
task-notification row that started no turn); the wake's turn started within 60 ms of the submit in
each, so the engine was idle.

Background work: a subagent or background shell running while the main loop is idle is not
"generating, or running a tool" in rule 4's sense. The session cannot receive mail in-turn, so the
wake is the only delivery. I count these as allowed. (Background shells could not be counted
reliably from the transcript.)

### The five queued wakes

| Session | Wake submitted (UTC) | Rival prompt | Rival row vs submit | Mail sent before append | Wake waited | Wake's own turn |
|---|---|---|---|---|---|---|
| `eb41f352` | 2026-10-18T08:00:41.629 | human, typed | 140 ms before | 63.9 s | 80.3 s | `inbox`, "The wake brought nothing" |
| `5544a549` | 2026-11-01T05:29:20.362 | human, queued during the turn | turn ended 10 ms before | 5.8 s | 68.3 s | `inbox`, "The wake-up carried nothing" |
| `8a03868a` | 2026-11-09T00:50:30.623 | human, typed | 24 ms before | 94.5 s | 74.1 s | `inbox`, "no new messages" |
| `5544a549` | 2026-11-11T06:25:25.395 | human, typed | 47 ms before | 54.3 s | 130.9 s | `inbox`, "Nothing new arrived" |
| `6c55826b` | 2026-11-12T21:05:36.002 | task-notification, queued | 14 ms before | 9.8 s | 37.3 s | `inbox`, "The inbox is empty" |

In all five the hidden rows were appended before the rival turn's first API request (56 to 108 ms
later), so the rival turn read the mail. The wake turn cost one model call and one `inbox` call for
nothing. Send times are `messages.decision_at` from the daemon's database, read-only.

## Mechanism

`busy()` in `integrations/claude/mod/hooks/register.js` is true only between a main `turn.start` and
its `turn.complete`. The engine holds a prompt before `turn.start` in two places the mod cannot see:

1. **Enter to `turn.start`** (3 cases). Waking mail arrives while the user has a draft in the
   composer; `pump()` holds it (`held draft`). The user presses Enter: the composer is empty, no
   turn is open yet, so the next `pump()` appends the rows and submits the wake. The mail waited 54
   to 95 s and was released within 140 ms of the user's prompt row. `tests/delivery.test.ts` "a
   draft that is cleared lets the submit go at the next tick" codifies exactly this. What runs
   `pump()` that quickly after Enter is inferred, not observed (the 1 s tick alone would not land
   within 140 ms three times; a watch line caused by the UserPromptSubmit hook's check-in is the
   likely trigger). There is no mod ledger in production to confirm.
2. **`turn.complete` to the next queued prompt** (2 cases). Mail arrives after the turn's last tool
   call. `onTurnComplete` calls `pump()` at once; a prompt already in the engine's queue (a message
   the user typed during the turn, or a task notification) has not reached `turn.start`, so the mod
   submits. The wake has `queuePriority: later` and runs after the queued prompt's turn.

`onTurnStart` already clears a pending wake ("any turn sees the appended rows"), but by then the
submit has been issued and cannot be withdrawn.

Adjacent, not counted: after 120 s of a continuous draft (`DRAFT_WAIT_MS`) the mod submits the wake
anyway. The session is idle, so it is not a violation, but the user sees a turn start while typing.

## Proposed fix (not applied)

Both parts are in `register.js`; neither adds model-visible text.

1. **Entering prompt counts as busy.** Add an `on('prompt.submit')` handler that, for a prompt whose
   origin is not this plugin, sets `S.turns.entering = now` before `next(e)` and clears it at the
   main `turn.start`, when `next(e)` resolves with `drop`, or after a bounded hold checked the way
   `assumedBusy` is. `busy()` includes it. The handler never calls `$.prompt.submit` (the engine
   rejects that, `submit-race-spike.md` t18). Closes window 1, and window 2 whenever the engine
   dequeues before the mod's final `idleOpen` check.
2. **Turn-boundary settle.** After a main `turn.complete`, the wake submit (not the append) waits
   `TURN_SETTLE_MS` (500 ms proposed) so a prompt the engine already holds enters first; its
   `turn.start` then clears the pending wake. Cost: up to 0.5 s more latency on the 4% of wakes that
   follow a turn end directly.

To verify before committing to part 1: that the engine dispatches `prompt.submit` for a composer
prompt before the UserPromptSubmit hooks run on current Claude Code (the 2.1.295 spike shows the
event and its order relative to `turn.start`, without such hooks). A short private probe mod settles
it; it needs a Claude credential.

### Tests

Unit, `integrations/claude/mod/tests/delivery.test.ts`:

- draft-held waking item, then a composer `prompt.submit`, then a tick with an empty box before
  `turn.start`: no submit; after `turn.start` the rows are delivered and no wake is pending;
- `turn.complete` with a waking item queued, then `prompt.submit` and `turn.start` of a queued
  prompt inside the settle window: one append, no submit;
- control: `turn.complete` with nothing queued: the wake is submitted once the settle elapses;
- a dropped `prompt.submit` (hook block) clears the entering state and the wake goes out;
- replace "a draft that is cleared lets the submit go at the next tick" with the cleared-by-deletion
  case (no `prompt.submit` event), which must still submit.

Live, opt-in `scripts/wake-e2e.py` cases for Claude:

- `enter-race/group`: idle recipient with a typed draft, p0 send (held), driver presses Enter;
  pass when the transcript has no `wake up` row after that turn and the turn read the item;
- `queued-prompt/group`: recipient in a hold turn, a second prompt typed during it, p0 sent after
  the last tool call; pass when no wake turn follows the queued prompt's turn.

## Not done

- No live reproduction: the transcripts carry exact rows for all five, and a live run needs a
  credential from the coordinator.
- The mod tests were not run (`claude plugin test`).
- Hooks-only Claude has 9 native wakes in the sample, all idle; too few to say more.
