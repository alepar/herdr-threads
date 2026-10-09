# super-auto run — 2026-10-09-claude-mod-inbound-delivery

flags: planOneShot=t skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: roast-code
codeMechanism: Workflow
resumeChange: 2026-10-09 · "Goal set: finish super-auto work, prepare integration branch ready for merge into main, notify /herdr tab 'main' for merging and release cutting" · phase 7 prepares the branch (sweep at tip, base absorbed) and hands the merge and release to the Herdr tab 'main' instead of merging locally

idea: build herdr-threads native Claude Code inbound delivery via a herdr-threads Claude Code mod (per spike on branch spike/claude-mod-delivery, docs/research/claude-mod-delivery-spike/README.md, and research ~/Documents/Claude_Code_Mods_Delivery_Research_20261008/report.md): a `herdr-threads watch` streaming subcommand, a bundled mod that checks in (pane env + session id), delivers mid-turn via tool.call context, idle via $.prompt.submit (engine decides idleness; hold after aborted turns; never queue while busy), lazy via $.session.append, acks receipts; daemon routes to the mod while its watch stream is connected and falls back to hooks+send-keys otherwise, resuming from unacked cursor; installer support (CLAUDE_CODE_PLUGIN_DIRS or plugin install); TRUST-POLICY provenance for mod check-in; repeated race stress tests.
branch: super-auto/claude-mod-inbound-delivery
base: main
spec: 2026-10-09-claude-mod-inbound-delivery-design.md
epic: ht-j16
approvals:
- top-split · auto · ht-j16.1 LEAF, ht-j16.2 LEAF, ht-j16.3 LEAF, ht-j16.4 LEAF, ht-j16.5 LEAF, ht-j16.6 LEAF, ht-j16.7 LEAF, ht-j16.8 LEAF (gate), ht-j16.9 LEAF
- coverage-round-1 · c1..c11 applied auto · c12 rejected auto · R1..R13 canonical (coverage-round-1-requirements.md) · requirements: 13 · mapped: 13 · unmapped: 0 · r-new folded into c1, c2, c4, c5
- coverage-round-2 · c13..c31 applied auto · c32 noted · requirements: 17 · mapped: 17 · unmapped: 0 · divergence: findings 12 → 19 · novel 19/19 (100%) · widening: yes (no round 3 by cap; design roast covers the settled tree)
parked:
- 2026-10-09-claude-mod-inbound-delivery-roast-design-1.md · escalation · "idle check vs submit non-atomic: a user Enter between the mod's idle check and the engine's acceptance can queue the plugin prompt behind the user's turn; spike check needed"
- 2026-10-09-claude-mod-inbound-delivery-roast-design-1.md · escalation · "$.session.id() inside session.end may return the ending session's id; restart path must re-read later"
- coverage-round-2 · degraded-verdict · "coverage widened in round 2 (12 → 19 findings, 100% novel); round-2 fixes are not re-reviewed by coverage — design roast reviews the settled tree"
roastDesignRound: 2
roast-design: 2026-10-09-claude-mod-inbound-delivery-roast-design-1.md, 2026-10-09-claude-mod-inbound-delivery-roast-design-2.md
stepBackDesign-round-1: patch — 11 findings fixable in place; four clusters (D12 override layer folded into D2–D8, delivered predicate per path, install env/managed-policy checks, ack per-id result taxonomy) plus reload turn-state
roastDesignExit: converged at round 2 (Should-fix 8 confirmed [converged], 0 Blocking); punch list of 8 applied inline as spec/bead text, no re-roast; iteration-1 escalations (submit atomicity under Esc; $.session.id() in session.end) remain parked
graph-pass: depth 4→3 · width 2.5→3.3 · applied 1 · parked 0
codeBuckets:
  completed: ht-j16.1, ht-j16.2, ht-j16.3, ht-j16.4, ht-j16.5, ht-j16.6, ht-j16.7, ht-j16.10
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: not ready (F1 /clear wake race via per-generation is_live; F3 mod runs bare herdr-threads without state dir/endpoint; F2 notices starve while channel live; T7 managed-settings cache filename guessed) — code-final-review-1.md
  sweep: SWEEP DEFERRED (caller-owned)
  slowness:
  worktreesKept:
  processSweep: stopped 0 · survived 0
roastCodeRound: 1
