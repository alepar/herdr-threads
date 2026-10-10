# super-auto run — 2026-10-09-session-keyed-seat-bindings

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design
codeMechanism: Workflow

idea: Goal: implement the new herdr-threads seat-bindings model we designed in this session, absorbing the Hermes global-enrollment work (ht-4ne, branch super-auto/hermes-global-enrollment, unmerged) it sprang from. Flags: planOneShot=false (interactive Mode A design for remaining questions), skipPlanRoast=false, skipCodeRoast=false, autonomous=true from after the design. The Codex CODEX_EXEC_SERVER_URL pane-env installer step runs separately on branch codex-daemon-env and is out of scope here.
branch: super-auto/session-keyed-seat-bindings
base: main
spec: 2026-10-10-session-keyed-seat-bindings-design.md
epic: ht-54r

approvals:
- top-split · human · ht-54r.1 LEAF, ht-54r.2 LEAF, ht-54r.3 LEAF, ht-54r.4 LEAF, ht-54r.5 LEAF, ht-54r.6 LEAF, ht-54r.7 LEAF, ht-54r.8 LEAF, ht-54r.9 LEAF, ht-54r.10 LEAF, ht-54r.11 LEAF, ht-54r.12 LEAF, ht-54r.13 LEAF, ht-54r.14 LEAF, ht-54r.15 LEAF, ht-54r.16 LEAF, ht-54r.17 LEAF, ht-54r.18 LEAF, ht-54r.19 LEAF, ht-54r.20 LEAF, ht-54r.21 LEAF, ht-54r.22 LEAF, ht-54r.23 LEAF
- design-mode · human · Mode A interactive design by user request at launch; autonomy from phase 3 (reserved: design questions asked, not parked)
baseAbsorbed: cb6052fc → 520a1780, 1 conflicted files
