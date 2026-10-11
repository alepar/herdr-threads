# Active Watch recovery regression repair

Reviewed contract correction under ht-0lo.8.4; user fully autonomous, design roast off, code roast on. Parent owns execution, review, integration and finish.

## Task 1: Restore notice-clear continuation and immediate owner recovery

Work only in `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-watch-owner-replay`, baseline `9ca1a64d`. This is the fresh source-author branch; D remains parent-owned. Read TRUST-POLICY before protected source edits.

Requirements and exact values are in the following files, read them first and follow verbatim:
- `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-notice-clear-continuation-triage.md` (actual notice-only Clear failure; narrow first checkpoint).
- `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-watch-owner-replay-design.md` (SHA256 ff7fee56d0a93a0b8658966960561f04ff15b5ed18b5323e62f51d561bbdcf3b).
- `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-watch-owner-replay-requirements.json` (SHA256 699d5a22610b6f2dd235fac3420db08ac1dc01fa9deaeed99d33aeb9de860a40, all 17 requirements).
- `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-watch-owner-replay-design-review.md` and `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-watch-owner-replay-design-rereview.md` (WRD-1/WRD-2 dispositions, exact counterexamples).

### Sequential checkpoints

1. FIRST implement only the bounded carried-Notice/current-active-epoch continuation condition from the triage. One production-file commit, no unrelated refactor. Send parent commit and exact source patch immediately, before owner-replay mutations. Existing elected stale-attention control is genuine causal RED: `/private/tmp/ht-notice-controls-successor.log`, actual Notice and Attention2, no Clear2, owned leaks0. Add a source-only focused lib regression if useful that follows only actual advertised more, never unconditional Pull or fabricated CheckIn/ACK/stimulus. Parent runs GREEN. Keep Confirm zero-discovery and existing proof/work limits.
2. Implement the independently cleared owner-replay design, then meaningful dedicated tests for the 17 requirements, mapping reused current executed leaves honestly. Existing actual immediate reconnect remains causal RED: `/private/tmp/ht-resume-activation-082385c7/nextest-run.log`, no 60s waiver. Never weaken or delay this integration assertion. Include WRD-1 count4 and WRD-2 final full-page and post-store current wake outlook tests; these are load-bearing.
3. Preserve notice continuation fix in the final successor; self-review exact diff, format only, commit truthful trailers and send DONE/concerns with complete report. Parent runs Cargo, actual mutants and independent code review.

### Owned source/test paths

Required and conditional production paths are exactly the requirements JSON. All test paths there are admitted. Additionally admit existing `tests/service/delivery_pull.rs` for actual DomainService/frozen canonical OfferWindowFixture regressions; reuse its existing fixtures rather than inventing a new framework. Existing registration only; no new binary or target. No wire or schema changes. No harness-specific core branch. Reconcile only stale A7 channel enumeration and bridge notice comment while preserving semantics/provenance/limits.

### Parent lane and global constraints

No Cargo, clippy, build, test, plugin execution, Herdr command/write or subagent dispatch by author. Parent sole Cargo lane uses CARGO_BUILD_JOBS=4 and existing absolute D target; helper must allocate no target. Static rustfmt/diff inspection allowed. Parent source freeze is independent of your helper edits. No shared Herdr restart/stop/kill, no foreign process action, real user config write, push, stash, Main merge or worktree removal. Test children use existing test_support-owned machinery; nextest process groups for process-wide behavior. Full suite forbidden routinely while ht-zo4 open.

Every commit gets truthful `Outside-adapter:` changed-path accounting; expected new production model-visible text is 0 bytes. Any actual text needs expected-size accounting and token rules.

### Report

Write complete report to `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-watch-recovery-task-1-report.md`: baseline/full commit/tree hashes; changed paths; source/test mapping for every requirement; exact old/new blobs or SHA256; staged checkpoint commit/patch; TDD evidence with supplied actual RED and parent GREEN pending; work/query/output proof; self-review/concerns; no tests-run claim before parent evidence. Return short status/commits/one-line static validation/concerns only. Explicit QUIESCENT required before final transfer.
