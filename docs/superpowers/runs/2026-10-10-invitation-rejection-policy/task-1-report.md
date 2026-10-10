# Task 1 report — ht-xms.1

Status: implemented and committed on `task-ht-xms.1`; task worktree clean.
Commit: `d8c19b30` — Clarify mandatory rejection of bad-fit invitations.

Changed only `integrations/skill/SKILL.md` and Reject help in `src/cli/commands.rs`. Top-level agents must reject each decided bad-fit ordinary invitation by exact invitation ID with a meaningful nonblank reason, without leaving it pending or repeatedly asking permission. Missing/truncated metadata is inspected first; genuine uncertainty can stay pending; preapproved/required memberships retain their procedures. Subagents never accept/reject. Guidance and help explain automatic native warning delivery at the next hook/check-in or inbox without a separate send. Added a concrete command example. Guide is 198 lines, within its existing <200-line contract.

Verification:
- `cargo fmt --check`: passed after final edits.
- `git diff --check`: passed after final edits.
- `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-policy/target nice cargo test --locked --all-features --lib cli::skill::tests`: 15 passed, 0 failed. Cargo explicitly compiled `/Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-guidance`; compiled guide equality and concision checks passed.
- Same target/cargo flags with `cli::commands::tests::rejection_requires_exact_invitation_and_bounded_nonblank_reason`: 1 passed, 0 failed. Command executed from task worktree, but shared target was overwritten by runtime worktree while queued; result reported 3582 filtered tests versus 3562 in skill run. Controller should account for shared-target artifact provenance when reviewing this parser evidence.
- Controller reported fresh pressure consumer passed all six scenarios; this is controller evidence, not a test run by this implementer.

Self-review: all brief requirements checked against the final diff; no new prose-string tests added. No full suite, shared server mutations, real configuration writes, Herdr thread writes, pushes, stashes, or spawned helper daemons. Combined lint/default-feature checks and independent review remain controller-owned. Shared target lock contention delayed both focused runs; nice emitted a sandbox priority warning but cargo ran and exited successfully. Git commit required scoped escalation for worktree index/ref metadata and succeeded.
