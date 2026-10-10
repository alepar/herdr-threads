# super-auto run — 2026-10-09-agy-harness-adapter

flags: planOneShot=false skipPlanRoast=false skipCodeRoast=false autonomous=true
resumeChange: 2026-10-09 · "let's /super-auto this. Fully autonomouns from now, both roasts on" · autonomous on after interactive root design approval; nested brainstorms Mode B; both roasts on (unchanged)
phase: roast-design
codeMechanism: Workflow

spec: 2026-10-09-agy-harness-adapter-design.md
epic: ht-poh
idea: let's pick up adding support for agy. we implemented a prototype of routing agy via openrouter to avoid gemini subscription for testing, so lets use that. /superpowers:super-auto interactive design where we do the briefing, look around, brainstorming and finalizing design together, fully autonomous after that, roasts on. One of the goals - prove our new shiny adapter model. So one of the things I want us to be on the lookout for - how much we will need to touch code outside the adapter. Will we have to extend it scope? Rework existing implementations? callers? -> that can be a final report at the end
branch: super-auto/agy-harness-adapter
base: main

roastDesignRound: 1
roast-design: 2026-10-10-agy-harness-adapter-roast-design-1.md

parked:
- 2026-10-10-agy-harness-adapter-roast-design-1.md · escalation · "R5/contract rule 4: Herdr may report agy idle while a subagent generates (herdr issue #2354, open per 2 seats; PR #4278 status and fullyIdle semantics unverified), so a Native wake could type a nudge into a busy agy. Patched in-design: agy ComposerPolicy reads the subagent 'Generating' status as Unreadable (no wake); human to confirm on Herdr 0.9.1."

approvals:
- top-split · auto · ht-poh.1 LEAF, ht-poh.7 LEAF, ht-poh.2 PROMOTE, ht-poh.3 PROMOTE, ht-poh.4 PROMOTE, ht-poh.5 LEAF, ht-poh.6 PROMOTE
- coverage-round-1 · c1 auto applied, c2 auto applied, c3 auto applied, c4 auto applied, c5 auto rejected, c6 auto applied, c7 auto applied, c8 auto applied, c9 auto applied, c10 auto rejected, c11 auto applied, c12 auto rejected, c13 auto applied, c14 auto applied, c15 auto rejected · R1–R13 canonical (+R14 R15 R16 from r-new) · requirements: 13 · mapped: 12 · unmapped: 1 (R13)
- coverage-round-2 · c16–c28 auto applied, c29 auto rejected, c30 auto applied, c31 auto rejected, c32 auto rejected, c33–c35 auto applied, c36 auto rejected, c37 auto applied · requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 14 → 26, novel 85%, widening: yes (round 1 inputs hid owns lines and mis-matched epic goals; fixed for round 2) · integration sweep ht-poh.11
