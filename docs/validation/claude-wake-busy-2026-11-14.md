# Claude wake submitted while busy: investigation

Date: 2026-11-14. Branch `investigate/claude-wake-busy`. The investigation commit changed no product code;
the fix and its tests follow it on the same branch.

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

## Probe: event order on Claude Code 2.1.325

A throwaway mod logging `prompt.submit` (before and after `next(e)`), `turn.start` and `turn.complete`,
in a private tmux server with a scratch config and a UserPromptSubmit hook that sleeps 1.5 s. Times in ms
from the Enter keypress.

| Scenario | Observed |
|---|---|
| Prompt typed while idle | `prompt.submit` in +35 (origin `composer`, composer already empty), hook start +39, hook end +1541, `turn.start` +1550, `prompt.submit` settles +1558 |
| Prompt typed during a tool call | `prompt.submit` in +11 with the open `turnId`, settles when the hook ends; the prompt is absorbed into the running turn, no `turn.start` |
| Prompt typed during the final answer | `prompt.submit` in and out at Enter; no second dispatch later; `turn.complete`, then the queued prompt's `turn.start` 19 ms after it |
| Local slash command | no `prompt.submit` |

So `prompt.submit` precedes the turn hooks and brackets window 1 exactly. Window 2 has no event between
`turn.complete` and the queued prompt's `turn.start`; only a short wait closes it.

## Fix (applied on this branch)

Both parts are in `integrations/claude/mod/hooks/register.js`; neither adds model-visible text.

1. **Entering prompt counts as busy.** An `on('prompt.submit')` handler counts dispatches of other origins
   in flight (`S.entering`); `busy()` includes it. It ends when `next(e)` settles: by then a started turn
   is in `turns.open`, a drop frees the wake at once, and a dispatch that settled with no turn open starts
   the settle below. The handler only observes; it never calls `$.prompt.submit` (the engine rejects that,
   `submit-race-spike.md` t18). Nothing is appended while a prompt is entering, so the item goes to that
   turn's first tool boundary, or to the idle path when the turn ends.
2. **Turn-boundary settle.** For `TURN_SETTLE_MS` (500 ms) after a main `turn.complete`, the wake submit
   waits (ledger `held`, reason `settle`); the append is not delayed. A queued prompt's `turn.start`
   clears the pending wake, as before, and its turn reads the appended rows. `$.clock.after` re-runs the
   pump when the settle ends; the 1 s tick is the fallback. Cost: 0.5 s more latency on the 4% of wakes
   that follow a turn end directly.

The entering count and the settle live in memory only (a mod reload starts them at zero).

### Tests

Unit, `integrations/claude/mod/tests/delivery.test.ts` (each failed before the change):

- a prompt entering holds the wake until its turn starts, and that turn's tool boundary carries the item;
- a prompt queued behind the turn starts first: one append, no submit;
- control: nothing queued, the wake is submitted when the settle elapses (not at 499 ms);
- a dropped prompt clears the entering state and the wake goes out at once;
- a prompt that enters without starting a turn releases the wake after the settle;
- the mod's own wake prompt entering is not a rival;
- "a draft that is cleared ..." is renamed to the deleted-draft case, which still submits at the next tick.

`tests/stress.test.ts` models both gates (steps `prompt.in`, `prompt.out`) and fails on a submit while a
prompt is entering or inside the settle. The settle lowers the submit rate per schedule, so the schedule
count went from 800 to 2400 to keep the coverage floors (1.4 s).

Live, opt-in `scripts/wake-e2e.py --harness claude --cases enter-race/group,queued-prompt/group` (mod
Claude at T = 0). Both count `wake up` rows in the engine queue from the recipient's transcript.

- `enter-race/group`: an ACK request is held by a typed draft; the driver presses Enter; a 2 s
  UserPromptSubmit hook, added to the scratch settings for these cases only, keeps the prompt entering.
  Pass: no wake queued or on screen, the item ACKed `via: context` in that turn.
- `queued-prompt/group`: an ACK request and a follow-up prompt arrive while the turn writes its final
  answer. Pass: no wake queued or on screen, the item ACKed `via: append`, the follow-up's turn shows it.
- A run whose `via` is the other one did not stage the window and fails (`window_staged`).

### Live results (Claude Code 2.1.325, Herdr 0.9.5, recipient `cl-t0`)

| Binary | Case | Wakes queued | Wake on screen | ACK via | Result |
|---|---|---|---|---|---|
| control (same tree, `register.js` from the investigation commit) | enter-race/group | 1 | 1 | append | FAIL |
| control | queued-prompt/group | 1 | 1 | append | FAIL |
| fix | enter-race/group | 0 | 0 | context | PASS |
| fix | queued-prompt/group | 0 | 0 | append | PASS |

The control reproduces both windows live. In the control's enter-race the mod appended and woke during
the 2 s hook, hence `via: append`.

Also run on the fix: `scripts/test-claude-mod` (187 pass), `scripts/wake-e2e.py --self-test` (238 pass),
clippy with `-D warnings`, `scripts/check-harness-neutral`, `scripts/check-wire-compat`,
`cargo test claude_mod` (26 pass) and `cargo test mod_delivery` (68 pass).

## Not done

- The full Rust suite and the default wake e2e matrix were not run; both are due before a merge.
- Each live case ran once per binary.
- What runs the pump within 140 ms of Enter in production is still inferred. The fix does not depend on it.
- Hooks-only Claude has 9 native wakes in the sample, all idle; too few to say more.
