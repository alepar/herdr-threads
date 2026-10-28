# Roast: ht-i4tm (six CI slices), main..bead/ht-i4tm

Inline review against the super-roast rubric (subagent reviewers not available to this run).

Scope: `.github/workflows/ci.yml` (matrix 5 -> 6, CPU line, comments),
`.github/workflows/full-suite.yml` (comment), `tests/release/workflows_test.sh` (matrix
assertion), `docs/dev/build-speed.md` (evidence section). No Rust, test or product code.

## Claims checked

- Root cause is runner speed, not slice balance. Evidence: three runs' per-job `Locked build`
  durations (same commands, same cache) against test-step durations; the pairs track (381 s
  build / 155 s test, 592 / 297). Per-shard median test duration (0.09-0.12 s fast,
  0.14-0.20 s slow) and per-module sums move together, so it is not a few heavy tests landing
  on one slice. The slow slice changes between runs. Held.
- A time-informed partition would not help. Slices carry 1598-1612 tests each; the summed
  time differs because the runner differs. Held.
- The four named long tests are essential waits: the wake-unreceived test waits for the real
  30 s ladder retry in a detached daemon (37-40 s on every runner, so wall, not CPU); the
  config_wipe test asserts a 12 s hold window and a 7 s no-warning window; the mod_delivery
  capacity test drives 257 real sends and ~26 pages; Q4 counts SQLite work and scales with the
  runner. None has an incidental sleep to remove without weakening it. Held.
- Projection ~250 s on the slowest observed runner: 297 s x 5/6 = 248 s. Per-shard fixed costs
  (front-loaded `threads-required` chains) are a few seconds and split with the tests. Held,
  as a projection.

## Findings

1. (fixed) The doc's median ranges were first stated as 0.09 / 0.15-0.20 s; the data say
   0.09-0.12 / 0.14-0.20 s. Corrected.
2. (accepted) One more runner per push: ~10 min of build outside the timer. The repository
   is public, so hosted Linux minutes are free (build-speed.md). Peak concurrency per run
   goes from ~16 to ~17 jobs; two overlapping pushes already exceed the free plan's 20
   concurrent jobs and queue. Queueing happens before the shard timer starts, so it cannot
   trip the 300 s guard.
3. (checked) No required status checks name `Tests k/5` (ruleset `protect-main` has only
   deletion and non-fast-forward rules), and no script matches job names, so the rename to
   `Tests k/6` breaks nothing.
4. (checked) The CPU line runs before the timer under `set -euo pipefail`; a failing `lscpu`
   inside the command substitution cannot fail the step (echo's status is the line's).
5. (note) `yq` and `actionlint` are not installed locally, so `tests/release/workflows_test.sh`
   was not run here; the edited assertion was checked by parsing the YAML (matrix
   `[1, 2, 3, 4, 5, 6]`). CI's actionlint job runs it.

No open findings.
