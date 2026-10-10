# super-auto run — 2026-10-09-session-keyed-seat-bindings

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design
codeMechanism: Workflow

idea: Goal: implement the new herdr-threads seat-bindings model we designed in this session, absorbing the Hermes global-enrollment work (ht-4ne, branch super-auto/hermes-global-enrollment, unmerged) it sprang from. Flags: planOneShot=false (interactive Mode A design for remaining questions), skipPlanRoast=false, skipCodeRoast=false, autonomous=true from after the design. The Codex CODEX_EXEC_SERVER_URL pane-env installer step runs separately on branch codex-daemon-env and is out of scope here.
branch: super-auto/session-keyed-seat-bindings
base: main

approvals: design-mode · Mode A (interactive) by user request at launch; autonomy applies from phase 3 (reserved decision: design questions are asked, not parked)
baseAbsorbed: 3264587b → merge at launch (main cb6052fc v0.6.0 into branch cut from ht-4ne tip 3e308254), 1 conflicted file (src/cli/hook.rs: connect_pane + CompletionLink)
