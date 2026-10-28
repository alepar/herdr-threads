super-roast verdict: clean (0 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (a local multi-agent messaging daemon) with a strict test-discipline culture. A spurious HostUnavailable on a first claim is a retryable request failure, not data loss.
inputs: bead/ht-jhcl@6480a0897 vs 6e918785b (branch base; main has since moved to 398f6da16, so `main..bead/ht-jhcl` also shows the v2.6.0 prep commit reversed)
coverage: unavailable — inline run, no coverage object; lanes covered inline: correctness, concurrency, test-causality
independence: none (inline) · rung: inline

## Change under review

`HostSessions::attach_locked` (src/service/host_sessions.rs) now publishes `locator -> host_id`
into the store's `HostIndex` before inserting the session into the registry map (was: map, then
index).

## Confirmed findings

None.

## Rejected (with reason)

- [Should-fix] src/service/host_sessions.rs attach_locked — "index-before-map opens the reverse
  window: the store accepts the host id while `HostSessions::get` misses it". Rejected: a
  concurrent `resolve` for the same locator misses the read-lock fast path and blocks on the
  attach lock, which is held until the map insert is done, so no request reaches the store via
  that locator in the window. The one store-id -> session reader (src/service/archival.rs:191)
  already handles `None` as HostUnavailable/incoherent, and the archival lane is started from
  the first-attach hook, which runs after the map insert.
- [Nit] HostIndex contract "must not hold a host whose workers are not yet running" — still
  holds: `start_session` has spawned both workers before the publish; nothing after the
  publish can fail and unwind.
- [Nit] poisoned `sessions` lock — the end state (index entry, no map entry) is the same as
  before the change; not introduced here.
- [Should-fix] test still waits only on `attached_count()` — rejected: the map write lock is
  released after the index write, and the test's read-lock acquisition synchronizes with that
  release, so `attached_count() == 1` now implies the index entry is visible. Stress evidence:
  26/553 failures before, 0/405 after, same CPU-contention harness.

## Unverified nits (spot-checked)

- [FYI] No deterministic regression test for the ordering (it would need a failpoint between
  the two publishes); the existing test plus the stress run is the evidence. Not worth a new
  failpoint for a two-line ordering.

## Escalations (need human)

None.
