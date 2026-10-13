# Message delivery priority

Run: `docs/superpowers/runs/2026-10-13-message-priority-delivery/` · design mode A (questions relayed by seat
`seePN0MKW`, thread `tXPkAdb0O`; answers recorded in that thread, messages `mwRZ1YxXs`, `myas79LQ8`, `mXmhe1P2d`).

## Goal

A sender chooses each message's delivery priority (an integer 0..=9, 0 most urgent) and each recipient seat
chooses a nudge threshold T: an idle recipient is woken by a pending item exactly when its priority is `<= T`
(priority 0 always; a negative T wakes on everything). Lazy/ordinary delivery modes and lazy invitations are
gone: messages default to priority 1 and T defaults to 0, so a plain send notifies nobody, while every
invitation and every ACK request is priority 0. Inbox, hook pages and the Claude mod present pending items in
(priority ascending, oldest first) order. 1:1 direct pairs join both seats on creation, with no invitation.

## Decisions (user-approved)

| # | Decision |
|---|---|
| D1 | The threshold is **per seat** (it replaces `seats.wake_on`). No per-thread override in this change. |
| D2 | "Does it nudge?" is decided **at wake time** against the seat's current T, never frozen at send. Raising T can wake once for items already pending. |
| D3 | `--require-ack` implies priority 0; an explicit `--priority` above 0 together with an ACK request is refused. |
| D4 | CLI: `--priority N` / `-p N` (0..=9). `--nudge` stays as an alias for `--priority 0` (conflicts with `-p`). `--lazy` is removed. |
| D5 | Recipient command `herdr-threads nudge-threshold [N]` (bare shows the value; negative = every item nudges). `wake-on all\|nudge` stays as a deprecated alias: `all` → -1, `nudge` → 0; the stored `wake_on` migrates the same way. |
| D6 | Every thread invitation is priority 0, always nudges and always requires acceptance by a deadline (no lazy invitations, no confirmation-policy variants). Recipients still answer explicitly (accept or reject on topic and goal). Actionable warnings are priority 0. |
| D7 | 1:1 direct pairs **auto-join both seats** on creation: no invitation; the first direct message's own priority alone decides whether it wakes. This also fixes today's mismatch between the docs (which describe pairs as automatic) and the behaviour (the peer had to run `accept`). |
| D8 | Model-visible framing: the header kind word is always `message` (the `lazy` kind word goes away); a `[p<N>]` marker is added only when N != 1. |
| D9 | Priority governs **notification only**. Receipts, deadlines and overdue warnings exist only for explicit `--require-ack` recipients. A priority-0 message without `--require-ack` wakes but creates no receipt, deadline or warning. |
| D11 | Ordering key: (priority asc, deadline asc with no deadline last, created asc), approved with the design (`mpqluOeQD`). |
| D10 | Compatibility: old clients and daemons map `ordinary` ↔ p0 and `lazy` ↔ p1, capability-gated in both directions; stored rows migrate the same way. |

Consequences of D6 for flags: `invite --lazy` / `--require-ack` and `handoff --require-invite-ack` are removed;
`invite --deadline` and `handoff --invite-deadline` stay (default: the instance `invitation_default_ms`, 5 min); a
missed invitation deadline warns the inviter, as today.

## Delivery model

**Priority** is an immutable property of a message, recorded at send (`messages.priority`,
`send_preparations.priority`, part of the frozen intent and its semantic digest; retry cannot change it).
Invitations and warnings have a fixed priority of 0 and store none.

**Every recipient of a message gets one delivery row.** Today's `lazy_recipients` table (pending/displayed
state, frozen audience at seal, monotonic per-seat ordinal, bounded pending indexes, atomic publication through
the manifest) generalizes to all messages: it gains `priority`, and every audience member gets a row, human seats
included, except explicit `--require-ack` recipients, who get today's receipt instead (`prepared_recipients` /
`receipt_state` with `ack_required=1`, deadline, overdue warning, display-ACK), so each (message, seat) has exactly
one pending item. Non-explicit members no longer get receipts: the "every joined agent owes an ACK" behaviour of
today's `--nudge` ends (D9).

The audience rule is unchanged: joined, nonretired participants at the sealed snapshot, excluding the author,
plus explicit ACK recipients.

**Wake.** `wake_seat_attention` keeps its frontier components and replaces the `lazy` component's
`wake_on == All` condition with "the newest pending delivery row of this seat with `priority <= T`". Invitations
(now always confirming), explicit ACK receipts and actionable warnings stay unconditional wake sources (priority
0 <= any T >= 0, and a negative T wakes on everything). The existing once-per-frontier memory
(`wake_work.last_lazy_ordinal`) keeps a row from waking twice. Because the frontier reads the current T, raising T
schedules at most one wake for the newest already-pending eligible row (D2); lowering T cancels nothing that
already ran.

The Delivery contract invariants keep their shape: the rows a hook or channel delivers are every pending item;
wake applies only to idle sessions (rule 4); who performs the wake stays the per-harness variation (rule 6).
AGENTS.md's rules 1, 2 and 5 are restated in priority terms (below).

**Ordering.** Every presentation surface (inbox text/JSON/machine, hook pages, watch drains, mod batches) emits
pending items by the key **(priority asc, deadline asc with no deadline last, created asc)**, then a fixed
tiebreak **(source rank, per-source ordinal)** so the order is total and stable across pages (user amendment
`mpqluOeQD`: a deadlined item outranks an older item without one).

- *priority*: the message's priority; invitations and actionable warnings are 0.
- *deadline*: the pending item's own deadline: `receipt_state.deadline_at` for an explicit-ACK receipt,
  the invitation's deadline for an invitation; delivery rows and warnings have none (sort last within their
  priority).
- *created*: the publication `decision_seq` (messages, invitations, warning publications). It is the
  instance-wide, monotonic counterpart of `created_at`; wall-clock `created_at` is not used for ordering because
  it can step backwards.
- *source rank*: invitations 0, warnings 1, receipts and pre-upgrade binding-unread ordinary rows 2 (those
  legacy rows are p0, deadline from their receipt if any), delivery rows 3; *per-source ordinal*: the row's ordinal.

Inbox v2 stops round-robin rotation between sources and instead, each step, peeks the head of every unfinished
source (one indexed `LIMIT 1` seek per source, each counted against `CANDIDATE_LIMIT`) and emits the smallest key;
the row limit, byte limit and work bound are unchanged, and a work-limited page ends with a continuation, never a
false `empty`. How each source yields its items in key order (exact index-backed delivery rows, a bounded sorted window for the
deadline-bearing p0 sources, warnings already in order), the v47 index shapes and the cursor format are specified
in the inbox-merge subspec ([2026-10-13-message-priority-delivery--inbox-merge-design.md](2026-10-13-message-priority-delivery--inbox-merge-design.md)). The continuation records the last emitted full key and the captured high waters; an item published after capture
belongs to the next traversal, as today. Out-of-order display within a thread is made safe by one rule (roast r1, cluster C2): a delivery row is pending for a reader
unless it was displayed for that reader. The binding read cursor still advances only over the contiguous displayed
prefix, but it is never the sole test: v47 adds a sparse per-binding mark table `binding_row_displays(binding_id,
message_id)` written when a binding completes display of a delivery row above its cursor, read by every binding-level
unread/pending predicate (inbox Unread and delivery-row filters, hook, accept), and pruned when the cursor passes the
row. Seat-level predicates (wake frontier, attention digest) test `lazy_recipients.state`. Test: p0 seq 10 displayed
before p1 seq 5 in one thread is not re-offered; seq 5 still is. The watch stream and mod carry `priority` and `deadline_at` on each
item and sort every batch they frame by the same key (arrival order breaks the final tie); separate batches stay
in arrival order.

## Surfaces

- **CLI send** (`send`, `send --seat`, `human send`): `--priority/-p N`, `--nudge` alias, default 1. `--deadline`
  requires explicit ACK recipients. Validation happens before a body is read or an intent is recorded.
- **CLI invite**: always confirming, `--deadline` optional. **Handoff**: `--invite-deadline` stays,
  `--require-invite-ack` goes; the automatic pair route auto-joins, so it creates no invitation at all; the
  handoff body keeps its ACK requirement (therefore p0).
- **Recipient threshold**: `nudge-threshold [N]` (range -1..=9; any negative input is stored as -1),
  cooperative accountable mutation as `wake-on` is today (subagents refused); `wake-on` prints its mapped value
  and a one-line deprecation note.
- **Inbox text / JSON / machine**: rows in priority order; text rows carry `[p<N>]` when N != 1; the `[lazy]` marker
  goes away. JSON carries `priority` on every message item.
- **History / read / body / search**: `[p<N>]` (N != 1) replaces the `[lazy] <id>` annotation, via the companion
  metadata query (`MessageDeliveryModesQuery` gains `priority`).
- **Hook pages** (`delivery_pull`, `hook_inbox`, agy, Hermes): order as above; framing per D8.
- **Watch stream and Claude mod**: `WatchMessage` gains `priority` and `deadline_at`; `wakes` is computed by the daemon against the
  seat's T. The mod sorts each batch it frames (wake and passive) by the ordering key (arrival breaks the final tie)
  before framing, so one injection is in order; separate batches stay in arrival order.
- **Attention digest**: the lazy class becomes the delivery-row class; the token keeps its `v2` spelling (the lazy
  key is now the newest pending delivery row).
- **Direct pairs** (D7): `resolve_direct_pair` writes the peer's joined membership, membership interval, read
  cursor init and a join event instead of an invitation; `direct_threads` keeps the two reserved seats. Accept is
  no longer needed for new pairs; pending pair invitations created before the upgrade still accept as today.

## Compatibility and migration

- **Schema** (next migration, v47): `messages.priority` and `send_preparations.priority` (INTEGER, CHECK 0..=9,
  immutable trigger), backfilled `ordinary` → 0, `lazy` → 1; `lazy_recipients.priority` backfilled from its
  message; `seats.nudge_threshold` (INTEGER CHECK -1..=9, default 0), backfilled `all` → -1, `nudge` → 0. The old
  `delivery_mode`, `wake_on` and `confirmation` columns stay, frozen for history and old-reader audit, and are no
  longer read for decisions; new rows write `delivery_mode` = `ordinary` iff the message has explicit ACK
  recipients (keeping the existing CHECK and audit valid). Existing pending receipts stay owed; existing pending
  lazy invitations keep their recorded no-deadline, non-waking behaviour; new invitations are written
  `require_ack`.
- **Wire**: `SendMessage.priority: Option<u8>` (omitted → derived from `delivery_mode`: ordinary → 0, lazy → 1),
  capabilities `send.priority_v1`, `seat.nudge_threshold_v1`, `direct.auto_join_v1`. The daemon accepts an old
  `Invite.confirmation` and records `require_ack` regardless; it ignores an old pair `creation_policy` and
  auto-joins.
- **New client, old daemon**: p0 → `ordinary`, p >= 1 → `lazy` (the exact priority is lost; the client says so
  once on stderr when N > 1); `nudge-threshold -1|0` → `wake-on all|nudge`, other values refused with an upgrade
  hint; invitations fall back to `require_ack`.
- **Old client, new daemon**: works through the derivations above. Reads stay old-readable: the daemon keeps the
  existing inbox v2 / watch item variants (a delivery row is still the `LazyMessage`/`Lazy` variant with its
  daemon-computed `wakes`, a receipt is still `Message`) and only adds `priority` (and `deadline_at`) as optional
  fields, so an old CLI or an already-installed old Claude mod parses and frames them as before, and its
  `wakes`-driven promotion follows the threshold. The new client frames from variant + priority (D8). An old
  client's `WakeOnQuery` gets `all` when T < 0, else `nudge`. An old client's lazy `Invite` is recorded as a
  confirming invitation; an old client's direct send creates an auto-joined pair (its creation policy is ignored).
- **New client, old daemon, 1:1 pairs**: without `direct.auto_join_v1` the client keeps sending the legacy pair
  creation policy, so the old daemon's invitation/accept path works unchanged.
- **Overdue warnings** come only from receipts, which exist only for explicit ACK recipients (D9), so a non-ACK
  message never produces one; pre-v47 receipts stay owed as recorded.
- **Person-authored "priority"** (summaries/catch-up `is_priority`, the wake-e2e note) is a separate, unchanged
  axis; it is not a delivery priority and does not enter `DeliveryOrderKey`.
- `PROTOCOL_VERSION` does not change.

## Legacy readers, wire validators and receipt-side effects (design roast round 1)

**Every legacy reader is converted to the question it actually asks** (cluster C1): "does this (message, seat) have a
delivery row" → `lazy_recipients` presence; "does it have a receipt" → `prepared_recipients`/receipt presence; "is
it a message at all" → the predicate is dropped; "does this seat wake" → `NudgeThreshold`. For set-based SQL,
`NudgeThreshold` has a store-side SQL fragment (owned with the seat threshold store API) that joins
`seats.nudge_threshold` and compares row priority, so no statement writes `p <= T` by hand. Any scan bounded by a
newest-N window that was correct only because every row was eligible (`wake_frontier`, `has_wake_rows`,
`PENDING_SUMMARY_CAP`) becomes a seek on the v47 index per admitted priority. Inventory and owners:

| Reader | Owner |
|---|---|
| `migrations/0026` triggers `lazy_recipient_source`, `lazy_recipient_display_published`, `lazy_manifest_source` (mode checks) — v47 drops and recreates them on row presence / priority | schema (`ht-yha.2`) |
| `store/messages.rs` digest payload, validation, duration, `stage_recipient`, replay lookup | send (`ht-yha.3`) |
| `lazy_delivery` completion (`recorded_mode`, `lazy_completion_state`) | send (`ht-yha.3`) |
| `lazy_delivery` inbox (`publication_eligible`, binding unread SQL, `seat_lazy_pending`, pending pages) and the per-binding displayed marks | DeliveryRowSeek (`ht-yha.5.2`) |
| `lazy_delivery` wake (`wake_frontier`, `oldest_pending_at`, `has_wake_rows`), `note_published`'s `wake_work` insert (`wake_on='all'`), `store/attention.rs`, `store/wake.rs`, `seats.rs` wake_on readers, per-item `wakes` | wake (`ht-yha.4`; `note_published` is edited by `ht-yha.5.2` using the wake child's SQL fragment) |
| `lazy_delivery` direct (`direct_lazy_display_admitted`, episode floors), `store/control.rs`, `delivery/invitation.rs`, handoff `automatic.rs` / `topology_handoff/attachment.rs` ordinary checks | pairs/invitations (`ht-yha.6`) |
| `queries.rs` `MessageDeliveryModes` (`recorded_mode` at the query), `invitation_content.rs` presentation SQL | framing (`ht-yha.8`) |
| `queries.rs` inbox_v2 region (private eligibility SQL, unread/receipt arms, thread scope, presentation reconcile), `read_cursors.rs` `RECEIVABLE` | merge (`ht-yha.5.4`) |
| `delivery/assemble.rs`, `service/mod_channels.rs`, the mod JS | watch/mod (`ht-yha.9`) |
| CLI (`commands.rs`, `mod.rs`, `journal.rs`, `direct_send.rs`, `wake_on.rs`, `handoff.rs`) | CLI (`ht-yha.7`) |

The integration sweep greps `src`, `integrations` and `migrations`. Tests: a mixed-ACK send (receipt for the explicit
recipient, delivery rows for the rest; display, completion and wake all work); one p0 row behind more than 1000
pending p1 rows wakes a T=0 seat (candidate and query plan asserted).

**Wire validators, both mixed-version directions** (cluster C3):

- `MessageDeliveryModes`: the reply struct is `deny_unknown_fields`, so `priority` is returned only when the request
  sets a new `with_priority` flag (omitted when false); only a client that sees `send.priority_v1` sets it. Old
  CLIs get the old shape.
- Send digest: the payload digest covers the request as sent — `priority` is included iff the request carries it.
  A journaled pre-v47 intent is retried in its original shape (no priority) and the daemon derives priority from its
  `delivery_mode`. The new client sends `priority` and omits `delivery_mode`; an explicit `delivery_mode: lazy`
  together with `priority: 0` is refused (`InvalidRequest`). Test: v46-prepared, v47-retried send for both shapes.
- `--deadline`: the daemon refuses a deadline without explicit ACK recipients from any client (`InvalidRequest`,
  detail naming `--require-ack`), including an old client's `--nudge --deadline`; the new CLI refuses it before
  reading the body; help text updated.
- `DirectThreadEnsured`: an auto-joined pair reports `(AlreadyJoined, no invitation)`, the one no-invitation shape
  the existing validator accepts, for both new and existing pairs. A journaled pre-upgrade selection that carried an
  invitation meets a pair created before the upgrade and still reports `ExistingPending` as before.
- Invitations from human and operator callers: the human-`RequireAck` refusal is dropped for thread invitations;
  human and operator invitations are recorded confirming with the deadline (they already had deadline and wake as
  legacy timed invitations). New client, old daemon: `human invite` and `--operator` keep sending today's
  `confirmation: None`.

**Receipt-path side effects** (cluster C4), assigned explicitly: a human seat named as an explicit ACK recipient gets a
priority-0 delivery row, not a receipt (humans owe no ACK); the unavailable-recipient warning is kept for priority-0
sends on the delivery-row path (the sender asked for urgency) and is not produced for priority >= 1; `carry_pending`
stays receipt-side.

## Docs and model-visible text

- **AGENTS.md** Delivery contract: 1. Messages default to priority 1 and seats default to T = 0, so a default send
  never wakes. 2. A pending item wakes an idle session only when its priority is `<= T` of the seat. 5. ACK
  requests, invitations and actionable warnings are priority 0. Rules 3, 4, 6 unchanged; the "What we do today"
  list and the wake-e2e section updated.
- **TRUST-POLICY.md**: direct pair creation joins both reserved seats (consent is the user's explicit decision,
  D7); delivery-row bookkeeping replaces lazy bookkeeping wording; receipts only for explicit ACK recipients;
  accepted limits that mention lazy updated. Amended in the same commit as the behaviour change.
- **Skill** (`integrations/skill/SKILL.md`), README, `docs/agent-usage.md`, mod/agy/Hermes READMEs, CLI help
  goldens: priority, threshold, `[pN]`, pair auto-join. Each new model-visible string states its size
  (`[p<N>]` is 4 bytes plus a space; the threshold output is one row).
- **Naming**: "priority" in summaries/catch-up (person-authored messages, `is_priority`) is internal; the new term
  in code is `delivery_priority` where the two meet.

## Testing

- Store/protocol: migration backfills and DDL audit; frozen priority in the intent digest; delivery rows for every
  recipient (human included) and receipts only for explicit ACK recipients; wake frontier at T = -1, 0, 3 against
  p0/p1/p5 rows, including raising T with pending rows; inbox v2 merged order with continuations across
  priorities and sources, within the work bound and with large unpublished backlogs; the ordering key's
  tiebreaks (same priority with and without deadlines, equal deadlines, equal decision_seq across sources) and
  stable continuation across pages that split equal-priority runs.
- CLI: flag matrix (`-p`, `--nudge`, removed `--lazy`, ACK + p>0 refused, deadline rules), `nudge-threshold`,
  `wake-on` alias, old-daemon fallbacks.
- Direct pairs: auto-join on first send and on handoff; pre-upgrade pending pair invitations still accept.
- Mod: priority-sorted framing; agy header parsing of `message` with `[pN]`.
- `scripts/wake-e2e.py`: cases send-p1 / send-p0 / send-p3 / send-ack × group/1:1, invite, with recipients at
  T = 0 / -1 / 3; a busy recipient is never nudged; ordering of a mixed page.

## Out of scope

Per-thread thresholds; priority on invitations or warnings other than 0; changing the summary/catch-up notion of
person-authored "priority"; any version gating.

## Key abstractions

**DeliveryPriority** — new value type (newtype over u8, 0..=9), a message's immutable notification priority
- Owns: the range check, the default (1), the fixed p0 for invitations, ACK requests and actionable warnings, the legacy mapping both ways (`ordinary` ↔ 0, `lazy` ↔ 1; p >= 1 → `lazy` with the lossy-downgrade flag for an old daemon), and the `[p<N>]` marker (only when N != 1).
- Hides: the numeric encoding and the D10 legacy derivation; old-wire derivation, client fallback and the v47 backfill all use this one mapping, so `DeliveryMode` is never matched to decide anything.
- Must not: decide wake (NudgeThreshold does), or create receipts/deadlines (D9: send preparation does, for explicit ACK recipients only).
- Relationships: leaf in `src/protocol`; store, CLI, delivery and watch depend on it; the digest and frozen intent serialize it.
- Lifecycle: plain `Copy` value, parsed at the CLI/wire boundary, read from `messages.priority`.
- Errors: out of range → `InvalidRequest` before any body is read; an out-of-range stored value → `StoreCorrupt`.

**NudgeThreshold** — new value type (-1..=9, default 0), the per-seat T (D1)
- Owns: the single wake predicate "this priority wakes this seat" (`p <= T`; -1 admits all), clamping negative input to -1, and the `wake-on` alias mapping (`all` ↔ -1, `nudge` ↔ 0; other values are not expressible to an old daemon).
- Hides: the threshold rule; every wake decision calls this predicate, the threshold-aware frontier, or (for set-based SQL such as `note_published`'s `wake_work` insert, `has_wake_rows`, `oldest_pending_at`) the store-side SQL fragment owned with the seat threshold store API — the full reader list is in §Legacy readers.
- Must not: be cached or frozen at send (D2); every wake/`wakes` decision reads the seat's current value.
- Relationships: `src/protocol` leaf; `store::seats` reads/writes it; `lazy_delivery::wake_frontier` takes it as input.
- Errors: an old-daemon-inexpressible value → client error with an upgrade hint; out of range → `InvalidRequest`.

**DeliveryOrderKey** — new value type with a total `Ord`, the presentation order of one pending item
- Owns: the tuple (priority asc, deadline asc with none last, publication `decision_seq` asc, source rank, per-source ordinal), the source-rank table (invitations 0, warnings 1, receipts and pre-upgrade binding-unread ordinary rows 2, delivery rows 3), and its continuation encoding.
- Hides: the "no deadline last" rule and the stable tiebreak; no caller compares these fields by hand.
- Must not: run queries; each source's indexed seek yields rows in key order, the key only compares heads.
- Relationships: shared by the inbox v2 merge (store), the watch batch sort (delivery assemble/page) and the cursor (`protocol/pagination`); the mod's JS sort mirrors it, checked against shared golden fixtures as `frame()` is today.
- Lifecycle: built per candidate row; the last emitted key persists only in the cursor token.

**Inbox v2 merge** — new module-level mechanism in `store/queries.rs mod inbox_v2`, replacing `rotate`: a k-way min-head merge over the existing closed `Source` enum
- Owns: each step, peeking the head of every unfinished source (one `LIMIT 1` seek each, counted against `CANDIDATE_LIMIT`) and emitting the smallest `DeliveryOrderKey`, within the existing row/byte/work bounds; a work-limited page ends with a continuation, never a false `empty`.
- Hides: head peeking and resume-after-key; callers (inbox text/JSON/machine, `delivery_pull`, `hook_inbox`, accept's thread-scoped display) still see one ordered page.
- Must not: walk an unbounded unpublished backlog (2026-10-07 bounded-walk rule): delivery rows carry the publication bound inside their partial-index seek, the other sources filter after their seek; or decide settlement (the settle path keeps display/ACK).
- Relationships: `Source` stays a closed enum with `match` (fixed kinds, growing operations: peek-head, resume-after-key, done); each arm delegates to its source's seek (delivery rows, unread, receipts, invitations, warnings).
- Lifecycle: per request; state round-trips through `InboxBatchV2CursorState`.

**InboxBatchV2CursorState** — extended: the last emitted key plus captured high waters replace the rotation pointer.
**lazy_delivery (delivery rows)** — extended: rows for every audience member except explicit ACK recipients, humans included, each with `priority` and a copied publication `decision_seq`; `wake_frontier` becomes "newest pending row admitted by T", the one function both `wake_seat_attention` and the `store/wake.rs` re-check call.
**settle::ModeFact** — extended: chosen by which row the (message, seat) actually has (receipt or delivery row), not by `delivery_mode`.
**FrameItem / frame() (Rust and JS)** — extended: carries priority; kind word always `message`; `[p<N>]` from `DeliveryPriority` (D8).
**WatchMessage** — extended: gains `priority` and `deadline_at`; the daemon computes `wakes` via `NudgeThreshold`.
**resolve_direct_pair** — extended: writes the peer's join (membership, interval, cursor init, event) through one join-recording helper shared with the caller's join (the same inserts are duplicated at 5 sites in `store/control.rs` today); `EnsureCreationPolicy` is ignored (D7).
**Send-priority resolution (CLI)** — extended into one function shared by `send`, `send --seat`, `human send` and handoff: `-p`/`--nudge` conflict, `--require-ack` forces p0 and refuses explicit p > 0 (D3), `--deadline` needs ACK recipients, old-daemon downgrade.

Variation: fixed source kinds with growing operations → closed `Source` enum in `mod inbox_v2`; legacy wire/storage spellings → mapping functions on `DeliveryPriority` and `NudgeThreshold` (the one place D10 lives); who performs the wake stays `HarnessAdapter`/`DeliveryPolicy` (contract rule 6), nothing per-harness added.
Walk-through: `send -p 3` → `DeliveryPriority(3)` frozen in the intent → audience delivery rows with priority 3 and copied `decision_seq` → `wake_seat_attention` → `wake_frontier` filtered by `NudgeThreshold` (T=0: no wake; T=3: one reserved wake) → hook page via the merge in `DeliveryOrderKey` order, framed `[p3]` → mod sorts the batch by the same key. Per-thread thresholds (out of scope) would touch one effective-threshold lookup and the two call sites that read it; predicate, merge and framing unchanged.
Per spec §Compatibility: `delivery_mode`, `wake_on` and `confirmation` stay written but never read for decisions — except stored `confirmation='lazy'` on pre-upgrade invitations, which keeps them non-waking and deadline-free.
Per spec §Delivery model (Wake): invitations, explicit ACK receipts and actionable warnings stay unconditional wake sources rather than calling `NudgeThreshold`; equivalent because they are p0.

Digest:
- DeliveryPriority: owns range, default, fixed-p0 classes, legacy mode mapping and the `[pN]` marker;
  new legacy spellings are added only here; callers never match on `DeliveryMode` to decide anything.
- NudgeThreshold: owns the `p <= T` wake predicate and the `wake-on` alias mapping;
  per-thread overrides go into the code that looks it up; callers never compare priority against T inline.
- DeliveryOrderKey: owns the total presentation order and source ranks;
  a new pending-item source gets a rank here; callers never sort pending items by individual fields.
- Inbox v2 merge: owns the min-head merge across sources under the work bound;
  a new source is one `Source` arm with peek and resume; callers never interleave sources themselves.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
