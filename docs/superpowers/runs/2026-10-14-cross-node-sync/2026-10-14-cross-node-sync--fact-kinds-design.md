# Cross-node sync: fact kinds and write-path conversion — design

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · Bead: ht-5fr.4

## Goal

Every replicated mutation (seat lifecycle, thread create, direct pair, topic/name/archive, invite and answer, join and leave, send, receipt settlement) is authored as one v1 fact and materialized only by that fact's `FactKind` handler, on the authoring node and on every other node alike. As a result, two stores that applied the same facts hold the same replicated rows. A local recipient of a remote fact is delivered to and woken exactly as for a local send. Remote seats never get wake, liveness, observation or archival work. Overdue warnings come only from the node that authored the obligation.

## Problem description

After ht-5fr.3 the journal, the `ApplyEngine`, `FactKind`/`FactRegistry` and `ThreadRegisters` exist, but only `node.genesis` and `node.retag` use them. Every other write still goes straight to the base tables inside a command's `apply` closure:
- `src/store/control.rs`: `create_thread_impl`, `resolve_direct_pair`, `invite`, `operator_orphan_invite`, `accept`, `reject`, `join`, `leave`, `set_topic`, `set_thread_name`, `set_archived`, `begin_retirement_fence`;
- `src/store/messages.rs`: the `prepare_send_step_impl`/`publish_send` pair and `insert_publication`;
- `src/store/receipts.rs`: `ack_impl`, `ack_mod_delivered`, `delivery_confirm`, `ack_policy_delivered` (all through `settle_pending`);
- `src/store/seats.rs`: the two seat inserts (allocation ~L2614, operator fresh ~L3411);
- `src/store/archival.rs`: automatic archival.

These writers mix three kinds of work: the decision (A2 validation, IDs, deadlines, audience), the replicated rows, and local-only work (delivery rows, `wake_work`, filter revisions, derived info/warn events, attribution of the local binding). Nothing in them handles a seat whose home is another node. The deadline lane warns about every pending obligation in the store, wherever it came from. Root §6–§8 fixes the kind list, says that the author fixes the audience, and says that overdue decisions stay with the deciding node. This subepic converts each writer into an authoring half and a handler, extracts the local delivery tail, and makes every node-local worker ignore remote seats.

## Main challenges

The send path is two-phase: bounded preparation transactions stage `prepared_recipients`/`lazy_recipients` before a constant-size publish. The handler must reuse those staged rows on the authoring node, yet build them from the payload on every other node, with the same result. Receipts are not written at publish: the `send_attention` work job projects them later through `effective_receipt`. That job must run on every node so that receipt state is replicated, while still bumping attention only for local seats. Attribution columns (`native_observation`, `ack_observation`, `accepted_observation`, `sender_binding_id`) describe a local binding that other nodes do not have. Membership transitions have no fact ID, so a re-issued fact (ht-5fr.6) would toggle intervals twice unless the handler can recognize it. The two members of a direct pair can create the same `d…` thread and invite each other while partitioned, and the result must converge without waking anyone. The ACK info event groups up to 100 receipts per thread, while root and ht-5fr.3 fix one settlement key per receipt, and timeline rows are immutable. Seats are allocated for every observed pane long before they matter to another node, and their agent/human kind is only known once a binding exists. Service-managed threads (`managed_owner_author_id`, `service_*`, `requirement_episodes`) depend on node-local service registrations. Remote message traffic bumps `timeline_revision`, which today invalidates every in-flight local send preparation (research 02 R2). Finally, every converted writer must keep today's single-node behaviour, which the existing store suites pin, byte for byte.

## Key decisions made

Each kind is one module under `src/sync/facts/`. Each handler calls one store-level materializer (`pub(crate)` in the table-owning store file), so SQL stays in `src/store`. Commands keep validation and the decision in their closures, then author one or more facts, in a fixed order, through `journal::author`. A local seat is published (`seat.created`) the first time a fact references it, not at allocation. Its kind is taken from its open binding at that moment.

Local attribution is staged by the command in a local table and taken by the handler. Remote facts get a `replicated_from:<uuid>` attribution, so attribution columns are never replicated.

Membership facts carry a home-node transition ordinal `n` for idempotence.

`message.sent` carries the full author-decided audience. Its handler either adopts the local sealed preparation (keyed by the derived preparation ID) or synthesizes an identical one from the payload. The `send_attention` job runs on every node. DeliveryTail's one attention writer is a no-op for remote seats.

`receipt.settled` is one fact per receipt and carries a settle-batch tag, so the grouped ACK info event is appended once per (batch, thread), when the batch's last receipt in that thread applies.

Crossed direct-pair invitations settle `already_joined` from either arrival order.

`messages` and `invitations` gain `origin_node`. The overdue recorder refuses obligations authored elsewhere, and the due scans, sparse-due triggers and warning attribution are scoped to match.

Automatic archival authors `thread.register archived` and skips threads with any non-retired remote member.

Service-managed threads stay node-local in v1: they are never journaled and refuse remote seats. Their writers call the same materializers through a local-only entry point.

The publish fence drops `captured_timeline_revision` for preparations with no unavailable warnings.

## Decision points

### 1. Module layout and ownership

**Recommended.**

| File | Contents |
|---|---|
| `src/sync/facts/seat.rs` | `SeatCreated`, `SeatRetired` |
| `src/sync/facts/thread.rs` | `ThreadCreated`, `ThreadDirect`, `ThreadRegister` |
| `src/sync/facts/invitation.rs` | `InvitationCreated`, `InvitationAnswered` |
| `src/sync/facts/membership.rs` | `MembershipJoined`, `MembershipLeft` |
| `src/sync/facts/message.rs` | `MessageSent` |
| `src/sync/facts/receipt.rs` | `ReceiptSettled` |
| `src/sync/facts/payload.rs` | shared payload value types (`SeatKind`, `RecipientSpec`, `ReceiptObligation`, `SettleHow`, `AnswerOutcome`, `RegisterField`, `SettleBatch`) and the `EntityKey` builders `seat_key`, `thread_key`, `invitation_key`, `message_key`, `receipt_settle_key`, `answer_key` |
| `src/store/delivery_tail.rs` | `DeliveryTail` (§10) |
| `src/store/attribution.rs` | the local attribution staging table and `Attribution` (§3) |
| `src/store/materialize.rs` | `MaterializeInput` (§2) and the local-only entry point |

- `FactRegistry::builtins()` (ht-5fr.3) gains one `.with::<K>()` per kind above, all with `MAJOR = 1`.
- Every handler is a few lines: decode, build `MaterializeInput` from `ApplyContext`, call the materializer, map errors with `FactError::from_api`.
- The materializers live with their tables:
  - `seats::materialize_seat_created` and `seats::materialize_seat_retired` (`src/store/seats.rs`, `src/store/control.rs`);
  - `control::materialize_thread_created`, `control::materialize_thread_direct`, `control::materialize_register`, `control::materialize_invitation`, `control::materialize_answer`, `control::materialize_membership`;
  - `messages::materialize_message`;
  - `receipts::materialize_settlement`.

**Considered.**
- Materializing SQL inside `src/sync/facts/`: it would put table SQL in two trees, and `src/sync` would depend on every store table.
- One module per kind (11 files): related kinds share private helpers. Grouping by entity halves the files and keeps "one module plus one registry entry" for a future kind.

### 2. Authoring pattern and the local-only exception

**Recommended.** A converted command keeps its two closures. `validate` is unchanged. `apply` (the decide step, A2) does the following, in order:
1. Decide exactly as today: validation, IDs from `public_ids::fresh`, deadlines from `decision.utc`, the audience from the sealed preparation.
2. Call `seats::ensure_published(tx, &seat)` for every local seat the facts will reference (§4).
3. Stage local attribution where the kind has one (§3).
4. Author the facts in a fixed per-command order through `journal::author::<K>` (or `author_above` for registers). Each fact is applied inline by its handler.
5. Read back whatever the `CommandResult` needs.

No command writes a replicated table directly after this subepic. A test-support assertion enforces it (Testing, "write-path closure").

Per-command fact order:

| Command | Facts, in order |
|---|---|
| `create_thread*` | `thread.created`, `membership.joined{creator}` |
| `ensure_direct_thread` (new pair) | `thread.direct`, `membership.joined{caller}`, `invitation.created{recipient}` |
| `invite` / `operator_orphan_invite` | `invitation.created` |
| `accept` | `invitation.answered{accepted}` |
| `reject` | `invitation.answered{rejected}` |
| `join` / `leave` | `membership.joined` / `membership.left` |
| `set_topic` / `set_thread_name` / `archive` / `reopen` | `thread.register` |
| `publish_send` (and its bootstrap and delivery variants) | `message.sent` |
| `ack`, `ack_displayed`, `ack_mod_delivered`, `delivery_confirm`, `ack_policy_delivered` | one `receipt.settled` per newly settled receipt, in request order |
| seat retirement fence | `seat.retired` (only if the seat is published) |
| archival lane decision | `thread.register{archived}` |

Commands that today return early without writing (`AlreadyJoined`, an existing pending invitation, an already-joined `join`) author nothing.

**`MaterializeInput`** (`src/store/materialize.rs`) is the plain struct every materializer takes:

```rust
pub(crate) struct MaterializeInput<'a> {
    pub decision: DecisionInstant,   // local apply instant
    pub decision_seq: u64,           // the entry's one position (ht-5fr.3 §7)
    pub origin_node: NodeUuid,
    pub origin_ms: i64,              // envelope HLC ms
    pub local_origin: bool,          // origin node is self (any incarnation)
    pub provenance: &'a str,         // "local" or "replicated_from:<uuid>"
    pub suppress: &'a dyn Fn(&Transaction<'_>, &EntityKey) -> Result<bool, ApiError>,
}
```

Handlers build it from `ApplyContext`, using `decision_seq`, `origin.node`, `origin_ms()`, `is_local_origin()`, `provenance()` and `delivery_suppressed`.

**Local-only exception: service-managed threads.**
- Threads with `managed_owner_author_id IS NOT NULL` depend on node-local `service_authors` registrations. In v1 they are node-local: never journaled, never holding a remote seat.
- New column `threads.journaled INTEGER NOT NULL CHECK(journaled IN (0,1))`: 1 for threads created by `thread.created`/`thread.direct`, 0 for managed threads created by `service_controls::ensure_thread`.
- Commands on a `journaled = 0` thread skip step 4. They call the same materializer through `materialize::local_only(tx, decision) -> MaterializeInput`, which allocates `decision_seq` with `next_decision_seq`, sets `origin_node = self` and `local_origin = true`, and never suppresses. This covers the service writers (`service_controls`, `service_send`, `accept_required`, `requirement_episodes`) and ordinary commands that target a managed thread.
- `service_controls::invite` and every invite, send or join on a managed thread refuse a remote seat with `Unsupported` "service threads are node-local".
- A local seat is not published by membership in a managed thread.

**Considered.**
- Replicating managed threads: needs replicated `service_authors` and requirement episodes, a new feature outside the bead's kind list.
- Keeping managed threads on the old direct writers: two copies of every materializer. The `MaterializeInput` seam keeps one copy.
- Authoring a single "command" fact that bundles several effects: it breaks "membership transitions are authored only by the seat's home node" when the caller and invitee live on different nodes, and it defeats per-kind parking.

### 3. Payload conventions, attribution, keys and time

**Recommended.**
- **Field rules** (FactKind review rules of ht-5fr.3 §9 apply).
  - Payloads carry public IDs and origin data only, never local positions: no `decision_seq`, sequence, ordinal, episode, revision or `host_id`.
  - Field names are snake_case. Every struct is `#[serde(default)]`-tolerant and never uses `deny_unknown_fields`.
  - Times are origin wall-clock Unix ms (`i64`).
- **`sent_at` and deadlines.** The author's `decision.utc` is the deadline clock (root §7). Payloads carry it as `sent_at`/`created_at`/`settled_at`/`answered_at`. Handlers store these values as the row's `decision_at`/`created_at`/`acked_at`.
  - Root §5 says `messages.sent_at` is the origin's HLC time. The authoring HLC is `tick(decision.utc)`, whose ms equals `decision.utc` except after a backwards wall step, where the HLC runs ahead by at most the step. Storing the payload's wall value keeps `deadline_at − sent_at` equal to the frozen duration on every node, which `effective_receipt` relies on.
  - This is a refinement, not a change of ordering: arrival order stays the apply order and LWW still uses the envelope HLC.
- **Attribution.** New local table, in the baseline:

  ```sql
  CREATE TABLE fact_attribution_staging (
    staging_key TEXT PRIMARY KEY,
    binding_id TEXT,
    binding_generation INTEGER NOT NULL CHECK(binding_generation >= 0),
    observation TEXT NOT NULL CHECK(length(CAST(observation AS BLOB)) <= 4096)
  ) STRICT;
  ```

  - `attribution::stage(tx, key, &AccountableActor, observation)` is called by the command before authoring.
  - `attribution::take(tx, &MaterializeInput, key) -> Attribution { binding_id: Option<String>, generation: i64, observation: String }` is called by the materializer. When a staged row exists, it is deleted and returned. Otherwise `binding_id = None`, `generation = 0` and `observation = {"provenance":"<provenance>","decided_at":<origin_ms>}`: `replicated_from:<uuid>` for remote origins, and `local_journal` for an own-old-incarnation entry (wipe or restore pull-back) that has no staged row.
  - Staging keys:
    - `send:<message>`;
    - `ack:<message>/<seat>`;
    - `answer:<invitation>`;
    - `join:<thread>/<seat>/<n>`.

    They are local strings and are never `EntityKey`s.
  - TRUST-POLICY: `replicated_from:` comes from ht-5fr.9. `local_journal` is new: the task that first writes it checks `TRUST-POLICY.md` for the value and adds it in the same commit if missing (AGENTS.md trust rule).
- **Keys.**

| Kind | `creates` | `needs` |
|---|---|---|
| `seat.created` | `s…` | — |
| `seat.retired` | — | `s…` |
| `thread.created` | `t…` | creator `s…` |
| `thread.direct` | `d…` | `seat_lo`, `seat_hi` |
| `thread.register` | — | thread |
| `invitation.created` | `i…` | thread, inviter, invitee |
| `invitation.answered` | `i…/answer` | `i…` |
| `membership.joined` / `.left` | — | thread, seat |
| `message.sent` | `m…` | thread, author, then every recipient while the total stays ≤ 256; past that, thread and author only |
| `receipt.settled` | `m…/s…` | `m…` |

  The deps vector already orders a recipient's `seat.created` before the message (the author had applied it), so dropping recipients from `needs` past 256 changes only the parked-creator case. There, a recipient whose seat row is missing is a `Rejected` "unknown recipient seat <id>", which is replayable.
- **Idempotence** (root §5, ht-5fr.6 re-issue):
  - seat, thread, invitation and message facts are no-ops when their ID row exists;
  - `thread.direct` updates only the earliest-creator fields;
  - registers are LWW;
  - `invitation.answered` is a no-op unless the invitation is `pending`;
  - membership facts compare `n` (§7);
  - `receipt.settled` is a no-op unless the receipt is `pending`.
- **Payload size.** The largest is `message.sent`: body ≤ 64 KiB plus ≤ 100 explicit recipients and the thread audience at about 60 bytes per recipient. A 1000-member audience is about 125 KiB, under the 512 KiB cap. `MessageSent::validate_for_author` refuses a payload over 448 KiB with `InvalidRequest` "audience too large to replicate" before authoring.

**Considered.**
- Replicating the observation JSON: it leaks session, execution and boot IDs to other nodes for no use there, and it is about 250 bytes per fact.
- An `ApplyContext` sidecar for local extras: it would change ht-5fr.3's surface. A staged row in the same transaction is the pattern the root already chose for preparations.
- Storing the HLC ms as `decision_at` and deriving the deadline from it: the author cannot know its HLC before `journal::author` runs, so `deadline_at` and `sent_at` could disagree after a clock step.

### 4. Seats: publication, `seat.created`, `seat.retired`, remote rows

**Recommended.**
- **Publish on first reference.** New column `seats.published INTEGER NOT NULL DEFAULT 0 CHECK(published IN (0,1))`, with the CHECK `is_local=1 OR published=1`.
  - `seats::ensure_published(tx, seat)` is a no-op for remote or already published seats. Otherwise it reads `schema::open_binding_role(tx, seat)` (`Some("human")` → `human`, anything else → `agent`), authors `seat.created`, and the handler sets `published = 1`.
  - Allocation (~L2614) and operator fresh seat (~L3411) stay local inserts and author nothing. Panes that never take part in a replicated fact cost no journal entries.
  - Accepted limit: a seat's published kind is fixed. If its occupant later switches between human and agent, other nodes keep the published kind. Only the ACK expectation of remote recipients depends on it.
- **`seat.created` v1** payload `{seat, kind: "agent"|"human", display_name: null}`. `display_name` is reserved and always `null` in v1, because seats carry no name today. The created time is the envelope HLC, so the payload does not repeat it.

  `materialize_seat_created`:
  - row exists and is local: set `published = 1` (the authoring node);
  - row exists and is remote: no-op;
  - row absent and `home_node ≠ self`: insert `seats(id, instance_id = self node, state = 'remote', role = 'native', home_node = origin, is_local = 0, remote_kind = kind, published = 1, generation = 0, created_at = origin_ms, host_id/target_id/structural columns NULL)`;
  - row absent and home is self (own-old-incarnation pull-back after a wipe): insert a local `unresolved` seat (`unresolved_reason = 'other'`, `published = 1`) on the `herdr_hosts` row with the greatest `last_attached_at`. With no `herdr_hosts` row it is `Rejected` "own seat restore needs an attached host", which `sync unapplied replay` re-attempts. The seat cannot resume until C1 or an operator rebinds it, as with any unresolved seat.
- **`seat.retired` v1** payload `{seat, reason: "closed"|"operator", retired_at}`.
  - **Author side.** `control::begin_retirement_fence` authors it when the seat is published, after its existing local fence writes (bindings end, offers cleared, `wake_work` reset). The handler then performs only the replicated part, which for the local origin is a no-op on the already-retired row plus the derived audit.
  - **Remote side** (`materialize_seat_retired` when the seat is remote):
    1. `UPDATE seats SET state='retired', retired_at=?retired_at, retired_seq=?decision_seq WHERE id=? AND state!='retired'`.
    2. `bump_lifecycle_revision`, the directory filter bumps, and a `retirements` row (`origin = 'replicated'`) so that the existing deadline-lane retirement job (`advance_retirement`) settles the seat's pending invitations and receipts as `recipient_retired` and closes its memberships, as it does for local seats.
  - `retirements` gets `origin TEXT NOT NULL DEFAULT 'local' CHECK(origin IN ('local','replicated'))`. Its `closure_*` columns become nullable, with `CHECK((origin='local') = (closure_boot IS NOT NULL))`.
  - The job's `record_overdue_if_pending` calls warn only for obligations this node authored (§13).
  - A `message.sent` that applies after the job passed (its `send_seq > cutover_seq`) settles that recipient as `recipient_retired` at apply (§8), so both arrival orders converge.
- **Eligibility.** `seats::eligible_delivery_recipient` branches on the local-seat predicate: a remote seat is eligible iff `state = 'remote'`, and a local seat keeps today's check. `invite`'s target check and `walk_audience`'s explicit-recipient check accept `state = 'remote'` seats as not retired. `resolve_direct_pair` accepts a remote recipient.

**Considered.**
- Authoring `seat.created` at allocation: one fact per observed pane, most of which never matter, and the agent/human kind is unknown before a binding.
- Authoring a `seat.kind` change fact: outside the root's kind list. The kind affects only ACK expectation.
- Retiring an unrestorable own seat locally: a local-only retirement would diverge from every other node, and retiring on heuristic evidence breaks TRUST-POLICY.

### 5. Threads: `thread.created`, `thread.register`, `thread.direct`

**Recommended.**
- **`thread.created` v1** `{thread, topic, goal, name: Option<String>, creator}`.
  - `materialize_thread_created` inserts `threads(id, instance_id = self, topic, goal, name, created_at = origin_ms, updated_at, journaled = 1)` if absent.
  - It then calls `thread_registers::upsert` for `topic` and, when present, `name`, with the entry's `(hlc, origin.node, ingest_seq)`, and copies the winners. A register can never be ahead of its thread's creation because `thread.register` needs the thread.
  - It appends the derived `create` info event (`append_event_once_with_decision_seq`, key `create:<thread>`) and bumps the directory filter.
  - The creator's membership is the next fact (§2).
- **`thread.register` v1** `{thread, field: "topic"|"name"|"archived", value: Option<String>}`. `archived` values are `"1"`/`"0"`, and a `null` name clears it.
  - **Authoring:** `author_above::<ThreadRegister>` with `thread_registers::current` as the floor. The command's existing checks stay as A2 on the local view: the topic CAS on `topic_revision`, archived/joined/owner checks, and name validation.
  - **`materialize_register`:**
    - `upsert` returns `Won` or `Lost`. On `Lost` it does nothing more (no event).
    - On `Won` it copies the value to `threads.topic`/`name`/`archived` when it differs.
    - When the value changed, it applies today's side effects:
      - topic: `topic_revision+1` and the `topic`/`directory` filter bumps;
      - name: the `name/all` filter bump;
      - archived: `directory_revision+1`.

      It then appends the derived info event with key `register:<thread>:<field>:<uuid8>:<counter>`, using the existing payload shapes (`topic`, `name`, `archive`/`reopen`).
  - **Receivers never reject:** a message or join into a thread they see as archived still applies (root §6).
- **`thread.direct` v1** `{thread, seat_lo, seat_hi}`. `thread` must equal `protocol::ids::direct_thread_id(seat_lo, seat_hi)` and `seat_lo < seat_hi`; otherwise `Rejected`.
  - **Author:** `resolve_direct_pair` replaces `public_ids::fresh` with `direct_thread_id`, and finds an existing pair by `threads.id = d…` instead of `direct_threads(instance_id, seat_a, seat_b)`.
  - **`materialize_thread_direct`:**
    - If absent: insert `threads(id, topic = goal = "1:1 <lo> <hi>", created_at = origin_ms, journaled = 1)` and `direct_threads(instance_id, seat_a = lo, seat_b = hi, thread_id, created_hlc, created_node)`. The default topic is not written to the register (root §6).
    - If present and `(hlc, origin.node) < (created_hlc, created_node)`: update `created_hlc`, `created_node` and `threads.created_at`, so every node converges on the earliest creator. Nothing else changes, and no event is appended.
    - The derived `create` info event (key `create:<thread>`) is appended once, by whichever apply inserts the row.
  - New `direct_threads` columns: `created_hlc INTEGER NOT NULL`, `created_node TEXT NOT NULL` (hex32 CHECK).
- **Crossed creates.** A and B open the same pair while partitioned. Each node authors `thread.direct`, joins its own seat and invites the other. After exchange both seats are joined, and each invitation settles `already_joined` (§6) with no wake. The pair ends with one thread, two joined members and two settled invitations on every node.

**Considered.**
- Writing the direct default topic as a register at the earliest HLC: root §6 forbids it, and the label is a pure function of the pair.
- Letting `thread.created` also join the creator: it would hide a membership transition inside a thread fact and need the transition ordinal in two kinds.

### 6. Invitations: `invitation.created`, `invitation.answered`, `already_joined`

**Recommended.**
- **`invitation.created` v1** `{invitation, thread, inviter, invitee, created_at, duration_ms, deadline_at: Option<i64>, confirmation: Option<"lazy"|"require_ack">}`. `deadline_at` is `None` exactly when the invitation is lazy.
  - Root's "content, required flag" maps to `confirmation` and `duration_ms`. An invitation has no free text today: the context an invitee sees is derived from the thread on its own node (`invitation_content`). `require_ack` is the required flag. Service requirement invitations (`requirement_episodes`) are node-local (§2).
  - **`materialize_invitation`:**
    1. No-op if the ID exists.
    2. `episode = service_controls::next_invitation_episode(thread, invitee)`, a local position.
    3. Insert `invitations(…, state, created_at, created_decision_seq = decision_seq, frozen_duration_ms = duration_ms, deadline_at = deadline_at.unwrap_or(created_at + duration_ms), confirmation, inviter_seat_id = inviter, origin_node)`.
    4. If the invitee is `joined` (membership row `state = 'joined'`), the row is inserted with `state = 'already_joined'` and nothing else happens: no membership change, no DeliveryTail, no deadline.
    5. Otherwise the membership row becomes or is inserted as `invited` (today's UPDATE/INSERT). The derived `invite` info event (key `invite:<invitation>`) is appended, and the membership revision and directory/inbox filters are bumped. If the invitee is local and `!suppress(answer_key(invitation))`, `DeliveryTail::invitation(tx, invitee, timed)` runs (§10).
  - `invitations.state` CHECK gains `'already_joined'`. The digest and presentation triggers already treat any non-`pending` state as settled. `inviter_seat_id` becomes `NOT NULL` for journaled threads (the payload always names the inviter, including the former policy-less direct path); service invitations on local-only threads keep `NULL`.
- **`invitation.answered` v1** `{invitation, outcome: "accepted"|"rejected", reason: Option<String ≤ 256 B>, answered_at, n}`. `n` is the invitee's membership transition ordinal for an accept (§7), and `0` for a reject.
  - **`materialize_answer`:** a no-op unless the invitation is `pending`. Otherwise:
    - **Late overdue.** If `answered_at > deadline_at` and the invitation is timed, call `record_overdue_if_pending` (it acts only on the inviter's node, §13).
    - **Accepted:** `UPDATE invitations SET state='accepted', accepted_at = answered_at, accepted_actor_seat_id = invitee, accepted_generation/accepted_observation` from `attribution::take(answer:<id>)`. Then the membership transition of §7 to `joined` with `n`, and the derived `accept` event. Then `settle_superseded_invitations(thread, invitee)`: every other pending invitation of that seat in that thread becomes `already_joined`.
    - **Rejected:** today's `invitation_rejections` insert, the `voluntary_state` update, and the derived rejection warn event (`append_attributed_event_once`, key `reject:<invitation>`).
    - Both clear the invitation's warning condition and bump the filters.
  - `creates = i…/answer`, so a node that ingested its own answer suppresses delivery of that invitation on catch-up.
- **`settle_superseded_invitations`** also runs at the end of every `membership.joined` apply. Whichever of "invitee joined" and "invitation created" applies first, the invitation ends `already_joined`. This is the deterministic convergence of crossed invites on every node, including relays. An `already_joined` settlement is not an answer: it wakes nobody, records no overdue, and its `deadline_at` stays as stored.
- **Episodes** are local positions. The payload never carries one, and `invitations UNIQUE(thread_id, seat_id, episode)` holds per node.

**Considered.**
- Authoring an explicit settlement fact for `already_joined`: either node could author it, so it would need a tie-break, and a relay could hold the invitation pending forever if that fact were parked. The derived rule needs no fact.
- Rejecting the second invitation on apply: root §6 says receivers accept what the author allowed.

### 7. Membership: `membership.joined`, `membership.left`

**Recommended.**
- Payload `{thread, seat, n, at}`, where `n ≥ 1` is the seat's home-node transition ordinal for that thread.
  - New column `memberships.home_transition INTEGER NOT NULL DEFAULT 0`. The home node computes `n = home_transition + 1` at authoring, inside the transaction.
  - Membership transitions of a seat are authored only by its home node and totally ordered in its log (root §6), so a re-issued or duplicate fact has `n ≤ home_transition` and is a no-op.
  - Accept (§6) uses the same counter.
- **`materialize_membership(joined)`** with `n > home_transition`:
  - set `state = 'joined'`, `voluntary_state = 'joined'`, `joined_at = at`, `left_at = NULL`, `home_transition = n`, and a new local `episode` (`next_invitation_episode`);
  - insert a `membership_intervals` row with `joined_seq = decision_seq`;
  - `read_cursors::init_for_join` when the seat is local;
  - the derived `join` info event (payload `{"action":"join","seat","generation","observation"}` from `attribution::take`, key `join:<thread>:<seat>:<n>`);
  - `settle_superseded_invitations`;
  - the membership revision and directory/member/inbox filter bumps.
- **`left`** with `n > home_transition`: `state = 'left'`, close the open interval with `left_seq = decision_seq`, the derived `leave` event, and the same bumps.
- A joined fact for a seat whose membership row is `retired` is a no-op that still records `home_transition`. Retirement is terminal.
- Command checks stay authoring-side A2: `join` refuses with a pending invitation, `leave` requires `joined`, and pair guards apply.

**Considered.**
- Using the envelope counter as the idempotence key: a re-issue under a new incarnation changes the counter.
- Comparing `(hlc, node)`: a home node's HLC is not monotone in counter (ht-5fr.3 §3).

### 8. Messages: `message.sent`

**Recommended.**
- **Payload v1:**

  ```text
  { message, thread, author, body,
    mode: "lazy"|"ordinary", nudge: bool,
    relays_user: bool, user_intent: Option<..>,
    sent_at, duration_ms,
    recipients: [ { seat, receipt: "none"|"ack", deadline_at: Option<i64> } ] }
  ```

  - `require_ack` is implied by any `receipt: ack`.
  - `recipients` is the complete audience the author fixed (root §6). It is sorted by seat ID and excludes the author.
- **Author** (`publish_send`, inside the publish closure after the existing revision checks):
  1. Read the sealed preparation's `prepared_recipients` (ack recipients, `frozen_duration_ms`, `eligible_at_snapshot`) and `lazy_recipients`.
  2. Build `recipients`:
     - `ack` for prepared rows, with `deadline_at = sent_at + duration` when `eligible_at_snapshot = 1`, else `None`;
     - `none` for lazy recipients and for joined human members.
  3. `ensure_published` the author and every local recipient, stage `send:<message>` attribution, then author.

  The prep phase (`stage_recipient`) gets one change for remote recipients: `expects_ack = remote_kind != 'human'` (instead of `open_binding_role`), `availability_provenance = 'remote'`, `eligible_at_snapshot = 1`, and no unavailable-warning evaluation, because the sender has no availability evidence for another node's seat.
- **`materialize_message`:**
  1. **No-op** when `messages.id` exists.
  2. `prep = send_preparation_id_for_message(message)` (the inverse of `derived_message_id`, so `m<tag><8>` ↔ `p<tag><8>`).
  3. **Local branch.** When `local_origin` and `send_preparations(prep)` is `sealed` with no manifest, it adopts the staged rows. The staged audience must equal `recipients`: the same seat set, the same ack flags, and frozen durations matching `deadline_at − sent_at` for eligible rows. Otherwise it is `Rejected(StoreCorrupt "staged audience differs from authored audience")`, which aborts the command. Root walk-through: "reuses a local preparation's staged rows, keyed by message ID".
  4. **Synthesized branch** (remote origin, or own old incarnation without a preparation):
     - insert `send_preparations(id = prep, operation_scope = 'replicated:<origin uuid>', operation_key = message, digest = SHA-256(payload bytes), status = 'sealed', delivery_mode = mode, origin = 'replicated', captured_* = current values, interval_high_water = current max interval ordinal)`;
     - one `prepared_recipients` row per `ack` recipient, local or remote (`frozen_duration_ms = max(1, deadline_at − sent_at)` and `eligible_at_snapshot = 1` when `deadline_at` is `Some`; else `duration_ms` and `0`; `availability_provenance = 'replicated'`), in payload order as `receipt_ordinal`;
     - one `lazy_recipients` row per local recipient of a lazy message;
     - no `prepared_unavailable_warnings`: unavailable warnings are the sender's derived events.

     `send_preparations` gets `origin TEXT NOT NULL DEFAULT 'local' CHECK(origin IN ('local','replicated'))`. The cleanup and abandon paths ignore `replicated` rows, which are born sealed and published.
  5. **Publication** is `insert_publication`, refactored to take `MaterializeInput` and a `PublicationAuthor`:
     - `messages(decision_at = sent_at, decision_seq, sequence = threads.next_sequence, origin_node, native_observation`/`sender_binding_id` from `attribution::take`);
     - `send_manifests`;
     - `next_sequence`/`timeline_revision`/filters;
     - then `DeliveryTail::message(...)` (§10).

     A `PublicationAuthor::Native` built from the payload carries `observation` and `binding` from the attribution (`binding = None` for remote). `messages.sender_binding_id` is nullable already.
  6. **Retired recipients.** For each `ack` recipient whose seat is `retired` at apply, `project_receipt` writes `recipient_retired` (`retired_at` = the seat's `retired_at`) in the job instead of `pending` (§4).
- **Remote recipients** get exactly the `prepared_recipients` row (and, through the job, a `receipt_state` row). No lazy row, no attention, no wake (root §6 "remote recipients only get receipt rows").
- **Programmatic (service) sends** target managed, local-only threads and are never journaled (§2).

**Considered.**
- Recomputing the audience on receivers: rejected by the root (research 02 R5).
- Writing `receipt_state` directly in the handler instead of the job: it doubles the projection logic that `effective_receipt` and the job own, and diverges from local sends.
- A new `replicated_messages` table: every reader (inbox, catch-up, summaries) would need a second source.

### 9. Receipts: `receipt.settled`

**Recommended.**
- **Payload v1** `{message, seat, how: "acked"|"displayed"|"mod_delivered"|"policy"|"retired", settled_at, batch: {id, last_in_thread: bool}}`.
  - `batch.id` is the authoring command's `OperationId` (≤ 64 bytes).
  - `last_in_thread` is true on the batch's last settled receipt of each thread. A batch's facts are consecutive entries of one log, so they apply in order on every node.
  - `retired` is decoded but never authored in v1: retirement settles on every node from `seat.retired` (§4). A `retired` fact is materialized as `recipient_retired`.
- **Author.** `settle_pending` keeps its validation and its decision of `newly`. It then, per message in request order:
  1. stages `ack:<m>/<s>` attribution (today's observation, with `action_provenance: cooperative_inbox_display` for displayed ACKs);
  2. authors `receipt.settled` with `how` from the entry point (`ack` → `acked`, `ack_displayed` → `displayed`, `ack_mod_delivered` → `mod_delivered`, `delivery_confirm`/`ack_policy_delivered` → `policy`).

  The batch-level writes move into the handler: the info event, `delivery_observations` and the filter bumps.
- **`materialize_settlement`:**
  1. A no-op unless the effective receipt is `Pending` (or `NotRequired` for a human `acked`).
  2. **Late overdue:** if `settled_at > effective_deadline`, call `record_overdue_if_pending`, which is effective only on the sender's node (§13).
  3. Upsert `receipt_state` to `acked`, using today's statement with `acked_at = settled_at`, `ack_actor_seat_id = seat`, generation, observation and binding from `attribution::take`, and the new column `settle_batch = batch.id`.
  4. `clear_warning_conditions_for_receipts` (thread, seat).
  5. When the seat is local: the `delivery_observations` upsert and the inbox filter bump.
  6. When `last_in_thread`: append the derived `ack` info event for `(seat, thread)`. Its `messages` list is `SELECT message_id FROM receipt_state WHERE seat_id=? AND settle_batch=? AND message_id IN (SELECT id FROM messages WHERE thread_id=?)`, with today's 4096-byte fallback to `count` + `message_ids_sha256`. The key is today's `"<event_prefix>:<len>:<seat>:<len>:<batch>:<len>:<thread>"`, so a local ACK writes the same event key it writes today.
- `receipt_state` gains `settle_batch TEXT CHECK(settle_batch IS NULL OR length(settle_batch) <= 64)`.
- The sender's node "clears pending overdue tracking" (root §6) through step 3: the receipt leaves `pending`, the sparse-due trigger deletes its row, and step 4 closes the warning condition.

**Considered.**
- One fact carrying the whole batch: root and ht-5fr.3 fix `creates = "<message>/<seat>"` per receipt for catch-up suppression.
- One info event per receipt: up to 100 timeline rows per ACK, against the token-efficiency rules.
- Appending the event from the command only: remote timelines would then lack ACK history, which root §6 says each node derives.

### 10. DeliveryTail

**Recommended.** `src/store/delivery_tail.rs`, the only writer of the local delivery rows for replicated and local facts alike:

```rust
pub(crate) struct DeliveryTail;
impl DeliveryTail {
    /// After the message, manifest and staged rows exist.
    pub fn message(tx, m: &MaterializeInput, message: &MessageId, prep: &str,
                   mode: DeliveryMode, ack_recipients: i64, warnings: i64) -> Result<(), ApiError>;
    /// Invitation to a local invitee that is not suppressed.
    pub fn invitation(tx, invitee: &SeatId, timed: bool) -> Result<(), ApiError>;
    /// The one attention writer: `wake_work` upsert, a no-op for remote seats.
    pub fn attention(tx, seat: &str, reason_bits: i64) -> Result<(), ApiError>;
}
```

- **`message`** writes the `send_attention` work job (`work:send:<message>`, `high_water = ack_recipients + warnings + 1`) for every ordinary message with at least one ack recipient or warning, **on every node**. The job projects `receipt_state` for all ack recipients (a replicated table) and calls `attention` per recipient, and `attention` skips remote seats. For lazy messages it runs `lazy_delivery::note_published`, whose `wake_on='all'` statement gains `AND s.is_local=1` (and, through the synthesized branch, holds only local recipients).
- **Suppression.** Before staging delivery for a local recipient, `materialize_message` asks `m.suppress(receipt_settle_key(message, seat))`. A suppressed recipient keeps its `prepared_recipients` row (receipt state is replicated) but gets no lazy row. The job's projection for it calls `attention` with a skip flag, so no `wake_work` bump happens. That flag is the new `send_manifests.suppressed_seats TEXT` (a `'\n'`-joined sorted list, NULL when empty), read by `send_unit`. Settled history therefore never wakes anyone (root §7).
- **`invitation`** writes today's `wake_work` reason-bit-1 upsert when `timed`, through `attention`. The inbox filter bump stays with the materializer.
- `materialization::bump` (send, warning attribution, service notifications) becomes `DeliveryTail::attention`. `control.rs`'s two inline `wake_work` upserts (`invite`, `resolve_direct_pair`) move into the invitation materializer's DeliveryTail call.
- **Coalescing.** `wake_work` is one row per seat with an `attention_version` counter, so N facts for one seat during bulk catch-up produce one row and one dispatch (root §7). The Wakes lane is kicked by the table as today. ht-5fr.2's per-host wake workers select by `seats.host_id`, and remote seats have none.
- **Invariant**, enforced by test: no `wake_work`, `lazy_recipients`, `warning_offer`, `in_turn_offer` or `catch_up` row ever names a seat with `is_local = 0`.

**Considered.**
- Skipping the `send_attention` job on receivers and writing `receipt_state` inline: two projection paths (§8).
- Filtering remote seats in each caller: the root's DeliveryTail digest says handlers never write `wake_work` directly. One writer makes that checkable.

### 11. Derived info and warn events

**Recommended.** Each materializer appends its derived events with the existing writers, `append_event_once_with_decision_seq` and `append_attributed_event_once_with_decision_seq`, passing `MaterializeInput.decision_seq`. Keys are built from fact identity, so every apply of the same fact on one node maps to one key:

| Event | Key | Written by |
|---|---|---|
| create | `create:<thread>` | `thread.created`, `thread.direct` |
| join / leave | `join:<thread>:<seat>:<n>` / `leave:<thread>:<seat>:<n>` | membership |
| invite | `invite:<invitation>` | `invitation.created` (not for `already_joined`) |
| accept / reject | `accept:<invitation>` / `reject:<invitation>` | `invitation.answered` |
| topic / name / archive / reopen | `register:<thread>:<field>:<uuid8>:<counter>` | `thread.register` on `Won` and changed |
| ack | §9 key | `receipt.settled` `last_in_thread` |
| retire_seat audit | existing `retirement_audit_key` | retirement job (local and replicated) |

- Overdue, unavailable and transition warnings are decisions of the deciding node (§13). They are not apply effects and stay where they are today.
- Writers that allocate a position internally (`record_overdue_if_pending`, `invitation_content::record_context_change`) run inside the apply transaction on the same node counter, which keeps apply order. Handlers never call `next_decision_seq` themselves (ht-5fr.3 §7).
- The re-issue key `<uuid8>:<counter>` is the register's origin entry. A re-issued register fact (ht-5fr.6) loses LWW (equal `(hlc, node)`) and appends nothing.

**Considered.** Operation-ID keys as today: receivers have no operation ID.

### 12. Catch-up suppression usage

**Recommended.**
- Handlers never decide suppression. They only ask `MaterializeInput.suppress` (from `ApplyContext::delivery_suppressed`) with:
  - `receipt_settle_key(m, s)` per local recipient in `message.sent`;
  - `answer_key(i)` in `invitation.created` for a local invitee.
- Suppressed means:
  - no lazy row, no attention, no `wake_work`, and no invitation `wake_work`;
  - the replicated rows (message, `prepared_recipients`, invitation, membership `invited`) are still written;
  - the later settlement fact, which is in the journal by definition, then settles them through the normal handler.
- Own-old-incarnation entries (ht-5fr.3 §8 (a)) suppress every local recipient, because `delivery_suppressed` is true for any key there. Restoring a node therefore never wakes its seats for history.

**Considered.** Suppressing by age (`sent_at` older than N): it would drop real messages delivered after a long partition.

### 13. Deadline scope: only the deciding node warns

**Recommended.**
- New columns:
  - `messages.origin_node TEXT CHECK(origin_node IS NULL OR length(origin_node)=32)`: set for ordinary messages, NULL for derived events;
  - `invitations.origin_node TEXT NOT NULL`;
  - an index `messages_origin_ordinary ON messages(origin_node) WHERE kind='ordinary'`.

  Both are set from `MaterializeInput.origin_node`. Local-only writers set self.
- **One guard.** `schema::record_overdue_if_pending` (`src/store/schema.rs`) first resolves the obligation's deciding node:
  - `Receipt{message}` → `messages.origin_node`;
  - `Invitation(i)` → `invitations.origin_node`.

  When that node is not `local_node.id`, it returns `OverdueOutcome { inserted: false }` and writes nothing. Every overdue path (due scan, extension lapses, ACK/answer late warnings, the retirement job) goes through it, so "recipient nodes never emit overdue for remote senders" (root §7) holds by construction.
- **Scans are scoped too, for cost:**
  - the `receipt_due_sparse_insert`/`_update` triggers add `AND (SELECT origin_node FROM messages WHERE id=NEW.message_id) = (SELECT id FROM local_node)` to their `WHEN`, so the sparse due table holds only authored receipts and `scan_due` walks nothing else;
  - `invitation_due::select_invitation_due_candidates` adds `AND i.origin_node = (SELECT id FROM local_node)`, using the new partial index `invitations_due_origin ON invitations(origin_node, deadline_at, ordinal) WHERE state='pending'`;
  - `receipts::scan_extension_lapses` adds the same `messages.origin_node` predicate.
- **Warning recipients are local.** `materialization::attribute`/`warning_unit` and the service-notification unit deliver only to local seats, through `DeliveryTail::attention` and `AND s.is_local=1` in their recipient selection. The deciding node's overdue warn event is visible in its own timeline. Remote nodes never derive it. This is accepted: the warning is the sender's.
- **Catch-up extensions** (`catch_up.extension_until`) exist only for local bindings. A remote recipient's receipt is judged on its frozen `deadline_at`. Accepted limit, recorded in the module doc: a remote recipient that is mid catch-up can be reported overdue earlier than a local one would be.
- The sleep/wall-jump hold is ht-5fr.13's. It consumes this scope change and adds nothing here.

**Considered.**
- Deriving "authored here" from `actor_seat_id`/`inviter_seat_id` locality: it fails for NULL inviters and service authors, and needs a join in every scan.
- Filtering only in the scans: the ACK, answer and retirement late-warning paths would still warn on recipient nodes.

### 14. Remote seats in node-local workers

**Recommended.**
- ht-5fr.1's `seats::LOCAL_SEAT` (`"is_local=1"`) and `is_local_seat` are the only locality test. This subepic owns their use: every seat-selecting query in the paths below includes `LOCAL_SEAT` (or `s.host_id = ?`, which excludes remote seats because they have no host):
  - **Observation and reconciliation** (`seats.rs` snapshot apply, absence closure, allocation, retirement-by-observation, recovery baseline): ht-5fr.2 adds `s.host_id=?` and `LOCAL_SEAT`. This subepic adds the remote-seat regression tests.
  - **Liveness** (`service/liveness.rs`, `store/bindings.rs` sampling): it selects through `occupant_bindings`, which remote seats never have. The `is_local=1` guard goes on the seat join.
  - **Wake** (`store/wake.rs` `wake_candidates`, `wake_batch_seats`, poke due scans): `AND s.is_local=1` on the seat join, in addition to ht-5fr.2's host filter.
  - **Archival** (§15).
  - **Retirement-by-observation:** only local seats can be closed by a snapshot. Only `seat.retired` retires a remote seat (§4).
- Remote seats have no bindings. `effective_registered_availability`, `open_binding_role` and `carry_pending` return "none" for them, and their callers in `stage_recipient` take the remote branch (§8) before calling those functions.
- **Reads** (`src/store/queries.rs`):
  - Participants, thread show, directory and member listings include remote seats. Their pane columns are NULL, and their state renders as `remote` (one short word, in the existing state column, so there is no new line per seat).
  - Listings that exist to show panes (`panes`, the picker directory, `seats --panes`) filter `LOCAL_SEAT`.
  - The per-node `unreachable` marker is ht-5fr.13's.
  - Model-visible change: the state value `remote` (6 bytes) replaces a pane label on remote rows. Nothing else changes.

**Considered.** A separate `seat_directory` table for remote seats (research 02 R4): ht-5fr.1 already chose remote rows in `seats` with trigger guards.

### 15. Automatic archival with remote members

**Recommended.**
- In `store/archival.rs` `scan_members`, one condition is added before the quiet-period evaluation (the place ht-5fr.2 §8 names):
  - a thread is eligible for automatic archival only if `NOT EXISTS (SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id WHERE m.thread_id=?1 AND m.state IN ('joined','invited') AND s.is_local=0 AND s.state!='retired')`;
  - a thread that fails it is deferred with the existing "not eligible" outcome and no quiet-period bookkeeping, so it costs one indexed probe per scan.
- Threads with `journaled = 0` are unaffected (they have no remote members).
- **The decision is replicated.** Where the archival lane sets `threads.archived = 1` today, it authors `thread.register{archived, "1"}` through `author_above` inside its background transaction. The handler copies the value and appends the archive event. Other nodes therefore see the thread archived too. Explicit `archive`/`reopen` author the same kind. The archival lane commits kick nothing new: `journal` kicks `Lane::Apply`, whose pass finds nothing ready.

**Considered.** A local-only automatic archive: the thread would be archived on one node and open on the others, with no LWW record to converge on.

### 16. Send fence narrowing (research 02 R2)

**Recommended.**
- `publish_send`'s two revision comparisons and `prepare_send_step_impl`'s restart check compare `captured_timeline_revision` only when the preparation's `warning_count > 0`.
- Rationale: the timeline fence protects the warning sequence span that `insert_publication` reserves (`base + warning_count + 1`), and the unavailable-warning conditions, which `ensure_unavailability_reopen_ready` re-checks at publish. The ordinary sequence base is read from `threads.next_sequence` at publish, so it is never stale.
- The other four revisions (membership, lifecycle, eligibility, config) still fence the audience.
- The task first lists every reader of `captured_timeline_revision` (`messages.rs`, `service_send.rs`, `summary.rs` fixtures) and confirms that no other guarded value depends on it. `service_send.rs` keeps the full fence: managed threads see no remote traffic.

**Considered.**
- Not bumping `timeline_revision` for replicated messages: readers use it as a cache token, so their caches would go stale.
- A separate remote-only revision: it changes every reader for the same effect as narrowing the one fence.

### 17. Error classification per kind

**Recommended.** Handlers return `FactError::Rejected` (recorded `failed`, counted as processed, and `sync unapplied replay`-able) for:
- payload decode errors;
- a `thread.direct` ID mismatch;
- an unknown recipient seat (the needs overflow case);
- the local-branch audience mismatch, which only occurs at authoring and so aborts the command;
- `own seat restore needs an attached host`;
- constraint violations (`Conflict` from `store_error`).

Store errors pass through as `FactError::Store` via `from_api`. No handler rejects because of the local membership, archive or eligibility view (root §6).

Commands surface authoring errors unchanged. A `parked_dep` at authoring (for example, inviting a remote seat whose `seat.created` was parked on this node) returns ht-5fr.3's `Conflict` "fact <kind> not applicable: <reason>".

## Testing

All store tests use ht-5fr.3's seeded identities. Two `SqliteStore`s in one test act as nodes A and B, linked by `JournalPort::serve` → `ingest` → `apply_pending` (`test_support::fact_exchange::sync(a, b)`, a new helper in `src/test_support/fact_exchange.rs`). No daemon is spawned.

- **Per-kind unit tests** (`tests/sync/facts_*.rs`, modules of `tests/combined.rs`):
  - payload JSON round trips;
  - additive unknown fields are ignored;
  - `creates`/`needs` match the §3 table, including the 256-key cutoff for `message.sent`;
  - `thread.direct` with a wrong ID is `Rejected`.
- **Idempotence:** apply every kind twice on one store, by direct `apply_one` replay of the same entry and by a re-authored copy under a second incarnation via `begin_incarnation`. Every replicated projection is unchanged, and no second event, interval or attention write happens.
- **Convergence** (`tests/store/fact_convergence.rs`):
  - **send/ACK:** A sends to B's seat; after sync, B has the message, a `pending` receipt and a `wake_work` row for its local seat; A has the receipt row and no `wake_work` for the remote seat. B ACKs; after sync, both show `acked` with the same `acked_at`, A's sparse-due row is gone, and A's timeline has the `ack` event.
  - **ACK batch of 3 in 2 threads:** exactly 2 `ack` events on both nodes, with the same keys and message lists.
  - **Crossed direct pair:** A and B each run `ensure_direct_thread` toward the other while unsynced. After sync, there is one `d…` thread with the same `created_at` (earliest creator) on both, both seats are joined, both invitations are `already_joined`, and neither seat got an invitation `wake_work` from the crossing.
  - **Three nodes (A authors, C relays):** C applies B's `membership.joined` before A's `invitation.created`, and the reverse order on a fourth store. Both end `already_joined`.
  - **LWW:** concurrent topic writes on A and B converge to the higher `(hlc, node)` and append exactly one topic event on the losing node's side when the winner arrives.
  - **Archive:** concurrent archive and reopen converge. A message into an archived thread still applies on the receiver.
  - **Retirement:** B retires its published seat while A sends to it concurrently. Both arrival orders end with that receipt `recipient_retired` on both nodes, and only A records overdue.
  - **Membership re-issue:** join, leave, then a re-issued join with `n=1` is a no-op.
- **Catch-up suppression:**
  - B authors `receipt.settled`, B's store is wiped (fresh store with the same seed), B re-ingests its old log and A's log. The message applies with no `lazy_recipients`/`wake_work` for the settled seat, and the settlement then applies.
  - An unsettled message in the same pull-back also gets no wake (own old incarnation).
- **Deadline scope:**
  - with an injected clock past `deadline_at`, `scan_due` on B (recipient node) inserts nothing, and on A (sender) inserts one overdue for the remote recipient;
  - the same for invitations through `scan_invitation_due_batch`;
  - an ACK after the deadline records the late overdue only on A;
  - `record_overdue_if_pending` on a remote-origin obligation returns `inserted: false`;
  - the retirement job on B warns nothing for A's messages.
- **Remote seats ignored:** a store with remote seat rows that runs a snapshot reconciliation, a wake pass (`wake_candidates`), a liveness sample and an archival scan changes no remote row and selects none. The DeliveryTail invariant query (§10) returns zero rows after every convergence test (shared assertion `assert_no_local_delivery_for_remote(store)`).
- **Archival:** a thread with a joined remote member is never archived after the quiet period. The same thread archives once the remote member's `seat.retired` applies. An automatic archive replicates as a register.
- **Publication:** a seat allocated by observation authors nothing. Its first send publishes it once, with `kind` from its binding. A human-bound seat publishes `human`, and a remote sender then stages no ACK expectation for it.
- **Local branch:** a forced mismatch between the staged audience and the payload aborts the send with nothing committed.
- **Write-path closure:** a test-support SQLite authorizer hook (`test_support::write_guard`) is armed around every converted command in the existing store suites. A write to any replicated table outside an apply savepoint or a `materialize::local_only` scope (managed threads, §2) fails the test. Replicated tables: `seats` (insert of remote rows, `state`/`retired_*`), `threads`, `direct_threads`, `invitations`, `invitation_rejections`, `memberships`, `membership_intervals`, `messages` (ordinary), `send_manifests`, `prepared_recipients`, `receipt_state`, `thread_registers`. The existing store suites (`tests/store/messages.rs`, `author_role.rs`, control, receipts, archival, invitation suites) pass unchanged except for expected ID shapes and the documented event keys.
- **Fence:** 20 remote messages applied into a thread while a local seat sends 10 lazy messages through the prep/publish pair. All 10 publish without a `snapshot changed` conflict. A prep with an unavailable warning still conflicts on a timeline bump.
- **Oracle:** `test_support::journal_oracle::replicated_projection` (ht-5fr.3) gains the converted tables' replicated columns:
  - `threads(id, topic, goal, name, archived, created_at, journaled)`;
  - `direct_threads(thread_id, seat_a, seat_b, created_hlc, created_node)`;
  - `seats(id, home_node, kind = coalesce(remote_kind, published kind), retired = state='retired', retired_at)` (normalizing local versus remote);
  - `memberships(thread, seat, state, home_transition)`;
  - `invitations(id, thread, seat, state, deadline_at, confirmation, inviter, origin_node, accepted_at)`;
  - `invitation_rejections(invitation_id, reason)`;
  - `messages(id, thread, actor_seat_id, body, decision_at, origin_node)` for `kind='ordinary'`;
  - `prepared_recipients(message, seat, frozen_duration_ms, eligible)`;
  - `receipt_state(message, seat, state, deadline_at, acked_at, settle_batch, retired_at)`.

  Attribution columns, every local position and every delivery table are excluded. A scripted multi-command workload (create, pair, invite, accept, reject, send lazy and ordinary, ACK, topic, archive, retire) replayed into an empty store matches in strict mode.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral` (all new files are core and name no harness), `nice scripts/check-default-features`, and focused `nice cargo test --locked --all-features facts fact_convergence messages receipts control archival invitation wake`.

Suggested task cut (files):
1. Schema additions (`threads.journaled`, `seats.published`, `memberships.home_transition`, `invitations.origin_node`/`already_joined`, `messages.origin_node`, `send_preparations.origin`, `send_manifests.suppressed_seats`, `receipt_state.settle_batch`, `direct_threads.created_*`, `retirements.origin`, `fact_attribution_staging`, the trigger and index changes): `migrations/node/0001_node_baseline.sql`, `src/service/kicks.rs` (classification).
2. `MaterializeInput`, attribution staging, payload types and key builders: `src/store/materialize.rs`, `src/store/attribution.rs`, `src/sync/facts/payload.rs`, `src/test_support/fact_exchange.rs`.
3. DeliveryTail extraction (behaviour-preserving, local only): `src/store/delivery_tail.rs`, `src/store/materialization.rs`, `src/store/lazy_delivery.rs`, `src/store/messages.rs`, `src/store/control.rs`.
4. Seats: `ensure_published`, `seat.created`, `seat.retired`, replicated retirement, eligibility: `src/sync/facts/seat.rs`, `src/store/seats.rs`, `src/store/control.rs`.
5. Threads: `thread.created`, `thread.register` (commands and archival authoring), `thread.direct`: `src/sync/facts/thread.rs`, `src/store/control.rs`, `src/store/archival.rs`.
6. Membership kinds: `src/sync/facts/membership.rs`, `src/store/control.rs`.
7. Invitation kinds and `already_joined`: `src/sync/facts/invitation.rs`, `src/store/control.rs`, `src/store/invitation_content.rs`.
8. `message.sent` (author, local and synthesized branches, remote-recipient staging): `src/sync/facts/message.rs`, `src/store/messages.rs`, `src/store/materialization.rs`.
9. `receipt.settled` and the batch event: `src/sync/facts/receipt.rs`, `src/store/receipts.rs`.
10. Deadline scope: `src/store/schema.rs` (`record_overdue_if_pending`), `src/store/receipts.rs`, `src/store/invitation_due.rs`, `src/scheduler/deadlines.rs`, `src/store/materialization.rs`.
11. Remote seats in workers and reads, plus the archival rule: `src/store/wake.rs`, `src/store/bindings.rs`, `src/store/archival.rs`, `src/store/queries.rs`, `src/service/liveness.rs`.
12. Local-only managed threads and the send-fence narrowing: `src/store/service_controls.rs`, `src/store/service_send.rs`, `src/store/messages.rs`.
13. Convergence, oracle projection and write-guard suites: `tests/store/fact_convergence.rs`, `tests/sync/facts_*.rs`, `src/test_support/journal_oracle.rs`, `src/test_support/write_guard.rs`, `tests/combined.rs`.

Order: 1 → 2 → 3 → 4 → (5, 6) → 7 → 8 → 9 → (10, 11, 12) → 13.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
