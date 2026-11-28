# Final review, cluster 6 (evidence), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout was at HEAD c3e9ab0be with a clean status when I checked; I made no checkout changes.

## Scope

The cluster is seven new committed log files under `evidence/` (slices 000052-000058):

- `evidence/ht-z41e.3/{checks,clippy,golden,lib-tests}.log`: the ht config seat CLI task (ht-z41e.3). Lint passed (exit 0). check-harness-neutral, check-wire-compat and check-default-features passed (rc=0). Golden surfaces: 15 of 15 passed. Lib unit tests: 275 of 275 passed, including the config_seat, seat_config registry/store and service tier tests.
- `evidence/ht-z41e.8/selftest.log`: the wake e2e driver self-test (Ran 317, OK). It includes the SideQueueCases and QueueEvidence side_sessions tests.
- `evidence/ht-z41e.11/{checks,seat_config_sweep}.log`: the integration sweep task. Lint and the three check scripts returned rc=0. The 3 seat_config_sweep tests passed.

## Analysis

- Convention: `evidence/<bead>/` is a directory already established at base (ht-3bi.6.2, ht-54r.84 and others). The new files follow that layout.
- Restore integrity: b38e8080c deleted `evidence/`, and ee028ca34 restored it. `git diff --name-status base..head -- evidence/` lists only the seven additions (A). No pre-existing evidence file is lost or modified at head.
- Secrets: I scanned all seven files for API-key, JWT, `token=` and `refresh_token` patterns and found no matches. The Codex refreshed-auth line in selftest.log names a self-test fixture path under `/tmp/sp-6cd888ee/...-test/auth.json`, not a real credential. Absolute `$HOME/...` worktree paths appear in cargo output. Base already has 35 evidence files with the same paths, so this is not new exposure.
- Staleness against head: each log was captured at its own task head, not at the integration head. Three later commits changed the code these logs cover:
  - 136484396 (ht-z41e.21, root-marker redesign) changed `tests/integration/seat_config_sweep.rs` after the ht-z41e.11 log.
  - c0ec30cec (ht-z41e.25) changed `scripts/wake-e2e.py` and the self-tests after the ht-z41e.8 log. Head now has 279 + 39 = 318 self-test functions against the 317 recorded.
  - b38e8080c (ht-z41e.22) changed the queued-wake receipt.
  The logs are valid per-task evidence. They are not qualification of the reviewed head, and nothing in this cluster claims to be.
- golden.log records a `dead_code` warning for `default_seat_for_test` (src/cli/mod.rs:4327). The ht-z41e.3 clippy.log captured afterwards passes. At head the function carries `#[cfg(test)]` (src/cli/mod.rs:4330), so the warning was fixed in-task. It does show that golden.log was taken on an earlier tree than the committed one.
- Requirement baseline B1-B10 and S1: this cluster holds evidence only and delivers no behaviour. Tracing each row from a production entry point belongs to the code clusters and the seam reviewer. B8's spike evidence (`spike-side-identity.md`, run-2/run-3 side-results) lives under `docs/superpowers/runs/`, outside this cluster. The spec's live testing line (spikes S1-S4, `side-queue/group` under `--queue-proof`, existing queue-proof cases with per-seat enablement) has no evidence here. The open beads ht-z41e.10, ht-z41e.30 and ht-z41e.31 carry that live qualification at the final SHA. Delivering it is the seam/gate reviewer's call and is not decided by this cluster.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No committed evidence shows lint, the check scripts, golden surfaces, lib tests, the wake e2e self-test or seat_config_sweep passing at head c3e9ab0be. The logs predate ht-z41e.21, ht-z41e.22 and ht-z41e.25, so a head-level run belongs to gate ht-z41e.27 or the final qualification.
- No live wake e2e evidence (side-queue/group, side-native-defer/group, queue-proof matrix with per-seat enablement) is in this cluster. That work is owned by the open beads ht-z41e.10, ht-z41e.30 and ht-z41e.31.

## Deferred OK
- evidence/ht-z41e.3/golden.log was captured on a tree before `#[cfg(test)]` was added to `default_seat_for_test` and records a dead_code warning that the committed tree no longer has. The log is informational only; the later clippy.log in the same directory passes.
- evidence/ht-z41e.8/selftest.log (317 tests) and evidence/ht-z41e.11/seat_config_sweep.log are per-task snapshots superseded by later commits (c0ec30cec, 136484396). They should not be cited as head qualification.
