# super-auto run — 2026-10-07-lazy-messages

flags: planOneShot=t skipPlanRoast=f skipCodeRoast=f autonomous=t
resumeChange: 2026-10-08 direct user redesign of native send default — lazy by default, explicit --nudge for attention, ACK implies nudge; interim --ordinary superseded; applied within current code run, wire/saved omission stays Ordinary.
phase: roast-code
codeMechanism: ordinary-subagents

idea: lgtm, let's $super-auto this, fully autonomous, both roasts are on
branch: super-auto/lazy-messages
base: main
spec: ../../specs/2026-10-07-lazy-messages-design.md
epic: ht-big
roast-design: 2026-10-07-lazy-messages-roast-design-1.md
roastDesignRound: 1

approvals:
- design-review · approved
- coverage-round-2 · auto no new findings; C1/C2 closed · requirements: 13 · mapped: 13 · unmapped: 0
- coverage-round-1 · C1 auto applied (ht-big.2.2); C2 auto applied (ht-big.2.1, ht-big.2.2, ht-big.3, ht-big.6) · requirements: 13 · mapped: 13 · unmapped: 0
  R1 Existing frozen participants receive nonurgent lazy announcements only at natural explicit inbox checks.
  R2 Lazy delivery creates no wake, poke, notification, model turn, ACK obligation or automatic adoption.
  R3 Send --lazy is immutable in durable intent and rejects ACK seat/pane/deadline combinations at CLI and daemon.
  R4 Bounded frozen recipient staging/publication/cleanup survives retry and restart with no receipts or attention and indexed work-limited pending scans.
  R5 Explicit v2 inbox supports bounded UTF-8/body/source paging and stable captured high waters with distinct lazy items and truthful continuations.
  R6 JSON, machine and explicit-seat discovery remains read-only; v1 hooks/check-in remain actionable-only.
  R7 Only fully written/flushed contiguous default-text bodies settle exact lazy deliveries idempotently under canonical claim with no receipt/adoption evidence.
  R8 Mixed ordinary ACK and lazy completion recover through independently durable frozen intents, exact retry references and selective cleanup despite either failure.
  R9 Frozen Human/operator or agent claim/harness/scope remains correctly classified through submission, retry, completed presentation and cleanup.
  R10 Postjoin, leaving, retired-seat and archival behavior preserve addressed deliveries without creating new backlog or attention.
  R11 Canonical lazy markers and complete body history/search/summary discovery preserve summary formats/cache and ordinary timeline semantics.
  R12 Ordinary digest/v1/old-daemon compatibility and unsupported lazy-send refusal preserve existing behavior with bounded read-only mode metadata.
  R13 Isolated homes/private owned children, meaningful focused regression gates and reviewed exact migration prerequisite integration deliver merge-ready changes without touching frozen release/shared server.
- top-split · auto · ht-big.1 LEAF, ht-big.2 PROMOTE, ht-big.3 LEAF, ht-big.4 PROMOTE, ht-big.5 PROMOTE, ht-big.6 LEAF, ht-big.7 LEAF, ht-big.8 LEAF, ht-big.9 LEAF

parked:
- user brief · degraded-verdict · "Full-suite sweep delegated to main; worker runs focused gates only, per explicit user scope."
- user brief · degraded-verdict · "Merge/main-only push already authorized subject to main granting actual mutation window; preserve release freeze."

graph-pass: depth 8→8 · width 1.6→1.6 · applied 0 · parked 0

codeBuckets:
  completed: ht-big.1, ht-big.9, ht-big.2.1, ht-big.4.1, ht-big.2.2, ht-big.8, ht-big.3, ht-big.4.2, ht-big.5.1, ht-big.5.2, ht-big.6, ht-big.7, ht-big.10, ht-big.11
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: ready
  sweep: SWEEP DEFERRED (caller-owned)
  slowness: ordinary-subagents serial cap1; historical detector queue/idle measurement invalid; profile recovered13landed/13planned, no Workflow throughput claim
  worktreesKept:
  processSweep: stopped 0 · survived 0
roastCodeRound: 2
roast-code: 2026-10-07-lazy-messages-roast-pr-1.md
stepBackCode-round-1: patch — preserve invocation context consistently across continuations; independent payload/batching/proof-retention claims.
scopeFilter-round-1: [Blocking] src/store/queries.rs:5959 in-scope — Blocking, always in-scope
scopeFilter-round-1: [Should-fix] src/cli/mod.rs:2057; src/store/queries.rs:5959 in-scope — Incorrect behavior in the goal-named explicit inbox path: the supplied human continuation prevents the originating participant from completing the displayed announcement body.
scopeFilter-round-1: [Should-fix] src/protocol/output_compact.rs:77; src/protocol/results.rs:896 punch-list — The goal does not name invitation goal payloads or preservation of invitation rendering; this defect concerns an adjacent inbox behavior.
scopeFilter-round-1: [Should-fix] src/cli/follow.rs:813 punch-list — Batching delivery-mode lookups in human follow is a performance improvement outside the goal-named natural explicit inbox delivery behavior, with no correctness defect established.
scopeFilter-round-1: [Nit] src/cli/journal.rs:688 punch-list — Bounded cleanup of abandoned local display proofs is extra hardening; the finding establishes retained files and scan work, not incorrect delivery or violation of the goal's attention boundary.
scope-filter: 2 in-scope · 3 punch-listed
