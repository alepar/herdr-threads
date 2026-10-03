<!-- upstream-feedback draft — PARKED (autonomous run), not filed. Target: alepar/superpowers. Findings were also relayed live to the maintainer session. Full analyst evidence (ledger line refs) is summarized here; the run's ledger is .superpowers/sdd/ht-p03-plan/progress.md. -->
# 2026-10-01-remaining-herdr-threads-findings: the merge lane closes beads on a detached HEAD, and per-task "pre-existing failure" claims hid a regression until the final review

Plugin: started at 6.4.2-alepar4.3 (5c92bce). The code phase moved through 4.5 (104b54e), 4.6 (2bf1d53) and 4.10 (2c36ff3); upstream is now at 4.11 (0431bea).

The run had 199 beads (150 task, 29 bug, 16 feature, 4 epic; 80 of them sp:review) and 477 blocks edges. Its design-time critical path was 13 deep, with a width of 5.8. super-code launched 8 times (1 died, 2 stopped). Design and PR roasts each ran 2 rounds. It was autonomous, ran 2026-10-01 → 2026-10-03, and took about 1.5 days.

## Defects

### 1. The merge lane committed on a detached HEAD and closed beads; the refused ref update left merged work off the branch (twice)
- **Evidence:**
  - Ledger l.617/618, Tasks 90/91 (ht-p03.135/.136): BLOCKED-AUTH on `git branch -f <integration> HEAD` / `git update-ref`. The merge was committed on a detached HEAD and the bead was closed in bd while the branch ref stayed at f79901da.
  - run.md codeLaunch-6/7-result: the integration worktree was on a detached HEAD again in the next launch (a4688e9b). Each time, the coordinator fast-forwarded by hand.
  - coordinator.js mergeStep (~l.1557) assumes the branch is checked out and never verifies it.
- **Premise to verify:** what detached the worktree. Candidates are a dispatched agent's checkout, or the 6214eab/0cdaa12 per-task removal/stop.
- **Fix shape:**
  - Precondition: `git symbolic-ref -q HEAD` = `refs/heads/<integration>`. Otherwise return merged:false (`detachedHead`) and close nothing.
  - Close the bead only once the branch ref equals the merge commit.

### 2. "Pre-existing / identical at base" failure claims were accepted unverified against the moving tip; a real regression survived every per-task gate
- **Evidence:**
  - These ledger lines are `minor (deferred)` on tasks that closed `review clean`: l.253 ("not checked against the base"), l.360 ("not shown pre-existing"), l.404/414/457 ("plausible, not verified").
  - run.md codeLaunch-3-result: the final review found the setup_cli regression and the Pacer kick-during-backoff regression. Fix beads ht-p03.104–.109 followed.
- **Premise to verify:** the failing set at `merge-base <integration> <task>`, or a run-scoped expected-failures artifact, can be checked mechanically.
- **Fix shape:**
  - A claim that test X pre-exists must carry X's result at the fork point, or an entry in the expected-failures artifact.
  - An unproven claim is graded Important.

### 3. mergeCheck compiled one feature configuration (`--all-features`) and missed a default-feature break (the inverse of #12 defect 5)
- **Evidence:**
  - Every `Launch:` mergeCheck used `--all-features` only, and 89 merge checks passed.
  - The final review's must-fix "default-feature build" became bead ht-p03.138.
  - The workaround was `scripts/check-default-features` (~3 s).
- **Fix shape:**
  - Derive mergeCheck from the CI build matrix: at least default plus all-features.
  - Pre-flight warns when mergeCheck covers fewer configurations than CI.

### 4. coverage-precheck still fails under macOS BSD awk once the flag-sweep ledger has more than one entry (reported live; NOT fixed)
- **Evidence:**
  - `super-design/scripts/coverage-precheck:70` passes newline-joined keys via `awk -v`. It has not changed since da9f562.
  - Reproduced: `/usr/bin/awk -v k="$(printf 'a\nb')"` fails with "newline in string", rc 2.
- **Fix shape:**
  - Pass the keys via ENVIRON or a file operand.
  - Add a /usr/bin/awk case to the script's tests.

### 5. `TMPDIR=<task worktree>/.tmp` (0cdaa12) exceeds the 104-byte macOS socket-path limit in nested worktrees
- **Evidence:**
  - Sweep notes: about 170-byte paths made 43 lib tests fail with SUN_LEN errors.
  - The ht-p03.20 TMPDIR was 155 bytes before any file name.
  - Agents improvised `/private/tmp` symlinks, and one was refused (ledger l.656).
- **Fix shape:**
  - When the worktree-local temp path is long, the dispatch supplies a short per-task temp root.
  - That root is passed to stop-run-processes and removed at task cleanup.

### 6. Improvised bulk destructive commands are refused and quarantine the whole task as BLOCKED-AUTH, though a narrow path existed (recurrence of #11 Design Q A / defect 3)
- **Evidence:** 7 BLOCKED-AUTH lines in the ledger:
  - 4 scripted conflict-marker stripping or `rebase --continue` loops (l.175, 185, 282, 304)
  - 2 ref rewrites (defect 1)
  - 1 `pkill -f "cargo test --locked"` (l.656), issued after 0cdaa12's "never stop a process the command does not match" rule. It would have killed a concurrent pre-sweep.
- **Fix shape:**
  - Name the forbidden forms in the merge and implementer prompts.
  - On a refusal, take the narrow path (per-hunk edit or re-implement on the tip; stop-run-processes only) before returning BLOCKED-AUTH.

### 7. super-code dispatches a human-gate bead, and a terminal leaf waiting on it, to the planner/implementer
- **Evidence:**
  - Ledger l.621: ht-p03.134, labelled `human-gate` and "closed by the coordinator", was dispatched and returned BLOCKED.
  - l.615: the native rerun leaf was dispatched before its gate existed.
  - Nothing in skills/ handles gates.
- **Fix shape:**
  - `ready-in-tree` skips gate-labelled beads and reports "waiting on gate".
  - super-auto creates gates with a label and blocked-by lines.

### 8. Review re-entry BASE = merge-base(integration, branch) collapses to the branch tip once the branch is merged (same class as #12 defect 3)
- **Evidence:**
  - Ledger l.588 (ht-p03.126): BASE was the task commit itself, so the review package was empty and the task went to pending retry.
  - coordinator.js ~l.1406.
- **Fix shape:** take BASE from the recorded mergeBase or workspace base, or from the merge's first parent.

### 9. A merge-check fix was made on a stale task base; it duplicated a sibling's merged change and escalated (medium confidence)
- **Evidence:**
  - Ledger l.644–647 (ht-p03.141): a Cargo.toml `[[test]]` conflict with ht-p03.138.
  - Two RESOLVE attempts escalated, and the coordinator merged it by hand.
- **Fix shape:**
  - Rebase onto the tip and re-run mergeCheck before fixing.
  - Fix only failures that persist on the tip.

## Design question
### A. Where does a terminal "final-SHA" evidence leaf run relative to the code roast and fix loop?
A 2–4 h native-matrix leaf with a "one final SHA" rule sat in phase 3. Every phase-5 fix would have made its evidence stale, so the run added a gate by hand (run.md reorder-2026-10-02).
- **Hold until phase 5 exits:** one expensive run.
- **Run in phase 3:** the roast sees the evidence, but every fix means a rerun or a recorded deviation.

Please state the position explicitly either way.

## Doc gap
- **Base drift in long runs:** main moved 82 commits mid-run. That caused a 47-file combine, a migration renumber and a protocol-version reconciliation. super-auto doesn't say:
  - when to absorb base drift
  - who owns the conflict pass
  - that the roast and the sweep must judge the combined tree

## Already fixed (do not re-litigate)
- Coverage input size: 1ad8d37.
- Planner null abandoning a round; close-in-tree-epics with bd 1.3: 2bf1d53.
- Per-task worktree removal: de70e68.
- Process stop at task completion: 1e15092.
- stop-run-processes over-matching: 2c36ff3.
- read-ledger safeguards failure: 655fa95 (4.11). The ledger itself still grows unbounded (119 KB, 376 deferred minors), so the progressive-disclosure ledger proposal remains open.

## Not established
- The detached-HEAD actor is unknown.
- Defect 9 rests on a single occurrence.
- The launch-1 planner stall is attributed to the network.
- Ledger counts were rebuilt by grep with no ledger-check cross-check, so they are lower bounds.
- Analyst model: Opus 5.5, fresh context.
