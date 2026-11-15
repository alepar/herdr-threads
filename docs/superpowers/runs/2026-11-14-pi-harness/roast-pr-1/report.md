super-roast verdict: Blocking (4 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: 1 of 3
profile (assumed): Internal same-user cooperative agent messaging; identity, receipt provenance, bounded delivery and priority-correct native wake are core contracts. The integrated branch includes neutral seams and qualification drivers. Current design and TRUST limits apply; the historical channel-wake design review is superseded.
inputs: pi-harness@62bc385d0b3623243db43c2536658eec6451f9bd vs 91ce0cdeb8e87dcc16c79f4a556fabffb7946967
coverage: scouts 16/16 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, deploy-safety, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, dependency, hygiene-docs) · raw 4 → deduped 4 → panel 4 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage bounded, provenance available — basis: "Frozen head 62bc385d0b3623243db43c2536658eec6451f9bd; public qualification/task-3-final.md and JSON map 14 scripted cases across exact hashes plus narrow footer fix/new binary evidence. Live qualification rejected before key read; user authorization pending. Initial full suite 9606 pass/10 fail/878s/leaks0; focused test-only corrections reviewed green. Final checks/native3/fresh full attempt2 not complete at dispatch. Baseline951s@4, ht-n4xh budget open; no regression inferred. HC/runtime and live invitation gaps remain."
independence: same-family (OpenAI GPT) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 4 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=4) · reproduce 4/0/0 · refute 4/0/0 · ground 4/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/1/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 1/1/1/0 · testing 0/0/0/0 · test-causality-lifecycle 1/1/1/0 · test-isolation-environment 1/1/1/0 · test-suite-cost 0/0/0/0 · dependency 0/0/0/0 · hygiene-docs 0/0/0/0

The engine coverage basis above records dispatch-time status. Coordinator supplemental evidence supersedes its live-authorization and unfinished-native/check statements: the exact frozen candidate passed native receipts, compaction and channel qualification with zero cleanup leaks, and shared format, clippy, default-feature, neutral-core and wire checks passed. Live Pi-only OpenRouter qualification is authorized but unexecuted; corrected full-suite attempt2 is held. Initial full-suite evidence remains 9606 pass, 10 fail and 878s; focused corrections passed. The 951s baseline and open ht-n4xh budget issue do not establish a branch timing regression.
Source readiness remains bounded with available provenance. Scripted/native evidence does not close hc runtime or unexecuted live invitation/provider coverage. All 16 scouts and 12 panel seats returned; stage completion does not establish source completeness. Historical design roasts are not a prior PR iteration. No convergence is claimed.

## Confirmed findings
- [Blocking] integrations/pi/extension/herdr-threads.js:130 — Buffered notices can disappear permanently before any model boundary receives them. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace a new p0 notice arriving behind 256 equal-priority retained records through onLine and trim, which evicts it and sets replayDebt. Canonical watch presentation settles carried notices at src/store/invitation_content.rs:2755-2762; src/store/attention.rs:1333-1348 advances the notice frontier. src/cli/watch.rs:735-763 flushes and confirms independently of model acceptance, retiring the outbox replay protection. Replacing watch cannot recover the discarded notice after confirmation. Reload also saves only proposals at extension line 293 and creates an empty pending map at line 327, dropping unproposed notices.
  evidence: This is a static end-to-end loss demonstration, not an executed failure. It concerns informational notices, not ordinary-mail ACK corruption. Design D4 and the Pi README require retaining bodies until a qualified boundary and recovering displaced bodies; loss before any boundary violates that core delivery purpose, so the Blocking floor applies despite the panel's Should-fix ratings. The intentional watch-flush confirmation contract remains valid.
  fix-shape hint: Give already-consumed notices retention and reload recovery that survives until qualified model admission; use bounded backpressure or a recoverable handoff rather than evicting them as replayable mail. Cover notice saturation and reload after normal watch confirmation.
- [Should-fix] scripts/pi-tui-qualify.py:602 — The transformed-boundary negative case can pass without transforming its target frame. [lanes: test-causality-lifecycle]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats identify the same permitted ordering: sender CLI returns, transform/release markers are written, both boundaries run before asynchronous watch delivery, and the pending assertion passes without a target proposal. Watch delivery before the explicit retry then permits the expected acknowledgment. The fixture transforms only entries already present; case_negative never asserts the exact target's presence and replacement, unlike cancel_settle_probe's target checks at lines 567-568.
  evidence: This demonstrates an unsound qualification predicate, not a reproduced production false ACK or proof that retained runs used this ordering. Passing native qualification does not supply the missing causal assertion. The production core-purpose floor is not established by this test gap alone.
  fix-shape hint: Assert an observer record showing the exact target ID and full body in the pre-transform proposal and its actual replacement, while preserving pending and successful-retry assertions; synchronize recipient readiness where necessary.
- [Should-fix] integrations/pi/extension/tests/extension.test.mjs:369 — A failed native IO assertion skips watch-child cleanup and can hang qualification. [lanes: test-isolation-environment]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm that the test starts an interval-running child, waits without a deadline, asserts connected state, and only then calls watcher.stop(). Its finally block removes only the scratch directory. A wrong-state assertion skips stop; a missing row never reaches it. nodeIO retains the child and piped stdout, and the documented command is plain node --test.
  evidence: Static failure-path risk, not an injected or observed current failure. Seats grounded the event-loop premise in [Node child-process documentation](https://nodejs.org/api/child_process.html#subprocessunref) and [Node test documentation](https://nodejs.org/api/test.html), which documents forceExit as default false. Required owned-process cleanup and bounded qualification justify Should-fix; no production delivery failure is established by this finding.
  fix-shape hint: Await watcher.stop() in guaranteed teardown and bound the first-row wait so both assertion failure and absent delivery reach cleanup.
- [Nit] integrations/pi/extension/herdr-threads.js:310 — A missing mode witness disables the integration without the required local diagnostic. [lanes: observability]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm that ctx.mode undefined takes the early non-tui return before owner creation or diagnostic invocation; later callbacks reject the absent owner. Design D1 explicitly requires a local diagnostic for missing mode or required APIs. The adjacent missing-sessionManager path warns, but the undefined-mode test at extension.test.mjs:423-425 checks only that no commands occur.
  evidence: Severity dissent only: reproduce and ground rate Should-fix; refute rates Nit because refusing enrollment remains correct and no unsafe enrollment or false receipt follows. The seats agree on the defect. No external Pi-version premise or executed failure is claimed.
  demoted: The profile is an internal same-user tool, and the defect is a missing compatibility diagnostic on an intentionally disabled capability; the correct refusal preserves the core enrollment and receipt contracts.
  fix-shape hint: Distinguish absent mode evidence from explicitly excluded modes, emit the existing compact local capability diagnostic for absence, and assert it in the undefined-mode test.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none

