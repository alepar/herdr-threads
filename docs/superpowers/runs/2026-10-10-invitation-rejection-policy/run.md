# super-auto run — 2026-10-10-invitation-rejection-policy

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: report

idea: I want to add instructions to herdr-threads skill to explicitly reject invitations we choose to not accept due to bad fit. rejection should have a reason, which gets delivered to the thread as a warning system message.
branch: super-auto/invitation-rejection-policy
base: main

spec: 2026-10-10-invitation-rejection-policy-design.md
epic: ht-xms

approvals:
- 2026-10-10 user: "super-auto is fully autonomous, both roasts on"; design checkpoints waived explicitly, main merge gate retained from handoff.
- promotion: ht-xms.1 LEAF; ht-xms.2 LEAF (session override of size-only PROMOTE: one atomic existing rejection-to-warning flow, no undecided interface or independent deliverables); ht-xms.3 LEAF.

roastDesignRound: 1

codeBuckets:
  completed: ht-xms.1, ht-xms.2, ht-xms.3
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: CLEAN

roast-design: 2026-10-10-invitation-rejection-policy-roast-design-1.md

coverage-round-1: coverage-round-1.md
requirements: 7 · mapped: 7 · unmapped: 0 (none)
coverage-round-2: coverage-round-2.md
requirements: 7 · mapped: 7 · unmapped: 0 (none)

roastCodeRound: 1

roast-code: 2026-10-10-invitation-rejection-policy-roast-pr-1.md
