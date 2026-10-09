---
super-roast verdict: Blocking (7 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): Local same-user cooperative production CLI/daemon with durable SQLite state and native topology creation. Persistent seat attribution, original actor, migration preservation and idempotent replay are critical; labels are discovery hints. No adversarial caller verification or privilege-escalation claim solely from the cooperative invoker controlling their own input.
inputs: super-auto/handoff-topology@06a0abda05e1af9e88ced8a116a91251f3dc7d36 vs main@6f1c4e6ae44ffc8d2e7ba65022e19380b2fc87db
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 17 → deduped 10 → panel 9 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (gpt-6.1-sol) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 9 · rr 0.89 · rg 0.78 · fg 0.89 · unanimous 0.78 · ground-loo 0.88 (n=8) · reproduce 6/3/0 · refute 7/2/0 · ground 8/1/0
lane-yield (found/confirmed/unique/refuted): correctness 3/3/0/0 · security 0/0/0/0 · premortem 2/2/0/0 · simplicity-design 2/2/1/0 · hot-path-perf 1/1/0/0 · concurrency-async 1/1/1/0 · data-migrations 0/0/0/0 · deploy-safety 1/0/0/1 · api-contract 3/3/0/0 · observability 2/1/0/1 · testing 2/1/1/0 · hygiene-docs 0/0/0/0

## Confirmed findings
- [Blocking] src/cli/topology_runtime.rs:534 — Recovery can rearm an attempt using an assertion invalidated by an intervening native submission. [lanes: concurrency-async]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Reproduce and ground seats trace an initially Prepared attempt N under one shared journal. src/cli/topology_runtime.rs:465-477 samples it under the operation lock; :534 releases that lock before the frozen assertion is decided.
  A normal retry can then reserve N, submit creation and finish with OutcomeUnknown without canonical creation evidence (src/cli/topology_handoff.rs:449-532). Recovery reacquires the lock but checks only immutable identity (src/cli/topology_recover.rs:174-217,245-250).
  The same N still passes NotCreated because creation/attachment are absent (src/store/topology_handoff/attempts.rs:441-446), so :483 allocates N+1. A later retry discards older progress without creation (:380-400) and can create a second tab.
  TRUST-POLICY.md:629-630 requires exclusion while inspecting and asserting noncreation/quiescence. Its mistaken-operator limit at :633-637 does not cover the runtime invalidating a correct assertion before deciding it. This violates conservative replay and duplicate-topology prevention.
  The held-lock test at tests/cli/topology_recover.rs:473-483 does not cover a retry that finishes inside the unlocked interval.
  fix-shape hint: Keep inspection and assertion decision under continuous exclusion, or reject a decision whose inspected submission state changed.

- [Should-fix] src/cli/topology_handoff.rs:1423; src/cli/topology_handoff.rs:1398 — Successful zero-submission rearm bypasses the recovery report and becomes an attachment usage error. [lanes: correctness, premortem, simplicity-design, api-contract, observability]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace the production typed NotSubmitted outcome through src/cli/topology_handoff.rs:524-528 and close_not_submitted at :958-983. The store closes the old attempt and returns the next Prepared attempt (src/store/topology_handoff/attempts.rs:92-105,310-317).
  Only Err from run_bootstrap_retry invokes write_pending (:1393-1408). Ok(Prepared) bypasses output and fails the Attached requirement at :1418-1423 with bootstrap lacks canonical attachment.
  The public runtime propagates the error without a report (src/cli/topology_runtime.rs:392-407); main prints the generic error and InvalidRequest maps to exit 2. No recovery reference, next attempt or pinned continuation is emitted.
  The approved topology design at docs/superpowers/runs/2026-10-07-handoff-topology/2026-10-07-handoff-topology-design.md:80 requires those recovery fields. Durable state and later pending-ops discovery survive; duplicate submission and data loss are not established.
  fix-shape hint: Handle successful nonterminal bootstrap results explicitly and render the pending report for the new Prepared attempt.

- [Should-fix] src/cli/handoff_delivery.rs:761 — Existing-peer delivery omits retained partial work and pinned recovery guidance when a child operation fails. [lanes: correctness, api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace fresh publication at src/cli/topology_runtime.rs:207-221 into execute_locked. Shared stage_work saves each observed child independently (src/cli/handoff.rs:181-246).
  An invitation or send failure after earlier successful children propagates through src/cli/handoff_delivery.rs:486 and :761 before the sole output writes at :781-787. Completion reply loss at :503 also returns before presentation.
  The intent, observed IDs and canonical fence remain, but text and JSON receive no partial report, compound recovery reference or pinned continuation. src/main.rs:70-78 prints only the original error.
  The inherited partial-failure contract requires completed steps, exact IDs, failed/uncertain phase and a recovery command (docs/superpowers/specs/2026-10-03-cli-names-handoff-design.md:203-206). The legacy base writer supplies a partial report before propagating failure; the new delivery route does not.
  fix-shape hint: Render retained observations, honest failed/uncertain phase and the frozen continuation before propagating the delivery error.

- [Should-fix] src/store/topology_handoff/attachment.rs:627 — Retained successful launch reports are validated against current prompt and argv composition, creating an upgrade compatibility dependency. [lanes: simplicity-design]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats identify only caller options in BootstrapLaunch (src/protocol/handoff.rs:220-225), with no saved renderer/composition version. The successful launch saves its actual report, and replay skips launching when that report exists (src/cli/handoff.rs:615-646).
  Linked completion recomputes expected argv with the current cli::handoff::bootstrap and compose_native_argv using empty owned arguments (src/store/topology_handoff/attachment.rs:606-638,696).
  A later build that changes prompt wording can reject an unchanged genuine report saved before completion. Both fences remain live; the retry does not regenerate the report by launching again.
  The CLI completion preview and completed presentation also recompose argv (src/cli/topology_handoff.rs:1163-1180,1570,1841). Only completed canonical storage replay bypasses that check (attachment.rs:667-695).
  The approved topology design at :50-52,68 requires frozen original options and retained-report replay. This is a conditional maintenance/upgrade risk; identical current binaries are not shown rejecting their own reports.
  fix-shape hint: Freeze an exact launch contract or dispatch validation through a persisted renderer/composition version, including CLI replay validation.

- [Should-fix] src/archival_legacy.rs:712 — Every retained bootstrap child triggers another full directory lookup and additional recurring background writer transactions. [lanes: premortem, hot-path-perf]
  verdict: confirmed (reproduce ✗ / refute ✗-survived / ground ✓)
  evidence: Refute and ground seats confirm that src/archival_legacy.rs:704-722 opens a new Directory for each bootstrap child; :598-647 reads it to EOF even after finding its parent. No cross-child lookup index is retained (:491-501).
  C children among M directory entries therefore add C*M entry visits and roughly C*M/32 lookup pages. These are structural counts, not measured latency. Child progress survives downstream failures and is removed only after completed presentation (src/cli/topology_handoff.rs:1448-1471,1878-1895); retained terminals increase M.
  Each lookup page calls archival_pass and coherent pages also archival_next (src/service/archival.rs:75-109). These paths execute deciding transactions and archival_instances updates (src/store/archival.rs:69,114,1006,1036); pending pages continue immediately (:176-180).
  The reproduce seat rejects materiality because pages remain bounded, foreground writer fairness remains and no total traversal deadline is promised. The confirming seats address those same facts: they claim cumulative recurring work and transaction amplification, while expressly excluding starvation, latency deadlines, incorrect archival decisions and data loss.
  The concrete minority evidence is therefore addressed; the default confirmed route stands. Page bounds and fairness limit severity, but do not eliminate the introduced multiplicative work.
  fix-shape hint: Reuse parent associations across one coverage traversal and avoid deciding writer turns for lookup-only progress while preserving conflict checks.

- [Should-fix] src/cli/topology_handoff.rs:1678 — Bootstrap pending reports emit pending as the inspection subcommand, which the CLI does not expose. [lanes: correctness, api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats identify the literal pending appended to the pinned prefix at src/cli/topology_handoff.rs:1677-1678 and emitted as inspect_argv.
  The public command is PendingOps/pending-ops (src/cli/commands.rs:1011; src/cli/mod.rs:678-691). The full enum and argv preprocessing provide no pending alias or rewrite, and inferred subcommands are disabled.
  Following the generated text or JSON command therefore fails parsing instead of inspecting durable work. The report tests at tests/handoff_topology_cli.rs:1446-1451 check routing strings without parsing inspect_argv.
  The approved topology design at :80 requires ready guarded inspection argv. Durable work and valid retry alternatives remain; the defect is the supplied inspection continuation.
  fix-shape hint: Generate pending-ops and parse the emitted inspection argv through the public command parser.

- [Should-fix] tests/cli/topology_handoff.rs:3307 — Zero-submission tests stop below the writer and miss the missing public recovery report. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats identify tests/cli/topology_handoff.rs:3307-3325 and :4346-4365 calling Fixture::run, which invokes resume_to_attachment directly (:1071-1084) without a writer.
  Those assertions correctly accept Prepared attempt 2 while src/cli/topology_handoff.rs:1393-1423 later turns that successful result into an attachment error with empty report output.
  The test at :2449-2473 injects a downstream launcher refusal after attachment, so it does not cover typed tab-creation NotSubmitted. Successful replay after NotSubmittedRecordUnavailable reaches the same missed writer branch (:418-419).
  The approved topology design at :80,90 requires recovery reporting and CLI coverage of pre-submit proven no-effect. This is the coverage gap for the zero-submission defect above, not evidence of an additional native effect or data-loss mechanism.
  fix-shape hint: Exercise the writer with both typed zero-submission fixtures and assert retained reference, namespace, next attempt, continuation and absence of downstream effects.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] src/store/schema.rs:529 — Startup advances existing databases to schema 27, preventing the base binary from reopening them.
  verdict: rejected (reproduce REJECT / refute REJECT / ground REJECT).
  reason: All seats confirm the mechanical rollback observation but find no promised binary downgrade or automatic pre-upgrade backup. The approved design requires the additive startup migration, and the store contract requires refusal of unknown schemas (docs/design/herdr-threads/2026-09-27-herdr-threads--store-design.md:17).
  evidence: Reviewed src/store/schema.rs:529-540 migrates to 27; base src/store/schema.rs:404-410 rejects 27 and :511-526 already uses unconditional forward migration for 26. The demonstrated refusal preserves the database; irreversible loss or a violated rollback requirement is not established.

- [FYI] src/cli/topology_handoff.rs:530 — The bootstrap coordinator drops the native failure cause from its unknown-outcome diagnostic.
  verdict: rejected (reproduce REJECT / refute REJECT / ground CONFIRM at Nit).
  reason: The majority confirms information loss but finds no requirement to preserve the nested cause. The approved topology design at :62-64 intentionally groups uncertain failures under outcome_unknown and specifies conservative fencing and inspection/recovery facts.
  evidence: src/cli/topology_handoff.rs:524-532 drops the ApiError payload; :985-991 supplies the generic unknown diagnostic. The ground seat confirms only its usefulness for troubleshooting and explicitly excludes a safety-contract violation, so it supplies no unanswered concrete premise that would overturn the rejection.
  The separate invalid inspect_argv finding above remains confirmed; this rejection does not validate that command.

## Unverified nits (spot-checked)
- [FYI] tests/cli/cooperative.rs:3186 — The current-versus-initial actor test covers refusal without a positive control for valid current and unused opposite-harness initial.
  verdict: unverified nit; one refute-seat spot check CONFIRMed at FYI, not panel-strength verification.
  evidence: The spot seat identifies the refusal-only test at tests/cli/cooperative.rs:3186-3240 and retained initial.or(current.as_ref()) gate at src/cli/mod.rs:2460-2464. Ordinary run_selected supplies initial only for lifecycle (:1553-1568), so no ordinary-CLI exploit is demonstrated.
  pre-existing on base: src/cli/mod.rs:2392-2395 on main@6f1c4e6ae44ffc8d2e7ba65022e19380b2fc87db. The spot seat says the added current-first check does not introduce or worsen the existing valid-current/opposite-initial rejection.

## Escalations (need human)
- none
---
