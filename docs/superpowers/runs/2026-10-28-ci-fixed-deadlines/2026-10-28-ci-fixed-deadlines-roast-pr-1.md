super-roast verdict: Should-fix (5 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling (herdr-threads test suite); the diff is test-only, no production code, wire or store change. Blast radius: CI reliability and suite wall time.
inputs: ci/fixed-deadlines@a6f8beb6c vs main@94414e6d3
coverage: lanes correctness, concurrency-async, premortem, simplicity-design, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost (8 of 16; triage skipped, security/hot-path-perf/api/data/deploy/dependency/observability/hygiene not run: test-only diff) · 4 scouts, 42 raw → 14 deduped · verification inline by the orchestrator (no judge panel) · remainder-capped: 0
independence: partial (subagent scouts, inline verification) · rung: manual fan-out

## Confirmed findings
- [Should-fix] tests/cli/lazy_markers.rs:1062 — `lazy_markers_manifest_warning_history_and_body_render_readonly` also searches a SystemClock store (100 ms search wall budget), the ht-3wm class left unfixed [lanes: testing]
  verdict: confirmed (inline: `search control --thread t` at :1108 on the SystemClock context at :1062)
  fixed: 6aafe796c
- [Should-fix] tests/integration/no_leaks.rs:210-215 — a stale `/tmp/ih-reaper.<pid>` lock starts no reaper; the lock-only wait spent 60 s and blamed a running reaper [lanes: test-isolation-environment, correctness, premortem, concurrency-async]
  verdict: confirmed (isolated-herdr.sh:132 `mkdir ... || return 0`; header documents stale locks)
  fixed: 0a7e0688e (wait follows the reaper process; lock without reaper reported and removed)
- [Should-fix] tests/integration/no_leaks.rs:215,244 — a reaper stuck past the guard was reported but left running, untagged (leak checker cannot see it), lock left behind [lanes: test-isolation-environment, test-causality-lifecycle, premortem]
  verdict: confirmed (reaper exec'd with tags removed, isolated-herdr.sh:136-137; no kill on that path)
  fixed: 0a7e0688e (SIGKILL the reaper, remove its lock)
- [Should-fix] tests/integration/mod_delivery.rs:138-160 — `wait_progressing` had no overall bound; a slow but advancing worker ends in nextest's terminate without the attention-work diagnostic [lanes: test-suite-cost, testing, correctness, premortem]
  verdict: confirmed
  fixed: 9d21916e8 (ceiling 6 x STEP, fails through the diagnostic path)
- [Should-fix] tests/integration/mod_delivery.rs:2038-2051 — the frame drain applied to every ActivationRelay watch connection, not only the capacity flow that withholds frames [lanes: test-isolation-environment, test-causality-lifecycle, premortem, simplicity-design]
  verdict: confirmed (drain sat in the generic watch branch)
  fixed: 9d21916e8 (drain only with a CapacityFlow)

## Unverified nits (spot-checked)
- [Nit] tests/integration/latency.rs:455-466 — resend hides the refusal count on success. Fixed in a22c1037c (attempts printed in the summary line).
- [Nit] tests/integration/latency.rs:462 — resend predicate copies summary_flow's `cli_send` (stdout+stderr there, stderr here; errors go to stderr, main.rs:139). A shared helper is a follow-up, not this scope.
- [Nit] no_leaks.rs — the reaper lock path is spelled in a third place (spawn.rs:133, isolated-herdr.sh). Follow-up.
- [Nit] mod_delivery.rs — `Result<T, i64>` as a progress signal in a one-call-site helper. Kept: small and documented.

## Rejected (with reason)
- wait_progressing progress could be faked by failed passes — rejected: `completed_units` grows only on committed units (materialization.rs advance_work; work.rs), confirmed by two scouts.
- retention could prune completed jobs and make the counter non-monotonic — rejected for this fixture: UTC is frozen, retention cutoff never reached (latent only).
- unbounded drain buffer memory — rejected as a defect: bounded by test lifetime; and now limited to the capacity flow.
- second queued Attention frame blocks later frames — rejected: same ordering and credit semantics as before the change (one credit, in-order forwarding).
- drain thread join could hang — rejected: `upstream.shutdown(Both)` precedes the join and unblocks the dup'd fd; traced by the scout.
- SlowMonotonicClock makes non-search store deadlines 100x more lenient — rejected as a defect: same accepted trade-off as the q5 fixtures (6a5865ff1); leniency only.
- pid reuse before the reaper's 0.2 s owner poll — pre-existing, not introduced; the new wait now bounds and names it.

## Escalations (need human)
- ht-1tl3 root cause unconfirmed: the macOS failure log (3be83dfe3) is not available; the resend fixes a race shown under load in party sends (3 of 4 pinned runs) and proven by injection on the final send, but the failing assertion of the original run is unknown. Other load-sensitive assertions (p95 < 1 s, prompt within 60 s, follower within 20 s) are untouched.
- ht-oxdk root cause unconfirmed: same missing log; fix proven only against a slow-exit Herdr stand-in. The 60 s guard is sized from sleep-loop counts (~36 s nominal), not measured loaded wall time.
- The resend in latency.rs is a retry of a definitive refusal that the design accepts (d6ebe55a5); the owner may prefer a synchronization point over resending.
