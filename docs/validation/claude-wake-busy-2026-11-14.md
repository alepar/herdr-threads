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

Both parts are in `integrations/claude/mod/hooks/register.js`; neither adds model-visible text. In both,
only the wake submit waits. The append is immediate, so the other prompt's turn reads the rows in its
first request (as it did in all five incidents) and its `turn.start` clears the pending wake.

1. **No wake while a prompt is entering.** An `on('prompt.submit')` handler counts dispatches of other
   origins in flight (`S.entering`). While the count is above zero the wake is held (ledger `held`, reason
   `entering`). When `next(e)` settles: a started turn has already cleared the wake, a drop frees it, and
   a dispatch that settled with no turn open starts the settle below. A dispatch the engine abandons (Esc
   during a slow hook, a hung hook) is released by the tick after 120 s with an empty prompt box and no
   open turn, like the post-abort and predecessor holds. The handler only observes, and the pump it
   triggers runs from `$.clock.after(1)`: the engine rejects a `$.prompt.submit` attributed to a
   `prompt.submit` dispatch (`submit-race-spike.md` t11, t12, t18).
2. **Turn-boundary settle.** For `TURN_SETTLE_MS` (500 ms) after a main `turn.complete` the wake is held
   (reason `settle`), so a queued prompt's `turn.start` comes first. `$.clock.after` re-runs the pump when
   the settle ends; the 1 s tick is the fallback. Cost: 0.5 s more latency on the 4% of wakes that follow
   a turn end directly.

The entering count and the settle live in memory only (a mod reload starts them at zero).

The first version of this fix (89389553c) also held the append while a prompt was entering, and had no
bound on the entering state. Review found both: a text-only turn then never saw the item and a wake
followed it anyway, and an abandoned dispatch would have blocked every later wake. 09d3cf7f0 corrects them.

### Tests

Unit, `integrations/claude/mod/tests/delivery.test.ts`:

- a draft-held item is appended when the prompt enters; a text-only turn follows; no wake, ACK via append;
- an item arriving while a prompt is entering is appended at once, and no wake follows its turn;
- an abandoned dispatch releases the wake after 120 s, not while a draft is in the box, and its late
  settlement changes nothing;
- a prompt queued behind the turn starts first: one append, no submit;
- control: nothing queued, the wake is submitted when the settle elapses (not at 499 ms);
- a dropped prompt frees the wake, submitted from the timer and not inside the dispatch;
- a prompt that enters without starting a turn releases the wake after the settle;
- the mod's own wake prompt entering is not a rival;
- "a draft that is cleared ..." is renamed to the deleted-draft case, which still submits at the next tick.

The test harness's timers now fire only when the test clock reaches them, so twelve older tests state the
settle explicitly (`h.elapse(500)`).

`tests/stress.test.ts` models both gates (steps `prompt.in`, `prompt.out`, the 120 s release) and fails on
a submit while a prompt is entering or inside the settle. At the end of each schedule it clears every hold
and fails if a wake is still pending or a waking item was never delivered. The settle lowers the submit
rate per schedule, so the schedule count went from 800 to 2400 to keep the coverage floors.

Live, opt-in `scripts/wake-e2e.py --harness claude --cases enter-race/group,queued-prompt/group` (mod
Claude at T = 0). Both count `wake up` rows in the engine queue from the recipient's transcript. They run
alone: their 2 s UserPromptSubmit hook sits in settings every Claude recipient of the run shares, so a
`--cases` list that mixes them with other cases is refused.

- `enter-race/group`: an ACK request is held by a typed draft; the driver presses Enter; the hook keeps
  the prompt entering. The prompt asks for a text-only turn that names every item received. Pass: no wake
  queued or on screen, the item named in that turn, ACK `via: append`.
- `queued-prompt/group`: an ACK request and a follow-up prompt arrive while the turn writes its final
  answer. Pass: no wake queued or on screen, ACK `via: append`, the follow-up's turn shows the item.

### Live results (09d3cf7f0, Claude Code 2.1.325, Herdr 0.9.5, recipient `cl-t0`)

| Binary | Case | Wakes queued | Wake on screen | Item shown | Result |
|---|---|---|---|---|---|
| control (same tree, `register.js` from c60fc2a09) | enter-race/group | 1 | 1 | yes | FAIL |
| control | queued-prompt/group | 1 | 1 | yes | FAIL |
| fix | enter-race/group | 0 | 0 | yes | PASS |
| fix | queued-prompt/group | 0 | 0 | yes | PASS |

Each case ran once per binary. The same result held for 89389553c against its own control.

Also run on 09d3cf7f0:

- full Rust suite, unfiltered (`cargo nextest run --locked --all-targets --all-features -j 4`): 9590 passed,
  64 skipped, 951 s at the 4-thread cap; `scripts/check-no-leaked-processes --run-id`: no leaked processes
  or temp files;
- `scripts/test-claude-mod`: 189 pass.

Run on 89389553c and not repeated (the later commit changes only the mod, its tests and the e2e driver's
prompt-race cases): clippy with `-D warnings`, `scripts/check-harness-neutral`, `scripts/check-wire-compat`,
`scripts/wake-e2e.py --self-test` (238 pass).

## Not done

- The default wake e2e matrix was not run; it runs in the next release qualification.
- What runs the pump within 140 ms of Enter in production is still inferred. The fix does not depend on it.
- Hooks-only Claude has 9 native wakes in the sample, all idle; too few to say more.
