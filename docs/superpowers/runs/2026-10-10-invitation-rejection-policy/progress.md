# super-code ledger — epic ht-xms

Launch: integrationBranch=super-auto/invitation-rejection-policy integrationWorktree=/Users/alepar/AleCode/herdr-threads/.worktrees/invitation-rejection-policy mode=autonomous caller owns finish. config.gate="nice cargo test --locked --all-features --lib store::control::tests::invitation_rejection"; config.sweep="focused rejection + warning/materialization/skill tests; cargo fmt --check; nice cargo clippy --locked --all-targets --all-features -- -D warnings; nice scripts/check-default-features". No full suite per AGENTS.md.
Ruling: retain ht-xms.2 as one leaf despite size-only promotion — one atomic existing control-to-warning flow; all contracts decided; costs a larger focused review if wrong.
Ruling: user explicitly requested fully autonomous, both roasts on; no design checkpoints, retain final merge gate.
Evidence: new timely-rejection warning regression fails against old code (info vs warn); baseline 20 matched tests passed, 2 unrelated socket transport cases sandbox-denied. New isolated cold build 5m14s; first incremental test build 55s.

Detector: round 1 — parallelism: 2 ready · cap 2 · peak in-flight 2; no shared-file deferrals.
Task 1: dispatched task-ht-xms.1 from e992b68b; brief task-1-brief.md; report task-1-report.md.
Task 2: dispatched task-ht-xms.2 from e992b68b; brief task-2-brief.md; report task-2-report.md. Root red regression transferred into Task 2 checkout.
