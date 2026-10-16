# super-auto run — 2026-10-15-inbox-injection-auto-ack

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: roast-code
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
  completed: ht-794l.1, ht-794l.2, ht-794l.3, ht-794l.4
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: ready (final-review-code.md; caller final-SHA sweep and combined-node proof pending)
  sweep: SWEEP DEFERRED (caller-owned)
  slowness: fresh task-path links and peer contention exceeded build budgets; merged-tree lint 1m32 and 2m04; no demonstrated source-only regression
  worktreesKept:
  processSweep: stopped 0 · survived 0
roastCodeRound: 1
