# super-auto run — 2026-10-01-b5-trust-policy-guards

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: code

idea: Implement epic ht-rzi (B5 cooperative trust policy guards) per TRUST-POLICY.md and beads ht-rzi.1-.6; design already exists (TRUST-POLICY.md + filed beads), start from the design's coverage checks; autonomous from there; raise questions only on contention about the ht-rzi.2 decision (Herdr agent_session report is a hint for reattachment); both roasts on; merge back into main at the end.
branch: trust-model-invariants
base: main

spec: 2026-10-01-b5-trust-policy-guards-design.md
epic: ht-rzi
assumption: branch name trust-model-invariants (user-designated, pre-existing worktree .worktrees/trust-model-invariants) replaces the super-auto/<slug> convention.
assumption: removed deps ht-rzi.{1,2,3,5} -> ht-p03.2 (B4, other run's epic, not started) so this epic can drain; whichever run lands second resolves seats.rs conflicts and migration-number collisions (B4 plans a v10 migration skeleton; ht-rzi.2 needs a schema change).
assumption: user pre-authorized the phase-7 merge into main ("merging back into main at the end"; goal "complete ht-rzi and merge result to main").
escalationPolicy: ask the user only on contention about the ht-rzi.2 decision; everything else autonomous.

approvals:
- top-split · auto · ht-rzi.1 LEAF, ht-rzi.2 LEAF, ht-rzi.3 LEAF, ht-rzi.4 LEAF, ht-rzi.5 LEAF, ht-rzi.6 LEAF (pre-filed decomposition adopted per user; promotion review skipped by instruction)
- coverage-round-1 · canonical R-list: R1-R16 (coverage-round-1-requirements.md; R17-R22 appended from r-new) · requirements: 16 · mapped: 16 · unmapped: 0 · auto 14 applied (C1-C14 in coverage-ledger.md): edge ht-rzi.2<-ht-rzi.1; amended .1 .2 .3 .4 .5; new leaf ht-rzi.7 docs; integration sweep ht-rzi.8
- coverage-round-2 · canonical R-list: R1-R22 (coverage-round-2-requirements.md) · requirements: 22 · mapped: 22 · unmapped: 0 · divergence: findings 14 → 14, novel 100%, widening: yes · auto 14 applied (C15-C28): seam contract ht-rzi.9 (pane-agent port) with .2 .3 .4 depending; amended .1 .2 .3 .4 .5 .7 .8

roast-design: 2026-10-01-b5-trust-policy-guards-roast-design-1.md, 2026-10-01-b5-trust-policy-guards-roast-design-2.md
roastDesignRound: 2

parked:
- coverage-round-2 · degraded-verdict · "coverage widening: yes (round 2 found 14 novel findings; round-2 fixes C15-C28 are not re-reviewed — the design roast and integration sweep ht-rzi.8 absorb them)"
- coverage-round-2 · degraded-verdict · "Seam integration bead for ht-rzi.9 folded into Integration sweep ht-rzi.8 rather than created separately"

- 2026-10-01-b5-trust-policy-guards-roast-design-1.md · escalation · "ht-rzi.2 Herdr agent_session hint: roast confirmed mandatory-match causes timing-dependent false refusals; user asked to choose (1 diagnostic-only / 2 re-read / 3 keep mandatory); stop hook forbade pausing, so option 1 (diagnostic-only, recommended) applied as an assumption — user may overturn"
- 2026-10-01-b5-trust-policy-guards-roast-design-1.md · escalation · "Claude Code #24265: resume may emit startup(new id)+resume(orig id); unverified on current versions — ht-rzi.2 capture must check; resume-only continuity covers one order"
stepBack-round-1: redesign — applied: Herdr agent_session hint as C1 veto → diagnostic-only, C1 gated on hook payload source=resume + existing native_session (dissolves 2); hold lift → one helper releasing all hold representations behind a reconciliation fence (cluster patch)
- roast-design-1 fixes · auto · spec decisions 1 and 3 amended; ht-rzi.1, ht-rzi.2, ht-rzi.9 rewritten, ht-rzi.7 patched (native_session reuse, seatless check-in defined in .2, migration only if CHECK widening needed); fixes applied inline as design edits rather than separate fix beads
- roast-design loop exit · converged at round 2 (Should-fix 3 confirmed [converged], 0 Blocking) · punch list applied inline: persisted reconciliation marker + single B5 migration moved to ht-rzi.1 (CHECK widening, continuity_diagnostic, marker columns); continuity intent replay on lost reply in ht-rzi.2
graph-pass: depth 4→4 · width 2.2→2.2 · applied 0 · parked 0 (4 candidates, none shortens depth alone; judge not dispatched)
