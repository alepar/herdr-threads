super-roast verdict: clean (1 nit) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling (herdr-threads); the diff is test-fixture code only (tests/golden_surfaces.rs), no production path, so blast radius is the test suite's reliability.
inputs: ci/golden-fixture@c08fb0cc5 vs main@94414e6d3
coverage: unavailable — manual run, no coverage object · lanes ran: correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async + triage-activated (inline triage: test-only diff) testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost (10/10 scouts returned) · raw 22 → deduped 13 · judge completion: verification inline by the orchestrator, no judge panels dispatched
independence: partial (subagent scouts, inline verification) · rung: manual fan-out

## Confirmed findings
- [Nit] tests/golden_surfaces.rs readiness wait — the "owned daemon exited" and timeout panics carry no daemon log; the daemon's stderr is in the isolation root, which `TestIsolation::drop` removes while the panic unwinds (src/test_support/isolation.rs:279-283) [lanes: premortem]
  verdict: confirmed (inline: read of the drop path; daemon `run` writes its log to process stderr, src/daemon/logs.rs:87)
  fix: 717175fbe (after rebase onto main), both panics append the last 4 KiB of daemon.stderr; checked once by forcing the condition false.
- [FYI, preExisting] tests/golden_surfaces.rs vs tests/handoff_topology_cli.rs — the topology seed and readiness fixture is a hand copy in two modules of the same binary, so every fixture fix needs a second port (the reason for this bead) [lanes: simplicity-design, testing]
  verdict: confirmed; out of scope (a shared test_support helper is a follow-up, not this fix).
- [FYI, preExisting] the reconciled-marker rule is restated as raw SQL in the fixture (and in hook_entrypoint, topology_activation, trust_policy) instead of a store query [lanes: simplicity-design]
  verdict: confirmed; not introduced by this diff.

## Rejected (with reason)
- The readiness query reads one `herdr_hosts` row with no WHERE, unlike the old `count(*)` over any row (correctness, concurrency-async, testing, premortem, test-causality-lifecycle, hot-path-perf note). Rejected on evidence: `attach_host` upserts `ON CONFLICT(endpoint)` (src/store/hosts.rs:48), and the seed and the daemon attach the same endpoint, so the table holds exactly one row; the same shape is upstream (447f4c9b8).
- Readiness depends on the daemon publishing an epoch newer than the seed's; if not, every Topology waits 60 s (testing, premortem). True by construction: the daemon resumes after the persisted epoch (src/host/native.rs:801, 817); a change there now fails loudly with the named row, as the handoff roast concluded.
- A later epoch bump could still rewrite rows mid-test (concurrency-async). Not introduced by this diff, no trigger found in this file; the goldens carry no epoch or structural fields. Same residual risk as the handoff roast.
- No committed regression guard for the injected delays (testing). An injection knob would add test-only surface to the stub host; the proofs are recorded in the commit messages (as in the handoff fix).
- The inline readiness loop duplicates `wait()` (simplicity-design, testing). Kept to match the upstream fixture line for line, so the two copies stay comparable until they are merged.
- Two deadline closures with a bare 5000 (simplicity-design). They read different clocks (host system clock vs the injected store clock); the per-call bound is unchanged from before.
- `Seeded { epoch, turns }` (simplicity-design). A named struct is clearer than a tuple; each field has its consumer.
- Per-call budgets remove the seed's aggregate bound (premortem). Intended: one deadline across unbudgeted schema creation was the defect (reproduced by injection); per-call bound unchanged, nextest's hang guard bounds the total.
- Extra daemon-startup latency per fixture (premortem). Measured: the file's 14 tests ran in 7.1 s on the fixed fixture, and the 4 Topology tests in 5.8-7.4 s with the old one.
- SteppingClock extra reads change `turns == 3` (concurrency-async). The budget read happens before each call, outside the writer turn's quantum; `seeding_stages_the_rest_after_a_spent_writer_quantum` passes.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
