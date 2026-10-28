super-roast verdict: Should-fix (2 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (herdr-threads, a local multi-agent messaging daemon). The change is test-only (tests/cli/hook.rs); its blast radius is CI reliability, not users.
inputs: ci/hermes-load@01655807f vs main@eeaa5a35f (fixes landed as ddf2dc2eb, rebased on main@94414e6d3) (stated intent: fix the load-dependent flake in continuity_gate::qualified_startup_and_clear_coordinator_retain_lifecycle_deadline, ht-wur)
coverage: 4 lanes ran (correctness, testing, test-causality-lifecycle, simplicity-design); triage, security, premortem, hot-path-perf and concurrency-async were not dispatched (a test-only diff of 28 lines) · 15 raw → 7 deduped → 0 panel / 0 spot-checked by seats; every finding was checked by the orchestrator through reproduction or by reading the code · judge completion n/a · remainder-capped: 0
independence: partial (subagent scouts, inline verification) · rung: manual fan-out

## Confirmed findings
- [Should-fix] tests/cli/hook.rs:3632: the new upper bound `seen[0].1 <= lifecycle + 2` brought load sensitivity back in the other direction. If production `budget()` is preempted between its `instant_now()` and `monotonic_now()` reads, the digest deadline rises above `lifecycle + 2`. [lanes: correctness, testing, test-causality-lifecycle, simplicity-design]
  verdict: confirmed (orchestrator: arithmetic on src/cli/hook.rs:1496 `budget()` and SystemClock in src/app.rs; three scouts found it independently)
  evidence: budget() returns mono(t2) + (deadline - instant(t1)). Any stall between t1 and t2 raises the deadline one for one, and the bound allowed only 2 ms of that.
  fix-shape hint: put the call on a held clock so both reads return the same instant. Done in ddf2dc2eb.
- [Should-fix] tests/cli/hook.rs:3618: the sibling `qualified_current_coordinator_expired_digest_never_dispatches`, and to a lesser degree `..._shares_one_tool_deadline`, have the same root cause. Their 1500 ms and 2500 ms windows start before the in-process setup in check_in_seat_with_turn, so a loaded runner refuses the digest locally and `seen` comes back empty. [lanes: correctness, testing, test-causality-lifecycle]
  verdict: confirmed (orchestrator reproduction)
  evidence: with 1600 ms of setup injected before the check-in, main fails it: "expired callback still dispatched: []" (left 0). On the held clock the same injection passes all three tests.
  fix-shape hint: same held clock, advanced only by the fake digest's delay. Done in ddf2dc2eb.

## Unverified nits (spot-checked)
- [Nit] tests/cli/hook.rs:3626: the lifecycle test did not check the call order (`seen[0].0 && !seen[1].0`). Accepted: the check was added in ddf2dc2eb.
- [Nit] tests/cli/hook.rs:3457: the anonymous `(Vec, u64)` return value was used by only one of three callers. It went away with the held clock.
- [Nit] tests/cli/hook.rs:3539: the `delay` comment ("windows are wide enough ... on a loaded machine") was stale. It was rewritten.

## Rejected (with reason)
- [Nit] tests/cli/hook.rs:3557: `LIFECYCLE_BUDGET` is used for both the input and the expectation, so the test "compares itself". Rejected: choosing the budget is `budget_for`'s job and other tests cover it (tests/cli/hook.rs:2273). This test checks only that a lifecycle check-in keeps its given deadline.
- [Nit] tests/cli/hook.rs:3632: a fresh-budget-at-digest mutant would be caught only if setup took more than 2 ms. Moot on the held clock: no time passes before the digest, so the old and fresh budgets are identical. The dispatch-extension check (`seen[1].1 <= seen[0].1 + 2`) is unchanged.

## Escalations (need human)
- None.
