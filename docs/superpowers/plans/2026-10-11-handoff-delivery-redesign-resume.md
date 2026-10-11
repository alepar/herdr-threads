# Handoff Delivery Redesign Continuation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans for execution. This session reassesses and plans; it does not execute implementation or validation.

**Goal:** Complete the existing ht-0lo integration branch with current-source activation evidence, combined validation and review, then issue the previously authorized Main readiness notification.

**Architecture:** Retain canonical send-time deadlines, independent invitation acceptance and message receipt, stable hooks, and bounded V2 presentation. Finish active D, compose actual Main with existing A/B and reviewed D, validate C, then perform whole-epic review and the original PR roast. Fan out investigation and review aggressively; serialize source mutation, helper transfers, merges and Cargo.

**Tech Stack:** Rust, SQLite, Tokio, cargo-nextest, Claude mod JavaScript/TypeScript, beads.

**Spec:** `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md`, subsystem designs, current TRUST-POLICY and explicitly accepted refinements. This plan supplements `2026-10-11-handoff-delivery-redesign-finish.md`; its older snapshots and unchecked completed steps are historical, not instructions to repeat work.

## Global constraints

- Preserve epic ht-0lo, phase code, design roast round2, original flags/counters and `[degraded: design low coverage]`; no reset or replacement epic.
- Never push, stash, merge into actual Main, delete worktrees, stop shared Herdr, close foreign panes, or write real user harness configuration.
- Current user AGENTS instructions supersede older checkout copies. Read normative TRUST-POLICY before protected changes. No heuristic seat repair or adversarial caller verification.
- Messages default Lazy; ordinary Lazy delivery never wakes idle sessions unless wake_on all. ACK-required messages are nudge-class. Receipt ACK, presentation Confirm, explicit acceptance, registration and task completion remain distinct.
- One parent-owned Cargo lane: D's absolute warm target, CARGO_BUILD_JOBS=4, nice, locked/all-features. Freeze source for every command. Only one author mutates each worktree; no helper Cargo targets.
- Per change: fmt, strict all-target/all-feature clippy, harness-neutral. Before merges: default-feature guard. Focused suites only while ht-zo4 remains unresolved; fresh HT_LEAK_RUN_ID and scoped leak check after success or failure.
- Released/approved migrations 1..39 remain byte-identical; D adds40/41. Preserve B's modern matching Some-only2MiB allowance and legacy None1MiB limit, summary/sidecar constraints and ALLSLOTS test groups.
- D retains32blocks/65536encoded bytes; Pull433/Refresh160 physical visits; publication100visits/200keyed probes/3seeks; relevance13sources/26shared seeks plus declared witnesses;352sizing passes plus final encode; Confirm performs no discovery. Preserve accepted separately charged keyed work, carry155731/partial49415 and bounded caches.
- Migration40 startup quick_check and retained publication-prefix costs are accepted and distinct from healthy runtime bounds. Historical constant-read and third-attach-Conflict prose is superseded by explicit correction/Main026 evidence.
- Commits require truthful Outside-adapter trailers and accounting for new model-visible text. Preserve failed evidence; no copying an older verdict onto a new SHA.
- Top-level stays joined to active psa-global; subagents never run Herdr writes, default text inbox, ACK, accept or check-in. No project broadcasts to psa-global. Main notification only after actual readiness.

## Current assessment and evidence limits

Read-only six-agent reassessment on2026-10-11 observed:

| Component | Observed HEAD | State |
| --- | --- | --- |
| Run ROOT | 5cbe5966e31f53048ec08eb8c47e2b665f8a19cb | Clean before this planning document; not fully composed. |
| Main | 026ad3ab444779216748cc5733e2241e2834ebaf | Clean, no movement beyond recorded Main026. |
| D | 0fe06ecd05f8967ce607bd79c4dfae31ce574876 | Clean; tree fc73a5d0edd4f90338f0b354229edb83729fd253. Latest tests-only delta not executed. |
| Composition | 3ad07776ca21b5a8aea0cd624447142e536e0d40 | Clean; A/B present, Main026/D absent. |
| B | cdbf3c37e873a2cb571c098450602a53f81a6185 | Clean, retained reviewed source. |
| Mod helper | 7322b89abdb774b250adc81fe81a63ff1b450db2 | Exactly preserved modified delivery.test.ts and untracked guarded empty watch-activation.ts. |

D f8→0fe changes only `tests/integration/mod_delivery.rs`,98insertions/5deletions including formatting. Last actual f8 run selected35:32PASS/3FAIL; current0fe has no runtime or lint clearance. Capability is already advertised; do not repeat advertisement. Existing corrected cumulative production audits remain useful but do not close active runtime proof.

The two recovery fixtures previously waited for Attention/nonempty Confirm with only a passive Lazy invitation. Current source adds a real ordinary ACK-required message while preserving passive invitation and pending/no-ACK assertions. The positive test's global current=None/last=token condition remains unchanged; successor-token overwrite is a source-supported hypothesis, not the demonstrated failure cause.

The capture producer and plugin tests are separate proofs joined by raw stdout. Plugin external APIs and ACK replies are mocked; passing replay tests cannot establish actual daemon ACK effects or live Claude model uptake. Both producer and prepared consumer currently accept any string clear epoch; require correspondence to the actual captured attention epoch before treating the artifact as proof of scoped clear behavior.

The prepared mod diff and empty fixture match pause backups byte-for-byte. Diff SHA256:08ef2b51ec4b0584c84a33c806524848cddba8c3209e05f480f3f46d7545926f; fixture SHA256:ca741906f1f7385deec22d8da469ef3528cf1e08f628d0a2c05394dd60d2dc80. No positive capture exists. Parent read-only process-name inspection found no cargo/rustc/nextest or task-path matches; this is not a complete owned-process leak proof and expires before execution.

## Fanout and ownership

| Wave | Parallel independent roles | Serial parent gate |
| --- | --- | --- |
| 1 | Watch delta reviewer; Watch author diagnosis; plugin author preservation/provenance preparation; composition preflight; C inventory refresh; contract/evidence auditor | Freeze0fe, run exactly3 remaining Watch cases, scoped leaks. |
| 2 | If needed Watch author + fresh scoped reviewer; plugin author after genuine capture + separate literal/provenance reviewer; whole/schema/interface active-delta reviewers; C public-Ensure→registered-Hook fixture preparation | Transfer only after fresh author quiescence and exact path/hash mapping. Parent runs all Rust/plugin checks. |
| 3 | Main/D conflict reviewer; clean-auto-merge reviewer; schema/wire reviewers; C seam author only after frozen composed source is released | Compose observed Main then reviewed D serially; release frozen candidate to C author. After reviewed C transfer, freeze again and enumerate/run C and affected checks. |
| 4 | Fresh whole-epic reviewer, then original PR-roast triage/scouts/dedupe/distinct judges/reporter under their workflow | Promote only gated candidate; preserve rounds/qualifiers; serial fix transfers, final exact-tip checks and notification. |

Use fresh isolated agent contexts with exact paths/SHAs/contracts/output requirements. Reviewers do not author the oracle they approve. Six independent roles are useful now; use remaining slots for concrete independent reviews, not duplicate broad reconnaissance. Parent owns Cargo, process cleanup, tracker transitions, merges and durable reporting. No agent starts a dependency-bound task merely to fill a slot.

## Task 1: Re-establish the lane and run current Watch evidence

**Files:** D `tests/integration/mod_delivery.rs`; run-local pause reports and manifests. Read-only initially.
**Consumes:** exact0fe HEAD/tree, prior f8 failure log, current pending/no-settlement contract.
**Produces:** actual three-case result/capture or a diagnosed conditional correction brief.

- [ ] Re-observe HEAD/tree/clean state, Main and task-owned processes immediately before execution. Verify scoped checker and source-owner quiescence; freeze D.
- [ ] Obtain independent exact f8→0fe delta review while the parent executes the current cases. Do not transfer edits during Cargo.
- [ ] Execute only the following selection from D, recording SHA/tree, executable SHA256/build correspondence, log hash, selected names/count, exit, timing and cleanup. Generate a fresh UUID with `uuidgen`; inspect command output rather than inferring success.

```sh
export CARGO_BUILD_JOBS=4
export CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target
export HT_LEAK_RUN_ID="$(uuidgen)"
export HT_ACTIVATION_CAPTURE_PATH="/private/tmp/ht-watch-${HT_LEAK_RUN_ID}.jsonl"
nice cargo nextest run --locked --all-features --test integration -E 'test(=mod_delivery::activation_v2_watch_lost_confirm_response_replays_exact_subset) | test(=mod_delivery::activation_v2_watch_replacement_replays_uncommitted_output) | test(=mod_delivery::activation_v2_elected_watch_capabilities_lease_output_confirm)'
scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"
```

- [ ] Before run, enumerate with the same `cargo nextest list --locked --all-features --test integration -E` expression. Require exactly3 selected/executed. Run the leak command separately even after failure. Capture existence alone is not positive evidence.
- [ ] If positive fails, read canonical current/last/mask and actual matched Pull/Confirm diagnostics. Only if successor overwrite is demonstrated, direct one author to add a post-upstream-Confirm/pre-child-reply gate. Keep original pre-dispatch gate; assert this token's retirement/control mask while child cannot Pull again; forward unchanged response, retain pending/no-ACK/no-accept/lazy and explicit clear checks. No timeout increase or response-only oracle.
- [ ] Independently review any correction; run relevant cases after transfer. Review clear epoch correspondence as a separate proof gap, preserving full raw stream and unexpected successor lines. Do not assume it caused the existing failure.

**Exit:** Three actual passing cases on a reviewed successor, genuine positive output and scoped zero owned leaks; otherwise precise actual failure and correction gate remain open.

## Task 2: Complete plugin proof and active D clearance

**Files:** mod helper `integrations/claude/mod/tests/delivery.test.ts`, `tests/fixtures/watch-activation.ts`; production `hooks/register.js` is a read-only reference absent an evidenced defect. D active test files and run evidence.
**Consumes:** Task1 successful raw capture and source/build provenance.
**Produces:** byte-preserving fixture, independently audited literal expectations, actual plugin and elected/affected results.

- [ ] Preserve helper dirty bytes/backups. Give author actual capture plus producerHEAD/tree/test/executable/captureSHA256 and byte length. No generated reconstruction of purported CLI output.
- [ ] Audit IDs/body literals/order/flags/wakes=false/epochs against fixture setup and contract, independently of renderer comparisons. Clear must correspond to captured attention epoch. Do not silently discard extra stream lines to preserve prepared count assumptions.
- [ ] Populate guarded fixture byte-for-byte and hand-reviewed literal arrays; retain explicit mocked-API boundaries and all existing tests. Review consumer ACK-ID/provenance, no acceptance/wake, busy/idle/clear behavior and raw chunk reconstruction.
- [ ] Parent runs actual `claude plugin validate --strict integrations/claude/mod` and `claude plugin test integrations/claude/mod` from helper with temporary HOME and CLAUDE_CONFIG_DIR. Preserve inherited real HOME; assign only per-command isolated HOME. Record real executed test count and capability refusal honestly. Use direct commands because scripts/test-claude-mod can return0 on version-based skip and replace caller config. No script repair is in this task.
- [ ] Refresh activation inventory, enumerate exact registered tests and execute required elected/affected Rust paths, including35-case active selection plus corrected service/private/capacity/schema/owner seams. Reuse older production proof only with exact unchanged-blob mapping; current tests require current results.
- [ ] Repair two stale preparation selectors: replacement is `mod_delivery::activation_v2_watch_replacement_replays_uncommitted_output`; omitted-capability control is `agy::activation::registered_agy_omitted_or_refused_capability_uses_historical_pull`. The35-case run is not the complete matrix. Enumerate actual `cli::watch::tests::`, `harness::agy::delivery::tests::` and `cli::hook::neutral_tests::` for capable ordered output/probe failures, codec/completion/fallback and neutral write/flush controls. Include these five existing integration controls explicitly: `mod_delivery::live_channel_suppresses_native_wake`, `mod_delivery::reconnect_within_grace_keeps_wake_suppressed_restreams_and_late_ack_settles_once`, `mod_delivery::fresh_watch_after_accept_prints_no_attention`, `mod_delivery::stale_attention_is_retracted_after_accept_and_after_a_notice_offer`, `mod_delivery::notice_published_while_live_reaches_the_next_pre_tool_use_once`. Reconcile every required row with discovered/executed evidence instead of assuming a union count covers it.
- [ ] Run fmt/clippy/neutral/default-feature and obtain whole/schema/interface active-delta clearance, linked to cumulative production reviews and exact successor. Reconcile stale manifests (e.g. corrected-D-clearance whole_review pending) by appending current evidence, preserving prior failures.

**Exit:** Active D and plugin engine proof completed with honest live-native limitations, current-source gates and independent review. Only then release D for composition.

## Task 3: Compose and execute C against actual combined source

**Files:** composition `src/store/service_controls.rs`, relevant clean auto-merges, `tests/store/control.rs`, TRUST-POLICY, Cargo metadata, nextest groups; C admitted `tests/cli/direct_send.rs` and explicitly admitted `tests/hook_entrypoint/agy.rs` public seam.
**Consumes:** reviewed active D, reviewed B, freshly observed Main, existing combined-tests.json98-name static inventory.
**Produces:** frozen reviewed Main+B+D+C candidate and actual combined evidence.

- [ ] Re-observe Main; refresh preflight if changed. Read-only preview currently predicts service_controls conflict for Main→composition, but it is not a second-merge oracle.
- [ ] Merge Main into existing composition, then reviewed D serially. Preserve Main historical requirement-link exclusion and pending-first acceptance helper together with D Lazy tuple/guards and command/context epochs. Review both conflicts and relevant clean auto-merges.
- [ ] Correct third-required-attach test only on composed source: fresh invitation/requirement IDs succeed; retained old history/command epochs stay unchanged; actual new thread mutation advances context; exact replay advances neither. Preserve modern-Lazy independent episode and released Main acceptance regressions.
- [ ] Confirm B Some-only2MiB/legacyNone1MiB, summary/sidecar constraints, ALLSLOTS2 groups, version0.8.1 and historical SQL bytes survive.
- [ ] Refresh98-name inventory against actual registration: six recovery tests plus92prepared selected names are preparation counts, not a pass oracle. Reinstate repaired replay test if previously excluded; append active paths and new C names; enumerate before running, reject missing/zero tests.
- [ ] C author adds actual public Ensure→registered Hook seam using existing real isolated fixture, with full topic/goal/current acceptance command and pending/no-ACK/no-accept/no-wake assertions on advertised V2. Admit Hook test path explicitly. Add required-command delivery sibling only if composed existing tests leave that seam uncovered.
- [ ] Parent runs six `combined lazy_recovery_context::` cases and refreshed exact nextest selection with configured groups, fresh run UUID and scoped leak check even on failure; also actual31→41/reopen, B wire controls, reattachment and advertised consumer paths. Use one granted warm lane; no second target.
- [ ] Run required premerge gates and fresh C/composed seam review; freeze final candidate. Close D/C/composition only from actual accepted evidence, not prepared inventories.

**Exit:** Actual combined source passes requirements and reviews; root promotion and PR roast remain separate gates.

## Task 4: Finish original review/report/readiness sequence

**Files:** original run.md/tracker, whole-epic review, PR-roast reports, final report/feedback/readiness manifest.
**Consumes:** Task3 exact reviewed/gated candidate and fresh Main base.
**Produces:** reviewable integration branch and truthful authorized Main notification.

- [ ] Obtain fresh whole-epic review before promotion; preserve original epic/buckets/flags and low-coverage qualification.
- [ ] Resume original super-code/super-auto sequencing at recorded phase; read current workflow prompts before execution. PR roast round1 uses frozen candidate/base, fresh triage/core+conditional scouts, dedupe and distinct reproduce/refute/ground judges. Reproductions share parent lane; state same-family independence honestly. Do not count this planning audit as a roast.
- [ ] Follow original scope filtering/caps/fix loop, preserving finding identities and dispositions. No unresolved Blocking may be called ready.
- [ ] Write prescribed final report and once-only upstream feedback draft locally; no external submission. Commit all metadata before final source stamp.
- [ ] On final integration tip run fmt/clippy/neutral/default-feature/affected combined gates with scoped leaks, and controlled single-source incremental build/lint measurements. Incremental build and per-change strict lint must each stay under60s. Retain historical89s broad lint honestly; a fresh over-budget result requires repair/disposition before readiness. Distinguish broad rebuild history from budget proof. Any later commit invalidates stamp.
- [ ] Revalidate exact Main seat and compatible project thread at send time, then issue authorized readiness notification with finalSHA/report/gates/qualifiers. Keep actual Main unmerged and all worktrees retained.

**Exit:** Branch/report ready for the user's merge decision; no Main merge/push/deletion.

## Self-review

This continuation distinguishes already completed repairs/advertisement from open runtime evidence; current0fe from historical f8/31e results; capture-based plugin proof from live-native uptake; static98-name preparation from actual execution. All settled requirements remain owned by the original finish plan and accepted refinements. No new architecture or scope expansion is required. Next executable action is Task1's three-case parent run, with parallel read-only review and preparation.
