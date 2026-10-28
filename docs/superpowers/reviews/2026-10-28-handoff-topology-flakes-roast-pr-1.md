super-roast verdict: clean (3 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling (herdr-threads); the diff is test-fixture code only (tests/handoff_topology_cli.rs), no production path, so blast radius is the test suite's reliability.
inputs: ci/handoff-topology@5c7a7aff9 vs main@94414e6d3
coverage: unavailable — manual run, no coverage object · lanes ran: correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async + triage-activated testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost (10/10 scouts returned) · raw 31 → deduped 9 · judge completion: verification inline by the orchestrator, no judge panels dispatched
independence: partial (subagent scouts, inline verification) · rung: manual fan-out

## Confirmed findings
- [Nit] tests/handoff_topology_cli.rs readiness wait — a timeout said only "timed out" and `.unwrap_or(0)` turned every read error into "not ready", so a timeout could not be diagnosed (the fix's own proof needed uncommitted instrumentation) [lanes: simplicity-design, concurrency-async, premortem, testing, test-isolation-environment]
  verdict: confirmed (inline: read of the code at 5c7a7aff9)
  fix: 447f4c9b8, the wait keeps and prints the last (recovery_epoch, reconciled_epoch, boot match) row or error with the seed epoch; checked by forcing the condition false.
- [Nit] tests/handoff_topology_cli.rs Fixture::seed — the seed returns its published epoch for the wait but the seat/binding INSERTs still hard-coded epoch 1 [lanes: simplicity-design, correctness, testing, test-causality-lifecycle]
  verdict: confirmed (inline; latent, the seed publishes epoch 1 today)
  fix: 447f4c9b8, the INSERTs bind the published epoch.
- [FYI, preExisting] tests/handoff_topology_cli.rs owned host stub — `lose` / `lose_after_publication` faults are consumed by whichever client sends the matching request first, the daemon included [lanes: test-isolation-environment]
  verdict: confirmed as source risk; not observed: peer-tagged request logs (SO_PEERCRED, experiment) of p1h5/8zt runs show no daemon pane.get in those tests; daemon pane.get only follows CLI requests that need it. Left as is.

## Rejected (with reason)
- Readiness now waits for the daemon's whole startup under the fixed 15 s WAIT, so the flake could move to a readiness timeout (testing, premortem, test-isolation-environment, test-suite-cost). Rejected on evidence: 138 post-fix file runs (4 loops x 16 threads, load 70-90) and 116 runs with the readiness fix alone had zero readiness timeouts; whole-file wall time unchanged (29 tests in 11.3-12.3 s, as before). The 15 s bound is measured from the wait's start.
- Per-call seed budgets remove the seed's aggregate bound (hot-path-perf, correctness, premortem, testing). Intended: one deadline across unbudgeted schema creation was the defect (organic failure in the hunt, reproduced by injection). The per-call bound is unchanged; the nextest hang guard and the call count (7-8) bound the total.
- Readiness proves only the daemon's first epoch; a later epoch bump (fenced host-call failure) could still rewrite recipient rows mid-test (correctness, concurrency-async, premortem, testing, test-causality-lifecycle). Not introduced by this diff and not observed in any hunt; the causality scout confirmed same-epoch passes leave the recipient unchanged. Recorded as a residual risk in the hand-off, with the serial stub host as its amplifier.
- Fresh SQLite connection per 10 ms poll (hot-path-perf, test-isolation-environment): pre-existing pattern; the wait lasts milliseconds longer than before.
- No committed regression guard for the injected delays (testing): an injection knob would add test-only surface to the stub; the proof is recorded in the commit messages instead.
- Readiness depends on the daemon resuming above the persisted epoch (premortem): true by construction (resume_after_epoch fetch_max(persisted+1)); a change there now fails loudly with the named row instead of silently.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
