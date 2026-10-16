# super-auto run — 2026-10-15-inbox-injection-auto-ack

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: report
codeMechanism: ordinary-subagents

idea: hooks/channels only direct to call inbox when directly injected messages doesnt cover entire inbox, ie there are more. agent should never need to call inbox explicitly, unless there's a spike of messages received and it overflows the size that hook/channel can inject in a single turn. messages injected by hooks/channels or read via inbox do generate automatic receipt ack. normally, agent should never have the need to call explicit ack cli command. let's fix via $super-auto and communicate to p2p tab (it might care about the second ruling specifically, make sure this auto ack works scross multiple nodes too). fully autonomous, both roasts on.
branch: super-auto/inbox-injection-auto-ack
base: main
spec: 2026-10-15-inbox-injection-auto-ack-design.md
epic: ht-794l

approvals:
- top-split · auto · ht-794l.1 LEAF, ht-794l.2 LEAF, ht-794l.3 LEAF, ht-794l.4 LEAF
- coverage-round-1 · canonical R-list: R1 Complete/empty hook/channel injection requires no explicit inbox or ACK call. · R2 Undisplayed overflow, truncation, failed/unavailable native delivery emits exact bounded inbox continuation. · R3 Fully delivered eligible native hook messages automatically settle existing receipt obligations after successful context write+flush with truthful provenance. · R4 Text inbox, channel and hook-pull preserve complete-display auto ACK, idempotence, read-only/partial/source safety and no-receipt defaults. · R5 Bounded channel batches preserve order, retained tail and recovery at 32 items/64KiB, with no partial/tail ACK. · R6 Canonical source validation and receipt authoring preserve cross-node convergence and truthful source; combined final candidate verification is caller-owned. · R7 Shared guidance and model-visible recipes explain automatic receipt ACK once and reserve inbox/manual ACK for actual fallback or exceptional reads. · R8 Preserve binding-specific streams, invitation consent, wake predicates, output limits and old-daemon compatibility. · requirements: 8 · mapped: 8 · unmapped: 0 · auto no findings; two valid independent reviews, no tree change; clean exit after round 1

roast-design: 2026-10-15-inbox-injection-auto-ack-roast-design-1.md, 2026-10-15-inbox-injection-auto-ack-roast-design-2.md
roastDesignRound: 2
stepBackDesign-round-1: patch — Gate attention-mark advancement on successful post-output completion so the next eligible hook can retry without unrelated mail; no higher-level redesign is needed.
graph-pass: depth 3→3 · width 1.3→1.3 · applied 0 · parked 0

codeBuckets:
  completed: ht-794l.1, ht-794l.2, ht-794l.3, ht-794l.4, ht-794l.8, ht-794l.9, ht-794l.10
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: ready (final-review-sweep-fix.md; affected sweep and scoped post-cap audit passed @ 4274f130)
  sweep: 4274f130 — 1572 passed, 0 failed, 0 errors, 4841 skipped; failing: none; command: nice cargo nextest run --locked --all-features --lib --test combined --test integration --test hook_entrypoint -E 'test(hook) | test(handoff) | test(lazy_inbox) | test(receipts) | test(mod_ack) | test(mod_delivery) | test(lazy_config_smoke) | test(lazy_settlement) | test(accept_delivery) | test(lazy_recovery_context) | test(cooperative) | test(delivery::page) | test(protocol::watch) | test(protocol::capabilities) | test(lazy_delivery)' @ 4274f13009409dbef10d26a7fb32a1dbf92bf666
  slowness: Fresh task-path Rust links/cold metadata and peer contention greatly exceeded1m build/lint budgets; initial all-target filtered build narrowed thereafter; Measured merged-tree lint1m32 and2m04 (actual checking, no lock messages); preserve limitation, no demonstrated source-only regression; Scoped process helper requires escalation; sandbox ps/kill may claim inaccurate stopped; authoritative final cleanup clean; .8 task clippy62s; merged-tree clippy1.35s no lock wait; final profile model108%actual; New profile retry wait/bottleneck includes earlier invocation time before sweep-fix tasks9/10 existed; latestcap2/historicalcap4 distinction. Not actual task idle delay.; Final sweep-fix merge checks23.86s/23.63s clippy, all gates under1minute.
  worktreesKept:
  processSweep: stopped 0 · survived 0
roastCodeRound: 2
roast-code: 2026-10-15-inbox-injection-auto-ack-roast-pr-1.md, 2026-10-15-inbox-injection-auto-ack-roast-pr-2.md, 2026-10-15-inbox-injection-auto-ack-roast-pr-post-cap-audit.md
stepBackCode-round-1: patch — independent sibling-isolation regression and stale policy wording need focused fixes; neither exposes a flawed design decision.
scopeFilter-round-1: [Should-fix] src/cli/hook_inbox.rs:365 in-scope — Missing regression coverage for goal-named automatic receipt settlement preserving pending delivery and independent wake eligibility for sibling bindings.
scopeFilter-round-1: [Nit] TRUST-POLICY.md:1702 punch-list — Correcting superseded historical policy wording is documentation hygiene; the goal names delivery and receipt-settlement behavior, and the operative policy sections already describe that behavior correctly.
scope-filter: 1 in-scope · 1 punch-listed
roastCodeExit: converged
sweepFix: 17 failing → ht-794l.9, ht-794l.10 · re-run 4274f130 — 1572 passed, 0 failed, 0 errors, 4841 skipped; failing: none; command: nice cargo nextest run --locked --all-features --lib --test combined --test integration --test hook_entrypoint -E 'test(hook) | test(handoff) | test(lazy_inbox) | test(receipts) | test(mod_ack) | test(mod_delivery) | test(lazy_config_smoke) | test(lazy_settlement) | test(accept_delivery) | test(lazy_recovery_context) | test(cooperative) | test(delivery::page) | test(protocol::watch) | test(protocol::capabilities) | test(lazy_delivery)' @ 4274f13009409dbef10d26a7fb32a1dbf92bf666
friction: 18 events
