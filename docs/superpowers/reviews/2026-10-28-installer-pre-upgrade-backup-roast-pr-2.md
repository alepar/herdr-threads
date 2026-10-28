---
super-roast verdict: Should-fix (2 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 2 of 3
profile (assumed): Production developer tool (local CLI plus daemon) used daily on two hosts with real message stores. The installer runs via curl|bash on macOS and Linux. Data loss of the store is the main blast radius; everything else is local disk, install latency and operator ergonomics.
inputs: installer-auto-backup@5b64f23fe vs 6631a0b90 (delta)
delta vs prior: 2 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 3 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 15/15 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, deploy-safety, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs, data-migrations) · raw 2 → deduped 2 → panel 2 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage unknown, provenance unknown — basis: ""
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 1/1/1/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · deploy-safety 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · test-causality-lifecycle 1/1/1/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 0/0/0/0 · data-migrations 0/0/0/0

Readiness note: the engine supplied no review-readiness basis (coverage unknown, provenance unknown, empty basis), so the verdict carries `[review readiness incomplete]` and `[low coverage]`. Every stage ran (no dead triage, scout, dedupe or judge seat; stage `lowCoverage` is false); the qualifier records the missing readiness evidence, not a stage loss. `convergenceEligible` is false, so no `[converged]`. This round reviewed only the delta 6631a0b90..5b64f23fe; the prior round's fixes landed in 6631a0b90 and were checked directly for the resolved marks below.

Prior-report tracking (iteration 1 confirmed findings, all addressed in 6631a0b90 "Address the PR roast of the pre-upgrade backup"):
- [Should-fix] docs/install.md:517 (restore recipe passes `--no-backup`, discarding the only copy of the live store) — **resolved**. docs/install.md:517 now says the downgrade uses the current installer, which "first backs up the store you are replacing; if that backup fails (a damaged store), copy `node/` aside by hand and pass `--no-backup`".
- [Nit] docs/install.md:517 / RELEASE.md:60 (old tag's install.sh rejects `--no-backup`) — **resolved**. Both now name the current (`main`) installer with `--version <old>` and state that "an older tag's own script has neither the backup nor the flag".
- [Nit] src/cli/pre_upgrade_backup.rs:240 (integrity-check non-`ok` branch untested) — **resolved**. `check_copy` was split out and `a_copy_that_fails_its_integrity_check_is_refused` (src/cli/pre_upgrade_backup_tests.rs:306) drives the non-`ok` branch and asserts the "the copy failed its integrity check" error.

## Confirmed findings
- [Should-fix] src/cli/pre_upgrade_backup.rs:189 — The new pre-copy prune pass does nothing in the steady state: it applies the same keep rules as the previous run's post-copy prune (newest 3, newest per schema, unreadable), so on a root only installer runs have written it never deletes anything, and the disk-full recovery that the code comment, the spec and docs/install.md now promise does not happen. [lanes: premortem]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `backup_and_prune` runs `prune(&root, None)` at line 188 before the copy and `prune(&root, Some(&backup))` after it. `prune()` keeps an entry when it is the newest of its schema, `index < KEEP_NEWEST` (3), unreadable, or equal to `taken`. The previous install's late prune leaves a set in which every entry already satisfies one of the first three rules (the just-taken entry sorts newest). The next install's early pass applies the same rules to the same set: the same 3 entries are newest and the same schema heads exist, so it deletes nothing. The 4th-newest entry is evicted only by the late prune, after the `VACUUM INTO` that needs the space. The early pass frees space only for hand-added entries or a run interrupted between rename and late prune. Three texts claim otherwise: the code comment at line 144 ("so a full disk can still take the copy"), spec line 95 ("before the copy without a just-taken entry (so a full disk can still take the copy"), docs/install.md:517 ("It prunes that directory before the copy (so a full disk can still take it"). The new test `prune_runs_before_the_copy_so_a_failed_copy_still_frees_space` seeds five same-schema entries, a state the installer never leaves behind, so it does not cover the steady state. All three seats confirm; the refute seat's limiter (fails closed, `--no-backup` and manual cleanup remain, no data lost) is why this is not Blocking. Severity: 2 seats Should-fix, 1 Nit; kept at Should-fix because the pass was added for exactly this purpose and three documents now state a recovery that does not occur. Not a data-loss floor: the install fails closed with nothing replaced.
  fix-shape hint: Either have the early pass keep only the newest 2 plus the per-schema heads (so it removes the entry the late pass would evict, never a schema head), or drop the disk-full claim from the comment, spec and docs/install.md and keep the early pass as the stale-entry sweep it actually is; add a steady-state test (three installer-shaped entries, then a failing copy) either way.

- [Nit] src/cli/pre_upgrade_backup_tests.rs:336 — The new unit test `prune_runs_before_the_copy_so_a_failed_copy_still_frees_space` does not depend on the ordering it is named for; it would still pass if the early prune ran after the failed copy in the error path, so nothing tests the "prune before copy" change this diff makes. [lanes: test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: The test writes `b"not a database at all"` as the store, so `copy_store` fails at open/VACUUM before writing anything. Its only assertions are `failure.prune.pruned == [pre-v2.4.1-..., pre-v2.4.2-...]` and `names(&root).len() == 3`. Moving `let early = prune(&root, None);` from line 188 into the `Err(reason)` arm after `fs::remove_dir_all(&staging)` (line 212) yields the same pruned list and the same count, so both assertions still hold. The success-path tests merge `early.pruned` and `late.pruned`, so they cannot tell the order either. All three seats confirm at Nit; production code is correct, the gap is regression protection for the reorder. No seat ran the mutant; the finding rests on reading the head source.
  fix-shape hint: Make the copy observe the pruned state: a test-only failpoint or closure in the copy step that records which entries exist when staging is created, then assert the doomed entries were already gone at that point.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this round. Note: iteration 1 spot-rejected "[FYI] src/cli/pre_upgrade_backup.rs:166 — Pruning runs only after a successful backup, so a full disk blocks every later upgrade" on the grounds that pruning before the backup would delete the last good copy first. 5b64f23fe then added a pre-copy pass. The Should-fix above is not a re-litigation of that rejection: it concerns the new pass's ineffectiveness and the claims added for it, not the absence of a pass.

## Unverified nits (spot-checked)
- none (no spot checks this round).

## Escalations (need human)
- none
---