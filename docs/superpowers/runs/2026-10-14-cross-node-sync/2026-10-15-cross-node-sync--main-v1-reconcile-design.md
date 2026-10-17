# Cross-node sync: reconcile with main v1 (priority model) — design amendment

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · amends [fact kinds](2026-10-14-cross-node-sync--fact-kinds-design.md) and `TRUST-POLICY.md` · main's design: [message delivery priority](../2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery-design.md) (epic ht-yha, shipped in v1.0.0)

## Goal

Make the cross-node sync design on `super-auto/cross-node-sync` match what `main` shipped after the branch last merged it (merge base `53e400ed`, main 204 commits ahead: v1.0.0, v1.0.1 and epic ht-yha). After this amendment:

- every replicated fact carries main's delivery model (message priority 0..=9, invitations always confirming), so a node that applies a remote `message.sent` or `invitation.created` makes the same local delivery decision for its own seats that main makes for a local send;
- 1:1 pairs auto-join both seats across nodes without breaking home-node authority over membership;
- the clean-start, `inviter_kind` and restore-window error-code decisions are written down;
- the main merge has a conflict-by-conflict resolution rule and a test list.

Sync has never been released, so every change is made in place at `kind_version` 1. This step changes spec and policy text only; a separate step implements it.

## What main shipped (summary of ht-yha as it bears on sync)

- **Priority** (`DeliveryPriority`, 0..=9, 0 most urgent, default 1) is an immutable property of a message: `messages.priority`, `send_preparations.priority`, and `lazy_recipients.priority` (the delivery row) all hold it. `--nudge` is `-p 0`. `--require-ack` forces priority 0 (D3). `--lazy` is removed.
- **Nudge threshold** `T` (`NudgeThreshold`, -1..=9, default 0) is a per-seat column (`seats.nudge_threshold`). An idle seat wakes on a pending item exactly when `priority <= T` (T < 0: always), decided **at wake time** against the seat's current T (D2).
- **Receipts only for explicit ACK recipients** (D9). Every other audience member, and a human named with `--require-ack`, gets a delivery row at the message priority. The audit column `messages.delivery_mode` is written `ordinary` iff the send named explicit ACK recipients, else `lazy`. Since ht-yha.66 that recorded value is model-visible: history, search, read and follow show the `[pN] <id>` read annotation only for a message recorded `lazy` whose priority is not 1.
- **Invitations** are always priority 0, always wake, and always carry a deadline (`--deadline`, else the instance `invitation_default_ms`, 5 min) with an overdue warning to the inviter (D6). A seat inviter's invitation stores `confirmation='require_ack'`; operator and service invitations store none. Lazy invitations are gone.
- **1:1 pairs auto-join** both reserved seats on creation, with no invitation (D7). The peer's join records a read-cursor floor at the join, so its first session is offered the opener.
- **Wire**: protocol 8. Mixed versions refuse each other. Stored data migrates in place by migration 47.

## Decisions

| # | Decision | Rationale |
|---|---|---|
| R1 | The replicated payloads adopt main's priority model, changed in place at `kind_version` 1. `message.sent` carries `priority` (and `require_ack`) instead of `mode`/`nudge`; `invitation.created` drops `confirmation` and always carries a deadline. | Sync has never been released, so no node holds v1 facts of the old shape and a major bump would only add dead parking paths. |
| R2 | Each pair seat's home node authors that seat's `membership.joined` when it applies the pair's `thread.direct` (inline for the creator's node, as an ApplyEngine follow-up on the peer's node). | Keeps home-node authority for every membership transition (root §6, N4) while reproducing main's auto-join. |
| R3 | Clean start stays: upgrading v1.0.x users get a fresh node store, and their `instances/` history stays on disk but is not shown. The CHANGELOG entry below needs user reconfirmation before merge to main. | Migrating a single-writer store into a signed multi-origin journal is a non-goal (root §Non-goals); v1.0.x shipping makes the loss visible, so the user reconfirms it. |
| R4 | `invitation.created` names its issuer kind (`inviter_kind`: seat, service, operator). Only a `seat` invitation names an inviter seat and is authored by that seat's home node; a service or operator invitation has no inviter seat and takes its authority from the signing origin node. | Already implemented (ht-5fr.4.10). The spec and N4 still said every journaled invitation names an inviter seat, which is false for operator and service invitations. |
| R5 | The restore-window authoring hold refuses with `ServiceBusy` "own history catching up", not `Unavailable`. Recorded as a spec deviation. | `ErrorCode` has no `Unavailable` variant; `ServiceBusy` is the existing retryable code, and callers retry both alike. |

Choices main forced beyond R1-R5 are listed in §6.

## 1. Priority in the replicated payloads (R1)

### 1.1 `message.sent` v1

Before (branch as of `e9365e57`, `src/sync/facts/message.rs`):

```text
{ message, thread, author, body,
  mode: "lazy"|"ordinary",            // DeliveryMode
  nudge: bool,                        // reserved, never set
  relays_user: bool, user_intent: Option<..>,
  sent_at, duration_ms,
  recipients: [ { seat, receipt: "none"|"ack", deadline_at: Option<i64> } ] }
```

After:

```text
{ message, thread, author, body,
  priority: 0..=9,                    // required; messages.priority exactly as main stores it
  require_ack: bool,                  // default false; the send named explicit ACK recipients (--require-ack)
  relays_user: bool, user_intent: Option<..>,
  sent_at, duration_ms,
  recipients: [ { seat, receipt: "none"|"ack", deadline_at: Option<i64> } ] }
```

- Removed: `mode`, `nudge`. Added: `priority` (no serde default: a payload without it fails to decode and is `Rejected`, because a defaulted priority would silently change wake behaviour) and `require_ack` (`#[serde(default)]`).
- Main persists four per-message delivery facts, and each maps onto the payload: `messages.priority` and `send_preparations.priority` come from `priority`; each delivery row's `lazy_recipients.priority` comes from `priority` too, because main stages every delivery row at the message priority (a human `--require-ack` recipient included, whose message is p0 by D3); `messages.delivery_mode` / `send_preparations.delivery_mode` come from `require_ack` (`ordinary` iff true, else `lazy`), which is main's rule (`invited_recipients` non-empty). The receipt-versus-row split per recipient is `recipients[].receipt`, as before.
- `require_ack` exists because `receipt` cannot tell a human explicit ACK recipient (which gets `none` under D9) from an ordinary audience member, yet the recorded `delivery_mode` decides the model-visible `[pN]` read annotation (ht-yha.66). Without it, two nodes would print different history for the same message.
- **Validation** (`MessageSentPayload::check`, run by the handler before authority; a failure is `Rejected(InvalidRequest)`, recorded `failed` and replayable): `priority <= 9`; `require_ack` implies `priority == 0` (D3); a recipient with `receipt: ack` requires `require_ack`; `deadline_at` is `None` on every `receipt: none` recipient. `require_ack` is no longer "implied by any `ack` recipient": a send whose only explicit ACK recipients are humans has `require_ack: true` and no `ack` recipient.
- **Author** (`publish_send`): `priority` is the sealed preparation's `send_preparations.priority` (main's `resolved_priority`, frozen in the intent digest); `require_ack` is `!request.invited_recipients.is_empty()`. `recipients` is built from the sealed preparation as before, now under D9: `ack` exactly for staged `prepared_recipients` (explicit ACK recipients that are not human; for a remote seat, `remote_kind != 'human'`), `none` for every other audience member, with `deadline_at = sent_at + duration` when `eligible_at_snapshot = 1`. Every joined agent no longer owes an ACK.
- **Local branch** (`materialize_message`, `local_origin` with a sealed preparation): the adoption check compares the staged preparation's `priority` with `payload.priority`, its `delivery_mode` with `require_ack`, its ACK seat set and timing with the `ack` recipients, and its delivery-row seat set with the local `none` recipients. A mismatch is `Rejected(StoreCorrupt "staged audience differs from authored audience")`, which aborts the command.
- **Synthesized branch** (remote origin, or own old incarnation without a preparation), on the receiving node:
  1. `send_preparations(..., delivery_mode = require_ack ? 'ordinary' : 'lazy', priority = payload.priority, origin = 'replicated')`, inserted `building` and sealed before the manifest, as today, so main's v47 triggers (`lazy_recipient_source` requires the row's priority to equal its preparation's; `lazy_manifest_source` checks the preparation and the message agree) hold.
  2. One `prepared_recipients` row per `ack` recipient, local or remote (unchanged).
  3. One delivery row (`lazy_recipients`, `priority = payload.priority`) per **local** `none` recipient that is not suppressed (`receipt_settle_key(message, seat)`, fact-kinds §12). Remote `none` recipients get no row on any node but their home node.
  4. `messages(..., priority = payload.priority, delivery_mode = require_ack ? 'ordinary' : 'lazy')`, then `DeliveryTail::message`.
- **DeliveryTail::message** (fact-kinds §10) takes `DeliveryPriority` instead of `DeliveryMode`. It runs `lazy_delivery::note_published` for every message with delivery rows: main's set-based statement copies the publication `decision_seq` onto the rows and inserts `wake_work` for each recipient seat whose `NudgeThreshold` admits the priority (main's store-side SQL fragment), and it keeps the branch's `AND s.is_local=1`. It writes the `send_attention` job (`high_water = receipts + warnings + 1`) on every node for every message with at least one `ack` recipient or warning; the job projects `receipt_state` for every ACK recipient and calls `attention` (a no-op for remote seats) for local ones. Explicit ACK receipts stay unconditional wake sources (p0 admits every T).
- **The wake decision is local to the recipient's node.** `seats.nudge_threshold` is a node-local column of a local seat: it is not replicated, `nudge-threshold` / `wake-on` on a remote seat is refused like every other action naming a remote seat (N5), and a remote seat row's value is never read (remote seats get no wake work). Main's D2 holds per node: the recipient's node reads the seat's current T at wake time, so raising T there schedules at most one wake for an already-pending replicated row.
- **Ordering.** Main's `DeliveryOrderKey` (priority, deadline, publication `decision_seq`, source rank, ordinal) needs no replicated field beyond `priority` and the receipt or invitation deadline, which the payloads carry. Its `created` component is the local publication `decision_seq`, so two nodes order equal-priority, equal-deadline items by their local arrival order, which is the root's "local arrival order" rule.
- **Sender-side effects stay sender-side.** Main's unavailable-recipient warning for priority-0 delivery rows (C4) is evaluated only for local recipients on the author's node; a remote recipient gets none (fact-kinds §8: no availability evidence for another node's seat). Overdue warnings come only from receipts, which exist only for explicit ACK recipients, and only on the sender's node (fact-kinds §13).
- **Size.** `priority` and `require_ack` add at most 35 bytes per payload; the 448 KiB authoring cap is unchanged.

### 1.2 `invitation.created` v1

Before:

```text
{ invitation, thread, inviter: Option<SeatId>, inviter_kind: "seat"|"service"|"operator",
  invitee, invitee_n, created_at, duration_ms,
  deadline_at: Option<i64>,                   // None exactly when lazy
  confirmation: Option<"lazy"|"require_ack">, // None for service/operator
  actor_label: Option<String> }
```

After:

```text
{ invitation, thread, inviter: Option<SeatId>, inviter_kind: "seat"|"service"|"operator",
  invitee, invitee_n, created_at, duration_ms,
  deadline_at: i64,                           // required: created_at + duration_ms
  actor_label: Option<String> }
```

- Removed: `confirmation`, and the `Confirmation` payload enum. `deadline_at` is required (no serde default; absent is a decode error, `Rejected`).
- **Validation**: `duration_ms > 0`; `deadline_at == created_at + duration_ms` (checked add); a `seat` invitation names `inviter`; a `service` or `operator` invitation's `inviter` is optional, v1 authors leave it absent, and a receiver ignores it for authority (§4).
- **Author**: every invitation command (`invite`, `human invite`, `operator_orphan_invite`, voluntary service invitations on journaled threads, `handoff --new-thread` invitations) follows main's `invitation_policy`: always confirming, `deadline = decision.utc + (--deadline | invitation_default_ms | 300 000 ms)`. A requested `confirmation` is accepted for old clients and ignored, and the human `RequireAck` refusal is dropped (main, ht-yha.6). Service requirement invitations stay node-local (fact-kinds §2).
- **`materialize_invitation`** on every node:
  1. no-op if the ID exists;
  2. insert `invitations(..., frozen_duration_ms = duration_ms, deadline_at = payload.deadline_at, confirmation = (inviter_kind = 'seat' ? 'require_ack' : NULL), inviter_seat_id = (inviter_kind = 'seat' ? inviter : NULL), inviter_kind, invitee_n, origin_node)`, which is main's stored shape for each issuer;
  3. `recipient_retired` and `already_joined` settlement as before (fact-kinds §6 step 4);
  4. otherwise the membership becomes `invited`, the derived `invite` event carries `deadline_at` (and `confirmation: "require_ack"` for a seat inviter, as main's event does), and for a local, unsuppressed invitee `DeliveryTail::invitation(tx, invitee)` writes the invitation `wake_work` unconditionally (every invitation is p0; the `timed` argument goes away).
- The deadline lane, `already_joined` supersession and `invitation.answered` are unchanged. A missed deadline warns the inviter only on the invitation's `origin_node` (fact-kinds §13).

### 1.3 Unchanged kinds

`receipt.settled`, `invitation.answered`, `thread.created`, `thread.register`, `seat.*` and `node.*` keep their payloads. `membership.joined` gains one optional field (§2.2). `thread.direct` keeps its payload; its handler gains the auto-join follow-up (§2.3).

## 2. Cross-node 1:1 auto-join (R2)

### 2.1 Rule

A pair's two `membership.joined` facts are authored by the two seats' home nodes, never by one node for both:

- **Creating command** (`resolve_direct_pair`, new pair): authors, in order, `thread.direct`, `membership.joined{caller, n = 1}` (the creating join, as today) and, **only when the recipient is a local seat**, `membership.joined{recipient, n = 1, actor = caller}`. It authors no invitation. A creation policy is validated for shape and otherwise ignored (main D7).
- **Peer's home node**: when it applies a `thread.direct` from another origin (or from its own old incarnation), the `ThreadDirect` handler queues, through `ApplyContext::follow_up`, one `membership.joined{seat, n = 1, actor = payload.creator}` for each pair seat that is local, not retired, and has no `memberships` row in that thread. ApplyEngine authors it right after the trigger's own update, inside the trigger's savepoint, so its deps include the `thread.direct` (journal-core follow-ups, as `node.retag` uses them).
- A pair seat that already has a row (a crossed create, a later leave, an earlier join) gets nothing: the auto-join happens at most once per seat per pair, exactly like main's.

The one helper `direct::auto_join_local_pair_seats(tx, ..., thread, creator)` decides "local, not retired, no row" for both callers, so the inline and follow-up paths cannot drift.

### 2.2 `membership.joined` gains `actor`

Payload `{thread, seat, n, at, actor: Option<SeatId>}`. `actor` is additive (`#[serde(default)]`, omitted when absent): the pair creator, carried only for the derived `join` event's `actor_seat`, which reproduces main's peer join event (`{"action":"join","seat":<peer>,"actor_seat":<creator>}`). It is not authority: the fact is still authored only by `seat`'s home node (N4), and an applying node keeps `actor_seat` only when the actor is one of the pair's two seats, else it drops the field; the fact itself is never refused for its actor (the same rule as `thread.register`'s actor).

Attribution: an auto-join is not an action of any binding. Its observation is `{"provenance":"direct_auto_join","decided_at":<at>}` with no binding, staged by the creating command for the inline peer join and used as the fallback for a follow-up with nothing staged. `direct_auto_join` is a new TRUST-POLICY A3 value (added with this amendment).

### 2.3 Materialize rules on the receiving node

- `materialize_thread_direct` is unchanged (create if absent, min-register over `(hlc, node_uuid)`, one `create:` event).
- `materialize_membership(joined)` is unchanged (ordinal `n`, interval, `settle_superseded_invitations`, filters), except the **auto-joined pair floor**: for a joined fact with `actor` set on a direct thread, `read_cursors::init_for_join` seeds the seat's floor at the pair's first local position (the thread's head as of its `thread.direct` apply) instead of at the join. On the creator's node and on an inline follow-up nothing deliverable precedes the join, so this equals main's "floor at the join event"; it matters only when the join is authored late (below), where a floor at the join would hide messages already delivered to the seat.
- Auto-join writes no invitation, no attention and no `wake_work`. Only the messages' own priorities wake the peer, against its node's threshold (§1.1), exactly as main's single-node pair, where "the first direct message's own priority alone decides whether it wakes" (D7).

### 2.4 The pair audience before the peer's join arrives

On the creator's node A, the remote peer b has no membership row until B's `membership.joined{b}` arrives. A sends at once, and the author fixes the audience from its own view (root §6), so without a rule b would be left out of every message sent before the round trip. Rule:

- `walk_audience` on a journaled direct thread also stages the other reserved seat when it is not retired and has **no `memberships` row** in that thread ("auto-join pending on its home node"). It is staged like any audience member (delivery row or receipt by D9), so `message.sent.recipients` names it and B delivers to b.
- `require_pair_lifecycle` and `validate_pair_send` accept such a peer as joined. A peer with a row keeps today's checks (a peer that left still needs an explicit re-invite).
- On a single node the rule never fires: the creating command joins a local peer in the same transaction.

### 2.5 Convergence

- **Normal**: A authors `thread.direct`, `joined{a,1}`; B applies `thread.direct` and authors `joined{b,1}` as a follow-up before it applies anything later in A's log (A's messages to the pair have deps on the `thread.direct`, and B's join is authored in the same transaction as that apply). Every node ends with one `d…` thread, both seats joined with `n = 1`, and no invitation.
- **Crossed creates** (A and B open the same pair while partitioned): each node authors `thread.direct` and joins its own seat. After the heal, each applies the other's `thread.direct` as a min-register update; its own seat already has a row, so no follow-up; the other seat's join arrives in the other's log. Result on every node: one thread with the earliest creator, both seats joined at `n = 1`, zero invitations, zero wakes from the pairing itself. This replaces the fact-kinds §5 "both invitations settle `already_joined`" outcome.
- **Partition after a one-sided create**: A's sends during the partition name b (§2.4). After the heal B applies `thread.direct`, auto-joins b, then applies the messages in A's log order; each writes b's delivery row (or receipt), and their wakes coalesce in b's one `wake_work` row (root §7). No wake comes from the join.
- **Relays**: a third node applies the `thread.direct` and both joins from two logs in any order; membership converges by ordinal (fact-kinds §7). It has no local pair seat, so it authors nothing.
- **Order independence**: the follow-up depends only on the receiving node's own membership row for its own seat, which only its own log writes, so the authored facts do not depend on cross-origin arrival order.
- **Restore window**: the Apply lane applies only the active self log while the own-history hold is active (root §7), so a foreign or own-old `thread.direct`, and with it the follow-up, never applies inside the hold. Past the hold's cap a wiped node can author `joined{b,1}` while its unpulled old tail holds another `joined{b,1}` (and later transitions). The two facts carry the same ordinal and state, the second applies as a no-op by `n`, and later old transitions apply normally, so the outcome matches the restore-window residual (root §12) with no visible difference.
- **Follow-up failure**: a follow-up authoring error rolls back the trigger's savepoint and records the `thread.direct` as `failed`. That can only be a store error (retried with the quantum) or a bug. The entry stays replayable with `sync unapplied replay`, and later entries that need the thread park on it.

### 2.6 Wakes

The pairing adds no wake beyond main's: no invitation exists, joins write no attention, and each message to the pair wakes the peer only when its priority is `<= T` on the peer's node. A crossed create wakes nobody.

## 3. Clean start after v1.0.x (R3)

Root §1's clean start stands. v1.0.0 and v1.0.1 have shipped, so it now affects real installs:

- The upgraded daemon keeps its store in `<state_dir>/node/` and never opens `<state_dir>/instances/`. v1.0.x threads, messages, memberships, invitations, pending receipts, overdue tracking, seat nudge thresholds and per-instance `settings.json` are not migrated and not shown; they stay on disk. `doctor` reports them as legacy data, and `herdr-threads legacy purge` removes them (node-identity §9).
- Hooks installed by v1.0.x keep working (`--state-dir` resolves the node; `--host-endpoint` is the claim's host). Seats re-enroll on their next call and get new tagged IDs.
- The wire is protocol 9 (§6, C1): a v1.0.x CLI or daemon is refused at the version check, so the CLI and daemon upgrade together.
- Downgrading after the upgrade runs v1.0.x against its untouched `instances/` store, without anything the node store recorded.

**CHANGELOG entry text** (added under `## Unreleased` in this commit; **needs user reconfirmation before merge to main**):

> - **Breaking: cross-node sync starts with a fresh store (needs user reconfirmation before merge to main).** herdr-threads daemons on machines linked by your own `ssh` now share one thread space, and every daemon is a node with its own identity and a new store under `<state_dir>/node/`. The upgraded daemon does not read the v1.0.x store under `<state_dir>/instances/`: existing threads, messages, memberships, pending invitations and ACKs, overdue tracking and seat nudge thresholds are not carried over or shown. They stay on disk; `doctor` reports them, and `herdr-threads legacy purge` deletes them. Installed hooks keep working, and seats re-enroll with new IDs on their next call. Wire protocol 9: upgrade the CLI and daemon together. To go back, reinstall v1.0.x; it still reads its own untouched store.

## 4. `inviter_kind` (R4)

- `invitation.created` names `inviter_kind`: `seat` (a seat's `invite`, `human invite`, handoff invitations), `service` (a registered service author's voluntary invitation on a journaled thread) or `operator` (`operator_orphan_invite`).
- `inviter` is required for `seat` and optional otherwise; v1 authors leave it absent for `service` and `operator`.
- **Authority** (fact-kinds §17): `AuthorRule::SeatHome(inviter)` for a `seat` invitation; `AuthorRule::Any` for `service` and `operator`, so the signing origin node is the authority. That origin is stored as `invitations.origin_node` and owns the deadline and overdue decision (fact-kinds §13). Re-issued invitations keep their origin and pass.
- **Storage**: `invitations.inviter_seat_id` is NOT NULL exactly when `inviter_kind = 'seat'` (the baseline trigger refusing a seat invitation without an inviter), on journaled and node-local threads alike. Service and operator invitations keep NULL. This replaces fact-kinds §6's "`inviter_seat_id` becomes NOT NULL for journaled threads".
- TRUST-POLICY N4 is amended in this commit.

## 5. Restore-window hold error code (R5)

Root §7 and fact-kinds §2 say the restore-window authoring hold refuses with a retryable `Unavailable "own history catching up"`. The implementation (`src/store/materialize.rs`, `OWN_HISTORY_HOLD_DETAIL`, `is_own_history_hold`) refuses with `ErrorCode::ServiceBusy` and that detail, because `ErrorCode` has no `Unavailable` variant. Both are retryable, and callers retry them the same way. **Spec deviation recorded**: the spec text now says `ServiceBusy`, and display entry points recognize the refusal through `is_own_history_hold` (code plus detail), never by the code alone.

## 6. Choices main forced beyond R1-R5

| # | Choice | Why |
|---|---|---|
| C1 | **Protocol 9.** Main moved to protocol 8 for the priority break; the branch had independently moved to 8 for `CallerClaim.host`. The merged daemon is protocol 9, with both doc lines. | A v1.0.x client speaks protocol 8 without host locators. Reusing 8 would let it pass the version check and then fail on request shapes, which is the half-working state main's protocol rule exists to refuse. |
| C2 | **`require_ack` in `message.sent`** (§1.1). | Main's recorded `delivery_mode` decides the model-visible `[pN]` read annotation (ht-yha.66). The per-recipient `receipt` field cannot reproduce it for human-only explicit ACK recipients. |
| C3 | **`priority` and invitation `deadline_at` are required payload fields** (no serde default). | A defaulted priority or a missing deadline would silently change wake and overdue behaviour on the receiver. A decode failure is a visible, replayable `failed`. |
| C4 | **Pair audience includes a peer whose auto-join is pending** (§2.4). | Without it, the creator's sends before the peer's join round trip would leave the peer out, which main's atomic two-seat join never does. |
| C5 | **Auto-joined pair floor at the pair's first position** (§2.3). | A follow-up join authored late must not hide delivery rows already written for the seat. In every on-time case it equals main's floor. |
| C6 | **`membership.joined.actor` and the `direct_auto_join` provenance** (§2.2). | They reproduce main's peer join event (`actor_seat` = creator) without attributing the peer's join to the creator's binding. The trust policy requires a defined provenance value for this new kind of attribution. |
| C7 | **Main's pre-upgrade compatibility code that a node store can never reach is dropped**: `legacy_receipt_audience` (resumed v0.11 preparations), the test-only `LEGACY_PAIR_MARKER` / `legacy_pending_pair` fixture, the v47 backfills and upgrade tests. Kept: the client-side derivation of priority for a journaled pre-v47 send intent (`resolved_priority` from `delivery_mode`), the frozen columns main still writes (`delivery_mode`, `wake_on`, `confirmation`, `legacy_pending`) and their readers. | Clean start: no node store holds pre-v47 rows. The CLI's intent journal does outlive the upgrade, so its derivation stays. |
| C8 | **The node baseline is edited in place** to absorb migration 47 (`NODE_DELTAS` stays empty). | `schema.rs`'s own rule: the baseline is edited in place until the first release that ships the node store. |

## Merge plan

`git merge-tree --write-tree HEAD main` (HEAD `e9365e57`, main `6421963f`) reports 57 conflicted paths: 55 content conflicts and 2 modify/delete conflicts (`src/store/lazy_delivery.rs`, `tests/store/lazy26_prerequisite.rs`). Merge main into the branch (never the reverse) in one merge commit with these rules, then implement §1-§5 in follow-up commits. Throughout: **main's delivery decision, the branch's authoring, materialize and node structure.**

| Area (paths) | Conflict | Resolution rule |
|---|---|---|
| Schema: `src/store/schema.rs` (2 hunks), new `migrations/0047_message_priority.sql` | The branch replaced the 46-step chain with `migrations/node/0001_node_baseline.sql`, `initialize`/`verify_baseline` and `NODE_DELTAS`; main added step 47 to the chain and its `initialize`. | Keep the branch's `schema.rs`. Fold 0047's fresh-store DDL into the baseline in place (C8): `messages.priority`, `send_preparations.priority`, `lazy_recipients.priority` / `decision_seq`, `seats.nudge_threshold`, `binding_row_displays`, `binding_pending`, every v47 index and trigger, the recreated `messages_immutable`, `lazy_recipient_source` and `lazy_manifest_source`. Drop the backfill `UPDATE`s and delete `migrations/0047_message_priority.sql`. Re-apply node edits to the folded objects (`local_node` for `host_instances`, `is_local` guards). Port main's v47 DDL audits (trigger counts, CHECK lists) to the baseline audit. |
| `src/store/lazy_delivery.rs` (modify/delete) | Main split it into `src/store/lazy_delivery/{mod,completion,direct,inbox,staging,wake}.rs`; the branch made three edits in the old file. | Take main's split. Re-apply the branch's edits in their new homes: `capture_high_water` reads `local_node`; `note_published`'s (now threshold-based) `wake_work` insert keeps `AND s.is_local=1`; `cleanup_unpublished` treats `origin='replicated'` preparations as published. |
| `src/store/control.rs` (1 hunk, plus main's edits) | The branch split it into `src/store/control/{direct,invitation,membership,retirement,thread}.rs` and converted every writer into authoring plus materializer. Main edited the monolith: pair auto-join, `invitation_policy`, dropped human RequireAck refusal, the `joined_in_thread` / `bump_member_directory` helpers. | Keep the branch layout. Port main's behaviour into the authoring halves (§1.2 for invitations, §2 for pairs). Main's direct SQL writers do not come back (write-path closure, fact-kinds Testing). Shared helpers go to the submodule that uses them. |
| `src/store/messages.rs` (6 hunks) | Main: `resolved_priority`, `AudienceWalk { ack, priority, legacy_receipts }`, `Staging`, D9 staging, the priority-aware `insert_publication`. Branch: fact authoring, local and synthesized branches, remote-recipient staging, `MaterializeInput` signature. | Main's staging decisions inside the branch's functions. `stage_recipient`'s remote branch: receipt iff explicit ACK and `remote_kind != 'human'`; otherwise a delivery row only for a local seat. `insert_publication` keeps the branch signature and writes `priority`. Drop `legacy_receipt_audience` (C7). |
| `src/store/queries.rs` (1 hunk) | Main moved `mod inbox_v2` into `src/store/queries/inbox_v2/*` (min-head merge); the branch edited `inbox_v2` in place (node-id and locality edits at branch lines ~6452 and ~7191). | Take main's module tree. Re-apply each branch `inbox_v2` hunk in the corresponding new file. Keep the branch's other `queries.rs` hunks (remote seats in listings, `state = remote`). |
| `src/store/wake.rs` (1 hunk) | Main: threshold frontier (`NudgeThreshold`, `oldest_pending_at` by admitted priority). Branch: `is_local` and per-host filters. | Main's predicate and frontier; the branch's seat filters (`s.is_local=1`, host routing through HostSessions) on every seat-selecting statement. |
| `src/protocol/wire.rs` (1 hunk) | Both set `PROTOCOL_VERSION = 8` with different doc lines; main added `automatic_response_family`; the branch added `valid_instance`. | Protocol 9 (C1), both doc lines plus a 9 line; keep both functions. |
| `src/cli/wake_on.rs` (2 hunks) and `tests/cli/wake_on.rs`, `tests/store/wake_on.rs`, `tests/store/wake_on_wake.rs` | Main replaced `wake-on` with `nudge-threshold` plus a deprecated alias; the branch changed addressing (state dir, node). | Main's content with the branch's addressing re-applied. A remote seat is refused (N5). |
| `TRUST-POLICY.md` (1 hunk, header) | Both appended amendment lines. | Union, in date order. Then make main's A5 "Automatic pair reservation" text point at N4's auto-join rule for cross-node pairs. |
| `docs/superpowers/specs/INDEX.md` (1 hunk) | Both added rows at the top. | Union, newest first. |
| `docs/release.md` (1 hunk) | Main reworded step 7 (unattended wake e2e, release matrix); the branch added step 7a (real-ssh two-node check). | Main's step 7 followed by the branch's 7a. |
| `CHANGELOG.md` | No conflict in the merge-tree run; this commit's Unreleased entry will meet main's Unreleased bullet. | Union under one `## Unreleased`. The sync entry keeps its reconfirmation marker until the user confirms. |
| Mod framing goldens: `integrations/claude/mod/tests/fixtures/frame-goldens.ts`, `integrations/claude/mod/tests/frame_golden.test.ts`, `scripts/gen-frame-goldens.mjs`, `tests/delivery/frame.rs` | Main's D8 framing (`message` kind word, `[pN]` marker, priority sort); the branch changed ID shapes (tagged IDs). | Main's generator and test code; regenerate the fixtures after the merge with tagged IDs. |
| Doctor goldens: `tests/fixtures/golden/doctor/all.json`, `hermes.json` | Both changed fields (branch: legacy and node, main: threshold). | Regenerate from the merged binary and review the diff. |
| Upgrade-path and schema tests: `tests/store/schema.rs` (7), `lazy_schema.rs` (10), `query_work_q4.rs` (8), `topology_handoff_schema.rs` (2), `lazy26_prerequisite.rs` (modify/delete) | The branch deleted the upgrade-path cases (clean start); main added v47 upgrade and audit cases. | Branch side for upgrade paths; keep `lazy26_prerequisite.rs` deleted. Port main's fresh-store shape assertions (v47 objects, trigger lists, immutability triggers) to the baseline. |
| Priority behaviour tests: `tests/cli/{lazy_send,lazy_markers,q5_continuation_public,commands}.rs`, `tests/service/{delivery_pull,lazy_hook_pull,mod_ack_dispatch,q4_body_controls}.rs`, `tests/store/{attention,binding_lazy,direct_lazy_readers,direct_lazy_writer_floors,effective,invitation_content,lazy_metadata,lazy_publication,mod_ack,mod_seat_view,queries}.rs`, `tests/hook_entrypoint.rs` (4), `tests/integration/{channel_activation,lazy_attention,session_bindings_smoke}.rs` | Main changed the expectations (priority, receipts only for explicit ACKs, auto-join); the branch changed fixtures (tagged IDs, `local_node`, journal-backed setup, `d…` pair IDs). | Main's assertions with the branch's fixture changes. Tests that build pre-v47 rows or legacy pending pairs are dropped (C7). |
| Pair and handoff tests: `tests/cli/direct_send.rs`, `tests/store/topology_handoff.rs`, `tests/store/handoff_fences.rs`, `tests/handoff_topology_authority.rs` | Main expects auto-join with no invitation; the branch expects `d…` IDs and journaled pairs. | Both: auto-join, no invitation, `d…` IDs. |
| Wire and capability tests: `tests/integration/wire_compat.rs`, `tests/protocol/{capabilities,output}.rs`, `tests/daemon/transport.rs` | Protocol number and capability lists. | Protocol 9. Capability list = main's (`send.priority_v1`, `seat.nudge_threshold_v1`, `direct.auto_join_v1`) union the branch's. |
| `tests/combined.rs` (1 hunk) | Module lists. | Union, minus the deleted upgrade suites. |

Files main changed that merge cleanly but need a semantic pass: `src/store/seats.rs` (threshold store API: refuse remote seats), `src/store/read_cursors.rs` (display marks and the pair floor, §2.3), `src/store/service_send.rs` (D9, node-local), `src/store/invitation_content.rs`, `src/store/handoff/automatic.rs` and `src/store/topology_handoff/attachment.rs` (pair route: no invitation, journaled via `resolve_direct_pair`), `src/store/effective.rs`, `src/store/bindings.rs`, and the branch's `src/sync/facts/*`, `src/store/delivery_tail.rs` and `src/store/materialization.rs`, which compile against main's types but still encode the old model until §1 lands.

Gate after the merge commit: `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral`, `nice scripts/check-default-features`, and focused tests (`schema`, `lazy`, `priority`, `facts`, `fact_convergence`, `fact_order`, `multinode`).

## Tests

Store level (two `SqliteStore`s, `test_support::fact_exchange`; update or add):

- `tests/sync/facts_message.rs`: round trip of the new payload; decode failure without `priority`; `Rejected` for `priority > 9`, for `require_ack` with `priority > 0`, for an `ack` recipient without `require_ack`, and for a `deadline_at` on a `none` recipient.
- `tests/sync/facts_invitation.rs`: round trip without `confirmation`; decode failure without `deadline_at`; `Rejected` for `deadline_at != created_at + duration_ms`; authority `SeatHome` for `seat`, signing origin for `service` and `operator` (a foreign origin passes for operator, a non-home origin fails for seat).
- `tests/store/fact_convergence_message.rs`: A sends p0, p1, p3 and `--require-ack` (one agent, one human) to a group with members on both nodes. B's rows: priorities equal A's; `delivery_mode` equal on both nodes (`ordinary` for both `--require-ack` sends); a receipt only for the agent ACK recipient; a p0 delivery row for the human. B's `wake_work` matches each local seat's T (T = 0, -1, 3), A has none for remote seats. Raising a B seat's T after apply wakes once (D2).
- `tests/store/fact_convergence_invitation.rs`: a seat invitation, an operator invitation and a service invitation across nodes. Every invitee's `wake_work` is written, and `confirmation` is `require_ack` / NULL / NULL on both nodes. An unanswered invitation is overdue only on its origin node.
- `tests/store/fact_convergence_membership.rs` / `fact_order.rs`: pair auto-join. One-sided create: B authors `joined{b,1}` as a follow-up with deps on A's `thread.direct`. Crossed creates in both apply orders: identical projection, zero invitations, zero `wake_work` from the pairing. Partition: A's three sends before B's join reach b in order. A wiped B past the hold cap: a duplicate `joined{b,1}` is a no-op.
- `src/test_support/journal_oracle.rs`: the projection gains `messages.priority`, `messages.delivery_mode`, `invitations.confirmation` (now deterministic) and `lazy_recipients(message, seat, priority)` for local seats in the per-node comparison; `seats.nudge_threshold` stays excluded (node-local).

Multinode (`tests/multinode/*`, daemons over the `dial_command` shim; rerun all, change or add these):

- `config_delivery.rs::wakes_follow_the_delivery_contract_across_daemons` → priority and threshold matrix across nodes: a p1 send to a T=0 seat on B wakes nothing and is pending; `-p 0` and `--nudge` wake once; `-p 3` wakes a T=3 seat and not a T=0 seat; T=-1 wakes on p1; `--require-ack` wakes, the ACK settles, and A shows `acked`; the sender is never woken.
- `config_delivery.rs::a_seat_no_fact_references_is_opened_as_a_pair_by_id` → both seats `joined` on both nodes, no `invitations` row for the pair, the opener is in b's inbox with no invitation item, a p1 opener wakes nobody, and a `-p 0` opener wakes once.
- New `config_pair.rs`: crossed pair creates under a partition heal to one thread with both joined, no invitations and no wakes. A one-sided create with three sends (p1, p0, p1) while B is unlinked: after the heal b is joined, gets all three in priority order, and is woken exactly once.
- New `config_invite.rs` (or a case in `config_delivery.rs`): `invite` across nodes wakes an idle T=0 invitee; accept round trip; the deadline is set on both nodes; an unanswered invitation warns only the inviter's node, after the cold-start overdue hold; `invite --lazy` is refused by the CLI.
- `config_direct.rs::two_nodes_direct_send_and_ack` → ACK across nodes under D9: receipts only for explicit ACK recipients; a human explicit recipient on B gets a p0 row and no receipt; overdue only on A.
- `config_wipe.rs`, `config_restore.rs`: the settled history uses p0 and `--require-ack`; a settled p0 row in the pulled-back tail does not wake, and an unsettled `--require-ack` is offered after the hold.
- `config_partition.rs`, `sweep.rs`: replace `--lazy`/`--nudge` arguments with `-p`; the sweep's projection includes priority; a pair created on both sides auto-joins; identical projections after the heal.
- `config_relay.rs`: the relay applies auto-join facts from two logs in either order.

Unchanged: the pre-release wake e2e stays single-node (root §13).

## Implementation changes (one line each, for tasks)

1. Merge main into `super-auto/cross-node-sync` per the Merge plan, with the gate run on the merge commit.
2. Fold migration 0047 into `migrations/node/0001_node_baseline.sql` (C8), delete `migrations/0047_message_priority.sql`, and port main's v47 DDL audits to the baseline audit.
3. Set `PROTOCOL_VERSION = 9` (C1) and update the wire and capability tests.
4. `MessageSentPayload`: drop `mode`/`nudge`, add required `priority` and defaulted `require_ack`, and add `check()` with the §1.1 validation.
5. `publish_send`: author `priority` from the sealed preparation and `require_ack` from the explicit ACK recipients; build `recipients` under D9.
6. `materialize_message`: the local-branch adoption check compares priority, delivery_mode and the delivery-row set; the synthesized branch writes `priority`/`delivery_mode` and delivery rows at the payload priority for local `none` recipients only.
7. `DeliveryTail::message`: take `DeliveryPriority`; note_published's threshold wake insert keeps `AND s.is_local=1`; the `send_attention` job only for receipts or warnings.
8. `InvitationCreatedPayload`: drop `confirmation` and the `Confirmation` enum, make `deadline_at: i64` required, and validate it equals `created_at + duration_ms`.
9. `materialize_invitation` / `insert_invitation`: derive `confirmation` from `inviter_kind`; `DeliveryTail::invitation` drops `timed` and always writes the invitee's `wake_work`.
10. Invitation authoring (`invite`, `human invite`, `operator_orphan_invite`, voluntary service invite, handoff invite): main's `invitation_policy`, with the human RequireAck refusal dropped.
11. `resolve_direct_pair`: author `thread.direct`, `joined{caller}` and `joined{recipient}` only for a local recipient; no invitation; the creation policy is shape-validated only.
12. `MembershipPayload`: add optional `actor`; the derived join event keeps `actor_seat` only for a pair seat.
13. `direct::auto_join_local_pair_seats` helper, shared by the creating command and the `ThreadDirect` handler's `follow_up`.
14. `read_cursors::init_for_join`: auto-joined pair floor at the pair's first local position (§2.3).
15. Attribution: `direct_auto_join` observation for auto-joins, staged inline and used as the follow-up fallback.
16. `walk_audience`, `require_pair_lifecycle` and `validate_pair_send`: include or accept a not-retired pair peer with no `memberships` row (§2.4).
17. Handoff automatic pair route (`handoff/automatic.rs`, `topology_handoff/attachment.rs`): no pair invitation, through `resolve_direct_pair`.
18. `nudge-threshold` / `wake-on` store API: refuse remote seats; the threshold stays node-local and is excluded from the oracle.
19. Drop the unreachable pre-v47 compatibility code and its tests (C7), keeping the client intent-journal priority derivation.
20. `journal_oracle::replicated_projection`: add `messages.priority`, `messages.delivery_mode` and per-node delivery-row priority; keep `invitations.confirmation`.
21. Store tests per §Tests (facts_message, facts_invitation, fact_convergence_*, fact_order).
22. Multinode tests per §Tests (config_delivery, new config_pair and config_invite, config_direct, config_wipe, config_restore, config_partition, config_relay, sweep).
23. Update fact-kinds Testing expectations still naming lazy, ordinary or pair invitations as the code lands, and append Post-Implementation Notes here.
24. Finalize the CHANGELOG entry after the user reconfirms R3 (remove the marker), before merging to main.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### 2026-10-16: items 11 to 23 implemented

What the implementation did relative to this design:

- **Items 11 to 13, 15 (pair auto-join authoring).** `resolve_direct_pair` authors `thread.direct`, `joined{caller}` and, for a local recipient only, `joined{recipient}`. A remote recipient's home node authors its own `joined` as the `ThreadDirect` follow-up, so a pair opened across nodes shows one seat joined until the recipient's node has applied the create. `MembershipPayload.actor` is optional and the derived join event keeps `actor_seat` only for a pair seat. The `direct_auto_join` attribution observation is the follow-up fallback.
- **Item 14 (read floor).** An auto-joined pair seat's cursors are seeded at the pair's `create:` event position, not the join-time head, so a late auto-join still sees the opener. A consequence the multinode wipe test shows: a wiped and restored pair seat's `inbox` walk starts at that floor and lists an already acknowledged opener as history (no `ack_candidate`); it has no delivery row and wakes nobody.
- **Item 16 (pending peer).** `require_pair_lifecycle` and the `walk_audience` explicit pass accept a not-retired pair peer with no `memberships` row yet, and `build_message_sent` names it `none`. Accepted snapshot race: for a raw thread send, if the peer's `membership.joined` lands between preparation and publish, the peer has a row past the preparation's interval high water and the payload leaves it out, as for any member that joins after the snapshot. A `--seat` direct send always names the peer through `direct_recipient`.
- **Item 17.** The handoff automatic pair route has no invitation.
- **Items 18, 20, 21.** `nudge-threshold` and `wake-on` refuse a remote seat; the oracle projects `messages.priority`, `messages.delivery_mode` and per-node delivery-row priority, and excludes the threshold.
- **Items 22 and 23 (this entry).** The multinode suite no longer carries any `pending ht-5fr reconcile item 22` ignore marker (13 tests had one). `config_pair` (crossed creates heal to one joined thread with no wake; a one-sided create delivers p0 before p1s with one wake) and `config_invite` (invitation wakes at T = 0, deadline and `require_ack` confirmation agree on both nodes, overdue only on the inviter's node, `invite --lazy` refused) are new. `config_delivery` runs the threshold matrix on one seat per case, `config_direct` adds the D9 person recipient and origin-only overdue, `config_relay` adds a relayed pair auto-join with no joined fact authored by the relay, `config_partition` and the sweep write into a pair at several priorities. The test harness gained `Cluster::human_seat` and a `set_threshold` step.

Where a multinode expectation differed from the Tests list above:

- A seat is woken for an invitation only while the invitation is still pending, and the notification backoff spaces a second wake of the same seat by about 30 s. The sweep therefore waits for each invitee's one wake before accepting, and judges the later `-p 0` and `--require-ack` sends on delivery only. The wake matrix itself runs on fresh seats in `config_delivery`.
- The wipe test no longer asserts that the settled opener is absent from the restored seat's inbox (see item 14 above); it asserts that no delivery row exists for it, that it is acknowledged, and that the seat is woken once, for the unsettled message.
- The partition test opens the pair before the cut, so the unreachable-node marker has a remote participant to name; a pair created while cut has only the creator joined until the heal, which `config_pair` covers.
