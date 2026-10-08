# super-auto run — 2026-10-07-handoff-topology

flags: planOneShot=t skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: code
codeMechanism: ordinary-subagents

idea: lgtm, let's $super-auto this, fully autonomous, both roasts ar eon
branch: super-auto/handoff-topology
base: main

spec: 2026-10-07-handoff-topology-design.md
epic: ht-qhz

approvals:
- top-split · auto · ht-qhz.1 LEAF, ht-qhz.2 PROMOTE, ht-qhz.3 LEAF, ht-qhz.4 PROMOTE, ht-qhz.5 PROMOTE, ht-qhz.6 LEAF, ht-qhz.7 LEAF, ht-qhz.8 LEAF, ht-qhz.9 LEAF
- coverage-round-1 · canonical R-list: R1 New-tab handoff creates native requested tab with exact root pane and no focus.; R2 Existing-peer handoff stages work without launch, registration or identity replacement.; R3 Grammar and journal freeze explicit target/channel, routing, cwd and native options; ambiguity refuses.; R4 Canonical A2/restore/incarnation/seat guards decide every live effect and never trust labels as authority.; R5 Practical skill priority chooses handoff then compatible-participant send then membership actions, without guessing ambiguity.; R6 Concurrent/resumed/lost-response creation attempts have at most one submission permission and preserve uncertain status.; R7 Human administrative recovery preserves original actor identity, explicit provenance and strict guarded adoption/no-created claims.; R8 Exact child create/invite/send/launch replay preserves invitation episodes and independent ACK/adoption semantics.; R9 Completed compounds are absorbing presentation/cleanup across binding/archive/restart without old journal/digest rewriting.; R10 New journal archival import validates exact namespace conservatively and completed fences dominate delayed samples.; R11 Model-free CLI/host tests cover three modes and failure boundaries with owned child/topology teardown and scoped leak evidence. · requirements: 11 · mapped: 11 · unmapped: 0 · auto GAP R11 → ownership/tests amended; auto UNOWNED-SEAM live-effect authority guards → adoptedcontract ht-qhz.1/integration ht-qhz.19
- top-split · auto · ht-qhz.1 LEAF, ht-qhz.2 PROMOTE, ht-qhz.3 LEAF, ht-qhz.4 PROMOTE, ht-qhz.5 PROMOTE, ht-qhz.6 LEAF, ht-qhz.7 LEAF, ht-qhz.8 LEAF, ht-qhz.9 LEAF, ht-qhz.19 LEAF
- coverage-round-2 · requirements: 11 · mapped: 11 · unmapped: 0 · auto no new findings; prior fixes verified by both reviews
- top-split · auto · ht-qhz.1 LEAF, ht-qhz.2 PROMOTE, ht-qhz.3 LEAF, ht-qhz.4 PROMOTE, ht-qhz.5 PROMOTE, ht-qhz.6 LEAF, ht-qhz.7 LEAF, ht-qhz.8 LEAF, ht-qhz.9 LEAF, ht-qhz.19 LEAF, ht-qhz.20 LEAF

roastDesignRound: 2
roast-design: 2026-10-07-handoff-topology-roast-design-1.md, 2026-10-07-handoff-topology-roast-design-2.md
stepBackDesign-round-1: patch — independent exact-attempt recovery, bootstrap-only cancellation/liveness and atomic linked successful-report-backed completion; no shared redesign
graph-pass: depth 7→7 · width 2.3→2.3 · applied 0 · parked 0
