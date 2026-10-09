# Handoff topology: user-requested pause and session transfer

The user requested a safe pause and a handoff to another session. The feature is unfinished and is NOT ready for Main. Continue the existing `ht-qhz` run only when the user authorizes the receiving session. Do not create a replacement epic, reset counters, or repeat completed leaves.

## Exact safe boundary

Original session: `01a119ba-61ad-7f73-8dd6-184d76fcfd58`, codex-1, last native location `w4:pE7` / `w4:t8C`.
Integration worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-workflows`; branch `super-auto/handoff-topology`.
Pre-checkpoint HEAD `54e0b391719cec9156225483aabacf645c13a24e`, tree `43612e1ea7e6cd41ab307cff0d59ce0fe242c32b`. The final metadata-only checkpoint HEAD/tree are in `/private/tmp/handoff-workflows-session-handoff.json` and `.superpowers/sdd/ht-qhz-plan/controller-user-pause-freeze.json`.

Both task worktrees are under integration `.worktrees/`:

- `super-auto-handoff-topology--task-ht-qhz.26`, branch `task-ht-qhz.26`: HEAD `627d1d4ca40252c623c85c33b4893c2bc37f2413`, tree `13f28882016411be5a05ab13c2b0386e98407a02`. Exact independently reviewed compiler packet FF-adopted, clean. Not integrated. Parent verified 2,311 source bindings with zero mismatches.
- `super-auto-handoff-topology--task-ht-qhz.25`, branch `task-ht-qhz.25`: HEAD `cfc8f5626244c92bcb76adbbbdd186247ed134b4`, tree `a56a5667b45a02931d2bdfe7facf2cb8ce4dfab8`. Quarantined, clean, unmerged.

No MERGE_HEAD exists in any of these worktrees. The prior progress entry saying Task26 “merging” recorded intent only: NO integration merge started. This checkpoint supersedes that intent. No product edits, builds, tests, or reviews were started for this pause.

## Remaining work, in order

1. Read AGENTS.md, TRUST-POLICY.md, existing plan/run, checkpoint and source-bound evidence. Verify current HEADs/status, actual Main and counters. Preserve all historical SQL: actual supplying Main has lazy26/adapters27; topology owns additive28. No adapter semantic prerequisite. Never import old topology27 or overwrite historical migrations.
2. Admit Task26 exact627 through a normal integration merge preserving its Main ancestry, not cherry-pick/rebase/history flattening. Its pre-profile parent9d has parents2ae88d0 and Main3d10e3b. Run actual integration build-only merge checks for all/default features, focused consumer controls and required lint/default-feature/fmt/diff checks, UUID cleanup and source/profile bindings. Preserve inherited Main whitespace failures separately. Only close `.26` and remove its ht-8l4 dependency after actual consumption/checks; shared ht-8l4 remains open for Task21. No duplicate compiler investigation.
3. Task18 / `.22`: durable pending output on NotSubmitted rearm, partial delivery and uncertain completion, including the actual outer CLI writer. The plan was amended ONLY for this task to six paths: src/cli/topology_handoff.rs, src/cli/handoff_delivery.rs, src/cli/topology_runtime.rs; tests/cli/topology_handoff.rs, tests/cli/handoff_delivery.rs, tests/handoff_topology_cli.rs. Implement meaningful behavioral RED/GREEN and independent review. No implementation started.
4. Task19 / `.23`: freeze historical V1 launch presentation/validation independently of current renderer/registry. Eight-path audit is saved; current native admission stays current. Task20 / `.24`: parser-valid pending-ops guidance, after Task18. Neither task implemented.
5. Task21 / `.25`: minimally compose reviewed recovery with actual Main/topology28 and qualify genuine changed-source incremental builds under60s. Do not import the entire old branch or its historical SQL. External cfc compiler packet9c1da535 exists but was NOT adopted. Its one genuine64.337s run remains a failure; no load waiver. Missing raw owner counts for5/50/29 are an evidence limitation; final consumer needs actual relevant controls, not fabricated backfill.
6. Reconcile actual Main advances precisely, preserving FULL durability in topology fixtures when Main test-support tagging defaults to relaxed. Complete fresh matched skill pressure evaluations (five control/five candidate families), original remaining PR roasts and final source-bound readiness. Main owns combined suite/CI, merge to Main, release, local install and tab closure.

## Counters and review state

Original sixteen leaves plus actor `.21` closed; root open. Both roasts enabled. Design roast2; original PR roast1 completed, remaining original PR2/3 within cap3. Final SDD fixwave1/1 and scoped rereview1/1 spent. Task21: TaskFix1 spent, RESOLVE0. Task26: TaskFix1 spent, RESOLVE1 spent. Other seam/invalid/check-abort counters remain0. No fresh allowance by fiat. All original attachments, rejections, minors and failed measurements remain evidence.

Task26 I1 accepted for exact627 adoption by fresh independent consumer review; I2 genuine old27-reader/current28 refusal passed separately. Final integration gates still pending. Prior stalled report is historical, superseded by this pause/adoption checkpoint.

## Evidence index

All paths below are relative to `.superpowers/sdd/ht-qhz-plan/` in integration:

- `ht-qhz-plan.md` SHA256 `4e7fbe0496d568083606e9bb667a39377db2aa8983c29e44b6a121882f8d0408`; `controller-task18-plan-amendment.json`, `controller-task18-plan-parent-verification.json`.
- `controller-user-pause-boundary.json`: exact clean source states, no merge, 2,311 bindings.
- `controller-compile-consumer-review.md` SHA256 `8a8f582697c31806bb2a5edbdd438038f8a709cfbdc20c176d6cab0a04153df9`.
- `controller-task22-ready-triage.json`; `task22-reviewed-prerequisite-adoption.md/.json`; `controller-compile-packet-observation.json`, `controller-compile-supplement-parent-verification.json`.
- `task-22-implementation.md`, `task-22-review.md`, `task-22-fix-pass.md`, `task22-source-bindings.json`, `task22-artifact-manifest.json`, `task22-evidence-transfer.json`, `controller-task22-fix-parent-verification.json`.
- `controller-task18-presentation-readonly-audit.md/.json`, `controller-task18-full-writer-scope.md`, `controller-task19-historical-readonly-audit.md/.json`, `controller-next-main-readonly-audit.*`.
- Task21 original/fix reports, triage and artifact manifests; keep quarantined cfc evidence.

External compiler report: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-compile-perf/.perf/ht-8l4-report.md`, SHA256 `f3a690602ce005be0f9c4bae304396273d88917195eb1747523d2b463e5416ec`. Supplement `.perf/review/INDEX.md` and bindings provide303 verified files. Owner review/cleanup files are later verbatim transcriptions, not reviewer-authored/raw terminal files; independent consumer review above supplies new exact-head review. Caches removed by owner; source/evidence retained. Test-profile change only: lto off, package codegen-units256, opt1 retained; no dev/release/clippy changes. Task26 all12 measured incremental runs below60; no blanket cfc or full-suite qualification.

## QUIET and ownership

All eleven listed subagents completed/QUIET. No parent running exec/wait handle, tests, private servers, helpers or owned fixtures. UUID cleanup for original Task26 author `3f7e6c33-9f1e-4e66-bcd7-e87b12213a64` and fix `e60da2cf-a70b-4064-8d46-2444e9a719a2` returned0 at this pause; raw logs `user-pause-cleanup-author.log` / `user-pause-cleanup-fix.log`. No signals sent. External compile owner is settled; foreign Main/release work untouched.

Keep all worktrees, source refs and evidence until independently verified DONE+MERGED. Do not close this tab yourself. Communicate only critical milestones/blockers to Main `w4:p1` via direct Herdr agent prompts. Threads/inbox polling remain suspended; remain left from PSA/lazy/dev. Main owns integration/release; feature owns implementation decisions without Main approach approval. No shared-server restarts, real harness/config writes, unowned topology closure, push/stash, full-suite campaign or native/model scope expansion. Preserve canonical A2, original actor, immediate argv1 human grammar, replay fences, typed launch gates, frozen routing/options, restore holds and independent invitation/receipt semantics.
