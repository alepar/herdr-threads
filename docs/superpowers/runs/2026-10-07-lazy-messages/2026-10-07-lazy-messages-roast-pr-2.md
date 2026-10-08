super-roast verdict: Blocking (3 confirmed)
mode: pr        iteration: 2 of 3
profile (assumed): Durable local coordination software used on real developer sessions. Trust is cooperative and same-user, with canonical daemon authority and honest receipt and actor provenance. Real data loss, migration corruption and forced attention violations matter; no network-adversarial caller model is assumed. Main owns the final integrated suite and release/install; this is isolated source review.
inputs: super-auto/lazy-messages@64b3dabb3642f2024d7c17c5eeb52de497f0206e vs main@4f7cad2ddadf0f3e9bf917a36917624821b3be77
delta vs prior: 3 new confirmed (1 Blocking) · 0 carried (0 Blocking) · 2 resolved · 0 regressed (0 Blocking) · 3 punch-listed (open)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 3 → deduped 3 → panel 3 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 3 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=3) · reproduce 3/0/0 · refute 3/0/0 · ground 3/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/1/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 1/1/1/0 · observability 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 1/1/1/0

Prior confirmed status:
- [Blocking] src/store/queries.rs:5959 — Explicit-seat and machine read-only continuation selector loss: resolved; not re-surfaced by the completed current scouts and panels. The current legacy-cursor finding concerns protocol selection, not lost selectors or settlement.
- [Should-fix] src/cli/mod.rs:2057; src/store/queries.rs:5959 — Human continuation namespace loss: resolved; not re-surfaced by the completed current scouts and panels.
- [Should-fix] src/protocol/output_compact.rs:77; src/protocol/results.rs:896 — Missing invitation goal payload: punch-listed (open), see iteration 1. The caller deliberately left this unfixed; absence from current packets is not resolution.
- [Should-fix] src/cli/follow.rs:813 — Serial per-message delivery-mode RPCs: punch-listed (open), see iteration 1. The caller deliberately left this unfixed; absence from current packets is not resolution.
- [Nit] src/cli/journal.rs:688 — Retained abandoned lazy display proofs and allocation scan growth: punch-listed (open), see iteration 1. The caller deliberately left this unfixed; absence from current packets is not resolution.

## Confirmed findings
- [Blocking] src/store/queries.rs:5720 — A lazy send to an otherwise quiet joined thread makes the recipient's Compact/Resume/Clear hook demand thread-summary work before continuing, without an explicit inbox check. (new) [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats verify src/store/messages.rs:738 publishes Lazy with kind='ordinary'. HotThreads at src/store/queries.rs:5717-5726 selects ordinary-kind messages without filtering delivery_mode; :5740-5742 adds Recent for a joined nonarchived thread solely because of the fresh lazy publication.
  evidence: With prior activity outside the default 24-hour window and no receipts, invitations or warnings, the lazy publication alone changes an empty recovery result into a hot thread. src/cli/hook.rs:1955-1978 reads it on top-level Compact/Resume/Clear; src/harness/mod.rs:576-599 retains Recent and :554-558 requires the thread-summary procedure for each hot thread before continuing. src/cli/hook.rs:483,518-521 emits this instruction before any inbox read.
  evidence: Reproduce, refute and ground distinguish the emitted work obligation from a wake RPC or guaranteed model execution. They address the permitted inclusion in summary sources: permission to read lazy content does not authorize new recovery work caused solely by its arrival.
  severity floor: The approved lazy design's Goal requires natural explicit-inbox delivery, and docs/superpowers/specs/2026-10-07-lazy-messages-design.md:19 explicitly forbids lazy arrival requiring new model work. The unsolicited mandatory recovery instruction violates this artifact's core passive-delivery purpose. This Blocking floor overrides the three Should-fix seat labels.
  fix-shape hint: Exclude lazy-only arrivals from recency-driven recovery obligations while retaining lazy content in explicitly requested timelines and summaries.

- [Should-fix] src/cli/mod.rs:908 — The CLI upgrades legacy inbox cursor continuations to v2 solely on daemon capability, making the v1 continuations still emitted by bounded check-in inbox pages fail instead of reaching the remaining items. (new) [lanes: api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats trace the same-build failure. src/store/mod.rs:1704-1715 retains the actionable-only legacy check-in inbox with a default 20-item bound. src/store/queries.rs:4838-4861 creates its legacy c3 cursor, :5422-5435 emits inbox --seat SEAT --cursor TOKEN, and src/protocol/output_compact.rs:811-813 prints inbox.next.
  evidence: For more than 20 actionable threads, following that command reaches run_wire. src/cli/mod.rs:907-913 changes Inbox to InboxBatchV2 solely on the advertised capability while retaining c3. src/protocol/commands.rs:792-794 and src/store/queries.rs:6287 require the v2 decoder; src/protocol/pagination.rs:248-255 rejects c3 as not a v2 cursor.
  evidence: Refute confirms selector and actor preservation at src/cli/output.rs:84-160 does not preserve cursor protocol. Base run_wire sent the legacy command unchanged. The approved design at :29-31 retains v1 hook/check-in handling, so the advertised continuation must remain executable.
  severity: This is an actionable-page compatibility failure with a fresh-inbox workaround; the evidence establishes no read-only settlement, lost data or violation of passive lazy arrival. Should-fix stands.
  fix-shape hint: Respect the supplied cursor namespace when selecting the inbox protocol, preserving legacy continuations against v2-capable daemons.

- [Should-fix] src/cli/commands.rs:2191 — The embedded daily-loop guide still teaches bare replies followed by waiting for hook notification, although those replies now default to lazy delivery and produce no notification. (new) [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats verify integrations/skill/SKILL.md:52 teaches bare send THREAD --body TEXT; :55 tells waiting peers to finish their turn for hook notification and forbids follow, sleeping and polling. The embedded guide contains no lazy or --nudge explanation. src/cli/skill.rs:9 embeds it, src/cli/mod.rs:351-354 prints it, and src/cli/commands.rs:827 advertises it.
  evidence: src/cli/commands.rs:2191-2199 now selects Lazy for the exact documented reply. src/store/messages.rs:549-570 stages a lazy recipient before ordinary receipt preparation; :762-778 creates send_attention only for Ordinary. A waiting peer with no unrelated attention receives no reply-triggered notification, leaving the documented loop stalled until an independent explicit inbox check.
  evidence: Refute and ground address owner coordination: docs/evidence/lazy-messages/send-guide-handoff.md:12-38 supplies correct default/nudge wording and integration-readiness.md:20 reports adoption, but neither changes compiled guide bytes. Base bare replies created attention, so the changed default introduces the mismatch.
  severity: The runtime correctly preserves the core lazy passivity contract. The defect is contradictory shipped operational guidance for replies intended to notify; Should-fix stands.
  fix-shape hint: Reconcile the embedded guide with the active owner and explain --nudge for replies expected to notify a waiting peer while preserving passive announcements.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] src/store/schema.rs:513 — Opening an existing store with this build permanently raises its schema version to 26, preventing binary-only rollback to the exact base daemon against the same store. (previously rejected; rejection preserved)
  verdict: rejected in iteration 1 (reproduce REJECT / refute CONFIRM / ground REJECT); not re-judged this round.
  reason: The prior reproduce and ground evidence establishes the expected forward-only migration boundary, not a violated rollback requirement. The approved design requires additive migration26; docs/operations.md:151 documents forward-only startup migrations. No current packet supplies materially changed evidence, data loss or failed migration.
  pre-existing on base: src/store/schema.rs:403-408,482-510; src/store/connection.rs:125-130.
  dissent: The prior refute seat confirmed the factual binary-only rollback limit at FYI and explicitly called it pre-existing. The majority addressed that evidence; the rejection stands.

## Unverified nits (spot-checked)
- [FYI] src/cli/journal.rs:697 — record_lazy_displayed_chunk duplicates the existing journal's durable contiguous-display algorithm and file transaction instead of sharing their implementation. (prior spot history retained; not re-judged this round)
  spot outcome: REJECT in iteration 1; retained as an unverified-nit entry, not promoted to panel verification.
  reason: The prior refute evidence at src/cli/journal.rs:742-747,995-1003 identifies duplication but no incorrect advancement, missing durability barrier or failed recovery. Full-claim identity, different read caps and the ordinary complete-body shortcut are semantic differences. Shared implementation remains optional refactoring; the approved reuse requirement concerns send preparation and publication.

## Escalations (need human)
- none
