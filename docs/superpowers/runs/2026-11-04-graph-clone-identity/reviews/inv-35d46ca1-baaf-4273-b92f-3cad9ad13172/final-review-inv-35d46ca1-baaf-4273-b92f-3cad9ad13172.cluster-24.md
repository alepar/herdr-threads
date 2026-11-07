# Final review, cluster 24 (tests), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both SHAs verified with `git rev-parse --verify`. The caller checkout stayed on `super-auto/graph-clone-identity` at the head, with a clean status, before and after the review. The review source was bound to the ledger before any write.

Inputs: slices 000135, 000136, 000137, 000147, 000150, 000151, 000159, 000160 and 000164. Opened beyond them to check named risks: `tests/integration/herdr_release_smoke.rs`, `tests/fixtures/herdr-matrix/codex.js`, `.config/nextest.toml`, `Cargo.toml`, `src/store/bindings.rs`, `src/store/seats/operator.rs`, `src/store/seats/continuity.rs`, `src/store/seats/observation.rs`, `src/test_support/isolated_herdr.rs`, `.github/workflows/full-suite.yml`, the head's new test files for their mounts, and the ht-ma6i.11 bead, report and review.

## What the cluster changes

- `tests/clone_identity_sweep.rs` (new, 1041 lines): ten end-to-end tests. Each uses a private Herdr, a daemon from the plugin's startup entry, the hooks `setup codex` installed, a stand-in Codex started from the pane's shell, and a real `service_session_v3` connection.
- `tests/combined.rs` and `tests/service.rs`: mount `facts_seat_reversible`, `continuity_key_session` and `reversible_session`.
- `tests/contracts.rs`, `tests/handoff_topology_authority.rs`, `tests/hc_messaging.rs`, `tests/hook_entrypoint.rs`, `tests/lifecycle_ux.rs`, `tests/named_seats.rs`: only `continuity_key: None` lines added to existing literals. No assertion changed.

## Sweep against the ht-ma6i.11 acceptance list

- Reserve to relaunch: `reserve_launch_close_unretire_relaunch` covers every step. The deadline check is exact (`after - before == unretired - cutover`). It pins same seat, joined membership, unchanged invitation count, and the frozen request settling through `inbox`.
- Live key: `second_agent_with_a_live_key_is_held` covers no new seat, hook rc 0, one alert after a repeated event, the conflict on `SeatInspect`, and a refused keyless resolve.
- Mixed live handoff: `live_handoff_with_mixed_panes` covers bare and exited-agent panes (`structural_shell_continuity`, no binding) and a live agent (`structural_process_continuity`, no hook of its own). A managed_launch-only seat is covered only by the ignored `herdr_release_live_handoff`, which the acceptance item does not require here.
- Finalize: `finalize_settles_and_frees_the_key` covers it.
- Cold restart and lag: `cold_restart_reattaches_keyed_fresh_agent` and `cold_restart_keyed_claim_during_lag_needs_no_later_event` cover it. The second pins the kept key, no alert, no further hook, and the reattachment after the lag ends.
- Reserve-with-seat, early relaunch, succession: each has its own test, and each matches its acceptance item.
- B3: `agent_exit_with_pane_open_keeps_the_seat` covers no retirement over two observation passes (11 s against `OBSERVATION_CADENCE` 5 s), kept membership and no new seat. Its relaunch binding waits prove nothing (see the next section).
- Wiring and TRUST-POLICY: this item leaves the cluster, and other cluster reviewers own it.

## Checks on specific risks

- Relaunch binding waits. `open_bindings` counts `ended_at IS NULL`. A session end only disconnects a binding, and `end_binding` runs only on exclusivity or retirement rules (`src/store/bindings.rs:911`). So the waits at `tests/clone_identity_sweep.rs:945` and `:955` pass on the exited session's own row. Instrumented run in a review-owned worktree: `open_bindings=1` after the first exit, before any relaunch. The same run asserted a `connected`, un-ended binding for each relaunched session's `native_session`, and both held (`session_bound=1`, keyed and keyless). The product does what B3 asks; only the committed test cannot see a seatless relaunch.
- `wait_reattached` is not affected. `rebind_unresolved_seat` ends every open binding of the seat (`src/store/seats/operator.rs:369`), and the wait also requires the `cooperative_continuity_key` decision, so `open_bindings >= 1` there means the successor's binding.
- `restore_held` straight after reattachment. The C1 transaction calls `lift_baseline_hold_if_clear` (`src/store/seats/continuity.rs:1072`), so the lift commits with the decision. A claim that lands after a pass's last unresolve but before its marker moves would lift only at the marker. By reading, the window is far shorter than an agent start; no failure was observed.
- `HOLD_PASS` trigger. It lives only in the private daemon's scratch database, and the writer connection drops before the fixture. The reviewers of ht-ma6i.11 already recorded its dependence on the schema.
- Mounting `herdr_release_smoke` by `#[path]` lists its six `#[ignore]` matrix tests under the new binary as well. `scripts/herdr-release-smoke` names `--test integration`, and the default profile does not run ignored tests, so nothing runs twice.
- nextest. `binary(clone_identity_sweep)` is caught by no earlier override that sets a test group, because the higher-priority filters are all binary-scoped to other binaries. `Cargo.toml` has `required-features = ["test-support"]`.
- Isolation. Every Herdr and CLI call goes through `IsolatedHerdr` or `Matrix::cli`, which scrub the environment. `Sweep` declares `service` first, so the session ends before the daemon stops.
- New test files. All ten files added under `tests/` are referenced from a mount or target.

## Requirement baseline, as far as this cluster reaches

- B1: `keyed_seat`, `wait_reattached` and the conflict tests drive it from the real hook through the real daemon, from a key the service reserved.
- B2: the close, un-retire and relaunch flows drive it through real `UnretireSeat` and `FinalizeRetirement` on a real session.
- B3: `agent_exit_with_pane_open_keeps_the_seat`, plus the instrumented run above.
- B4: nothing here changes Herdr; the tests run against the Herdr binary on PATH.
- Production paths other than the stand-in Codex hook (the Claude mod forward, the Hermes bridge) leave this cluster and are the seam reviewer's to trace.

## Runs and evidence

- The head's tree is `43572f92d6410bcee91e0dcd513fbc12d68c8b96`, the exact tree of the task 11 report: `heavy tb nice cargo nextest run --locked --all-features --test clone_identity_sweep`, 10 of 10 passed, leak check clean, clippy and `check-default-features` rc 0. Two more runs on the same content less a design note also passed. I reused that evidence instead of repeating the run, and did not re-run those checks myself.
- My one run: a detached worktree at the head, seeded by copy-on-write, with an uncommitted instrumentation patch to `agent_exit_with_pane_open_keeps_the_seat`. `nextest run ... agent_exit_with_pane_open`: 1 passed in 26.6 s. `scripts/check-no-leaked-processes --run-id 33b01668-59ff-4ad8-bc27-c92bcbe14241`: rc 0. (A first invocation piped to `tail` flagged only that `tail`, which had inherited the exported run id.) The worktree is removed.
- No full-suite run was made; the caller owns that, including whether the sweep's roughly 45 s of two-at-a-time time stays inside the 300 s budget.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `agent_exit_with_pane_open_keeps_the_seat` (`tests/clone_identity_sweep.rs:945`, `:955`): the committed waits pass on the exited session's still-open row, so a relaunch left seatless by a quiet hook stays green; the review's one-off run showed today's product binds both relaunched sessions, and no committed test pins that end to end.
- The sweep drives only the stand-in Codex hook; a keyed Claude or Hermes agent end to end is not covered here (per-layer tests live in other clusters, and spec §4.2 leaves live harness env filtering to qualification outside this run).
- `pane_owned_by_other_seat`, `seat_holds_other_key`, `session_matches_other_seat` and the claimless `ContinuityKeyPending` refusal have no real-daemon test in this cluster; they appear in store, hook and CLI tests elsewhere under `tests/`.
- G5 target-absence retirement has no real-daemon test in this cluster (10 observed minutes); `restored_target_absent` is pinned only in `tests/store/restored_pane.rs`, `tests/store/reversible_retirement.rs` and `tests/store/schema.rs`, and the bead's acceptance list does not ask for it here.

## Deferred OK
- Replace the two relaunch waits with a check on the relaunched session's own connected binding, using the `native_session` query `seat_succession` already reads through history; about ten lines and no product change.
- `tests/clone_identity_sweep.rs:463`: the direct-send refusal asserts only a nonzero exit, so a changed flag would satisfy it; asserting the precheck text the task report quotes would close that.
- `tests/clone_identity_sweep.rs:831`: `wait_for("the keyless seat", ...)` always answers `Ok`, so it waits for nothing; the `open_bindings == 1` assertion after it does the checking, and the wrapper could go.
- `sweep()` (`tests/clone_identity_sweep.rs:154`) skips whenever `herdr` is absent, even with `HT_SKIP_HERDR_TESTS` unset, where `IsolatedHerdr::new` would panic and `.github/workflows/full-suite.yml:97` expects failure; `herdr_present()` in `tests/integration/named_handoff.rs` has the same shape, CI's provisioned check and the other Herdr suites would still catch a missing binary, and the module doc states the skip.
- `continuity_key_session::` tests, mounted in `tests/service.rs`, hold the in-binary `IN_PROCESS_DAEMON` mutex but sit outside the nextest `service-daemon` group; `final-review-inv-35d46ca1-baaf-4273-b92f-3cad9ad13172.cluster-1.md` already reports this, and `graph_send` set the same precedent at the base.
