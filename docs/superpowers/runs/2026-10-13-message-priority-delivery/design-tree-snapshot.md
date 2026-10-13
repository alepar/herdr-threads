### ht-yha [epic] — Message delivery priority replaces lazy/nudge modes
Goal: senders choose a delivery priority 0..=9 (0 most urgent) per message; each seat has a nudge threshold T (default 0; negative = all) and an idle seat wakes on a pending item iff priority <= T. Lazy/ordinary modes and lazy invitations go away (default send p1; invitations and ACK requests p0). Inbox, hook pages and the Claude mod order pending items by (priority asc, deadline asc none last, created asc). 1:1 pairs auto-join both seats.

Spec: docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery-design.md (decisions D1-D11 are user-approved; read the whole spec before starting).

## Global Constraints
- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral`, `cargo fmt`; before merge also `nice scripts/check-default-features`.
- Tests: focused `nice cargo test --locked --all-features <filter>` / nextest for what you change; no routine full-suite runs (the run does one sweep at the end). New suites without process-wide state go into tests/combined.rs as #[path] modules; spawn test children only via herdr_threads::test_support::spawn.
- Every commit carries the trailer `Outside-adapter: <none | comma list of changed paths outside src/harness/agy/>`.
- Never `git push`, never `git stash`; never write ~/.claude, ~/.codex, ~/.aisw, ~/.gemini; never touch the shared Herdr server.
- Harness-neutral core: no harness names in src/cli, src/client, src/service, src/daemon, src/store, src/host, src/protocol, src/app.
- Trust policy: TRUST-POLICY.md is normative; a change that weakens an invariant or changes pair/receipt/delivery bookkeeping updates it in the same commit.
- Compat: no PROTOCOL_VERSION bump; new behaviour is capability-gated (`send.priority_v1`, `seat.nudge_threshold_v1`, `direct.auto_join_v1`); omitted wire fields keep historical payload digests unchanged.
- Model-visible strings: state their size in the commit message (AGENTS.md "Token-efficient communication").
<!-- key-abstractions digest: begin -->
- DeliveryPriority: owns range, default, fixed-p0 classes, legacy mode mapping and the `[pN]` marker; new legacy spellings are added only here; callers never match on `DeliveryMode` to decide anything.
- NudgeThreshold: owns the `p <= T` wake predicate and the `wake-on` alias mapping; per-thread overrides go into the code that looks it up; callers never compare priority against T inline.
- DeliveryOrderKey: owns the total presentation order and source ranks; a new pending-item source gets a rank here; callers never sort pending items by individual fields.
- Inbox v2 merge: owns the min-head merge across sources under the work bound; a new source is one `Source` arm with peek and resume; callers never interleave sources themselves.
- OrderClass: owns each Source class and the per-class head/advance/done operations; a new ordering strategy is a new arm here; callers never iterate sources or rotate.
- DeadlineWindow: owns window collection, key building for invitations/unread/receipts, sorting and the re-collected-vs-fresh skip rule (bounded so every page advances); a better deadline ordering changes only its internals; callers never sort deadline items or resume them by the global last key.
- DeliveryRowSeek: owns the one ordered (priority, decision_seq, ordinal) seek with the publication bound inside it and the post-seek binding filter; index or eligibility changes go here; callers, the presentation path included, never write their own lazy_recipients walk.
- PageSubset: owns which classes and walks each caller sees, applied at capture and decode; a new caller is one new variant; callers never set done flags or test ordinary_only/thread scope to hide classes.
<!-- key-abstractions digest: end -->

### ht-yha.1 [task] — Seam contract: priority protocol types and wire fields
Seam contract for the priority feature: compilable, inert protocol types and wire fields every other child builds on (spec §Key abstractions, §Compatibility).

owns: DeliveryPriority, NudgeThreshold, DeliveryOrderKey; wire fields SendMessage.priority (Option, omitted when absent), inbox v2 message item `priority`, MessageDeliveryMode result `priority`, WatchMessage `priority` + `deadline_at`, commands SetNudgeThreshold / NudgeThresholdQuery and result NudgeThresholdState; capability constants send.priority_v1, seat.nudge_threshold_v1, direct.auto_join_v1 (defined, NOT advertised yet — each implementing child advertises its own with its handler).

Deliver: new src/protocol/priority.rs with the three value types and their responsibility comments (what each owns, what it does not, where new behaviour goes): DeliveryPriority (u8 0..=9, default 1, `p0()` for invitations/ACK/warnings, legacy `from_mode`/`to_mode` with lossy-downgrade flag, `marker()` -> Some("[pN]") iff N != 1); NudgeThreshold (-1..=9, default 0, negative input clamps to -1, `admits(priority)`, `from_wake_on`/`to_wake_on`); DeliveryOrderKey (priority, Option deadline with None last, decision_seq, source rank enum: Invitation 0, Warning 1, Receipt/legacy unread 2, DeliveryRow 3, ordinal; total Ord; compact continuation encoding). serde: new fields `skip_serializing_if` absent so historical payload digests are unchanged.

Naming: where the new type meets summaries/catch-up `is_priority` (person-authored), code uses `delivery_priority`; doc comments say so.
Acceptance: compiles; unit tests for ranges, clamping, admits(), legacy mapping both ways, marker, Ord tiebreaks (deadline none last, equal deadline -> decision_seq -> rank -> ordinal) and continuation round-trip; existing digest/golden tests unchanged; clippy/harness-neutral clean.

files: src/protocol/priority.rs (new), src/protocol/mod.rs, src/protocol/commands.rs, src/protocol/results.rs, src/protocol/watch.rs, src/protocol/capabilities.rs

### ht-yha.2 [task] — Schema v47: priority, nudge threshold and ordered pending indexes
Schema migration v47 for priority storage (spec §Compatibility and migration, §Ordering index list).

owns: migration 0047 (DDL + backfill) and its schema audit.
Deliver: migrations/0047_message_priority.sql wired into src/store/schema.rs (LATEST_VERSION 47, stepped `if version <= 46` block, verify_* audit):
- messages.priority, send_preparations.priority: INTEGER NOT NULL CHECK 0..9, immutable triggers; backfill delivery_mode ordinary -> 0, lazy -> 1.
- lazy_recipients.priority (backfilled from its message) and lazy_recipients.decision_seq (backfilled from the published manifest/message decision_seq; NULL while staged).
- seats.nudge_threshold INTEGER NOT NULL DEFAULT 0 CHECK -1..9, backfilled wake_on all -> -1, nudge -> 0.
- delivery-row ordered indexes per the inbox-merge subspec (2026-10-13-message-priority-delivery--inbox-merge-design.md): lazy_recipients_pending_order ON lazy_recipients(seat_id, priority, decision_seq, ordinal) WHERE state='pending' AND decision_seq IS NOT NULL, and the thread twin (seat_id, thread_id, priority, decision_seq, ordinal) with the same predicate. No new receipt/invitation indexes (the deadline class reuses existing ones).
- old columns delivery_mode / wake_on / confirmation stay (no drops).
- In-flight pre-v47 state is left as recorded, deliberately (root spec §Compatibility): pending receipts stay owed (with their deadlines/overdue warnings), pending lazy invitations keep no deadline and never wake, pending pair invitations still need accept. A fixture-based migration test asserts each.
Acceptance: fresh DB and v46->v47 upgrade both produce the audited shape; backfill tests (ordinary/lazy messages, lazy rows, wake_on all/nudge seats); EXPLAIN QUERY PLAN shows the new delivery-row indexes used for a seat- and thread-scoped ordered pending seek; existing schema tests updated to 47.

files: migrations/0047_message_priority.sql (new), src/store/schema.rs, tests/store/schema.rs, tests/store/lazy_schema.rs

### ht-yha.3 [task] — Send publication by priority (delivery rows for all, receipts only for explicit ACK)
Send publication by priority (spec §Delivery model, D3, D9).

consumes: DeliveryPriority (needs seam contract), migration 0047 columns.
owns: send-side storage semantics: frozen message priority (written on every delivery row at staging), which recipients get delivery rows vs receipts, daemon-side validation; daemon legacy-request mapping for sends (old-client SendMessage delivery_mode ordinary->p0, lazy->p1); advertising send.priority_v1 with the handler; the TRUST-POLICY receipt/delivery-row amendment; replacing mode-dependent store decisions (recorded_mode, lazy_completion_state and other delivery_mode readers in the send/staging/completion store files — wake.rs readers belong to the wake child, inbox_v2 sources to the merge child, framing to the framing child) with priority / which-row-exists; overdue warnings therefore arise only from explicit-ACK receipts (tested), so legacy rows behave by the new model.
- Old-client test: an old-shape SendMessage (delivery_mode, no priority) against the new daemon yields p0/p1 rows with an unchanged payload digest; old clients can still read inbox/watch items (new fields are additive).
Deliver:
- SendMessage handling resolves priority: explicit `priority` if present, else derived from legacy delivery_mode via DeliveryPriority; frozen into send_preparations/messages and the semantic digest only when the field is present (old payload digests unchanged).
- Daemon validation: explicit ACK recipients require priority 0 (p>0 refused), a deadline requires explicit ACK recipients; out-of-range refused; checked before staging.
- Audience staging (store/messages.rs stage_recipient + lazy_delivery): every joined non-author recipient that is not an explicit ACK recipient gets a delivery row (lazy_recipients) with the message priority — human seats included; explicit ACK recipients get today's receipt (ack_required=1, deadline, unavailable-recipient warning). Non-explicit members get no receipt (D9: today's "every joined agent owes ACK" ends).
- Write legacy delivery_mode = 'ordinary' iff the message has explicit ACK recipients, else 'lazy' (keeps CHECK/audit valid; never read for decisions).
- Advertise send.priority_v1.
- In the same commit, amend TRUST-POLICY.md: delivery-row bookkeeping (generalized lazy bookkeeping, all priorities, human seats included) and receipts only for explicit ACK recipients (D9), including the accepted limits that mention lazy delivery.
Acceptance: store/service tests — p1 group send: rows for every member incl. human, zero receipts/warnings; p0 without ACK: rows only, no receipt/deadline/overdue warning; ACK send: receipt for explicit recipient, rows (p0) for the rest; ACK+p2 refused; deadline without ACK refused; old-client payload without priority: ordinary -> p0 rows, lazy -> p1 rows, digest unchanged; restart keeps priority; human seat receives the row.

files: TRUST-POLICY.md, src/store/messages.rs, src/store/lazy_delivery.rs, src/store/materialization.rs, src/service (send handler), src/daemon/control.rs (capability), tests/store/lazy_publication.rs, tests/service/*
blocked-by ht-yha.1: consumes DeliveryPriority type and SendMessage.priority wire field
blocked-by ht-yha.2: consumes migration 0047 priority columns
blocked-by ht-yha.13: consumes file split

### ht-yha.4 [task] — Wake by per-seat nudge threshold
Wake by per-seat nudge threshold (spec §Delivery model Wake, D1, D2, D5).

consumes: NudgeThreshold (needs seam contract), migration 0047 seats.nudge_threshold + delivery-row priority.
owns: seat threshold storage API, the SetNudgeThreshold/NudgeThresholdQuery daemon handlers and the legacy SetWakeOn/WakeOnQuery mapping (all->-1, nudge->0; WakeOnQuery reports all iff T<0, else nudge), the delivery_mode/wake_on reads in src/store/lazy_delivery/wake.rs and src/store/wake.rs (excluded from the send child), advertising seat.nudge_threshold_v1, the wake decision, per-item `wakes`, the attention-digest delivery-row class, and the model-visible digest/check-in wording that mentions lazy items (replaced with priority wording; size stated in the commit).
Deliver:
- store::seats nudge_threshold get/set (cooperative accountable mutation like set_wake_on; subagents refused); legacy SetWakeOn/WakeOnQuery handlers map through NudgeThreshold (all -> -1, nudge -> 0; WakeOnQuery reports all iff T<0... nudge otherwise); new SetNudgeThreshold/NudgeThresholdQuery handlers; advertise seat.nudge_threshold_v1.
- lazy_delivery::wake_frontier(seat, T): newest pending published row with priority admitted by T; called by wake_seat_attention (store/attention.rs) and the reservation re-check (store/wake.rs) — no other WakeOn branches remain.
- Per-item `wakes` computed once, in inbox v2 hydrate (store/queries.rs), with NudgeThreshold::admits against the current T; delivery/assemble passes it through unchanged (this bead owns `wakes`; the watch child only adds priority/deadline_at).
- Attention digest: the lazy class becomes the delivery-row class (any priority), token keeps its v2 spelling, the lazy key is the newest pending delivery row (store/attention.rs binding_pending_lazy/add_lazy, store/seats.rs lazy_pending/max_lazy_rowid, service/delivery_pull attention_version).
- Invitations, explicit-ACK receipts and actionable warnings remain unconditional wake sources; pre-upgrade lazy invitations stay non-waking.
Acceptance: store tests for T in {-1,0,3} x row priority {0,1,5}: wake candidate present iff admitted; raising T with pending rows yields exactly one wake (last_lazy_ordinal); lowering T stops new wakes; a p0 row wakes a T=0 seat once; wake_on legacy commands round-trip; existing wake_on tests rewritten to thresholds.

files: src/store/seats.rs, src/store/attention.rs, src/store/wake.rs, src/store/lazy_delivery.rs (wake_frontier), src/store/queries.rs (hydrate wakes), src/service (wake-on handlers), tests/store/wake_on.rs, tests/store/wake_on_wake.rs
blocked-by ht-yha.1: consumes NudgeThreshold type and SetNudgeThreshold/NudgeThresholdQuery commands
blocked-by ht-yha.2: consumes migration 0047 seats.nudge_threshold and delivery-row priority
blocked-by ht-yha.13: consumes file split
blocked-by ht-yha.14: consumes file split

### ht-yha.5 [epic] — Inbox v2 min-head merge in priority/deadline/created order
Inbox v2 min-head merge in priority order (spec §Ordering, user amendment D11).

consumes: DeliveryOrderKey (needs seam contract), migration 0047 ordered pending indexes and delivery-row decision_seq.
owns: Inbox v2 merge.
Deliver: replace `rotate` in store/queries.rs mod inbox_v2 with a k-way min-head merge: each step peeks the head of every unfinished source (Invitations, Warnings, Receipts, legacy binding Unread, delivery rows) via one indexed LIMIT 1 seek per source counted against CANDIDATE_LIMIT, emits the smallest DeliveryOrderKey; row/byte/work bounds unchanged; work-limited pages end with a continuation, never a false empty; publication filter after the seek. InboxBatchV2CursorState carries the last emitted key + captured high waters instead of the rotation pointer (cursor token versioned; an old-format token is rejected with the existing stale-cursor error so the client restarts the traversal). All consumers (inbox text/JSON/machine, delivery_pull, hook_inbox, accept thread-scoped display, ordinary_presentation_page) get the order for free. Per-row displayed state keeps out-of-order display safe; verify advance_through_displayed.
Acceptance: tests for merged order across sources (p0 receipt with deadline before p0 invitation without? -> deadline first; p0 rows before p1; equal priority: deadline asc, none last; equal deadlines: decision_seq; equal decision_seq across sources: rank), continuation stability when a page splits an equal-priority run, work-bounded pages with large unpublished backlogs (no unbounded walk; EXPLAIN plan), arrivals after capture deferred to the next traversal, mixed page display-ACK + completion unchanged.

files: src/store/queries.rs (mod inbox_v2), src/protocol/pagination.rs, src/store/lazy_delivery.rs (ordered seek), src/store/attention.rs (receipt/invitation/warning ordered seeks), tests/store/lazy_inbox.rs, tests/protocol/lazy_inbox_v2.rs
Subspec: docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery--inbox-merge-design.md (designed; children ht-yha.5.1-5.4 carry the leaf edges).

### ht-yha.5.1 [task] — Inbox v2 cursor format v2
Inbox v2 cursor format v2 (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery--inbox-merge-design.md §Cursor format v2, §Key abstractions InboxBatchV2CursorState).

consumes: DeliveryOrderKey continuation encoding.
owns: InboxBatchV2CursorState v2 layout and codec.
Deliver: src/protocol/pagination.rs (this bead is the one canonical definer of the DeliveryRows position triple; DeliveryRowSeek uses it): version byte 2; per-class position structs (DeliveryRows (priority, decision_seq, ordinal) triple — elided when the last emitted item is a delivery row; Deadline walk positions (invitation after/high, unread after/high, physical/manifest receipt after/high, next-manifest) plus window start and end positions; Warnings positions and offer cutoff) replacing the embedded v1 SeatAttentionCursorState; the last emitted DeliveryOrderKey; captured high waters; done flags; body continuation tagged with its Source. Canonical re-encode check, validate() bounds, binding tag unchanged. A v1 payload decodes to InvalidCursor with detail "cursor from an older daemon; run inbox again". Code that still builds v1 state is adapted by the merge bead; this bead may keep a temporary conversion shim only if needed to compile (removed by the merge bead).
Acceptance: codec round-trip property tests per class/position/done-flag combination; non-canonical and out-of-bounds rejected; v1 token -> InvalidCursor with the new detail; binding mismatch unchanged.

files: src/protocol/pagination.rs, tests/protocol/lazy_inbox_v2.rs
blocked-by ht-yha.1: consumes DeliveryOrderKey continuation encoding

### ht-yha.5.2 [task] — DeliveryRowSeek: exact ordered pending seek
DeliveryRowSeek: exact ordered pending seek on lazy_recipients (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery--inbox-merge-design.md §Delivery-row class, §Key abstractions DeliveryRowSeek).

consumes: v47 indexes lazy_recipients_pending_order + thread twin and the lazy_recipients.decision_seq column; the lazy_delivery module split.
owns: DeliveryRowSeek (with its responsibility comment) and writing the publication decision_seq onto delivery rows in note_published.
Deliver: in src/store/lazy_delivery/inbox.rs (post-split), replace the ordinal candidate SQL with one ordered seek: (priority, decision_seq, ordinal) > position AND decision_seq IS NOT NULL AND decision_seq <= dh, LIMIT 1, INDEXED BY the new index (thread twin for thread scope); binding post-seek unread/display filter (one visit per rejected row); thread-scoped stop below the first unread receipt-bearing row; private presentation eligibility (PRIVATE_LAZY_ELIGIBILITY_SQL) as a post-seek filter option so the presentation path never has its own walk. note_published (staging.rs) copies the publication decision_seq onto the message's rows. Position type = the cursor v2 DeliveryRows (priority, decision_seq, ordinal) triple defined by the cursor bead (the one canonical definition; do not define a second).
Acceptance: store tests: p0 row behind 500 pending p1 rows is the first seek result (one visit); 10k unpublished rows -> zero visited (EXPLAIN QUERY PLAN uses the index, no temp b-tree); thread-scoped and binding-filtered seeks; published rows get decision_seq exactly once; resume after a position.

files: src/store/lazy_delivery/inbox.rs, src/store/lazy_delivery/staging.rs, tests/store/lazy_inbox.rs, tests/store/lazy_publication.rs
blocked-by ht-yha.2: consumes v47 delivery-row ordered indexes and lazy_recipients.decision_seq
blocked-by ht-yha.13: consumes file split
blocked-by ht-yha.5.1: consumes cursor v2 DeliveryRows position triple (canonical definition)
blocked-by ht-yha.3: consumes staging.rs changes (priority on rows) landed first; same file

### ht-yha.5.3 [task] — DeadlineWindow: bounded sorted deadline-class window
DeadlineWindow: bounded sorted window over invitations, binding-unread and receipts (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery--inbox-merge-design.md §Deadline class, §Key abstractions DeadlineWindow).

consumes: DeliveryOrderKey; cursor v2 Deadline position struct; the inbox_v2 module split.
owns: DeadlineWindow (new file src/store/queries/inbox_v2/deadline_window.rs, with its responsibility comment).
Deliver: collect pending items from the existing invitation, unread and receipt walks (scan_effective_receipts, unread_ordinal, receipt_pending — unchanged) in walk order until 64 items, walks exhausted, or CANDIDATE_LIMIT/2 visits spent; build each item's DeliveryOrderKey (deadline: effective_receipt deadline, or invitations.deadline_at for require_ack invitations only, else None; decision_seq: effective receipt or created_decision_seq; rank per DeliveryOrderKey); keep the receipt/unread dedupe; sort; expose head/advance/done; record window start/end and walk stop positions; skip rule: re-collecting from start positions skips keys <= last emitted, a fresh window skips nothing.
Acceptance: window-level unit/store tests, driven by persisted start/stop positions (end-to-end paging versions belong to the merge bead): exact order when backlog fits; 70 pending receipts -> exact within each window, nothing lost or repeated across pages; window closes at the visit share with settled-history-heavy receipt walks and every page advances; deadline-less legacy invitations sort after deadlined p0 items; dedupe preserved.

files: src/store/queries/inbox_v2/deadline_window.rs (new), src/store/queries/inbox_v2/sources.rs, tests/store/lazy_inbox.rs
blocked-by ht-yha.1: consumes DeliveryOrderKey
blocked-by ht-yha.5.1: consumes cursor v2 Deadline position struct
blocked-by ht-yha.14: consumes file split

### ht-yha.5.4 [task] — Inbox v2 min-head merge loop, OrderClass and PageSubset
Inbox v2 min-head merge loop, OrderClass and PageSubset (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery--inbox-merge-design.md §Merge loop, §Caller subsets, §Tests; parent spec §Ordering).

consumes: cursor v2; DeliveryRowSeek; DeadlineWindow; the inbox_v2 module split.
owns: Inbox v2 merge, OrderClass, PageSubset (responsibility comments on each), and removal of delivery_mode reads in inbox_v2 sources.rs (continued_ordinary, receipt_pending).
Deliver: replace rotate with the min-head loop over OrderClass heads (DeliveryRows via DeliveryRowSeek, Deadline via DeadlineWindow, Warnings via the existing scan_global_logical_candidates path with its 2-visit reservation); emit the smallest DeliveryOrderKey; partly shown body resumes first; row/byte/work stops and never-false-empty unchanged; PageSubset (All | Presentation | ThreadScoped) applied at capture and after decode, replacing the scattered ordinary_only/thread-scope done-flag code; the presentation path routes through DeliveryRowSeek; remove any v1-state shim. Rewrite rotation-order tests (lazy_inbox_v2_bounded_round_robin_opportunity, q4 visit-99 test, invitation_content CancelClock call-count test, binding_inbox page vectors, delivery_pull presentation partial-body tests, queries.rs page/defer tests).
Acceptance: a p0 delivery row behind 500 pending p1 rows is first within one page; a partly shown body resumes first and its key becomes the last emitted key once complete; 70 pending receipts paged end to end lose and repeat nothing; merged order across classes per subspec §Tests (p0 deadline receipt first; p0 before p1; none-last; decision_seq and rank tiebreaks); read-only JSON paging split inside an equal-priority run is stable and complete; work-limited pages resume without skips; arrivals after capture deferred; caller subsets (presentation page, accept thread scope, hook page) produce the right classes; mixed display-ACK + completion unchanged.

files: src/store/queries/inbox_v2/mod.rs, src/store/queries/inbox_v2/capture.rs, src/store/queries/inbox_v2/sources.rs, tests/store/lazy_inbox.rs, tests/store/query_work_q4.rs, tests/store/invitation_content.rs, tests/store/binding_inbox.rs, tests/store/queries.rs, tests/service/delivery_pull.rs
blocked-by ht-yha.5.1: consumes cursor format v2
blocked-by ht-yha.5.2: consumes DeliveryRowSeek
blocked-by ht-yha.5.3: consumes DeadlineWindow
blocked-by ht-yha.14: consumes file split
blocked-by ht-yha.3: consumes delivery rows for all recipients with priority (send publication)

### ht-yha.6 [task] — Invitations always p0 + confirming; 1:1 pairs auto-join
Invitations always p0 + confirming; 1:1 pairs auto-join (spec D6, D7, §Surfaces Direct pairs).

consumes: capability constant direct.auto_join_v1 (needs seam contract).
owns: invitation confirmation policy collapse (incl. legacy old-client lazy Invite mapped to a confirming invitation, with an old-client invite test), pair auto-join, advertising direct.auto_join_v1 with the handler, the accept opening-mail display redefinition (src/cli/accept_delivery.rs filter and first_ordinary_walk_stop become priority-agnostic; pair openers arrive by normal delivery) with its AGENTS.md bullet, behaviour of kept pre-v47 pending lazy/pair invitations per spec §Compatibility (tested on the ht-yha.2 fixture), plus the TRUST-POLICY pair/invitation amendment.
Deliver:
- Every new invitation records confirmation 'require_ack' with the instance default or explicit deadline; daemon accepts an old Invite.confirmation=lazy and records require_ack; human/operator invites unchanged in effect (timed, wake). Pre-upgrade pending lazy invitations keep their stored behaviour.
- resolve_direct_pair (store/control.rs): extract one join-recording helper (memberships joined, membership_intervals, read_cursors init, join event) used by the caller's join and now the peer's join; the peer is joined on creation, no invitation row; EnsureCreationPolicy ignored; direct_threads keeps the two reserved seats. Pre-upgrade pending pair invitations still accept as today.
- Handoff automatic pair route: no invitation, body keeps its ACK requirement.
- Accept thread-scoped opening-mail display (src/cli/accept_delivery.rs, InboxQuery.thread, read_cursors::first_ordinary_walk_stop): shows pending delivery rows of every priority (no longer lazy-only) below the first unread receipt-bearing row; still used for invitations of ordinary threads and pre-upgrade pair invitations; AGENTS.md "What we do today" accept bullet updated in the same commit.
- Advertise direct.auto_join_v1. Update TRUST-POLICY.md (pair creation joins both reserved seats per the user's explicit decision D7; lazy invitation confirmation removed for new invitations; accepted limits mentioning lazy invitations) in the same commit.
Acceptance: store/service tests: direct send creates a pair with both seats joined and no invitation; first direct message delivered as a row at its priority (p1 does not wake T=0 peer; p0 does); invite always confirming with deadline, overdue warns inviter; old-client lazy invite recorded require_ack; pre-upgrade pending lazy and pair invitations still accept; handoff to existing seat via pair: no invitation, ACK-required body.

files: src/cli/accept_delivery.rs, src/store/control.rs, src/store/lazy_delivery.rs (direct_acceptance_floor), src/store/read_cursors.rs, src/cli/direct_send.rs, src/cli/topology_handoff.rs, TRUST-POLICY.md, tests/store/direct_threads.rs, tests/store/control.rs, tests/cli/direct_send.rs
blocked-by ht-yha.1: consumes direct.auto_join_v1 capability constant
blocked-by ht-yha.13: consumes file split
blocked-by ht-yha.3: consumes send publication by priority (first direct message as a delivery row)
blocked-by ht-yha.4: consumes wake by nudge threshold (p1 vs p0 pair wake acceptance)

### ht-yha.7 [task] — CLI: --priority/-p, --nudge alias, nudge-threshold, flag removals, old-daemon fallback
CLI surfaces for priority and threshold (spec D3, D4, D5, D6 flag consequences, §Surfaces CLI, §Compatibility new-client/old-daemon).

consumes: DeliveryPriority, NudgeThreshold, capability constants (needs seam contract).
owns: the new-client/old-daemon pair fallback (without direct.auto_join_v1, direct send keeps sending the legacy pair creation policy), Send-priority resolution (one function shared by send, send --seat, human send, handoff) and the CLI flag surface.
Deliver:
- send/human send/direct send: `--priority/-p N` (0..9), `--nudge` alias for -p 0 (conflicts with -p), `--lazy` removed (clap error text points to the default p1), `--require-ack`/`--require-ack-pane` force p0 and refuse explicit p>0, `--deadline` requires ACK recipients; validated before body read or intent record; journal persists priority (omitted when absent for digest stability).
- invite: remove --lazy/--require-ack, keep --deadline; handoff: remove --require-invite-ack, keep --invite-deadline.
- `herdr-threads nudge-threshold [N]` (bare shows; negative -> -1) and `wake-on all|nudge` as deprecated alias printing the mapped value + one-line deprecation note.
- Old-daemon fallback: without send.priority_v1 send p0 as ordinary, p>=1 as lazy (stderr one line when p>1 is lost); without seat.nudge_threshold_v1 map -1/0 to wake-on and refuse others with an upgrade hint; invitations fall back to require_ack.
- Help goldens updated.
Acceptance: tests/cli flag matrix (defaults, -p bounds, --nudge alias + conflict, removed --lazy, ACK+p>0 refused, deadline rules, direct ACK target rule kept), journal/retry keeps frozen priority, nudge-threshold and wake-on alias outputs, old-daemon fallbacks (fake daemon capability sets), help goldens.

files: src/cli/commands.rs, src/cli/mod.rs, src/cli/direct_send.rs, src/cli/handoff.rs, src/cli/journal.rs, src/cli/wake_on.rs (-> nudge threshold), tests/cli/lazy_send.rs, tests/cli/wake_on.rs, tests/fixtures/golden/help/*
blocked-by ht-yha.1: consumes DeliveryPriority, NudgeThreshold and capability constants

### ht-yha.8 [task] — Framing and markers by priority ([pN], message kind)
Model-visible framing and markers by priority (spec D8, §Surfaces inbox/history/hook pages).

consumes: DeliveryPriority marker (needs seam contract).
owns: old-reader compatibility of inbox v2 items (existing variants kept: delivery row = LazyMessage with wakes, receipt = Message; priority/deadline_at added as optional fields; old-client fixture test), confirming the agy/Hermes hook delivery paths only consume frame()/page with no lazy-kind branch left, removal of delivery_mode reads in framing code, FrameItem/frame() priority framing in Rust (kind from variant + priority), the CLI text markers, and populating priority from storage into inbox v2 message items (inbox_v2 hydrate.rs) and history/body/search rows.
Deliver: header kind word always `message` (no `lazy` kind) with `[p<N>]` (N != 1) in src/delivery/frame.rs and every Rust producer (delivery_pull/presentation, agy delivery header build + parse accepting both old `lazy` and new forms, Hermes path); inbox text rows (output_compact) and history/read/body/search annotation (`[lazy] <id>` -> `[pN]` via MessageDeliveryModesQuery gaining priority); JSON message items always carry `priority`; settle::ModeFact chosen by the (message, seat) row actually present (receipt vs delivery row), not delivery_mode. Shared golden fixtures for frame() so the mod can mirror them.
Acceptance: frame goldens (p0, p1 silent, p5, human/relay markers combined), inbox text/JSON goldens, history markers, agy parse of both header forms, settle tests for receipt vs row; model-visible size stated in commit.

files: src/delivery/frame.rs, src/delivery/presentation.rs, src/delivery/settle.rs, src/service/delivery_pull.rs, src/protocol/output_compact.rs, src/cli/output.rs, src/cli/lazy_display.rs, src/harness/agy/delivery.rs, tests/delivery/*, tests/fixtures/golden/*
blocked-by ht-yha.1: consumes DeliveryPriority marker
blocked-by ht-yha.2: consumes migration 0047 messages.priority column
blocked-by ht-yha.14: consumes file split (inbox_v2 hydrate.rs)
blocked-by ht-yha.4: consumes hydrate.rs wakes change landed first; same file

### ht-yha.9 [task] — Watch stream and Claude mod in priority order
Watch stream and Claude mod in priority order (spec §Surfaces Watch stream and Claude mod, D11).

consumes: WatchMessage priority/deadline_at and DeliveryOrderKey (needs seam contract); frame golden fixtures with `[pN]` (needs framing child).
owns: watch batch ordering (delivery assemble/page) and the mod's JS mirror of the order key and frame; the mod's idle (Channel) wake decision follows the daemon-computed `wakes` (priority <= T) only. Order contract: assemble/page preserve the inbox v2 merge order and sort only when they combine several pages or arrivals into one batch, with DeliveryOrderKey; the JS sort uses the same tiebreaks. This bead tests the combine-sort only; preservation of the merge order is verified by the cross-surface seam integration. The new mod frames from variant + priority; an old installed mod keeps working (spec §Compatibility).
Deliver: delivery/assemble.rs + page.rs populate WatchMessage.priority/deadline_at (pass `wakes` through as hydrated by the wake child; do not recompute it) and sort every emitted batch by DeliveryOrderKey (arrival breaks the final tie); integrations/claude/mod/hooks/register.js: item gains priority/deadline, one stable sort by the same key before framing in both the wake path (pump/appendBatch, onToolCall) and passive path (pumpLazy), frame() mirrors the Rust golden (`message` kind + `[pN]`), passive() keyed on `wakes` (no `lazy` kind). Separate batches stay in arrival order.
Acceptance: Rust tests for batch sort; mod tests (integrations/claude/mod/tests/delivery.test.ts) for sort order across mixed priorities/deadlines, framing goldens identical to Rust, passive/wake split by `wakes`.

files: src/delivery/assemble.rs, src/delivery/page.rs, src/protocol/watch.rs (population), integrations/claude/mod/hooks/register.js, integrations/claude/mod/tests/delivery.test.ts
blocked-by ht-yha.1: consumes WatchMessage priority/deadline_at and DeliveryOrderKey
blocked-by ht-yha.8: consumes frame golden fixtures with [pN]
blocked-by ht-yha.2: consumes migration 0047 delivery-row priority column
blocked-by ht-yha.4: consumes daemon-computed per-item wakes by threshold

### ht-yha.10 [task] — Docs and skill text for priority delivery
Docs and skill text for priority delivery (spec §Docs and model-visible text).

owns: AGENTS.md Delivery contract + "What we do today" + wake-e2e section text, integrations/skill/SKILL.md, README.md, docs/agent-usage.md, integrations/claude/mod/README.md, integrations/agy/README.md, integrations/hermes/README.md. TRUST-POLICY.md is NOT owned here: the send-publication child and the pair child amend it in their behaviour commits.
Deliver: rules 1/2/5 restated (default p1 and T=0 so default sends never wake; wake iff priority <= T; ACK requests, invitations and actionable warnings are p0); skill: priority/threshold/`[pN]` marker explained once, `--nudge` = `-p 0`, `nudge-threshold`, invitations always need an explicit accept or reject, 1:1 pairs auto-join; remove lazy wording; reword AGENTS.md's wake-e2e note on person-authored "priority" so it is not confused with delivery priority. docs/agent-usage.md also states the inbox ordering rule and its accepted limit: with more than 64 pending deadline-bearing p0 items (ACK receipts + confirming invitations), order is exact within each 64-item window. Keep text compact (Token-efficient communication); state sizes.
Acceptance: no remaining user-facing `--lazy`/lazy-mode/`wake-on` instructions except the deprecation note; skill guide test/golden (if any) updated; markdown links valid.

files: AGENTS.md, integrations/skill/SKILL.md, README.md, docs/agent-usage.md, integrations/claude/mod/README.md, integrations/agy/README.md, integrations/hermes/README.md, TRUST-POLICY.md
blocked-by ht-yha.11: consumes wake-e2e matrix (AGENTS.md section text)
blocked-by ht-yha.9: consumes final delivery file/constant names for What we do today

### ht-yha.11 [task] — Wake e2e matrix for priorities and thresholds
Wake e2e matrix for priorities and thresholds (spec §Testing, AGENTS.md Pre-release wake e2e).

consumes: CLI flags -p/--nudge/--require-ack and nudge-threshold (needs CLI child).
owns: scripts/wake-e2e.py cases and expectations, scripts/wake_e2e_selftest.py.
Deliver: cases send-p1 / send-p0 / send-p3 / send-ack x group/1:1, invite (always confirming), 1:1 auto-join (no invitation; opener at its own priority); recipients at T = 0 / -1 / 3 (replacing wake_on nudge|all); expectation: idle recipient wakes iff item priority <= T (invitations/ACK always); busy recipient never nudged; a mixed page lands in priority/deadline order. Driver self-test updated. The live run itself is a release-time human step and is NOT run by this bead.
Acceptance: `python3 scripts/wake-e2e.py --self-test` passes; expectation table unit-tested in the self-test.

files: scripts/wake-e2e.py, scripts/wake_e2e_selftest.py
blocked-by ht-yha.7: consumes CLI flags -p/--nudge/--require-ack and nudge-threshold
blocked-by ht-yha.3: consumes send publication by priority
blocked-by ht-yha.4: consumes wake by nudge threshold
blocked-by ht-yha.6: consumes invitations p0 and pair auto-join
blocked-by ht-yha.8: consumes [pN] framing in pane captures
blocked-by ht-yha.9: consumes mod Channel idle wake by daemon wakes

### ht-yha.12 [task] — Configuration smoke: T x priority x ACK/invitation/1:1
Configuration smoke: thresholds T in {-1, 0, 3} x priorities {0, 1, 5} x {ACK request, invitation, 1:1 first send} on a real in-process daemon.

consumes: send publication by priority, wake by threshold, CLI flags, pair auto-join (needs those children).
owns: tests/integration/priority_config_smoke.rs (new, own [[test]] or combined per AGENTS.md rules).
Deliver: one integration test that, per configuration, runs the CLI against an isolated daemon (isolated HOME/state dir, test_support::spawn) and asserts: wake candidate present iff priority <= T (ACK/invitation always), receipts only for explicit ACK, inbox order for a mixed set (p0 deadline, p0 none, p1, p5, invitation), pair auto-join. One decision per configuration; command per configuration listed in the test's doc comment.
Acceptance: test passes; owned processes stopped; scripts/check-no-leaked-processes clean.

files: tests/integration/priority_config_smoke.rs (new), Cargo.toml ([[test]] if needed), tests/combined.rs
blocked-by ht-yha.3: consumes send publication by priority
blocked-by ht-yha.4: consumes wake by nudge threshold
blocked-by ht-yha.6: consumes pair auto-join
blocked-by ht-yha.7: consumes CLI priority flags and nudge-threshold command

### ht-yha.13 [task] — Split file: src/store/lazy_delivery.rs
Move-only, behavior-preserving split of src/store/lazy_delivery.rs (1071 lines) so the parallel store children edit separate files (super-design hot-file rule).

owns: the module layout src/store/lazy_delivery/{mod.rs, direct.rs (episode/acceptance floors, direct display admission), staging.rs (stage_recipient, note_published, preparation_high_water, cleanup_unpublished), inbox.rs (capture_high_water, publication_eligible, pending_page*, binding_lazy_* readers, seat_lazy_pending, pending_summary), completion.rs (complete_*, lazy_completion_state, recorded_mode), wake.rs (wake_frontier, oldest_pending_at, has_wake_rows)}; mod.rs re-exports every public item so no caller changes.
No logic changes, no renames.
Acceptance: compiles; `nice cargo test --locked --all-features lazy` green; clippy and harness-neutral clean; diff is moves plus re-exports only.

files: src/store/lazy_delivery.rs -> src/store/lazy_delivery/*.rs

### ht-yha.14 [task] — Split file: src/store/queries.rs mod inbox_v2
Move-only, behavior-preserving split of `mod inbox_v2` out of src/store/queries.rs (lines ~6220-7469, ~1250 lines) so the wake child (hydrate `wakes`) and the inbox-merge children edit separate files (super-design hot-file rule; a concurrent edit against a moved block always conflicts).

owns: module layout src/store/queries/inbox_v2/{mod.rs (query, query_mode, current_agent, the main loop and page_v2/fits/done/source_done/rotate), capture.rs (capture, reader/bound/identity/binding/fence_holds), sources.rs (scan and the per-source arms, unread_ordinal, receipt_pending, continued_ordinary), hydrate.rs (hydrate_message, message, chunk)}; queries.rs keeps `mod inbox_v2;` with the same visibility, so callers are unchanged.
No logic changes, no renames.
Acceptance: compiles; `nice cargo test --locked --all-features inbox` and `lazy` green; clippy and harness-neutral clean; diff is moves plus module declarations only.

files: src/store/queries.rs, src/store/queries/inbox_v2/*.rs (new)

### ht-yha.15 [task] — Seam integration: cross-surface delivery order
Verify one ordering end to end across every presentation surface (coverage round 1 GAP: each surface was ordered only locally; root spec §Ordering, inbox-merge subspec §Caller subsets).

owns: tests/integration/priority_order_surfaces.rs (new).
Deliver: one integration test on an isolated in-process daemon (isolated HOME/state dir, test_support::spawn) that publishes one mixed pending set for a seat — a confirming invitation (p0, deadline), an explicit-ACK message (p0, deadline), a p0 message without ACK, a p1 and a p5 message, in an arrival order that differs from the expected order — and asserts the same order (DeliveryOrderKey) through: CLI `inbox` text and `--json` (including a continuation that splits the set), the hook page (delivery_pull), a watch drain, accept's thread-scoped display (its delivery-row subset) and the presentation page (its subset). The same mixed set is also written as a JSON fixture and fed through the Claude mod's sort and frame (integrations/claude/mod/tests) so the JS mirror is checked against the identical expected order. Small wiring defects found are fixed inline in the surface's own file; larger ones are filed as blockers.
Acceptance: test passes; owned processes stopped; scripts/check-no-leaked-processes clean.

files: tests/integration/priority_order_surfaces.rs (new), tests/fixtures/priority_order_mixed.json (new), integrations/claude/mod/tests/delivery.test.ts, Cargo.toml / tests/combined.rs as AGENTS.md requires
blocked-by ht-yha.5.4: consumes inbox v2 merge order
blocked-by ht-yha.9: consumes watch batch order
blocked-by ht-yha.6: consumes confirming invitations and accept display
blocked-by ht-yha.8: consumes priority populated into inbox items

### ht-yha.16 [task] — Integration sweep: message delivery priority
Final integration sweep for message delivery priority (super-design root sweep; unknown-unknowns net).

Deliver: verify the goal's main flows end to end on an isolated daemon — (1) default send p1 to a T=0 seat: no wake, delivered in-turn, `[p1]` not shown; (2) `-p 0` / `--nudge`: idle seat wakes once, no receipt; (3) `--require-ack`: p0, receipt with deadline, overdue warning on miss; (4) `nudge-threshold 3` then `-p 3`: wakes, `-p 4`: does not; (5) invite: p0 confirming with deadline; (6) first direct send: both seats joined, no invitation; (7) mixed inbox order; (8) old-shape client requests (delivery_mode send, wake-on, lazy invite) against the new daemon. Add integration tests no per-seam bead covers; sweep for unwired config values, capability advertisements, removed flags still referenced in help/docs, and remaining `delivery_mode`/`wake_on` decision reads (grep src/ and integrations/). Fix small gaps inline; file blockers for big ones.
Acceptance: the flows above pass in tests/integration; `rg -n "delivery_mode|wake_on|WakeOn::|DeliveryMode::" src integrations` shows only legacy-mapping, migration and frozen-wire sites, each justified in the commit; clippy, harness-neutral, default-features clean; owned processes stopped.

files: tests/integration/priority_sweep.rs (new), any small inline fixes
blocked-by ht-yha.1: consumes all leaves (integration sweep)
blocked-by ht-yha.2: consumes all leaves (integration sweep)
blocked-by ht-yha.3: consumes all leaves (integration sweep)
blocked-by ht-yha.4: consumes all leaves (integration sweep)
blocked-by ht-yha.5.1: consumes all leaves (integration sweep)
blocked-by ht-yha.5.2: consumes all leaves (integration sweep)
blocked-by ht-yha.5.3: consumes all leaves (integration sweep)
blocked-by ht-yha.5.4: consumes all leaves (integration sweep)
blocked-by ht-yha.6: consumes all leaves (integration sweep)
blocked-by ht-yha.7: consumes all leaves (integration sweep)
blocked-by ht-yha.8: consumes all leaves (integration sweep)
blocked-by ht-yha.9: consumes all leaves (integration sweep)
blocked-by ht-yha.10: consumes all leaves (integration sweep)
blocked-by ht-yha.11: consumes all leaves (integration sweep)
blocked-by ht-yha.12: consumes all leaves (integration sweep)
blocked-by ht-yha.13: consumes all leaves (integration sweep)
blocked-by ht-yha.14: consumes all leaves (integration sweep)
blocked-by ht-yha.15: consumes all leaves (integration sweep)

