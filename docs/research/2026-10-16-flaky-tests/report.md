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
