# super-auto run — 2026-10-18-bytes-first-journal

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: code
codeMechanism: Workflow
runTag: bytes-first-journal-20261019T012327
context: Context: claude · planner=200K/167K · implementer=200K/167K · reviewer=200K/167K · finalReview=200K/167K · assembler=200K/167K · warn: planner, finalReview on a 200K model → set ANTHROPIC_DEFAULT_OPUS_MODEL=<model>[1m] or config.models.<role> to a 1M model · warn: implementer, reviewer, assembler on a 200K model → set ANTHROPIC_DEFAULT_SONNET_MODEL=<model>[1m] or config.models.<role> to a 1M model
skillSource: /Users/alepar/.aisw/profiles/claude/claude-1/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar6.4/skills @ v6.4.2-alepar6.4 (6.4.2-alepar6.4)

idea: Rework the peer-wire-compat-hardening storage bytes-first before 2.3.0 (user decision via main tab msg mQ0V2b8xmSWQ): journal stores original canonical bytes + signature (+hash) as the row with parsed columns as a derived projection; unknown layouts stored with empty projection and parked_unknown (apply stalls as today's opaque_layout); relay opaque by default (serve raw bytes; WireItem::Opaque only to envelope.opaque.v1 peers, known shape unchanged); no journal_opaque table, no union readers; unreleased node delta 0005 rewritten to migrate existing rows by recomputing HTJRNL01 canonical bytes; promotion = re-derive projection and unpark (delivers R4; close ht-evlm as superseded). No wire change, 2.3.0 minor. Keep capability gating, golden fixtures/freeze script, 2.1.1 shadow decoder, check-wire-compat, CI tag fetch, invariant pins. Required tests: schema-4 store round-trips byte-identically through the migration; unknown-magic entry stored, relayed and promoted after a test-hook upgrade (successors unpark). Report SHA + evidence to main tab (dlx5Kb7kgke / sQ0VVjdFD4f9); do not merge to main.
branch: super-auto/bytes-first-journal
base: main
spec: 2026-10-18-bytes-first-journal-design.md
epic: ht-76u6
approvals:
- top-split · auto · all 7 children LEAF (no subepics)
roastDesignRound: 3
roast-design: 2026-10-18-bytes-first-journal-roast-design-1.md, 2026-10-18-bytes-first-journal-roast-design-2.md, 2026-10-18-bytes-first-journal-roast-design-3.md
stepBackDesign-round-1: local — orchestrator inline (no step-back dispatch): findings cluster on migration mechanics (FK rebuild, placeholder, marker, fk_check), state reuse, promoted serving; each has a concrete local fix; no mechanical trigger
stepBackDesign-round-2: local — orchestrator inline: schema-audit replay, magic-based decode, test list; no mechanical trigger
roastDesignExit: capped at round 3 — 0 Blocking; 3 confirmed (2 Should-fix, 1 Nit) fixed inline as spec/bead corrections (RAW_MAX, magic-based SQL classification, index list)
codeBuckets:
  sweep: SWEEP DEFERRED (caller-owned)
