# super-auto run — 2026-10-09-claude-mod-inbound-delivery

flags: planOneShot=t skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: report
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
- 2026-10-09 background commit security review · escalation · "mod frame relay/intent markers can be spoofed by peer-controlled body, thread or sender text (a fake marked header line); raw-body escaping was rejected twice under the cooperative model (design roast 1, PR roast 1); follow-up: delimit or escape peer text inside the frame now that markers carry meaning; recorded as a TRUST-POLICY accepted limit"
roastDesignRound: 2
roast-design: 2026-10-09-claude-mod-inbound-delivery-roast-design-1.md, 2026-10-09-claude-mod-inbound-delivery-roast-design-2.md
stepBackDesign-round-1: patch — 11 findings fixable in place; four clusters (D12 override layer folded into D2–D8, delivered predicate per path, install env/managed-policy checks, ack per-id result taxonomy) plus reload turn-state
roastDesignExit: converged at round 2 (Should-fix 8 confirmed [converged], 0 Blocking); punch list of 8 applied inline as spec/bead text, no re-roast; iteration-1 escalations (submit atomicity under Esc; $.session.id() in session.end) remain parked
graph-pass: depth 4→3 · width 2.5→3.3 · applied 1 · parked 0
codeBuckets:
  completed: ht-j16.1, ht-j16.3, ht-j16.6, ht-j16.5, ht-j16.7, ht-j16.4, ht-j16.2, ht-j16.10, ht-j16.19, ht-j16.18, ht-j16.22, ht-j16.20, ht-j16.17, ht-j16.21, ht-j16.23, ht-j16.25, ht-j16.24, ht-j16.9, ht-j16.27, ht-j16.26, ht-j16.28, ht-j16.29, ht-j16.30, ht-j16.32, ht-j16.31, ht-j16.33
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: not ready — code-final-review-8.md; its must-fixes resolved after it: finding 1 fixed at dc5b2f6c, live stress re-run at b562d1e4 (13/14 3/3, reload_mid_turn 2/3 ledger-only), sweep passed @ 11b01cd5
  sweep: 11b01cd5 · 4029/4029 passed, 43 skipped (matches nextest list) · test time 214 s, wall 359 s incl. build · leak check clean · clippy, fmt, check-default-features clean
  slowness: round 1 (fix re-entry): merge queue peaked at 5 — serial merge lane bottleneck
  worktreesKept:
  processSweep: stopped 0 · survived 0
roastCodeRound: 2
roast-code: 2026-10-09-claude-mod-inbound-delivery-roast-pr-1.md, 2026-10-09-claude-mod-inbound-delivery-roast-pr-2.md
stepBackCode-round-1: patch — Round 1, no recurrence yet; findings mostly independent, one registry-liveness cluster (per-generation liveness query + stale-snapshot closes) and one mod-test-enforcement cluster swept by rule
scopeFilter-round-1: [Should-fix] integrations/claude/mod/hooks/register.js:228; src/cli/watch.rs:719 in-scope — Backlogs over 100 unacked ids are rejected whole on every retry and never settle, so the goal's 'each fully delivered message settles its receipt' fails.
scopeFilter-round-1: [Should-fix] src/cli/watch.rs:340 in-scope — A transient Capabilities failure makes the mod stop watching for the whole session, so mod delivery in the goal-named path is wrongly and permanently disabled.
scopeFilter-round-1: [Nit] src/service/mod_channels.rs:288 in-scope — The race closes a fresh, healthy mod registration, which is incorrect behavior in the goal-named 'mod connected' tracking, though it self-heals.
scopeFilter-round-1: [Nit] src/service/mod_channels.rs:240 punch-list — cluster override: The channel being closed was genuinely stalled, so the outcome is nearly correct, unlike the pass() case where a healthy newer registration is wrongly closed.
scopeFilter-round-1: [Nit] src/cli/setup.rs:1369; src/cli/setup.rs:1401 in-scope — The mod install is part of the change, and an error exit leaves the bundled mod's settings entry removed, so the install does not leave a correct state.
scopeFilter-round-1: [Nit] src/service/mod_channels.rs:193; src/daemon/settings.rs:20 punch-list — The runtime kill-switch and its docs are not named by the goal; this is dead-code and doc divergence, not a delivery defect.
scopeFilter-round-1: [Nit] src/cli/setup.rs:1708 punch-list — Interactive-prompt gating in doctor and setup-all is setup UX the goal does not name, and it is not a delivery correctness defect.
scopeFilter-round-1: [Nit] scripts/test-claude-mod:13; scripts/test-claude-mod:1 punch-list — The finding concerns CI enforcement of existing tests, a quality improvement rather than a missing or failing test of goal-named behavior.
cluster dropped: mod-test-enforcement — `scripts/test-claude-mod` gets a required mode (fails instead of skipping when `claude` is absent or old), the integration sweep/AGENTS.md per-change check for mod changes invokes it, and the sweep runs the mod's installed argv with no extra flags so `r1 [Must-fix] integrations/claude/mod/hooks/register.js watch argv (F3 bare binary, no state-dir/host-endpoint)` is covered by the same enforced path once its own fix (setup hands the mod the hooks' `installed_argv`) lands.
scope-filter: 4 in-scope · 4 punch-listed
roastCodeExit: converged
regressionPass-round-2: [Should-fix] src/protocol/watch.rs:86 (marker lacks instance selectors) filed with [Nit] src/protocol/watch.rs:86 (lazy-row marker) folded in — same function, same fix; deviation: final-review-3 F1 (mod frame drops author_role/relays_user/user_intent) filed in the same re-entry instead of the punch list — correctness defect on the default-on delivery path of a goal-named behaviour
baseAbsorbed: 3d10e3bb → 81dbb244, 11 conflicted files (behavioural: setup.rs D8 port into main's Claude setup backend, claude version gate moved to setup-status only, watch --harness renamed to mod_harness)
baseAbsorbed: 25db37f3 → 9d66d07c, 0 conflicted files (7 test-only commits)
postLoopFix: final-review-5 F1 (stale mod queue after channel loss, breaks TRUST-POLICY A4) and F2 (unindented frame bodies, TRUST-POLICY limit mis-stated) filed as fix beads after loop exit instead of the punch list — invariant violation and a false policy statement; ht-j16.9 ran its fallback (copied profile not signed in), live run to be retried against the original spike profile
followUps: ht-182 (mod worker supervision), ht-22y (attention marker selectors), ht-oag (unbounded mod sets)
liveStress: ht-j16.9 live run (real Claude Code 2.1.295 TUI, 3 iterations) at c6caf381, merged as live-stress-driver: 11/14 scenarios 3/3, nothing lost, no duplicate acks; product defects D1 clear_rebind 0/3, D2 reload_mid_turn 1/3, D3 denied_tool 0/3 (docs/evidence/claude-mod-delivery/README.md)
baseAbsorbed: 9d66d07c → 90066db1, 0 conflicted files (1 test-schedule commit)
postLoopFix: live-stress D1–D3 filed as fix beads ht-j16.28/.29/.30 after loop exit instead of the punch list — duplicate delivery and a permanently disconnected mod on the default-on path of goal-named behaviour; live stress re-runs at the final SHA after they land
postLoopFix: final-review-7 Must-fix 1+2 (ht-j16.29 regressions: submit inside an open turn; ack without submit) filed as ht-j16.31; Must-fix 3 (D4 names) as ht-j16.32; Must-fix 4 (stale attention submit per session start) as ht-j16.33 — after loop exit instead of the punch list: loss path and spec-required behaviour on the default-on path
postLoopFix: final-review-8 finding 1 (predecessor batch with an attention block submitted twice; confirmed repro) fixed directly at dc5b2f6c with a delivery test, outside the coordinator; its stress-model and accepted-limit minors go to the punch list
liveStress: re-run at b562d1e4 (final code SHA): 13/14 scenarios 3/3; reload_mid_turn 2/3 — one extra predecessor_submit ledger entry after a submit resolved just before dispose, one submit and one presentation; leak check clean
sweepFix: none needed
