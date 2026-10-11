# Task 1 — ht-xms.1: shipped rejection guidance

Spec: 2026-10-10-invitation-rejection-policy-design.md
Workspace: /Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-guidance
Branch: task-ht-xms.1
Own only integrations/skill/SKILL.md and src/cli/commands.rs (Reject help).

Strengthen existing ordinary invitation rule: once top-level agent decides bad fit, MUST reject exact invitation ID with explicit meaningful nonblank reason; do not leave decided bad fit pending or repeatedly emit notices/ask permission. Inspect missing/truncated topic or goal first; genuinely unresolved fit may remain pending. Preapproved/required memberships retain existing procedures. Subagents never accept/reject. Document automatic reason delivery as a native warning to thread members at their next hook/check-in or inbox; do not add a separate send. Add one concrete command example with exact ID placeholder and meaningful reason. Update Reject CLI help coherently. Avoid tests asserting prose strings; existing compiled-skill and parser tests suffice plus fresh consumer pressure scenario at controller.

Baseline: current skill already directed rejection for clearly unrelated invitations; read-only pressure agent complied, but documented real incident repeatedly left decided bad fit pending. Requirement broadens to every decided bad fit and makes reason/delivery explicit.

Checks: cargo fmt --check; focused --lib cli::skill::tests and cli::commands::tests::rejection_requires_exact_invitation_and_bounded_nonblank_reason. CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-policy/target may reuse cache; cargo package must resolve this task worktree. No full suite, shared-server mutations, config writes, git push/stash. Root owns required combined clippy/default-feature checks and independent review. Commit only owned files on your own branch. Report status, tests, commit and limitations to task-1-report.md in integration run directory. Do not spawn subagents or write Herdr threads.
