# Flaky-test mechanisms, remedies, and herdr-threads audit

The observed failures have different causes: contradictory hook/wake expectations, a lifecycle close hidden by an in-flight refusal, subprocess startup guards, and a fixture that confuses daemon health with host attachment. Fix the causal boundary in each case. Longer sleeps, blanket serialization and retries do not establish those boundaries.

## Established local causes

| Flake | Causal failure | Remedy and reproduction |
|---|---|---|
| ht-223y, clear unread mail | SessionStart completely displays and ACKs short mail. A later idle wake is unnecessary, but the test requires one. Which operation wins depends on scheduling. | Separate short in-turn ACK from long pending-mail wake. Keeping the pane busy until hook completion reproduces the original timeout (21.67s). |
| Current V2 product ht-x0b9 (investigated from ht-7ykh) | Pull or Confirm can observe the ended binding before the consumer reads the queued lifecycle Close. Generic RPC refusal hides `binding_changed`. | On typed presentation refusal, consume the terminal frame within the existing one-second stream read bound. Preserve its reason/exit, never confirm a refused page. Deterministic unit sequence failed exit 1 vs expected 0 before fix. |
| ht-xrmw, both lifecycle crashed_* tests | Two five-second subprocess readiness/election guards encode a host scheduling assumption, while neighboring convergence tests already use LIVENESS. | Reuse the existing 60-second liveness guard for startup, fixture gate and shutdown. Keep metadata/identity assertions unchanged. Controlled six-second launcher delay reproduced both exact failures, then both passed with the new guard in 6.68s. The artificial delay is removed from final code. |
| ht-bmh5, config restore | A previously admitted peer pump can win daemon election store-only after resync. `daemon ensure` verifies health but does not attach its endpoint to an existing daemon. Own seat facts park on missing host; matching vectors are insufficient. | Explicitly attach the fixture's host after ensuring startup, before releasing peer admission. A deterministic existing-store-only-winner test failed host count 0 vs 1 before fix. No journal replay change is needed. |
| ht-41yy, multinode sweep follow-up | A send committed on A is followed by an immediate inbox read on B; the ACK path likewise checked only A's pending receipt. | Await B's applied message before inbox and B's pending receipt before ACK. Controlled partition reproduced the exact assertion; reconnect plus the wait passed. Test-only, no product defect established. |
| ht-2b3p, relay pair opener follow-up | C's send can be interleaved with incoming pair/auto-join facts; a changed audience snapshot is intentionally refused. Exact hosted revision/interleaving unconfirmed. | Establish independent opens by isolating C-B and observing backoff before A/C opens; assert C has no pair yet, then heal and retain every relay/convergence oracle. Baseline20 under CPU load passed; controlled missing-precondition oracle failed. |

The interventions and observed outputs are preserved in [local reproduction evidence](reproductions.md) [10]. The older v1.2.0 ht-7ykh schedule was not reconstructed; the current V2 ordering defect is tracked separately as ht-x0b9, and ht-7ykh remains open. The logs support these causal explanations; a simulated startup delay proves the guard weakness, not the exact CPU schedule of the historical run. Original CI evidence: run 39002761176, job 117308971075. Local overlapping-suite log: sweep9-out.log from the coordinator's retained evidence. Independent code review found no must-fix issues. Focused and full verification are recorded separately in [validation](validation.md).

## Research: patterns and effective remedies

<!-- facet: F1 -->
<!-- facet: F2 -->
<!-- facet: F3 -->
<!-- facet: F4 -->

| Pattern | What to prove | Effective response | Primary evidence |
|---|---|---|---|
| Async observation mistaken for completion | Which event actually precedes the assertion? stdout emission, confirmation, actor processing and process exit are distinct. | Use entered/release/completed handshakes and canonical state witnesses. | [Google triage](https://testing.googleblog.com/2021/03/test-flakiness-one-of-main-challenges.html), [FlakeRepro](https://www.microsoft.com/en-us/research/publication/flakerepro-automated-and-efficient-reproduction-of-concurrency-related-flaky-tests/) |
| Missing or expired fixture prerequisite | Does the resource still exist and remain held when the tested operation commits? | Hold lifetime explicitly; verify attachment/readiness separately from process health. A watchdog expiry fails setup. | [Google unique resources](https://testing.googleblog.com/2008/04/tott-avoiding-flakey-tests.html), [pytest isolation](https://pytest.org/en/latest/explanation/flaky.html) |
| Wall time used as synchronization | Is elapsed time the contract, or only a scheduling guess? | For liveness, await the exact condition with a generous bounded hang guard. For real latency contracts, retain the bound and arrange appropriate runner scheduling. | [Google triage](https://testing.googleblog.com/2021/03/test-flakiness-one-of-main-challenges.html), [Nextest groups](https://www.nexte.st/docs/configuration/test-groups/) |
| Shared state across test processes | What is truly shared across processes and runs? | Prefer private paths, endpoints, config and owned children. Use test groups only for unavoidable shared resources or measured scheduling contracts; an in-memory mutex cannot serialize nextest processes. | [Nextest groups](https://www.nexte.st/docs/configuration/test-groups/), [pytest isolation](https://pytest.org/en/latest/explanation/flaky.html) |
| Virtual-clock completion assumptions | Has the actor or off-runtime worker processed work after time advanced? | Await completion/fences; distinguish Tokio time from std time. | [Tokio advance](https://docs.rs/tokio/latest/tokio/time/fn.advance.html), [Tokio pause](https://docs.rs/tokio/latest/tokio/time/fn.pause.html) |
| Rare concurrency schedules | Can the failing interleaving be controlled and replayed? | Force a schedule with barriers/fault injection first. Stress tests add evidence; pure synchronization models may suit Loom. | [FlakeRepro](https://www.microsoft.com/en-us/research/publication/flakerepro-automated-and-efficient-reproduction-of-concurrency-related-flaky-tests/), [Loom](https://docs.rs/loom/latest/loom/) |

Primary-source evidence for the mechanism/remedy table is indexed in the bibliography: synchronization [1], empirical recurrence [2], process isolation [3], controlled interleavings [4], modeling limits [5], clock completion [6], cleanup [7], resource uniqueness [8], and clock domains [9].

The empirical Microsoft lifecycle study identifies asynchronous calls as a leading source in its six studied projects and warns that claimed fixes can fail to reduce recurrence. This supports repeated before/after validation, not a universal prevalence estimate for Rust or this repository. [Microsoft study](https://www.microsoft.com/en-us/research/publication/a-study-on-the-lifecycle-of-flaky-tests/)

Counterchecks matter: Loom requires instrumented synchronization and cannot directly model arbitrary system calls. Tokio clock advancement does not guarantee timer consumers have completed. Nextest groups coordinate one runner, not independent overlapping runner processes. Retries can help reproduce/classify a flake but a later pass does not repair its cause. These limits rule out a blanket “replace waits with fake time” or “serialize everything” remedy. [Loom](https://docs.rs/loom/latest/loom/), [Tokio advance](https://docs.rs/tokio/latest/tokio/time/fn.advance.html), [Nextest groups](https://www.nexte.st/docs/configuration/test-groups/)

## Repository-wide applicability

The audit broadly scanned tests, embedded tests across src, fixtures, scripts, integrations and runner configuration, then reviewed suspicious causal windows. It did not semantically inspect every line or dynamically confirm every candidate. The async portion alone scanned 841 files / 530,616 lines; this is lexical coverage, not a count of confirmed defects. Detailed evidence: [core and scripts audit](audit-core.md) and [async candidate data](audit-async.json).

High-confidence static candidates, separate from reproduced failures:

| Location at audit checkout | Risk | Proposed confirmation/remedy |
|---|---|---|
| tests/integration/agy_sweep.rs:364 | Observation mark is taken after wake-on(all) can already trigger the only wake. | Force wake before mark; capture mark before action. |
| tests/host_adapter.rs:777 | Accept plus 40ms is assumed to imply a partial request write. | Pause client before encoding; establish bytes present without draining before cancelling. |
| tests/store/sync_contacts.rs:390 | Canceller's timer starts before the parent's measured interval. | Pause parent before Instant; assert admission wait causally instead of elapsed lower bound. |
| tests/sync/incarnations_worker.rs:995 | Startup pass can satisfy an assertion attributed to own-history release. | Await initial worker completion, then release and observe the correlated kick. |
| src/harness/hermes/assets.rs:1436 | Fixed ten-second lock holder can exit before parent tests contention. | Gate release from the parent; expiry fails setup. |
| src/harness/hermes/runtime.rs:682 | Three-second cancellation watcher can expire before child entry. | Await child entered, then explicitly cancel. |
| integrations/hermes/test_bridge.py:669,716,1148 | Child request publication followed by .3s sleep does not hold the child live through unload/reload. | Use explicit child hold/release and join. |
| integrations/hermes/test_bridge.py:1133 | A .1s negative observation races the real one-second background refresh. | Freeze/advance the existing injected clock and await the exact iteration. |
| scripts/agy-gate/flows/delivery.py:46; flows/wake.py:77 | Five/nine-second fake responses can finish during the coordinator's CLI calls. | Hold fake-server response until all sends commit. |
| scripts/wake-e2e.py:1119,1188 | Busy status plus sleep does not prove tool entry or continued activity at send publication. | Tool entered/release barrier spanning actual publication. |

Positive controls already present include owned child/process cleanup, per-run scratch paths, private endpoints/config, causal server-completion helpers, virtual clocks and nextest groups. The audit did not establish a new core cross-process global-resource collision. Existing 100ms latency contracts and correctly stepped timer tests should remain strict. Lower-confidence negative-observation and budget candidates are documented in the detailed audits rather than labeled confirmed flakes.

## Feedback for Superpowers main

Feedback sent to the verified Superpowers main seat sveEUb5xA (wB:p8), thread tpW0uVN5m, message m1g04D5Gh. The [sent feedback](feedback.md) is preserved here.

Extend super-roast with a dedicated flaky-test scout lane, after its own comprehensive primary-source research into the usual suspects. This report supplies motivating cases and an initial vocabulary; it is not an exhaustive taxonomy or a ready-made scout specification.

The new lane should inspect tests and fixtures alongside changed production code, reconstruct the happens-before graph, distinguish observed entry from completion and continuing lifetime, classify liveness guards separately from performance contracts, and inspect runner process semantics. Its broader research should also cover test order/global state, random seeds and unordered iteration, real/virtual clock boundaries, resource leaks/exhaustion, filesystem precision/permissions, locale/timezone/platform differences, eventual consistency, external-service dependencies, probabilistic assertions and overstrict numerics. Those broader families are research directions, not findings established in this checkout.

Require every scout finding to include a concrete scheduling/environment counterexample, the relevant contract, an exact file/line, confidence, a minimal deterministic reproduction and a remedy preserving the oracle. “Contains sleep” is insufficient. Include positive controls and distinguish reproducible defects, static risks and untested hypotheses. Reject fixes that silently retry to green, skip failure paths, inflate every timeout, weaken assertions, or serialize independent fixtures without evidence. Judge/review should check that the proposed fix preserves what the test is meant to prove.

## Methodology and limits

Standard research used the installed search CLI for preflight, then web.run search/open after all six CLI providers failed on network access. Twelve search attempts (one failed CLI probe, eleven web queries), nine distinct primary documents read at least partially, and two failed original-document opens are recorded in the ledgers. Per-query considered-result counts are unknown because merged web responses did not preserve reliable attribution; no estimated hit counts are reported. Failed opens: Google 2017 “Where do our flaky tests come from?” and the guessed Nextest stress-testing URL; neither supports a claim here. No DOI checks were needed.

Retrieval stopped at the bounded search budget (`budget-exhausted`), not coverage saturation. Each facet has relevant direct evidence, but exhaustive taxonomy, cross-language empirical breadth and two post-readiness saturation probes remain incomplete. The Superpowers owner must perform that broader research for its independent scout design. Static audit candidates remain unexecuted. Authenticated live wake/agy gates were inspected as source only.

## Cold-host startup follow-up

The multi-host hook flake exposes another distinction: published capture versus completed reconciliation. A temporary service-side diagnostic reproduced3/20 failures and captured `ServiceBusy: recovery reconciliation of the current host epoch has not finished; retry`. [10] The initial test-only correction followed the existing policy but left a real fresh-startup enrollment gap. The user blocked2.0.0 on fixing that gap. The product correction now completes a genuinely empty saved-seat pass atomically with coherent publication; hosts with saved seats still require worker reconciliation, and C1 retry/enrollment guards remain intact.

Worker-free regression `cold_empty_host_continuity_is_decidable_before_the_worker_pass` failed deterministically before the product fix: expected confirmed NotFound but received ServiceBusy with the exact historical reconciliation refusal. [10] It passes after the fix, and the cold integration test again requires enrollment on the first callback. See [contract and reproduction analysis](reproductions.md#product-correction-atomic-empty-recovery-pass-ht-bryo) and [qualification results](validation.md).
## 2.0.2 backlog follow-up (2026-10-17)

Checkout: `.worktrees/flake-backlog`, branch `fix/flake-backlog`, base `92edebf67`.
This follow-up dynamically exercises the previously static candidates. All new
changes are test fixtures, test helpers, gate drivers or diagnostics; no new
product defect was established. Original assertions and latency contracts remain.

### Bead dispositions and controlled evidence

| Bead | Mechanism and result |
|---|---|
| ht-2si | Already fixed by `53179a94f` (ht-54r.73). `src/cli/mod.rs:2117` sends a subagent without its own identity down the read-only wire path before `role_claim`. The unchanged `lazy_display::lazy_display_json_machine_explicit_selector_readonly` passes 20/20 on a freshly compiled binary under four owned CPU workers. Its existing no-completion/no-ACK/no-lazy-progress assertions remain. Closed with evidence; no code change. |
| ht-86a | `src/store/seats.rs` legitimately returns a committed prefix after its 5ms quantum. A one-shot fixture then seals too early; a fixed 16-target offset can instead skip targets and fail the contiguous-slice guard. A 35-target stepping-clock regression failed deterministically with `StaleHostObservation: snapshot slice is not contiguous`. `test_support::snapshot::stage_all` now follows `visited` and the exact staged count, using the original stage and budget. Thirteen fixture sites across eight files use it. Intentional partial, duplicate, quantum and reopen controls stay explicit. Focused checks: 123; regression plus three identity fixture tests: 20 repetitions each, 80 checks. Four bootstrap-filter tests passed outside the sandbox, including all three native bootstrap fixtures. |
| ht-7ykh | The original clear/rebind case passes 20/20 under four CPU workers. The new relay holds a real Watch Pull before daemon admission, completes `/clear`, then releases it: the ended binding returns typed `NoBinding`, and the queued lifecycle Close remains `binding_changed`/exit 0, with mail pending and restreamed/ACKed in the new session. This complete new chain passes 20/20 under four CPU workers; relay helper controls pass 5/5 and the existing real capacity test passes. Contrary to the earlier historical qualification above, immutable v1.2.0 already advertises and prefers V2 presentation; a legacy-only path was not established. The preexisting ht-x0b9 unit RED (`exit 1` versus `0`) covers the generic refusal/Close ordering. This does not reconstruct the exact historical CI schedule. |
| ht-81q | Twenty loaded all-lanes runs plus one fresh build run retain and pass the original three 100ms commit-to-attempt assertions; the historical failure remains unreproduced. Its assertion was never captured, so no production latency cause is claimed. CI and full-suite workflows now preserve `HT_SWEEP_PANIC_FILE`; the hook is installed before Scene setup and restored after the daemon replaces it. A controlled `PATH=/nonexistent` startup failure exits 101 and persists its missing-Herdr panic, proving capture before Scene completion. A successful fresh sweep persists all three `SWEEP_TRACE` records, appended outside the measured interval. Existing nextest full-slot isolation is unchanged. |

### ht-fmww: all ten candidate families

| Family | Controlled schedule and causal correction |
|---|---|
| agy wake observation | `tests/integration/agy_sweep.rs`: let wake-on(all)'s wake complete before taking the old observation mark. Capture the mark before the action so the sole legitimate wake remains observable. |
| Partial socket write | `tests/host_adapter.rs`: a throwaway Clock gates the actual post-connect witness check (`src/host/transport.rs:202`), after the ping exchange, while the server proves no request bytes exist. The old accept-plus-40ms canceller then fails the original `!bytes.is_empty()` assertion 4/4. A pause inside the final write check can legitimately allow one chunk because its cancellation check already ran; an initial clock-ordinal gate therefore was not deterministic and was replaced by the actual boundary witness. The server now observes bytes with `MSG_PEEK` without draining them, cancels, and keeps the request unread until the client returns; the original partial-write/no-submission assertions remain. |
| Admission wait interval | `tests/store/sync_contacts.rs`: delay the parent 150ms after starting a canceller with a 100ms timer; the original >=90ms assertion fails. Start that same timer only after the writer is actually queued. The elapsed-time assertion remains unchanged. |
| Own-history kick attribution | `tests/sync/incarnations_worker.rs`: gate startup until after history release and replace the release callback with a no-op. The original test still passes (0.66s), proving a false positive. Await the startup idle event before release and require a new completed idle event afterwards. With the same no-op callback, the strengthened original pass assertion fails after its existing 5s guard. Restore the real callback in final code. |
| Hermes physical lock lifetime | `src/harness/hermes/assets.rs`: pause the parent 10.2s after lock-ready. The original ten-second child exits and install incorrectly succeeds for the test's intended contention point (`Ok` versus `Conflict`). Keep its stdin pipe owned by the parent until explicit stop; the same delayed schedule then passes. |
| Hermes cancellation monitor lifetime | `src/harness/hermes/runtime.rs`: pause 3.2s after starting the old three-second watcher. It expires before child entry and yields `Deadline` instead of `Cancelled`. Watch actual child entry until capture completion, then join. The same delayed schedule passes. |
| Hermes callback ownership | `integrations/hermes/test_bridge.py`: delay the coordinator .4s after child request publication. Three tests receive successful context before the generation/unload action and fail. Hold the actual child until explicit release, freeze the already injected bridge clock for these ownership tests, and release/join in `finally`. |
| Hermes refresh cadence | In the same Python suite, extend the .1s negative observation to 1.1s: a legitimate refresh gives two reads instead of one. Freeze and advance the injected clock, fence the reader's next cadence decision, and await exact failed-config publication. Controlled Python cases pass 4/4; their 20 focused repetitions pass 80/80; the complete bridge suite passes 43/43. |
| agy fake busy response | Replay the original response with a 20ms delay and resume publication after 50ms: 20/20 failures (`busy response expired before send commit`). The fake server now supports explicit held-entry/release with expiry classified as infrastructure failure; delivery, wake and equivalent downgrade flows span all sends with that hold. |
| Release driver busy tool | Replay the original `Recipient.busy_case` with its old sleep compressed to 20ms and publication delayed 50ms: 20/20 failures (`tool exited before send commit`). `BusyBarrier` atomically publishes actual tool entry and holds it through send/invite completion. An independent review found the initial non-atomic PID witness race; the final helper uses write-then-rename and reaps failed-test children. |

The two driver families' final regressions pass 20 repetitions, 160/160 checks.
Complete offline suites: fake Gemini 90/90 (3.558s), agy gate 140/140 (2.478s),
exact-final wake driver 147/147 (83.835s). No authenticated live agents were run;
these results validate fixture/driver boundaries, not a new live release qualification.
The wake cleanup work on `fix/reenroll-and-e2e-root` touches disjoint functions.

Final clean-source Rust repetitions also pass 20/20 each for partial writes,
admission waiting, own-history release, native agy wakes and the channel's
negative native-wake observation. The partial-write fix additionally passes
20/20 with the original failing post-connect delay still forced.

### Integrated qualification

The full run used `HT_LEAK_RUN_ID=F37976EA-4619-4C71-A479-4933D9C3D78F`:
`nice cargo nextest run --locked --all-targets --all-features`. It ran 7618
tests: 7617 passed, one failed, 48 were skipped. The sole failure was the
inventory's stale exemption for the deleted Hermes ten-second sleep
(`tests/service/loop_inventory.rs`), not a runtime or latency assertion.
Removing that obsolete tuple preserves the scanner and every assertion; all
three `loop_inventory` tests subsequently pass. Independent review confirmed
the correction does not weaken the inventory.

Wall time was **310.244s**, exceeding the 300s budget. The final release
failpoint proof took 269.983s; the test-phase start-to-proof completion includes
the preceding exclusive timing checks. This is an observed budget miss, not
evidence that the fixture changes caused a performance regression. No budget
or timeout was raised. The coordinator was informed and explicitly directed
no second full suite on this branch: the merged 2.0.2 candidate receives full
qualification. This branch does **not** claim a green full suite or a satisfied
full-suite speed budget.

After the full run, `scripts/check-no-leaked-processes --run-id
F37976EA-4619-4C71-A479-4933D9C3D78F` exited 0: no leaked test processes or
scratch files. Full output is `.tmp/flake-backlog/full-suite.log` in the retained
worktree. Both clippy variants, the default-feature warning guard, formatting,
harness neutrality, workflow assertions and actionlint passed before the
inventory correction. On the final corrected source, `loop_inventory` passes
3/3 (29.15s incremental compilation, 0.13s execution), all-feature clippy passes
in 25.63s, default-feature clippy passes in 23.88s, and
`check-default-features`, `check-harness-neutral`, `cargo fmt --check` and
`git diff --check` pass. Both independent review scopes report no remaining
actionable findings. All controlled-delay and missing-kick mutations are removed.

New model-visible fixture text: `BUSY_PROMPT` is 188 fixed bytes plus the tag and
quoted Python/helper paths; the helper code stays in a scratch file. The fake
expiry diagnostic is `FAKE-GEMINI-HOLD-EXPIRED ` (25 bytes) plus the step label.
Neither adds repeated per-message or per-turn explanation.

Local execution records (retained, not external research):

- `.tmp/flake-backlog/lazy-display-fresh-20.log` in this checkout.
- `/tmp/ht-86a-{before,focused,focused-more,repeat20}.txt`.
- `/tmp/hermes-flake-evidence/`: original snapshots, delayed-schedule patches,
  Rust RED/GREEN logs, Python RED/GREEN/repeat logs. Temporary delays are removed
  from final source.
- `/private/tmp/ht-rust-evidence/`: controlled Rust schedules and oracle mutation evidence.
- `/private/tmp/watch-wake-ba9fcd36-1dc3-490d-ba74-980c686983df/`: 20 loaded original
  watch and all-lanes runs, results and scoped cleanup log.
- `/private/tmp/watch-wake-f0cb3a9a-3f6a-44d8-9453-22f881d9a7a3/`: 20 fresh held-Pull
  regressions and scoped cleanup; `/private/tmp/watch-wake-fresh-sweep-diagnostics.log`
  retains the three successful timing traces. Controlled setup-panic records are
  in the first watch/wake evidence directory.
- `/private/tmp/repro_agy_old_hold.py`, `/private/tmp/repro_wake_old_busy.py`,
  `/private/tmp/repeat_live_gate_selftests.py`,
  `/private/tmp/live-gate-wake-selftest-exact-final.log`.

## Bibliography

[1] [Google: flakiness triage and remedies](https://testing.googleblog.com/2021/03/test-flakiness-one-of-main-challenges.html) — primary source, retrieved 2026-10-17.

[2] [Microsoft: lifecycle of flaky tests](https://www.microsoft.com/en-us/research/publication/a-study-on-the-lifecycle-of-flaky-tests/) — primary source, retrieved 2026-10-17.

[3] [Nextest: mutual exclusion and rate-limiting](https://www.nexte.st/docs/configuration/test-groups/) — primary source, retrieved 2026-10-17.

[4] [Microsoft: FlakeRepro](https://www.microsoft.com/en-us/research/publication/flakerepro-automated-and-efficient-reproduction-of-concurrency-related-flaky-tests/) — primary source, retrieved 2026-10-17.

[5] [Loom: deterministic concurrency testing](https://docs.rs/loom/latest/loom/) — primary source, retrieved 2026-10-17.

[6] [Tokio: advance](https://docs.rs/tokio/latest/tokio/time/fn.advance.html) — primary source, retrieved 2026-10-17.

[7] [pytest: flaky tests](https://pytest.org/en/latest/explanation/flaky.html) — primary source, retrieved 2026-10-17.

[8] [Google: avoiding flakey tests](https://testing.googleblog.com/2008/04/tott-avoiding-flakey-tests.html) — primary source, retrieved 2026-10-17.

[9] [Tokio: pause](https://docs.rs/tokio/latest/tokio/time/fn.pause.html) — primary source, retrieved 2026-10-17.

[10] [Local controlled reproduction evidence](reproductions.md) — local execution record, not an independent external source.
