super-roast verdict: clean (1 nit, fixed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (a local multi-agent messaging daemon). The change touches one integration test and no product code, so the blast radius is the test suite's signal quality.
inputs: bead/ht-4msq@32e586caf vs main@e46d26eef
coverage: one fresh scout covering the correctness, test-causality and claim-accuracy lanes · 4 raw → 4 deduped → verified inline by the author · judge panel not run
independence: partial (subagent scout, inline verification) · rung: manual fan-out

## Confirmed findings
- [Nit] tests/integration/lanes_latency.rs (idle hook) — the new idle-hook closure captures a `LaneProbe` clone, which makes a cycle (probe -> registry -> pacer -> idle hook -> probe). The store stays alive until the process exits.
  verdict: confirmed (author verified: `ProbeState.registry` holds `Arc<CommitKicks>` and the pacers, and the pacer owns the hook)
  evidence: src/app.rs ProbeState.registry/store; src/service/kicks.rs pacers; src/service/pacer.rs idle_hook
  fix-shape hint: replace the hook with a no-op once the measured pass's count is read. Done in the follow-up commit.

## Rejected (with reason)
- none

## Unverified nits (spot-checked)
- [FYI] With a two-dot diff against the current main, the branch looks like it reverts ht-i3pc. It does not: the branch is based on e46d26eef. The three-dot diff touches only the test file.
- [FYI] A retry of the inviter in the very next pass (a ladder regression) is no longer caught by this count. The old count caught it only by timing, which was the flake itself. The ladder spacing is pinned by the `most_attempts` test.
- [FYI] A failed completion commit is retried inside the same pass (`has_pending_completions`). That could still yield 3 commits, but only after a store failure, not under load. This was already true before the change.

## Escalations (need human)
- none

Scout confirmed that:
- The idle hook runs on the wake lane thread after the whole pass, with no pacer State lock held (pacer.rs `wait_blocking` -> `record_idle`).
- `commit_counts` takes only the probe mutex and atomic loads, so there is no deadlock.
- Every assertion the doc comment's "Kills:" list relies on still holds.
