# Live qualification (per-seat config and /side queue wake)

## Focused side-queue and root draft (ht-z41e.43)

Driver: `side-queue-focused/group` (private Herdr only) and `root-draft-queue/group` in `scripts/wake-e2e.py`,
verdicts self-tested (`python3 scripts/wake-e2e.py --self-test`, 350 tests, exit 0).
Binary under test: `bin/herdr-threads` built from the integration tip's Rust source (`cargo build --release` through
`heavy tb scripts/build.sh`), sha256 `2a096d766a6fe65b06a8e33f9c55b339d938283604360c751865742eb88d3ae1`.
Herdr: private 0.9.5 server (`scripts/wake-e2e-herdr 0.9.5`, sha256 from `tests/herdr-releases.tsv`
`14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d`). Codex: `codex-cli 0.172.0`, credential
`~/.aisw/profiles/codex/codex-2` (copied to a scratch CODEX_HOME). Evidence root:
`docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/`.

### Verdicts

| acceptance item | result |
|---|---|
| focused-pane side-queue: Accepted intent on the root, armed with no dwell, busy side, nothing typed into the side | FAIL (confirmed product defect, reproduced 4 times) |
| root draft refuses the queue and stays intact; Accepted wake after the clear | PASS |
| lost or submitted root draft, or queue wake into a busy root, recorded in report.md | none observed (`release_blocking` empty in the final rows) |
| evidence dirs recorded at one SHA | yes, see the manifest (the final rows run on driver commit `fc3612e3f`, code unchanged to `7c32531a0`) |

#### side-queue-focused/group: FAIL, no queue intent is ever prepared while a /side chat is displayed

Final row: `evidence/qualification/side-queue-focused/` (SHA `7c32531a0061cf89d009acdbfd70671fb0afd4b3`, tree
`46bc66d71499f2651f53d4b131e6ccb0657249f2`, started 2026-11-27T19:37:12Z, exit 1, rows 0/1, leak check clean).

- Setup held, read at the send: pane `focused` true; side binding turn not `ended` (busy, `sleep 100`); Herdr
  `agent_status` `working`; the screen capture in the row shows the side turn running beside the idle root.
- Observed: `intents: 0` in every one of 31 samples; `armed_s` null; `wake_work.last_outcome` `unsafe` on every attempt
  from the send until the end (retry_step flipping 0/1, i.e. a refusal restored each time); no wake typed, the side
  composer untouched (`side_typed` false); the root never woken. The refusal is the early `Unsafe` of the dispatcher
  (a queue refusal at admission would have left NoEffect intents, as the root-draft case does); the queue lane is
  never entered.
- It is not specific to focus or to a busy side. The unchanged `side-queue/group` (side displayed and idle, pane not
  focused) fails the same way at the same source: `evidence/qualification/diag-side-queue-control/` (SHA
  `bf335ef3f`, exit 1, `intents: 0`, `last_outcome` `unsafe`). The same recipient, queue proof and private Herdr
  with no /side chat passes: `evidence/qualification/diag-plain-queue-control/` (`send-p0/group`, 3/3 PASS, queue
  proof Accepted, 0 `inbox` calls). So the queue lane works for a lone root and is not reached once the process has
  a /side session. Side bit reading in these runs: side binding `concurrent` 1, `auxiliary` 0; root 0/0.
- Not diagnosed to a line of code: the daemon keeps no refusal reason (the `doctor` wake block and `diagnostics`
  are empty for the seat, kept in the row's `daemon_view`). Candidate sites are the dispatcher's pre-queue refusals
  (`src/notification/dispatch.rs`: the identity, composer-choice and `queue_candidate` gates). Filed for the
  coordinator as a release-blocking live defect of the amended queue admission at the final SHA.
- Earlier attempts, kept as evidence: `attempt-1-driver-defects/` (both cases; the driver took its busy-side
  facts 35 s before the send and could not read a split-view composer, a Codex archive confirmation then wedged the
  second case), `attempt-2-queue-lane-not-engaged/` (driver fixed, defect first seen), `attempt-4-side-queue-focused/`
  (with the daemon's diagnostics).

#### root-draft-queue/group: PASS

Final row: `evidence/qualification/root-draft-queue/` (SHA `fc3612e3f`, tree
`567a461eb792675fb9c2aaed27c8532257dbad66`, exit 0, rows 1/1, leak check clean).

- A draft typed in the root's own composer (no /side) was read back (`draft_typed`); the queue lane refused it at
  admission with NoEffect intents only (retry ladder 0 to 51 s, 10 intents), no armed, accepted or fulfilled intent
  and no root turn while it stood; it was still exactly the draft when cleared (`draft_intact`, no new wake marker)
  and nothing submitted it. After the clear the next attempt was an Accepted queue intent, fulfilled by a root turn
  that delivered the message (`codex_turn_delivery` passed, 0 busy-root samples). `release_blocking` empty.
- Reading of the original acceptance item ("a typed draft survives a queued root wake"): under the amended
  admission (any typed text refuses the queue) the draft is never queued over, so the observation is that it
  survives while the queue refuses, and the wake follows once it is cleared. Attempt 3
  (`attempt-3-root-draft-driver-defect/`) is the same product behaviour judged wrongly by the driver (it read the
  transcript's earlier wake lines as a wake typed over the draft); the driver now compares marker counts.

### The "alongside" runs

Not repeated here: the Codex `--queue-proof` matrix, `side-native-defer/group`, `side-nudge/group`,
`side-draft/group` and the `herdr_release_wake` smoke run are Task 23's steps at the same SHA; their evidence
directories are Task 23's. Task 23 had not run when this task ran.

### Test hygiene

`scripts/check-no-leaked-processes --run-id` after each run: clean for every row used above. The unfiltered check run
after the control run reported one process, a `herdr-threads daemon` of another worktree
(`.worktrees/release-233`); it is not this task's and was left alone (`diag-side-queue-control/leak-check.txt`).

### Fix pass: the focused side-queue defect, diagnosed and fixed (ht-z41e.43)

The finding above (queue lane never entered beside a /side chat) had three independent causes, found one live run
at a time (daemon log and database kept with `--keep`); the plain-root queue path was never at fault.

1. Lazy wakes reserved only the unread bindings (`src/store/wake.rs`, `load_candidate` and `reserve`). A /side
   session whose own turn had displayed the row was left out, while `for_seat_attention` (lazy included) made the
   dispatcher's text-evidence check read every binding: "wake text inbox marker: targets changed between reserve and
   submit", a one-target inbox-marker wake, no queue. A lazy frontier is now seat-level attention at both sites
   (test `a_lazy_wake_reserves_a_sibling_that_already_displayed_the_row`; it fails without the change).
2. On the live Codex the side chat's rollout is persisted and its hook payloads carry a `transcript_path`, so the
   null-path marker never set `auxiliary_session`, `wake_process_root` named no root and the wake stayed native
   (spike S1 saw null paths; both are handled). A fork's SessionStart now also reads the rollout's first line
   (`thread_source: tui_side_chat`, regular file only, bounded) and reports the same evidence
   (`a_fork_whose_rollout_names_a_side_chat_is_auxiliary_session_evidence`).
3. The wide-terminal side-by-side layout put both panes' composers on one row, so the composer read always saw typed
   text ("composer has typed text") and the amended admission refused. `src/harness/codex/composer.rs` now reads the
   root and side panes separately (`side_by_side_panes_read_per_pane`).

Driver: `settled_by_root(busy_side=True)` for the focused case, since the busy side's own events fall between the send
and the root's turn. Final rows at one SHA (`d8e87ea03`, tree `40a9d17467dee48c411441b228ababebe6b581df`, binary
sha256 `ce0daa6ebd68...`, private Herdr 0.9.5, Codex 0.172.0), exit 0, leak check clean (no processes or scratch left):

| row | evidence | result |
|---|---|---|
| side-queue-focused/group | `evidence/qualification/side-queue-focused-accepted/` | PASS: pane focused, Herdr `working`, side turn busy; Accepted intent on the root, armed 0.49 s after the send (first attempt, no dwell), woke at 6.0 s, item delivered in the root's turn, nothing typed into the side, root not mid-turn, side bits `auxiliary 1 concurrent 1`; `release_blocking` empty |
| root-draft-queue/group | `evidence/qualification/root-draft-queue-final/` | PASS: draft typed, queue refused with NoEffect intents only, draft intact and unsubmitted, Accepted wake after the clear; `release_blocking` empty |

Earlier attempts of this fix pass are kept (`attempt-5` to `attempt-7`), each failing at the next cause. The
unfiltered unit run shows one failure, `notification::queue::tests::pre_arm_retry_keeps_uncertain_started_helper_quarantined`
("invalid helper witness"), which fails identically on the unchanged task base in this environment.

## Live qualification extension (ht-z41e.31)

SHA `715d3a8ce25877e3b5617b2564c191fb0c12127c` (tree `0547b8794b389ce6abf1eb337c4a30f49efe45c5`; source of `bin/herdr-threads`
is identical, the last commit changed only `tests/integration/herdr_release_smoke.rs`), binary sha256
`ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2`, Herdr 0.9.5, Codex credential `codex-2`
(`--approve-for-me`). Evidence root `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/`,
machine-readable `live-qualification-manifest.json`. Every run's cleanup verdict: workspace closed, daemon stopped, auth copy
deleted, no problems; every leak check clean. `evidence/.../self-test`: 350 tests OK (log removed because test names mention
`auth.json`).

| invocation | exit | rows | evidence dir |
|---|---|---|---|
| herdr_release_wake (release-smoke) | 0 | 1/1 | `herdr-release-wake` |
| queue matrix `--queue-proof` | 1 | 59/60 | `codex-queue` |
| rerun `busy-invite/group` | 0 | 3/3 | `codex-queue-busy-invite-rerun` |
| side cases + flag-flip, queue, shared server | 1 | 0/10 | `side-queue` |
| same, private Herdr 0.9.5 | 1 | 1/10 | `side-queue-private` |
| native side cases, config-prose, resume-idle | 1 | 0/11 | `side-native` |

### herdr_release_wake /side step

The test's causality gap (roast-pr-5 nit) is closed: after the p0 is sent while the /side chat is displayed, the step now waits
for a `lane wakes: refused <seat>` daemon-log line (a native wake attempt made and refused) before asserting nothing was typed,
then wakes after the Ctrl-C close. Both runs pass (`herdr-0.9.5.log`: `herdr 0.9.5: wake matrix passed`).

### Queue matrix

59/60 rows pass; `facts.queue_seats` shows one `seat_config_written programmatic_wake on` per Codex seat. The one FAIL,
`busy-invite/group` T=0, reads `busy_turn_end_seen: "busy prompt not on screen"` (model-timing observation, wake counts 0,
joined true, `queue_transport_ok` true); the isolated rerun passes 3/3. Treated as a flake, reported for a fixer.

### Side cases (confirmed FAILs, reported not waived)

- `side-queue/group` PASSES on a private Herdr server: `side_first_event` = SessionStart/fork with `transcript_path` present,
  side bits auxiliary 1 / concurrent 1, root bits 0/0, `settled_by_root: true`, root woke in 9.0 s, nothing typed into the side.
  On the shared Herdr server (`side-queue`) it FAILS: wake outcome `unsafe`, not delivered in root (same bits and first event).
  Pane geometry is the only known difference.
- `side-nudge/group`, `side-draft/group`: every row FAILS with `side_setup_ok: false` in all three runs (the second /side binding
  was not established), wake outcome `unsafe`.
- `side-native-defer/group`: refusals while the side chat is displayed hold (`refused_while_side` unsafe, `side_typed: false`) but
  no root wake follows the close within the window.
- `flag-flip/group`: FAIL in both modes, no correlated stale-authority refusal observed (`armed: 0`, `accepted: 0`).
- `resume-idle/group`: FAIL, unavailable after exit as expected but the p0 was not delivered.
- `config-prose/group`: row FAIL, but its verdict is passed: calls were `config seat list` then
  `human config seat set --seat <seat> programmatic_wake on`, store value `on`, `approval_bypass: --approve-for-me`,
  `ask_bypassed: true`; the row failure came from `queue_transport_ok: false` in a native-mode run.

### Root transcript

`facts.root_transcript` in all four wake-e2e invocations: every hooked Codex root lists SessionStart/startup, UserPromptSubmit,
PreToolUse, Stop with `null_events: []` and `defect: false`; side-native `co-t0` also shows SessionStart/fork and /resume.
No null `transcript_path` on a root.

## Live defect ht-z41e.51 (side-native-defer/group)

- Source: 76fa45e016a4 plus this task's driver change; binary `bin/herdr-threads` sha256 ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2; private Herdr 0.9.5, Codex 0.172.0, recipient co-t0.
- Cause (driver defect, product correct): `side_defer_case` closed the side chat with a bare Ctrl+C. Codex answers Ctrl+C on an idle `/side` chat with an `Archive ...?` confirmation, so the side chat stayed displayed behind the dialog and the root was refused for the whole window. The daemon log of the reproduction (`docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/ht-z41e.51/run-1`, no fix) shows only `lane wakes: refused <seat> (ui human input or active turn)` lines, the `dispatch.rs` refusal for a displayed side composer, which is the documented behaviour. The ~28.6 s retry after the close was attempted and correctly refused. The window (SPACING_S + wake window from the close) was long enough; the close never happened.
- Fix: `side_defer_case` closes through `close_side_checked` (confirms the Archive dialog) and records `checks.close_side`; the dialog test is the pure `archive_dialog_open`, covered by `test_archive_dialog_open_needs_the_archive_prompt_and_its_cancel_choice` in `scripts/wake_e2e_selftest.py`. No product change, so no product regression test; the displayed-side refusal (ht-z41e.1) is untouched.
- Evidence: `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/ht-z41e.51/run-1` (FAIL before the fix, reproduction), `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/ht-z41e.51/run-2` (PASS after the fix: wake 1.52 s after the retry, `close_side: archive_dialog true, archive_dialog_after false`, retry_step 1). `commands.jsonl` is .xz.
- Verdict: side-native-defer/group passes live; the original FAIL was the driver's close, not the product.

## Live defect ht-z41e.52 (resume-idle/group)

Cause: the wake driver, not the product and not main. `resume_case` resumed the newest open top-level binding. After a
`/side` case that is the side fork (auxiliary 1, concurrent 1), so the case resumed a fork instead of the root session
(`side-native` run: bindings `01a21f00-85cd` auxiliary 1 vs root `01a21f00-2b6d`; hook trace SessionStart/resume on the fork).
The bisect against the fork point plus main's range was not needed: the failure does not reproduce without a preceding
side case, and with the driver fixed it passes after one.

Runs (private Herdr 0.9.5, Codex 0.172.0, `--codex-auth-home ~/.aisw/profiles/codex/codex-2`, binary built from
`76fa45e01` + the driver change; evidence under `evidence/qualification/ht-z41e.52/`):

| run | cases | result |
|---|---|---|
| `alone` | `resume-idle/group` | PASS (woke 5.83 s, delivered, wakes_on_p1 0) |
| `after-side` (old driver) | `side-native-defer/group`, `resume-idle/group` | resume-idle FAIL: resumed fork `01a21f45-121a`, `delivered: false`, `wakes_on_p1: 1` |
| `after-side-fixed` | same | resume-idle still FAIL, but resumed the root `01a21f4a-9d29`; the failed predecessor's pending p0 wake (side-native-defer, `unsafe` refusal) fires on resume and is counted as a p1 wake; both items are shown in that turn |
| `side-queue-then-resume` (`--queue-proof`) | `side-queue/group`, `resume-idle/group` | side-queue FAIL (`side_view_changed`, a separate side-queue finding), resume-idle PASS (woke 5.85 s, delivered, wakes_on_p1 0) with a side fork present |

Fix: `resume_root_session` in `scripts/wake-e2e.py` picks the newest open top-level binding that is neither auxiliary nor
concurrent, and raises when none exists; self-test `ResumeRootSession` in `scripts/wake_e2e_selftest.py`
(`python3 scripts/wake-e2e.py --self-test`: 354 tests OK).

Product question: a resumed `/side` fork was woken and the hook delivered both pending items in its turn (the model
summarised them without ids, so the id check failed). No evidence of a delivery defect for a resumed fork, so no bead was
filed. The `after-side-fixed` FAIL is a contamination by the still-failing `side-native-defer/group` (its own defect), not a
resume defect.

## Live defect ht-z41e.53 (side-queue/group, shared server)

Evidence: `evidence/qualification/ht-z41e.53/` (`shared-1` before the fix, `shared-2` after). Herdr 0.9.5, Codex 0.172.0,
credential `codex-2`, `--queue-proof`, owned workspace on the shared server, case `side-queue/group`.

**Refusing check.** Not a queue-admission status or identity check but the composer read inside it
(`queue_wake_admission`, `src/host/native.rs`): `refuse("composer has typed text")`. The shared-server run at ht-z41e.31 ran in a
155-column pane (`evidence/qualification/side-queue/pane-co-t0.txt`: root pane 103 columns, `│`, side pane 51), the private
server's pane and today's shared pane are 119 columns (divider at column 79). In the wide layout the narrower side pane wraps
its footer onto more rows, so its composer (`› Ask a follow-up question`) did not sit on the root composer's row.
`split_panes` (`src/harness/codex/composer.rs`) only split a screen when one row carried both prompts, so the screen read as one
pane, the root composer row `›   ...   │` became the text `│`, the idle-placeholder check failed and the attempt was refused
(10 NoEffect intents, 0 armed, outcome `unsafe`; the failing row's `side_view_before` shows the root composer row with an empty
right half, `results.json.xz` in `side-queue/`). The private run and a 119-column shared pane happen to align the two composers.
The exact wide "before" screen was not saved whole (the driver's command log clips it), so the cause is inferred from that row,
the geometry and the reproduced unit failure, not from a live wide-pane capture: the shared server's pane width follows the
user's terminal and could not be set to 155 columns from the task.

**Fix.** `split_panes` now takes the divider as the column holding a `│` on at least three rows and half the non-blank rows,
and splits when any row right of it starts with the side composer's `›`; the composers may sit on different rows.
Test `side_by_side_panes_read_per_pane_when_the_composers_sit_on_different_rows` (103/51 layout, side composer 2 rows above,
level with, and 2 and 3 rows below the root's; idle placeholder, root draft and side draft each) failed before the change
("side composer -2 rows from the root's: Unsafe { text: \"...│  GPT-6.2-Sol ...\" }") and passes after;
`side_by_side_panes_read_per_pane`, `admit_queue_wake_*` (3) and the `queue_side_dispatch` tests (6) stay green. No admission wording changed, so
TRUST-POLICY.md and AGENTS.md are untouched.

**Second, driver-side finding (shared-1).** At 119 columns on the unfixed binary the product worked (Accepted queue wake at
6.5 s, delivered in the root's turn, nothing typed into the side, `settled_by_root`), yet the row FAILed on `side_view_changed`:
the side pane's own earlier history (`Worked for 5s`, its `SIDE-QUEUE-DONE` answer) was not painted in the "before" capture and
was in the "after" one, with the same composer rows and no new wake marker. `side_view_state` now reads a side-by-side screen per
pane (root prompt rows and the side composer row; the side pane's history is not compared); a single-pane screen is compared as
before. Self-test `test_side_pane_verdict_reads_a_side_by_side_screen_per_pane`.

| run | source tree | result |
|---|---|---|
| `shared-1` (before) | `31fd08399fc77addec034241f07b55f2cd4f874e` | FAIL by the driver only: woke 6.51 s, delivered, `side_typed` false, `side_view_changed` true |
| `shared-2` (after both fixes) | `d7c63be7f726cd02188cb823ac8e0b975ecc5d6e` | PASS: Accepted queue wake at 8.01 s, `side_typed`/`side_view_changed` false, `settled_by_root`, delivered in root, side bits 1/1, root bits 0/0; cleanup clean |

Verdict: fixed. The original 155-column shared-pane failure is not reproduced live (the pane is 119 columns now); it is covered by
the unit test above built from the wide geometry, and the live shared-server row passes at 119 columns.

## Live defect ht-z41e.54 (side-nudge, side-draft, flag-flip driver)

All causes are driver defects; no product defect and no new bead. Binary `bin/herdr-threads` sha256 `ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2` (the ht-z41e.31 binary; `src/` unchanged), private Herdr 0.9.5 (`scripts/wake-e2e-herdr`), Codex 0.172.0, credential `codex-2`. Evidence root `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/ht-z41e.54/`.

### Causes and driver changes

- side-nudge/side-draft `side_setup_ok: false`: `open_sides` closed each `/side` chat with a bare Ctrl+C. Codex answers it on an idle side chat with an `Archive ...?` dialog (the ht-z41e.51 finding), so the first side stayed displayed, and every later `/side` and prompt ran inside it. The hook trace of `side-native` shows one fork session (`...c9ec6c`) for SIDE-1, SIDE-2 and the sibling turn. Fix: `open_sides` closes through `close_side_checked` (confirms the dialog), waits up to `SIDE_BINDING_WAIT_S` for a new side binding (`wait_new_side_binding`) before each close, and records `checks.side_setup` per side; `sibling_phase` closes the same way (`checks.sibling_close`); the pure `side_setup_ok` holds the verdict (root + `SIDE_FORKS` bindings).
- Close races found live: a Ctrl+C whose screen showed neither the dialog nor the main view (co-t0 side 2, run `native-side`; a hookless co-plain side that ignored three Ctrl+C in 9 s, run `native-side-2`) left the side chat displayed. `close_side_checked` now repeats Ctrl+C every `SIDE_CLOSE_RETRY_S` (5 s), at most `SIDE_CLOSE_ATTEMPTS` (8), while the side composer placeholder (`side_chat_displayed`) is on screen.
- side-draft under `--queue-proof`: all three hooked rows failed `queue_transport_ok: false` (expectation `accepted`, observed intents all `no_effect`, none armed). The root draft refuses the queue lane (the amended admission that `root-draft-queue` already judges), so the old expectation (queue wake despite a draft, ht-z41e.7) was stale. side-draft now judges like native in both modes: no wake, draft intact, delivered after the clear, refusal warning, queue effect `none` (`side_draft_passed` loses its `queued` argument; `QueueCaseEvidence.finish` expects `accepted` for a side case only when a wake is expected).
- flag-flip: on a private server with a clean pane the flip landed in the `prepared` phase and the trial passed `stale_authority_native` and delivered (the ht-z41e.31 FAILs came from the side chats left displayed by the earlier cases of the same invocation and, on the shared server, from the lane never arming). `flag_flip_verdict` additionally passes a flipped trial that never armed (`never_armed`, from `queue_proof(..., "none")`) and delivered, and still fails any trial that armed or accepted after the flip (`stale_authority_native_failed`) or delivered nothing (`not_delivered`).
- busy-invite/group T=0 (queue matrix, ht-z41e.31): failed once on `busy_turn_end_seen: "busy prompt not on screen"` (model timing) and passed 3/3 in the isolated rerun `codex-queue-busy-invite-rerun`. Recorded as a flake; no driver change.

### Self-test

`python3 scripts/wake-e2e.py --self-test`: 363 tests OK on the final rebased tree (350 at ht-z41e.31; 358 before the rebase onto ht-z41e.52/.53). New: `side_setup_ok` (root + 2 pass, root + 1 fail), `wait_new_side_binding` (new session only on the third poll; timeout), `open_sides` (close follows each new binding, no bare Ctrl+C, a side that never bound is recorded and fails setup), `side_chat_displayed`, `close_side_checked` (repeat until dialog/main view, bounded), `flag_flip_verdict` never-armed branches, `side_draft_passed` in both modes. The self-tests were written beside the code, not before it: no red run was captured.

### Live runs (source trees are the task branch commits; the driver files are the tested inputs)

| run | command (all `scripts/wake-e2e-herdr 0.9.5 --harness codex --codex-auth-home ~/.aisw/profiles/codex/codex-2`) | source tree | verdict | evidence dir |
|---|---|---|---|---|
| native-side | `--cases side-nudge/group,side-draft/group` | `6c6f1ea0084445da67604dd1b6ae466f10adab4a` | side-nudge 4/4 PASS; side-draft FAIL co-t0 (close race, side left displayed); run did not finish clean (exit 1) | `native-side` |
| native-side-2 | same, close-retry (3 attempts) | `25fa540df96d54d026e6b2386190936dcb269087` | 7/8; FAIL co-plain side-draft (hookless close ignored 3 Ctrl+C) | `native-side-2` |
| native-side-3 | same, 8 attempts, 5 s retry | `ad77ad78ff75a7bbf7a6dcdd2a44435b062dfe04` | 8/8 PASS, exit 0 | `native-side-3` |
| queue-side | `--queue-proof --cases side-draft/group,side-nudge/group,flag-flip/group` | `ad77ad78ff75a7bbf7a6dcdd2a44435b062dfe04` | side-nudge 4/4 PASS; flag-flip co-t0 PASS (stale refusal + native delivery, `armed 0 accepted 0`); side-draft 3 hooked FAIL (stale queue expectation, fixed after) | `queue-side` |
| queue-draft | `--queue-proof --cases side-draft/group` | `87f16d8326da38a54f40e4e7bb8b315959789337` | 4/4 PASS, exit 0 | `queue-draft` |
| final-native | `--cases side-nudge/group,side-draft/group` | `1f8c8cd118b91d87b238f00fa4d7bfba8a11a93f` (task branch HEAD before this evidence; integration tip incl. ht-z41e.52/.53) | 8/8 PASS, exit 0 | `final-native` |
| final-queue | `--queue-proof --cases side-draft/group,side-nudge/group,flag-flip/group` | same tree | 9/9 PASS (side-draft 4, side-nudge 4, flag-flip co-t0 one trial, stale refusal + native delivery), exit 0 | `final-queue` |

The first five runs used `bin/herdr-threads` sha256 `ce0daa6e...` (the ht-z41e.31 binary). The integration branch then gained ht-z41e.52/.53 (a `src/harness/codex/composer.rs` change for side-by-side panes), so the two final runs use the binary of the ht-z41e.53 worktree build (sha256 `3a039bd2b6fe76500eea100ae4e431ad2544aaeed3e5ad49016a409821bccae1`, same `src/` as the integration tip) and are the qualifying evidence.

Final state: side-nudge/group and side-draft/group pass live natively and under `--queue-proof`, and flag-flip/group passes live under `--queue-proof`, on the final driver and the current integration source (`final-native`, `final-queue`); no product defect found, no bead filed. Cleanup of every run: owned workspace closed, private server stopped, leak checks clean (see each `report.md`).
