super-roast verdict: Should-fix (1 confirmed) [low coverage] [review readiness incomplete]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (Rust CLI and daemon for agent message threads). The change is test-only: tests/** and #[cfg(test)] modules of src/harness/cache.rs, src/harness/hermes.rs and src/sync/discovery.rs. Blast radius is CI reliability; no shipped binary changes.
inputs: ci/fixture-budgets@ad83fa900 vs main@103e9fbec (stated intent: ht-bnb0, per-call CallBudgets where one real-clock deadline spanned several setup/test calls; topology_activation readiness wait requires the daemon's own reconciled epoch)
coverage: lanes ran 13/13 (core correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async; triage-activated testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, observability, api-contract, deploy-safety) · 46 raw → 13 deduped (dedupe done by the orchestrator, not a dedupe agent) → 5 panel / 8 spot-checked · judge completion 100% · remainder-capped: 0 · reviewReadiness not supplied
independence: same-family (Claude) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 5 · reproduce 2/3/0 · refute 3/2/0 · ground 3/2/0 (C/R/U)

## Confirmed findings
- [Should-fix] tests/integration/topology_activation.rs:460 `wait_for_daemon_reconciliation` — the stricter predicate (recovery_epoch > seeded epoch) puts the daemon's startup observation and reconciliation of the seeded seats under a fixed 5 s window. That is a new hard bound on daemon work under suite load, while the sibling wait in tests/handoff_topology_cli.rs uses WAIT = 15 s for the same predicate. [lanes: premortem, correctness, concurrency-async, test-isolation-environment, testing, hot-path-perf, test-suite-cost, test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ Nit / refute ✓ Nit / ground ✓ Should-fix)
  evidence: `let until = Instant::now() + Duration::from_secs(5)` vs handoff_topology_cli.rs:42 `const WAIT: Duration = Duration::from_secs(15)`. The author's measurement shows the old wait already returned at (2, 2) in the normal case, so the normal-case cost is unchanged; the loaded case is what the bound now exposes.
  fix-shape hint: same bound as the sibling wait. Fixed: 15 s, with a comment naming the sibling.
- [Nit] src/harness/cache.rs:955 `completed_journal_read_preserves_saved_bytes` — the first real-clock `budget` is reused after the cooperative page run for the `NotFound` read, and `read_completed` checks the deadline before NotFound (src/harness/context.rs:742-750). The sweep converted only `live`. [lanes: testing, test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✓ / ground ✓)
  evidence: post-fix injection (5.1 s sleep before the read): the old shared budget failed with left: DeadlineExceeded, right: NotFound, and the fresh `live()` passed.
  fix-shape hint: `live()` for the NotFound read; the cancelled control keeps `budget` (cancellation is checked first). Fixed.
- [Nit] tests/integration/topology_activation.rs:995 — the second `wait_for_daemon_reconciliation()` after `Fixture::new()` is redundant. [lanes: api-contract, simplicity-design, correctness, test-suite-cost, testing]
  verdict: confirmed (refute spot check)
  fix-shape hint: remove. Fixed.
- [Nit] tests/integration/topology_activation.rs:460 — the readiness predicate is duplicated verbatim from tests/handoff_topology_cli.rs (and the unmerged golden_surfaces copy). [lanes: simplicity-design]
  verdict: confirmed (refute spot check)
  fix-shape hint: a test_support helper once the golden_surfaces branch lands. Follow-up, not fixed here.
- [FYI] sweep-wide — about 16 hand-written `|| CallBudget {..}` closures, with no shared per-call constructor. [lanes: simplicity-design]
  verdict: confirmed (refute spot check)
  fix-shape hint: follow-up, e.g. a test_support constructor taking (clock, ms).

## Rejected (with reason)
- [B] wait lacks a daemon liveness check — rejected 2-1 (the ground seat called it a pre-existing Nit). The old inline loop had the same shape; a dead daemon still fails loudly, and the timeout names the last row.
- [F] tests/handoff_topology_authority.rs:254 still shares one budget — rejected 2-1. The two calls sharing it are back to back. The only call after the second Fixture::new is Capabilities, and a 5.1 s injection there did not fail the old test 3/3.
- [H] refusal checks still assert bare is_err() — rejected 3/3. Pre-existing; per-call budgets remove the accumulated-spend cause; pinning error codes is separate hardening.
- [C] swallowed query errors / busy_timeout overrun — rejected: the error is printed in the timeout message, and transient busy is correctly "not ready".
- [D] unfiltered herdr_hosts query_row — rejected: one host row by construction, same shape as the sibling.
- [J] connection epoch bound to the seeded epoch — rejected: NativeCli sets connection_epoch = epoch (src/host/native.rs:2534), and the sibling seed does the same.
- [L] per-call budgets drop the aggregate bound — rejected: the aggregate bound was the defect; nextest slow-timeout remains.
- [M] readiness is point-in-time, not quiescence — rejected: speculative, no epoch-bump path in these tests.

## Unverified nits (spot-checked)
none

## Escalations (need human)
none
