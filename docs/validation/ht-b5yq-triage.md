# ht-b5yq intermittent-test triage

Source: `510cc4d63` on `fix/ht-b5yq`, 2026-10-18. No product code changes.
The historical failures are described in the handoff-profile run's `report.md`
and `evidence/apply-lane-investigation.md`. Its original topology investigation
and raw failing invocation are unavailable; the coordinator also could not
recover the exact topology test name.

## Apply recovery: deterministic test completion race

`service::workers::apply_lane_tests::lane_fault_degrades_health_then_recovers`
waited for `pacer.attempts() == 0 && status.last_tick().is_some()` and immediately
asserted two puts had applied. `start_apply_worker` first runs startup replay,
which replays parked rows rather than newly ingested pending rows. After this
successful pass it clears backoff and publishes health/last_tick, calls
`post_commit`, then moves to pending application. The old condition therefore
allows an intermediate state with no pending puts applied.

The test now blocks the first successful post-commit callback with channels.
This uses the real production worker and SQLite stores, with the existing fault
and fake clock. At the barrier the old predicate is true and applied puts is
zero. Keeping the old expected count of two fails deterministically; see
[evidence](evidence/ht-b5yq/apply-red.log). Releasing the barrier and waiting for
`applied_puts() == 2` checks the intended recovery effect. The final test retains
the barrier and verifies both stages, without sleeps or a larger timeout.
The release sender drops before the fixture during unwinding, so the callback
unblocks before the fixture joins its worker; the callback also has a bounded
receive. Existing callback recording remains active.

This proves an invalid test completion assumption and a concrete interleaving
that produces the historical assertion. It does not instrument or establish
the exact schedule of the historical run, or attribute it to external load.

## Topology retry: attribution remains unknown

The source search covered topology/handoff retry tests, explicit thread spawning,
and their fixtures. The bounded stress selection is every test whose full name
contains `cli::topology_handoff::tests::` or `cli::topology_recover::tests::`, and
also `retry` or `concurrent`: 25 candidates (exact names in the results JSON).
This includes `phase13_operator_created_keeps_original_request_and_allows_second_agent_retry`,
both profile-directory retry cases, automatic historical retries, and
`conflicting_concurrent_assertions_never_authorize_both_dispositions`.

Runs use the ordinary libtest binary, all selected names with `--exact`,
`--test-threads=3 --nocapture`, plus one owned Python CPU companion. This tests
same-process concurrency rather than nextest process isolation. Each invocation
has a 90-second subprocess timeout, and the companion is terminated and waited
in `finally`. The topology source stays unchanged throughout. The binary was
rebuilt during triage for the apply-only test patch; it does not exercise that
patch in these selected runs. External Rust builds were also observed, but no
causal conclusion is drawn from their presence.

Source hazards worth capturing on a recurrence:

- The `Peer` fixture defaults to a real store clock unless `hold_store_clock()`
  is explicitly used. Some retry fixtures retain that default. Child cooperative
  permits last 250 ms; elapsed issuance-to-decision time could matter under
  preemption. This is a hypothesis, not a demonstrated failure here.
- CLI cooperative budgets are five seconds. Store/file operations and fixture
  setup under concurrent load can consume wall time. Journals and databases in
  the inspected fixtures are isolated; no shared mutable environment or static
  fixture state was identified in these modules.
- `conflicting_concurrent_assertions_never_authorize_both_dispositions` uses
  a barrier and shared operation lock; its assertion already includes both
  Results. It requires exactly one winner and one canonical decision.

Proposed diagnostic if topology fails again: retain the exact test name and
full Result for **each** invocation (especially the second retry), fixture phase,
canonical bootstrap status/attempt, and elapsed permit issuance-to-decision
samples when the error is permit expiry. Prefer assertion context on the retry
fixture to a speculative product change, blanket serialization or timeout
increase. A passing bounded run cannot exonerate the unnamed historical test.

## Validation

- Deterministic red probe: fails with applied puts 0 versus expected 2 while
  the old completion predicate is true; worker released and joined.
- `nice cargo test --locked --all-features --lib apply_lane_tests`: 12 passed,
  0 failed; test execution 4.15 s, build 2m 05s.
- Fixed affected test: 30/30 isolated process repeats passed in 47.453 s,
  alongside the concurrent topology run and its CPU companion.
- Topology: 20/20 concurrent libtest batches passed, each 25 selected tests,
  500 executions total in 358.013 s summed batch wall time. No reproduction;
  no topology code or test changes. Exact names, exit codes and durations:
  [results](evidence/ht-b5yq/topology-results.json),
  [last complete batch](evidence/ht-b5yq/topology-last-batch.log).
- All-target/all-feature clippy with warnings denied: passed
  ([log](evidence/ht-b5yq/clippy.log)).
- `scripts/check-harness-neutral`: passed, zero violations.
- `cargo fmt --check` and `git diff --check`: passed.
- Read-only independent review: no issues found in the apply test diff.
- `scripts/check-no-leaked-processes --run-id ht-b5yq-20261018-triage`:
  no leaked test processes or temp files. CPU companion terminated and reaped.
  No full suite run.

Build/lint durations include concurrent external Rust builds and exceed the
repository's one-minute incremental/lint budgets in this session. They are not
quiet performance measurements; no build-speed cause or regression is claimed.
`nice` reported that setpriority was denied in the sandbox and still ran the
requested commands; their exit results above are from the actual checks.

## Follow-up: multinode seat readiness

The coordinator supplied another failure from the suite timing tab:
`config_auto_ack::incomplete_read_only_and_refused_displays_leave_the_receipt_pending`
failed in `support::send` with `send --seat ... not_found`. Main at `a0be1a5d8`
was merged first (tree-identical to the already completed apply fix).

`support::wait_seats` used `Cluster::kinds`, whose query counts every journal
row regardless of `apply_state`. Incoming `seat.created` entries are stored
pending; only the later Apply pass calls `materialize_seat_created` to insert
the remote seat. A journal count is therefore not a readiness barrier for
resolving a seat ID. The helper now counts published rows in the materialized
`seats` table, preserving its exact-count contract and excluding unpublished
local allocations. This fixes the common fixture barrier for its callers,
without changing product behavior, retrying sends, or adding sleeps.

The regression uses two real, unlinked cluster nodes with a seat each, stops
both daemons, then serves the source's signed envelopes and ingests them into
the stopped receiver through the production journal API. At this deterministic
boundary the receiver has two `seat.created` journal facts but no remote seat
projection. The original readiness predicate incorrectly returns true and the
regression fails. After the fix it stays false; executing the production Apply
pass creates the remote seat and makes readiness true. All writes to the store
in this probe happen after the cluster's daemons stop.

Evidence and checks:

- [Deterministic red output](evidence/ht-b5yq/multinode-red.log): the old
  readiness predicate releases while the remote seat is absent.
- [Focused green output](evidence/ht-b5yq/multinode-focused.log): all seven
  tests pass (new regression, four auto-ACK cases, two direct-send cases),
  with two test threads, in 20.45 s.
- [Baseline loaded runs](evidence/ht-b5yq/mn-baseline-results.json): 20/20
  repetitions of the reported test pass. This does not reproduce its literal
  historical CLI error; the deterministic probe proves the invalid readiness
  assumption and missing seat projection that can cause it.
- [Fixed loaded runs](evidence/ht-b5yq/mn-fixed-results.json): 50/50 pass.
  Both stress runs used two parallel test processes and one owned Python CPU
  companion, with per-invocation timeouts; companions were stopped and reaped.
- [All-target/all-feature clippy](evidence/ht-b5yq/multinode-clippy.log):
  warnings denied, pass in 0.89 s. Warm incremental test build: 1.71 s.
  The initial multinode target preparation took 4m 02s; it is not a warm
  incremental measurement. The sandbox's `nice` warning remains as above.
- `scripts/check-harness-neutral`, `cargo fmt --check`, `git diff --check`,
  and `nice scripts/check-default-features` pass. Independent read-only review
  found no issues.
- Scoped leak checks for `ht-b5yq-multinode-mn-baseline`,
  `ht-b5yq-multinode-mn-fixed`, and `ht-b5yq-multinode-focused` found no
  leaked processes or temporary files. No full suite run.
