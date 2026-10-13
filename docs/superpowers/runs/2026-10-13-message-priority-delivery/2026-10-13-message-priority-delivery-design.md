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
- *source rank*: invitations, warnings, receipts, delivery rows (0..3); *per-source ordinal*: the row's ordinal.

Inbox v2 stops round-robin rotation between sources and instead, each step, peeks the head of every unfinished
source (one indexed `LIMIT 1` seek per source, each counted against `CANDIDATE_LIMIT`) and emits the smallest key;
the row limit, byte limit and work bound are unchanged, and a work-limited page ends with a continuation, never a
false `empty`. Each source's walk must yield its items in key order, so each gets a pending index in that order:
receipts (seat, deadline, decision_seq, ordinal), invitations (seat, deadline, decision_seq, ordinal), delivery
rows (seat, priority, decision_seq, ordinal), warnings (seat, decision_seq, ordinal). The delivery-row
`decision_seq` is copied onto the row at publication (staged rows have none and are not yet visible), so the seek
stays an index range scan; the publication filter is applied after the seek (the 2026-10-07 bounded-walk rule).
The continuation records the last emitted full key and the captured high waters; an item published after capture
belongs to the next traversal, as today. Out-of-order display within a thread is safe because each delivery row
records its own displayed state; the binding read cursor only advances over the contiguous displayed prefix, as
`advance_through_displayed` already does. The watch stream and mod carry `priority` and `deadline_at` on each
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
  seat's T. The mod sorts each batch it frames (wake and passive) by (priority, arrival) before framing, so one
  injection is in priority order; arrival across separate batches stays arrival order.
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
- **Old client, new daemon**: works through the derivations above.
- `PROTOCOL_VERSION` does not change.

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

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
