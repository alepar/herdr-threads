# Apply-lane failure: source investigation and deterministic probe proposal

Source-only assessment, 2026-10-18. No builds/tests run, no tracked files changed.
Candidate HEAD: `7546897944cd4701674a640e0a7f07c83dfa7266`; comparison base:
`b4d5f9628d17f869ab5b139b7f4ae3c933d02841`.

## Observed failure

`final-verification-prepared/full-suite.log:4239` reports
`lane_fault_degrades_health_then_recovers`, tests/service/apply_lane.rs:444:
applied puts = 0, expected 2. This occurred after the test's condition
`pacer.attempts() == 0 && status.last_tick().is_some()` became true.
Run: 7652 passed, 1 failed, 48 skipped, 387.486 s nextest summary.
`perf-observation.json` records concurrent compiler processes in the peer
herdr-095 checkout, including one 255% CPU sample. That is evidence of load,
not evidence that load caused this assertion.

## Executed path and ordering

1. `tests/service/apply_lane.rs:416`: fixture creates real stores with fake
   clock, installs Apply-origin fault, authors two puts on source and ships
   them before starting worker. `src/store/journal.rs:480` insert_pending
   stores incoming entries with apply_state='pending'.
2. `src/service/workers.rs:1288` starts Apply lane with replayed=false.
   Its first pass calls `SqliteStore::replay_startup`, not apply_pending.
3. `src/store/mod.rs:444` writer entry invokes lane_fault_check; connection.rs:859
   applies the fixture fault on this lane thread. Worker records failure/backoff
   without setting replayed=true. Test observes attempts=1 and Health failure.
4. Test clears fault and advances fake clock exactly to retry due. Pacer's
   retry decision (pacer.rs:206) permits the retry; replayed is still false.
5. `src/store/journal.rs:1737` replay_startup delegates to apply::replay with
   ReplayScope::Startup. `src/store/apply.rs:1045` selects ONLY parked_unknown
   and parked_dep, so pending incoming puts do not apply in this pass.
6. Successful startup replay returns zero applied/no more; worker calls
   pacer.on_success (attempts=0), status.record_success (last_tick=Some and
   failure cleared), THEN post_commit. Only after callback does it set
   replayed=true. Next inner iteration calls apply_pending; apply_pass
   (apply.rs:980) drains ready pending heads.

Therefore the test predicate admits an actual intermediate success state
before its asserted backlog effect. This establishes a source-level invalid
completion assumption, not an executed reproduction yet. A scheduler switch
between replay success publication and pending commit can explain observed
zero without any product apply failure. Exact original failing interleaving
was not instrumented, so that attribution remains a hypothesis.

## Comparison breadth

`git diff --name-only BASE -- src/service src/store src/sync src/node
src/test_support src/protocol/time.rs src/protocol/results.rs
tests/service/apply_lane.rs Cargo.toml Cargo.lock .config/nextest.toml`
is empty. This includes worker/pacer/fair_writer/kicks, journal ingest/replay/
apply/transaction machinery, registry/facts, isolation/fake-clock dependencies,
and dependency/config inputs, rather than just unchanged test text.
`src/ports.rs` DOES change: the sole diff is optional
CreateTabRequest.directory_context. Other protocol changes concern handoff and
capabilities. The apply fixture exercises neither tab creation nor handoff;
no fixture-executed changed behavior was identified. Matching source supports
existence of the same ordering at base, but does not replace running a base probe
or establish frequency, compiler/environment equivalence, or performance cause.

## Minimal deterministic probe (proposal; parent approval before execution)

In a separate owned diagnostic checkout/copy, add a diagnostic test alongside
the existing fixture, or temporarily instrument only its worker callback:

- Preserve fault installation, source authoring/shipping, initial failure
  observation, fault clearing and fake-clock advance exactly as original.
- Start the unchanged production worker with an ApplyPostCommit callback that,
  on its FIRST successful call only, sends a bounded-channel notification
  containing progress.applied/has_more and waits on a release channel.
  Subsequent callbacks return immediately. Use recv_timeout to bound both sides.
- After advancing clock, wait for notification. Callback is after on_success
  and record_success and before replayed transition/next pending pass.
- While worker is blocked there, capture original predicate and applied_puts.
  Expected: predicate=true, applied=0, progress.applied=0, has_more=false.
- Release callback BEFORE making assertions (so panic cannot deadlock fixture
  Drop); then wait for applied_puts==2 and stop/join worker. Also require Health
  cleared. Assert captured original predicate=true and captured puts=0.
- Log captured values. Run once at candidate and once at exact base with the
  identical diagnostic-only patch, same all-features settings, and scoped
  leak/temp cleanup. No sleeps, timeout inflation or synthetic compiler load.

If both runs expose the state and subsequently apply two puts, that demonstrates
the same premature predicate at base and candidate using real production worker
and store behavior. It refutes treating the predicate as pending-drain completion.
It still does not prove the uninstrumented failed run hit this exact boundary,
nor explain suite runtime. If no zero-applied callback appears, preserve actual
progress/state and revisit assumptions rather than alter expected results.

No product/test fix is proposed or applied in this diagnostic task.
