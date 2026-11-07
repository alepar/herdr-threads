# Final review (bounded): ht-ma6i graph clone identity, threads side

## Review basis

- Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 35441a7e57ed953993c9ed2e393f5e7a4c32022f, both verified with `git rev-parse --verify`; read from captured Git only, caller checkout untouched.
- Route: bounded. The input sidecar reads status unavailable, sourceAvailable true, wholeRangeReady false, 656 selected paths, 0 omitted, 3 retained gaps (run.md:151 inv-35d46ca1-baaf-4273-b92f-3cad9ad13172: incomplete claimed collection, invalid null-review bundle, incomplete split bundle). No whole-range package was built or loaded.
- Inventory: 656 changed paths, +92719/-526. 487 are run artifacts under docs/superpowers (unreviewed here); about 170 are product, migration, integration and test paths. Inspected hunks are listed in bounded-coverage.json; everything else is unreviewed.
- No full-suite measurement of this branch exists; the caller runs the sweep after this invocation. Nothing here treats the branch as tested.

## Requirement baseline trace

- B1 (continuity key approved): traced from production entry points. The hook reads `HERDR_THREADS_CONTINUITY_KEY` through `CallerEnv` (`caller_continuity_key`, top-level and non-subagent only, capability-gated on `seat.continuity_key_v1`), presents it on the C1 claim, enrollment (`resolve_seat` with key) and check-in (`ContextJournal::with_continuity_key`, `tool_boundary_check_in`); the mod's watch claim reaches it through `hook::watch_continuity`. The daemon judges it in one pure table (`key_verdict::judge`) inside the C1, ordinary-resolution and `register_available` transactions; conflicts are stashed and recorded after rollback and never fail the hook (`api_failure` makes them quiet). Service v3 serves reserve, release and alerts (`service_controls::operate`). End-to-end coverage exists in tests/clone_identity_sweep.rs (`reserve_launch_close_unretire_relaunch`, `cold_restart_reattaches_keyed_fresh_agent`, `second_agent_with_a_live_key_is_held`), on a private Herdr. Delivered in code; run evidence not verified here.
- B2 (pane close retires immediately; graph undo reverses it): the reconciliation `BeginRetirement` action calls `begin_retirement_authored`, which for a `Closed` retirement of a key-holding seat runs the shared `retirement_transition` and writes a `reversible_retirements` row instead of a job; obligations are not settled (`settle_local_seat` only clears wake work). `UnretireSeat` and `FinalizeRetirement` are wired through `service_controls::operate` to `reversible::unretire`/`finalize`, with the deadline shift, generation bump and fact emission; peers get `seat.retired_reversible`/`seat.unretired` (registered, held kinds, wire baseline extended). Delivered in code; the sweep test drives close, refused sends, un-retire, relaunch with the same seat id, deadline resumed and memberships intact.
- B3 (agent exit with pane open keeps the seat and memberships): no threads change was required; `agent_exit_with_pane_open_keeps_the_seat` pins it for keyed and keyless relaunch. Delivered.
- B4 (Herdr unchanged): no Herdr source is in the range; the only Herdr-side reads are existing `pane process-info` calls, now fenced on the responding server's boot. Delivered.
- No baseline row is undelivered on the inspected paths; human scope changes: none.

## Cross-task integration observations

- The key verdict composes correctly across the three deciding transactions: a check-in commits on `NotReady`/`Pending` (settle_key writes nothing), ordinary resolution refuses hold-bearing kinds and lets alert-only kinds proceed, and the C1 claim refuses before target guards. The reversible fence and the final path share `retirement_transition`, and `insert_retirement_job` is the single local writer that releases the key and shell fingerprint (plus the replicated writer, which also releases).
- Arrival-order convergence of the two new facts (ledger Task 14 concern, fork repair re-issuing `seat.unretired` first) is addressed at head by `kept_unretire` in `materialize_seat_retired_reversible` (commit e8708c2af); the episode-effect guard keys on (kind, seat, retired_at).
- The frozen fragment is composed into every due scan named by the spec (physical and sparse receipt due, extension lapses, soft pokes, invitation due) and the four write-driving sites (receipt projection, invitation apply, archival liveness, invitation origin warnings).

## Ledger triage

- Recurring blocker (stack parent not rebased, 3 tasks): process issue, resolved by later merges; no product effect at head.
- Recurring minor missing-evidence (3 tasks): process; evidence of those task runs is prose only, which is why this review does not rely on task-level test claims.
- Resolved at head: Task 2 missing-clear-path (operator fresh seat now calls `clear_conflict_on_target`), Task 2/3 unwired helpers and client-leaf gap (key now sent; `release_for_final` called by both job writers), Task 4 deferred B3 evidence (sweep test), Task 5 cold-restart race (fingerprint read fenced, 1910c51f6 and ba73dbcbb), Task 14 fork-repair order (e8708c2af).
- Escalated to must-fix: Task 13 test-runner-unreliable (a skipped/vacuous-test class), Task 9 limbo case plus Task 14/17/15 policy omissions (TRUST-POLICY is normative and AGENTS.md requires accepted limits in it).
- No parked or BLOCKED-AUTH lines exist in the ledger. The ledger's Metrics lines are UNAVAILABLE, so absence of parked lines is not read as clean.

## Findings

- src/cli/hook.rs:5169 `spawn_watchdog` calls `std::process::exit(0)` when the hook budget expires; an in-process test that enters the hook path can end its test binary (cargo test) or its own process (nextest) with exit 0, so assertions after that point never run and the test reads as passed. Ledger Task 13 reports `cargo test --lib cli::hook` stopping with exit 0 and no summary. The branch adds about 490 lines of hook tests (tests/cli/hook.rs) whose completion is therefore unproven. Mechanism pre-exists at base.
- src/store/seats/observation.rs `target_absence_evidence` and `restored_pane_evidence`: when the saved target is shown but its terminal is claimed by another seat, no watch starts and the absence row never counts (`TargetShown`/`None`), so that seat waits for an operator indefinitely; TRUST-POLICY.md G5 states that a shown target hands over to the watch rule, which is not what happens in this case (ledger Task 9 limbo-case-undocumented).
- TRUST-POLICY.md omits three accepted limits or rules the code has: the same-millisecond second episode read as a copy by `episode_effect_recorded` (doc comment only, ledger Task 14), the owner unresolve/retire/move clearing trigger for alert-only conflicts (ledger Task 17), and the Hermes gateway key limit in the Amended log (ledger Task 15).
- src/cli/hook.rs `api_failure`: any refusal whose detail starts with `KEY_CONFLICT_CAUSE` is made quiet, including a keyless agent's enrollment refused by a key-conflict hold; that agent stays seatless with no model-visible guidance, while spec 4.2 says the existing restore-hold outcome carries the conflict (ledger Task 7 fragile-string-classification).
- src/store/seats/key_verdict.rs `judge`: under reconciliation lag a bound key whose seat already owns the pane returns `NotReady`, so a keyed plain-CLI resolve of the caller's own resolved pane answers `ServiceBusy` instead of the seat during lag; the spec table makes this row a no-op and reserves lag handling for state-dependent refusals. Check-ins are unaffected (settle_key ignores `NotReady`).
- src/store/seats/cooperative.rs `register_available`: `clear_alert_only_if_owner_changed` runs on every check-in, keyless ones included, adding one query to the hottest write path; cheap, but unconditional.
- src/store/seats/reversible.rs `shift_pending_deadlines`: an obligation applied during the window (a peer or pre-3.1 send) shifts by the whole window rather than by the time it was frozen, extending its deadline; the ledger records this as an accepted limit in the policy text (Task 8).

## Verdict
not ready (bounded review only: whole-range review outstanding, no full-suite run, hook test completion unproven, normative policy omissions)

## Must fix before landing
- Prove or fix the hook tests' completion: src/cli/hook.rs:5169 `spawn_watchdog` exits the process with status 0 on budget expiry, which can end in-process tests vacuously (ledger Task 13); show that tests/cli/hook.rs and `cli::hook` lib tests run to their final assertion under nextest and cargo test, or keep the watchdog out of in-process test paths.
- Reconcile TRUST-POLICY.md G5 with observation.rs: a seat whose shown target's terminal is claimed by another seat neither watches nor counts absence and waits indefinitely; either count it or record it as an accepted limit (ledger Task 9).
- Add the missing TRUST-POLICY.md entries: same-millisecond episode limit of `episode_effect_recorded`, the alert-only conflict clearing trigger on owner unresolve/retire/move, and the Amended log line for the Hermes gateway key limit (ledger Tasks 14, 17, 15).

## Untested scope
- Whole-range review outstanding: this is a bounded captured-source review; provenance gaps at docs/superpowers/runs/2026-11-04-graph-clone-identity/run.md:151 inv-35d46ca1-baaf-4273-b92f-3cad9ad13172 (incomplete claimed collection, invalid null-review bundle, incomplete split bundle) are retained and unresolved.
- Unreviewed product paths: src/store/continuity_keys.rs (only reserve/release/bind read), src/store/seats/continuity.rs hunks (only head lines 330-680 read), src/store/seats/publication.rs, src/store/seats/operator.rs (two call sites only), src/identity/repair.rs, src/identity/reconcile.rs, src/ports.rs, src/protocol/*, src/cli/mod.rs (two resolve sites only), src/cli/journal.rs, src/store/mod.rs, src/store/kept_continuity.rs, src/store/queries.rs, src/store/control.rs, src/store/topology_handoff/attempts.rs, src/service/kicks.rs, src/service/launch_gate.rs, src/harness/*, src/test_support/mod.rs, src/store/seats/observation.rs hunks from @@ -2745,20 onward, src/client/service.rs hunks other than @@ -873 and @@ -939.
- Unreviewed tests and docs: every changed file under tests/ (tests/clone_identity_sweep.rs read in part only), TRUST-POLICY.md diff (grep only), docs/operations.md, docs/programmatic-service-client.md, .config/nextest.toml, Cargo.toml (test entry only), the hermes test_bridge.py last hunk, and all 486 run artifacts under docs/superpowers/runs/2026-11-04-graph-clone-identity.
- No full-suite measurement of this branch: the caller's nextest sweep, the leak checks and `scripts/check-default-features` (ledger Task 9 suite-not-run) have not run on head.
- tests/clone_identity_sweep.rs returns early when `herdr` is absent, so whether the B1-B3 end-to-end cases ran on any machine is not evidenced here.
- Real-Herdr qualification of live harness env filtering (spec 4.2, outside this run) and the pre-release wake e2e are not run.

## Deferred OK
- Window-applied obligations shift by the whole window and parked-fact nodes keep different terminal deadlines (ledger Task 8 accepted-limit-doc); recorded in policy.
- `NotReady` on an owned pane for a keyed plain-CLI resolve during lag; transient, retryable, check-ins unaffected.
- Unconditional `clear_alert_only_if_owner_changed` query per check-in; cheap, revisit if the check-in path shows up in profiles.
- Keyless agent silenced on a key-conflict hold (ledger Task 7); worth a follow-up bead that keys the quiet path on the caller presenting a key.
- ServiceBusy kept pending for every journaled service mutation (ledger Task 12); a retryable answer becoming retryable is the correct direction; document it (Task 12 doc-drift).
- Doc drift items: programmatic-service-client.md error codes and ServiceBusy pending paragraph, operations.md provenance table and 10-minute constant (ledger Tasks 6, 12, 5, 19).
- Test hygiene items: TDD-order departures, fixture substitution by raw SQL, duplicated fixture, wall-clock budget test, HOLD_PASS trigger in the lag test, scrub tests asserting lists, missing env-read scan name, widened is_own_pane visibility, comment formatting (ledger Tasks 3, 4, 5, 7, 8, 10, 11, 13, 20).
- Design-accepted limits: repeated shell read for a never-qualifying pane, unindexed journal scan for two rare kinds, stale `restored_target_absence` row inert by generation key, audit-only retirement columns, extra probe round trip on keyed lifecycle events, `has_nonretired_seat` counting a waiting seat (ledger Tasks 5, 14, 9, 11, 4).
- Unfenced `judge_shell_candidate` read guarded by ShellContinuityGuard (ledger Task 18); the recording read is fenced.
- Replicated final retirement of a local row not clearing alert-only rows until the next check-in (ledger Task 17).
- Peer-wire freeze follow-ups: new kinds unchained in the freeze fixture and `scripts/freeze-peer-wire --check` differing on manifest at/commit lines (ledger Tasks 8, 15).
- Flaky `harness::hermes::assets` Conflict failures under parallel cargo test (ledger Task 7); route to a fixer, not a landing gate.
- `kept_unretire` this_row exclusion redundant (ledger Task 21); harmless.
