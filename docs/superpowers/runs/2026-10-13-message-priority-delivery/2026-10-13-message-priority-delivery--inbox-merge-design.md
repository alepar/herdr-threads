# Message delivery priority — inbox merge

Parent spec: [2026-10-13-message-priority-delivery-design.md](2026-10-13-message-priority-delivery-design.md) ·
bead `ht-yha.5` (subepic of `ht-yha`) · design mode B (autonomous run).

## Goal

Every inbox v2 page (CLI inbox text/JSON/machine, hook pages, watch drains, accept's thread-scoped display,
the presentation page) emits pending items in `DeliveryOrderKey` order — priority asc, deadline asc with none
last, publication `decision_seq` asc, source rank, per-source ordinal — under the existing row, byte and
candidate-work bounds, with a continuation that resumes the same order.

## Problem description

Inbox v2 today rotates round-robin through five sources (Lazy, Unread, Invitations, Receipts, Warnings), each
walked by ascending per-source ordinal with pending/publication checks mostly applied after the seek. None of
the walks yields rows in key order: deadlines and decision sequences live on joined rows (`receipt_state`,
`send_manifests`, `invitations.deadline_at`), and the receipt walk visits settled history. A min-head merge is
only correct if each source can produce its next item in key order, cheaply and within `CANDIDATE_LIMIT`.

## Main challenges

The sources differ in shape. Delivery rows are the bulk (every non-ACK message, all priorities, possibly large
backlogs) and need an exact, index-backed order. Items with deadlines (explicit-ACK receipts, confirming
invitations) are all priority 0 and few by construction (they always nudge and ACK/accept on handling), but
their deadline is not on any indexable walked row and can be extended after send. Warnings are walked
instance-globally in `(decision_seq, event_offset)` order. The cursor must encode a resumable position for all
of these, and the partly-shown-body continuation must keep working.

## Key decisions made

Split the sources into three **classes** with different ordering strategies, merged by `DeliveryOrderKey`:
(1) **delivery rows** — exact order via a new partial index on `lazy_recipients(seat, priority, decision_seq,
ordinal)`; (2) **deadline class** — invitations plus the receipt-bearing sources (binding Unread and Receipts),
collected in a bounded **window** of up to `DEADLINE_WINDOW = 64` pending items in their existing walk order and
sorted in memory, exact whenever a seat's pending deadline-class backlog fits the window; (3) **warnings** —
already in key order (p0, no deadline, `decision_seq`). The cursor becomes format version 2: last emitted key,
per-class positions, captured high waters, and the body continuation. Version-1 tokens are rejected with the
existing `InvalidCursor` error.

## Decision points, by section

**Delivery-row class (exact).** Recommended: v47 adds `lazy_recipients_pending_order ON lazy_recipients(seat_id,
priority, decision_seq, ordinal) WHERE state='pending' AND decision_seq IS NOT NULL` and the thread-scoped twin
`(seat_id, thread_id, priority, decision_seq, ordinal)` with the same predicate. (Superseded 2026-10-14: see Post-Implementation Notes.) `decision_seq` is copied onto the
row at publication (`note_published`, owned by the wake child `ht-yha.4`) and backfilled by the migration, so "published and not past the captured
high water" is `decision_seq IS NOT NULL AND decision_seq <= dh` inside the seek — no unpublished row is ever
visited, which strictly improves the 2026-10-07 bounded-walk rule. The seek resumes with `(priority,
decision_seq, ordinal) > last` and `LIMIT 1`. Binding-scoped readers keep their post-seek unread/display filter, which also excludes rows marked
displayed for that binding in `binding_row_displays` (root spec §Delivery model, Ordering; each rejected candidate costs one
visit, as today). Considered and discarded: keeping the ordinal walk and
sorting a window (a p0 row behind a large p1 backlog would be invisible until the backlog drains — the exact
case priority exists for).

**Deadline class (windowed).** Recommended: invitations, binding Unread and Receipts keep their current walks
(ascending ordinal, existing indexes and pending/publication checks) but feed a window: the class collects
pending items until it holds `DEADLINE_WINDOW` (64) items or all three walks are exhausted, charging every visited
candidate to `CANDIDATE_LIMIT`. The window is sorted by `DeliveryOrderKey` (deadline from `effective_receipt` /
`invitations.deadline_at` for `require_ack` invitations, None otherwise; `decision_seq` from the effective receipt
or `invitations.created_decision_seq`). If the walks were exhausted, the class order is exact. If the window
filled first, the class emits the window in key order, then the next window from where the walks stopped
(accepted limit: with more than 64 pending deadline-class items, order is exact within each window and windows
follow arrival order; these items always nudge and settle on display, so such a backlog is pathological; the
limit is user-visible ordering only and is documented in `docs/agent-usage.md` by the docs child). Considered and discarded: (a) a new denormalized ordered projection of pending receipts with triggers
keeping deadline current across extensions — exact but a large new invariant for a small class; (b) dropping
deadline from the key for receipts — contradicts the user's ordering amendment.

A window also closes when it has spent `CANDIDATE_LIMIT / 2` visits, recording its end positions; it is then
emitted in key order like a full one. A page that stops mid-emission records the window's start and end positions,
and the next page re-collects exactly that window (strictly cheaper than a page's work budget) and skips items
whose key is <= the last emitted key, so no item is skipped and every page advances.

**Warnings class.** Recommended: unchanged walk (`scan_global_logical_candidates`), already in key order within
p0/no-deadline; its head competes in the merge like any other. Its 2-visit reservation stays.

**Merge loop.** Recommended: replace `rotate` with: each step, ensure each unfinished class has a head (delivery
rows: one indexed seek; deadline class: the next item of the sorted window, collecting a window if empty;
warnings: one candidate scan), compare heads with `DeliveryOrderKey`, emit the smallest, advance that class.
Heads already peeked are carried in memory within one request; across pages only positions are persisted, and a
peeked-but-not-emitted head is re-peeked (bounded: at most one seek per class per page start). Row/byte/work
stops and the never-false-`empty` rule are unchanged. A partly shown body resumes first, exactly as today, and
its key becomes the last emitted key once it completes.

**Caller subsets.** Recommended: unchanged semantics expressed as class subsets — CLI inbox, delivery pull, hook
inbox and watch use all classes; `ordinary_presentation_page` uses delivery rows plus binding Unread (deadline
class without invitations); accept's thread-scoped display uses delivery rows only (all priorities, below the
first unread receipt-bearing row, per the pair/invitation child `ht-yha.6`).

**Cursor format v2.** Recommended: `ib2:` prefix and binding tag unchanged; payload version byte 2 carrying the
last emitted `DeliveryOrderKey` (encoded by `DeliveryOrderKey`'s continuation encoding from `ht-yha.1`), the
delivery-row position (implied by the last key when the last emitted item was a delivery row, else stored), the
deadline-class walk positions (invitation, unread, physical/manifest receipt after/high, next-manifest flag), the
window start positions, the warning positions and offer cutoff, the captured high waters, the done flags and the
body continuation. Version 1 decodes to `InvalidCursor` ("cursor from an older daemon; run inbox again"); a client
that holds a v1 `next:` line across an upgrade reruns `inbox`. Considered and discarded: decoding v1 into v2
(no key information to recover; the traversal must restart anyway).

**Index list for v47 (`ht-yha.2`).** Only the two delivery-row indexes above are required; the deadline class
reuses existing indexes (`invitations_seat_decision`, `prepared_recipients` seat/ordinal, receipts seat/ordinal).
`ht-yha.2`'s receipts/invitations index items are withdrawn.

**Tests.** Merged order across classes (p0 deadline receipt before p0 deadline-less invitation/warning/row; p0
before p1; equal priority none-last; equal deadlines by decision_seq; equal decision_seq by rank); a p0 delivery
row behind 500 pending p1 rows comes first within one page and one seek; window overflow (70 pending
receipts) keeps exact order within windows and loses nothing across pages; continuation stability for read-only
JSON paging split inside an equal-priority run; work-limited pages with a partial window resume without skips;
large unpublished backlogs visit zero unpublished delivery rows (EXPLAIN plan on the new index); v1 cursor →
`InvalidCursor`; body continuation resumes first; caller subsets (presentation page, accept). Rewrite the
rotation-order tests (`lazy_inbox_v2_bounded_round_robin_opportunity`, the q4 visit-99 test, the CancelClock
call-count test, binding page-order vectors, cursor codec round trip, presentation partial-body tests).

## Key abstractions

Builds within the root spec's frame (`DeliveryPriority`, `NudgeThreshold`, `DeliveryOrderKey`, Inbox v2 merge); refines the Inbox v2 merge, never redefines it.

**OrderClass** — new closed enum in `mod inbox_v2` (`DeliveryRows | Deadline | Warnings`), what the merge compares heads across
- Owns: the class membership of the five `Source`s (Lazy → DeliveryRows; Invitations, Unread, Receipts → Deadline; Warnings → Warnings) and the three per-class operations as `match`es: ensure head, advance past the emitted head, done.
- Hides: that Deadline is fed by three walks; the merge sees three heads, not five sources.
- Must not: compare keys (`DeliveryOrderKey` does) or hold rows across requests.
- Relationships: the merge depends on it; arms delegate to `DeliveryRowSeek`, `DeadlineWindow` or the existing `scan_global_logical_candidates` path. `Source` stays the closed enum tagging an item's origin (and the body continuation's owner); it is no longer a rotation pointer.
- Lifecycle: static; heads live in memory for one request.

**DeadlineWindow** — new struct in `mod inbox_v2`, the windowed ordering strategy for the Deadline class
- Owns: collecting pending items from the invitation, unread and receipt walks in their existing order until the window holds `DEADLINE_WINDOW` (64) items, the walks are exhausted, or the window's visit share (below) is spent, charging every visited candidate to `CANDIDATE_LIMIT`; building each item's `DeliveryOrderKey` (deadline from `effective_receipt`, or `invitations.deadline_at` for `require_ack` invitations only; `decision_seq` from the effective receipt or `created_decision_seq`); sorting and emitting in key order; the existing receipt/unread dedupe (listed once, by Unread); tracking the window's start positions and the walks' stop positions.
- Hides: the exact-within-a-window limit, and the skip rule: re-collecting the same window from its start positions skips items whose key is <= the last emitted key, while a fresh window (started at the stop positions) skips nothing, since a later window may legitimately carry lower keys.
- Must not: persist peeked items (only positions round-trip); read deadlines anywhere but `effective_receipt` and `invitations`; decide settlement; be resumed by a global "key > last" seek.
- Relationships: uses the existing seeks unchanged (`scan_effective_receipts`, `unread_ordinal`, `receipt_pending`), `DeliveryOrderKey` from `ht-yha.1`; persists through `InboxBatchV2CursorState`.
- Lifecycle: rebuilt per request from cursor positions; at most one window collected per page start.
- Errors: none new; store errors propagate, exhausted work is the merge's `StopReason::Work`.

**DeliveryRowSeek** — extended `lazy_delivery` (`pending_page_in`): the ordinal walk becomes the exact ordered seek on the v47 indexes
- Owns: the seek `(priority, decision_seq, ordinal) > position` with `decision_seq IS NOT NULL AND decision_seq <= dh`, `LIMIT 1`; the binding post-seek unread/display filter (one visit per rejected row); the thread-scoped stop below the first unread receipt-bearing row; copying `decision_seq` onto rows in `note_published`.
- Hides: the index shape and SQL; no unpublished row is visited.
- Must not: be duplicated: the presentation path's own ordinal SQL in `scan` (`Source::Lazy`, `ordinary_only`) goes through this seek with `PRIVATE_LAZY_ELIGIBILITY_SQL` as its post-seek filter.

**PageSubset** — new closed enum (`All | Presentation | ThreadScoped`), the caller subsets
- Owns: one function that marks classes/walks done for a subset, applied at capture and after cursor decode: `Presentation` = delivery rows + the Unread walk; `ThreadScoped` = delivery rows only (an `Unknown` reader shows nothing).
- Hides: the subset rules now scattered across `capture(ordinary_only)`, `query_mode`'s post-decode normalization and the `q.thread.is_some()` block.
- Must not: add per-subset branches inside the merge or the classes. `ordinary_only` survives only as "private presentation eligibility" (body retirement, probe accounting).

**InboxBatchV2CursorState** — extended to format v2: version byte 2; per-class position structs (DeliveryRows triple, Deadline walk + window-start positions, Warnings positions and offer cutoff) instead of embedding v1's `SeatAttentionCursorState`; the last emitted `DeliveryOrderKey`; captured high waters, done flags, body continuation tagged with its `Source`. The delivery-row position is elided when the last emitted item was a delivery row. v1 decodes to `InvalidCursor` with detail "cursor from an older daemon; run inbox again" (same code; the detail distinguishes it from a malformed cursor or "inbox binding changed").

**Inbox v2 merge** — extended: `rotate` becomes a min-head loop over `OrderClass` heads; a partly shown body resumes first and its key becomes the last emitted key once complete; row/byte/work stops and the never-false-`empty` rule unchanged; the Warnings 2-visit reservation applies when its head needs a scan; each class's persisted position is authoritative, the last key serves only window re-collection skips and delivery-row position elision.

Variation: fixed classes with growing operations → closed `OrderClass`; fixed walks in the Deadline class → closed `Source` arms; fixed caller subsets → closed `PageSubset`. No traits: every kind has one implementation and the spec names no open-ended kinds.
Walk-through: a bound reader's hook page: capture (high waters, `PageSubset::All`) → each step ensures heads (DeliveryRows: one indexed seek, so a p0 row behind 500 p1 rows comes first; Deadline: collect and sort a window; Warnings: one candidate scan) → `DeliveryOrderKey` picks the minimum, emitted, its class advances → page stops on rows/bytes/work, cursor v2 stores positions, last key and body continuation. Replacing the window with an exact pending-receipt projection later touches `DeadlineWindow`'s internals and its cursor position struct only.
Per spec §Deadline class (gap fill): a window also closes when its visit share — `CANDIDATE_LIMIT / 2` visits — is spent, recording its end positions, so re-collecting a persisted window always costs strictly less than one page's work and every page advances (the receipt walk visits settled history, so an uncapped window could exhaust a page and loop).

Digest:
- OrderClass: owns each Source's class and the per-class head/advance/done operations;
  a new ordering strategy is a new arm here; callers never iterate sources or rotate.
- DeadlineWindow: owns window collection, key building for invitations/unread/receipts, sorting and the
  re-collected-vs-fresh skip rule (bounded so every page advances); a better deadline ordering changes only its
  internals; callers never sort deadline items or resume them by the global last key.
- DeliveryRowSeek: owns the one ordered (priority, decision_seq, ordinal) seek with the publication bound inside it
  and the post-seek binding filter; index or eligibility changes go here; callers, the presentation path
  included, never write their own lazy_recipients walk.
- PageSubset: owns which classes and walks each caller sees, applied at capture and decode; a new caller is one
  new variant; callers never set done flags or test ordinary_only/thread scope to hide classes.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### 2026-10-14, ht-yha.5.4 (merge loop, OrderClass, PageSubset)

- `Presentation` keeps the Receipts walk. Today's private presentation traverses binding Unread **and** Receipts
  (it excludes only invitations and warnings), so `PageSubset::Presentation` marks only the invitation walk and
  warnings finished; the subspec's "delivery rows + Unread" would have dropped pending receipts the presentation
  shows today. `capture` also skips the invitation high water for it (it neither reads nor bounds invitations, and
  the unindexed `max(ordinal)` over a seat's invitations grew with history).
- A partly shown body is pinned by the page's state before its head. A delivery-row body resumes straight from its
  `lazy_recipients` row; a deadline-class body (Unread or Receipts) needs the re-collected window's head to be that
  message (a head ordered elsewhere is an `InvalidCursor`), and the cursor stores the window's end positions
  (`DeadlineWindow::persist_partial`) so the re-collection is exactly the original window.
- `DeadlineWindow` no longer calls a walk whose position already reached its captured high water (an exhausted
  walk is not queried; the invitation query otherwise scanned the seat's invitations for an empty range).
- Delivery-row work: the row seek keeps two visits in reserve for the warnings scan while warnings are unfinished;
  a deadline window starts only when `WINDOW_VISITS + 8` work remains, so a page never exceeds `CANDIDATE_LIMIT`.
- Found while converting readers: a top-level completion used to flip `lazy_recipients.state` to `displayed`
  seat-wide at once, which hides the row from the ordered seek (it indexes `state='pending'` only) for a sibling
  top-level binding that has not displayed it. The flip now follows the cursor moves and happens only when no
  connected top-level binding still finds the row unread (`MARK_DISPLAYED_SQL`); TRUST-POLICY.md says so.
- `delivery_mode` readers converted: `RECEIVABLE_WHERE` and `continued_ordinary` rely on the `prepared_recipients`
  join, `ordinary_presentation_message_by_id` branches on whether the seat holds a delivery row, and hot-thread
  recency counts priority-0 messages (the legacy `ordinary` mode maps to priority 0).

### 2026-10-14, ht-yha.44 / ht-yha.52 / ht-yha.54 (per-binding views, pending-only deadline walks)

The delivery-row class seeks `binding_pending_order` / `binding_pending_thread_order` on the reading binding's own view; a seat-level reader min-merges the connected top-level views and `lazy_recipients_legacy_order` (`seek_pending` in `src/store/lazy_delivery/inbox.rs`). The two v47 `lazy_recipients_pending_*order` indexes were dropped. The 2026-10-14 ht-yha.5.4 note about `MARK_DISPLAYED_SQL` describes an intermediate state: the seat-wide `state` is no longer read by any seek. The deadline class's invitation and receipt walks seek pending-only projections (ht-yha.52), so settled and unpublished history costs no window visit.
