# super-auto run — 2026-10-02-harness-version-evidence

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design

idea: Epic ht-xoc (harness version evidence), base main (26eef585, B6 landed). Design already exists: spec docs/superpowers/specs/2026-10-02-harness-version-evidence-design.md (two design roasts already run: docs/superpowers/reviews/2026-10-02-harness-version-evidence-roast-design-{1,2}.md, round 2 converged) and the filed bead tree ht-xoc.1-.8 — start from the design's coverage checks. Fully autonomous (no questions), both roasts on (design roast and code roast). Merge back into main at the end (user pre-authorized). Test policy: ht-zo4 (flakiness side quest) is closed, so the full suite may run; respect AGENTS.md speed budgets on main.
branch: super-auto/harness-version-evidence
base: main
skillSource: ~/.claude/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar4.11/skills @ 6.4.2-alepar4.11 (6.4.2-alepar4.11)
migrated: session-loaded skill text was 6.4.2-alepar4.6; the run follows the installed and published 4.11 files read from disk instead of restarting the session
spec: ../../specs/2026-10-02-harness-version-evidence-design.md
epic: ht-xoc

approvals:
- top-split · auto · ht-xoc.1 LEAF, ht-xoc.2 LEAF, ht-xoc.3 LEAF, ht-xoc.4 LEAF, ht-xoc.5 LEAF, ht-xoc.6 LEAF, ht-xoc.7 LEAF, ht-xoc.8 LEAF (pre-filed decomposition adopted per invocation; promotion review skipped)
