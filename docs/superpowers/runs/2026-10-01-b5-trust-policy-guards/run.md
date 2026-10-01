# super-auto run — 2026-10-01-b5-trust-policy-guards

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design

idea: Implement epic ht-rzi (B5 cooperative trust policy guards) per TRUST-POLICY.md and beads ht-rzi.1-.6; design already exists (TRUST-POLICY.md + filed beads), start from the design's coverage checks; autonomous from there; raise questions only on contention about the ht-rzi.2 decision (Herdr agent_session report is a hint for reattachment); both roasts on; merge back into main at the end.
branch: trust-model-invariants
base: main

spec: TRUST-POLICY.md
epic: ht-rzi
assumption: branch name trust-model-invariants (user-designated, pre-existing worktree .worktrees/trust-model-invariants) replaces the super-auto/<slug> convention.
assumption: removed deps ht-rzi.{1,2,3,5} -> ht-p03.2 (B4, other run's epic, not started) so this epic can drain; whichever run lands second resolves seats.rs conflicts and migration-number collisions (B4 plans a v10 migration skeleton; ht-rzi.2 needs a schema change).
assumption: user pre-authorized the phase-7 merge into main ("merging back into main at the end"; goal "complete ht-rzi and merge result to main").
escalationPolicy: ask the user only on contention about the ht-rzi.2 decision; everything else autonomous.
