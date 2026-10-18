# super-auto run — 2026-10-18-peer-wire-compat-hardening

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: code
codeMechanism: Workflow
runTag: peer-wire-compat-hardening-20261018T171300
context: Context: claude · planner=200K/167K · implementer=200K/167K · reviewer=200K/167K · finalReview=200K/167K · assembler=200K/167K · warn: planner, finalReview on a 200K model → set ANTHROPIC_DEFAULT_OPUS_MODEL=<model>[1m] or config.models.<role> to a 1M model · warn: implementer, reviewer, assembler on a 200K model → set ANTHROPIC_DEFAULT_SONNET_MODEL=<model>[1m] or config.models.<role> to a 1M model
skillSource: /Users/alepar/.aisw/profiles/claude/claude-1/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar6.3/skills @ v6.4.2-alepar6.3 (6.4.2-alepar6.3)

idea: Implement all three recommendations from the node-API review research (reports: ~/Documents/HerdrThreads_Transport_Research_20261018/report.md and ~/Documents/HerdrThreads_SshTunnel_Research_20261018/report.md). Fully autonomous, both roasts on. (1) v2-envelope readiness: a future signed-envelope change must let an older node forward a newer envelope; today WireEnvelope is decomposed into named fields and the hash is recomputed from them (verify against src/sync/envelope.rs and the forwarding path first; if confirmed, design the smallest change that lets a node forward original canonical bytes plus signature opaquely, without changing the HTJRNL01 signed shape or breaking mixed-version peers on 2.1.1; if not confirmed, document why and add a test that pins the property). (2) Golden frame streams per tagged release: frozen real peer-session fixtures (hello, have/want/entries frames, signed envelopes for every fact kind, close and fork frames) decoded by current code in CI, modelled on tests/integration/wire_compat.rs and tests/fixtures/old_wire/; include the forward-compat half as far as feasible without retaining old binaries. (3) Compat lints: no deny_unknown_fields in src/sync/frame.rs and the peer wire types, every new wire field #[serde(default)], new frame variants gated by a capability string; as a check script or test following existing scripts/check-* conventions. Constraints: follow AGENTS.md; do not change the signed message-fact shape (ht-i4dd in progress elsewhere). When the integration branch is ready, send a p0 nudge to the main tab (thread dlx5Kb7kgke, seat sQ0VVjdFD4f9, confirm first) asking it to merge and release.
branch: super-auto/peer-wire-compat-hardening
base: main
spec: 2026-10-18-peer-wire-compat-hardening-design.md
epic: ht-j87z
approvals:
- top-split · auto · .1 LEAF, .2 LEAF, .3 LEAF, .4 LEAF (promotion SPLIT applied: .10), .5 LEAF, .6 LEAF, .7 LEAF, .8 LEAF, .9 LEAF
- coverage-round-1 · 12 findings: 11 applied, 1 rejected (cov-2) — see coverage-ledger.md; requirements: 12 · mapped: 12 · unmapped: 0 · all auto
- coverage-round-2 · 7 findings, all applied (auto) — see coverage-ledger.md; no round 3 (cap)
roastDesignRound: 2
findingsLedgerDesign: findings-ledger-design.md
stepBackDesign-round-1: local — shared-cause cluster swept as three rule clusters (chain-union, opaque-apply, cross-generation-contract); two-table choice kept
roast-design: 2026-10-18-peer-wire-compat-hardening-roast-design-1.md, 2026-10-18-peer-wire-compat-hardening-roast-design-2.md
roastDesignExit: round 2 — 0 Blocking, 5 confirmed (1 Nit, 4 Should-fix, all fix-regressions of round-1 fixes) fixed inline as mechanical spec/bead corrections; no round 3 (corrections, not new design decisions)
graph-pass: depth 8→8 · width 1.6→1.6 · applied 1 · parked 0 (edge .10 <- .4 dropped but .4 now depends on .10 for the status field set, so the chain length is unchanged; other 6 candidate edges kept, each names a real consumed artifact)
codeBuckets:
  sweep: SWEEP DEFERRED (caller-owned)
- review-collection · {"invocationId":"inv-A50B5B16-478E-47B7-A35A-E9FB071E2127","workspace":"/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-peer-wire-compat-hardening/.superpowers/sdd/ht-j87z-plan","reviewPath":"/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-peer-wire-compat-hardening/.superpowers/sdd/ht-j87z-plan/final-review-inv-A50B5B16-478E-47B7-A35A-E9FB071E2127.md","review":"pending","required":[],"settled":false}
- root-acceptance · {"version":1,"owner":"super-auto","sourceTree":"pending","requirements":[{"id":"whole-epic-review","kind":"review","owner":"super-auto"},{"id":"final-sweep","kind":"measurement","owner":"super-auto","command":"nice cargo nextest run --locked --all-targets --all-features (relevant subset per AGENTS.md; full suite not run routinely)","configuration":"all-features","requiredLive":false},{"id":"phase7-choice","kind":"human","owner":"user"},{"id":"main-tab-merge-release","kind":"human","owner":"main tab"}],"evidence":[],"reviews":[],"decisions":[],"recovery":[],"checkEvidence":[],"runFailures":[]}
