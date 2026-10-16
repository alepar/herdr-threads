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

Direct pairs auto-join both seats with no invitation: each pair seat's home node authors that seat's `membership.joined` when it creates or applies the pair's `thread.direct`, so crossed creates converge by membership ordinal (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §2). Crossed ordinary invitations settle `already_joined` from either arrival order. Supersession compares the invitee's joined transition ordinal with the `invitee_n` the inviter saw, never the current membership state, so it is order-independent (root §5 invariant; §3 audit).

Handlers reject a fact whose origin is not the kind's authorized author (§17); that is an authority check on replicated identity, not a view check.

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

**Restore-window authoring hold** (root §7). While `SyncState.own_history.active(now)` (ht-5fr.3 §7 step 3a), `FactRoute::Journal` refuses to author `receipt.settled`, `membership.joined`, `membership.left`, `invitation.answered` or `seat.retired` for a local seat with a retryable `ServiceBusy "own history catching up"` (amended 2026-10-15 (main v1 reconcile), a recorded deviation from the original `Unavailable`, which `ErrorCode` does not have; display entry points recognize the refusal by code and detail through `is_own_history_hold`: see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §5), before anything is written, because the node's unpulled old tail may hold a different fact for that key (root §5). Commands that author one of them (`ack`, `accept`, `reject`, `join`, `leave`, `create_thread*`, `ensure_direct_thread`, seat retirement) fail with that error and are retried by their caller. `settle_pending`'s display entry points (`ack_displayed`, `ack_mod_delivered`, `delivery_confirm`, `ack_policy_delivered`) do not fail the delivery: they skip the settlement and leave the receipts pending, so the bounded re-offer shows them again after the hold unless the old tail settled them. The retirement job retries after the hold. Sends, invitations, thread and register facts and `seat.created` are not held. The refusal lifts with the hold, at the latest at its 5-minute cap; a same-key fact authored after the cap while the old tail is unpulled is root §12's restore-window residual.

Per-command fact order:

| Command | Facts, in order |
|---|---|
| `create_thread*` | `thread.created`, `membership.joined{creator}` |
| `ensure_direct_thread` (new pair) | `thread.direct`, `membership.joined{caller}`, then `membership.joined{recipient, actor = caller}` only when the recipient is a local seat; no invitation (a remote recipient's home node authors its join on applying `thread.direct`; amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §2) |
| `invite` / `operator_orphan_invite` / voluntary service invite | `invitation.created` (`inviter_kind` seat / operator / service) |
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
  - seat, thread, invitation and message facts are no-ops when their ID row exists (a same-ID fact is a no-op only because it carries the same content: fresh IDs are random per mint, and a re-issue copies the payload bytes);
  - `thread.direct` is the one same-ID fact two nodes author with different content: it updates only the earliest-creator fields (a min-register, §5);
  - registers are LWW;
  - `invitation.answered` is a no-op unless the invitation is `pending`;
  - membership facts compare `n` (§7);
  - `receipt.settled` is a no-op unless the receipt is `pending`.
- **Order-independence audit** (root §5 invariant: every replicated column ends with the same value in any apply order). Facts of one seat's home node are totally ordered in its log, and a fact's deps order it after everything its author had applied, so only concurrent facts from different logs can arrive in either order. Per kind:

  | Kind | Concurrent facts that touch the same columns | Merge |
  |---|---|---|
  | `node.genesis`, `node.retag` | none (one log each; retag is decided by the higher uuid only) | — |
  | `seat.created` | none outside a fork (authored once, by the home node, when unpublished) | fork residual: kind |
  | `seat.retired` | `message.sent` / `invitation.created` to that seat from other nodes | a recipient or invitee retired at apply is settled `recipient_retired` at apply (§4, §6, §8); the retirement job settles the rest; both orders end `recipient_retired`. Fork residual: `retired_at`, reason |
  | `thread.created` | none (fresh ID); initial registers go through the LWW upsert | LWW |
  | `thread.direct` | the other pair node's create of the same `d…` | min-register on `(hlc, node_uuid)` over `created_hlc`, `created_node`, `threads.created_at` (§5) |
  | `thread.register` | registers from any node | LWW on `(hlc, node_uuid)`; an equal pair with a different value needs one node to author twice at one HLC (fork residual) |
  | `invitation.created` | the invitee's membership transitions (home node) and `seat.retired` | supersession by ordinal: `already_joined` iff the invitee's `last_joined_n > invitee_n` (§6), whichever applies first; `recipient_retired` iff the invitee is retired at apply |
  | `invitation.answered` | none outside a fork (the invitee's home node answers once, after the invitation applied there) | fork residual: outcome |
  | `membership.joined` / `.left` | `invitation.created` (sets `invited` only while no transition above `invitee_n` applied) | ordinal `n` (§7); `last_joined_n` drives supersession. Fork residual: two branches' facts with one `n` |
  | `message.sent` | `seat.retired` of a recipient | `recipient_retired` at apply (§8) |
  | `receipt.settled` | none outside a fork (the recipient's home node settles once, after the message applied there; its later `seat.retired` follows in the same log) | fork residual: `how`, `settled_at`, batch |

  The fork residual is root §12's accepted limit "forked-log same-key facts" (ht-5fr.6 §13 item 2): each node keeps whichever branch's fact it applied first, and the seat's next membership transition re-converges its membership. A wipe or restore creates such a pair only past the own-history hold's cap: while the hold is active, the restore-window authoring hold (§2) refuses the held kinds for local seats, and once the node's own old facts are ingested its commands see the old settlement, answer or transition and author nothing twice. A held kind authored after the cap while the old tail is still unpulled, or a `seat.created` authored during the hold, can still pair with a different fact in that tail; that is root §12's restore-window residual and resolves like the fork residual.
- **Payload size.** The largest is `message.sent`: body ≤ 64 KiB plus ≤ 100 explicit recipients and the thread audience at about 60 bytes per recipient. A 1000-member audience is about 125 KiB, under the 512 KiB cap. `MessageSent::validate_for_author` refuses a payload over 448 KiB with `InvalidRequest` "audience too large to replicate" before authoring.

**Considered.**
- Replicating the observation JSON: it leaks session, execution and boot IDs to other nodes for no use there, and it is about 250 bytes per fact.
- An `ApplyContext` sidecar for local extras: it would change ht-5fr.3's surface. A staged row in the same transaction is the pattern the root already chose for preparations.
- Storing the HLC ms as `decision_at` and deriving the deadline from it: the author cannot know its HLC before `journal::author` runs, so `deadline_at` and `sent_at` could disagree after a clock step.

### 4. Seats: publication, `seat.created`, `seat.retired`, remote rows

**Recommended.**
- **Publish at enrollment (amended, coverage R17).** New column `seats.published INTEGER NOT NULL DEFAULT 0 CHECK(published IN (0,1))`, with the CHECK `is_local=1 OR published=1`.
  - A local seat is published when it is resolved to an occupant: the transaction that enrolls its first open binding (`bindings::enroll_with_hooks`, the one insert path for occupant bindings) calls `seats::ensure_published(tx, seat)` after the binding row is inserted, so the kind is known. Other nodes therefore list the seat after sync with no prior interaction, and can invite it or open a direct pair with it. Publishing on first reference alone was rejected: a seat nobody had yet addressed stayed invisible on other nodes, so it could never be addressed from them (coverage R17).
  - `seats::ensure_published(tx, seat)` is a no-op for remote or already published seats. Otherwise it reads `schema::open_binding_role(tx, seat)` (`Some("human")` → `human`, anything else → `agent`), authors `seat.created`, and the handler sets `published = 1`. `FactRoute::Journal` still calls it for every referenced local seat (§2) as an idempotent backstop. It is never called on the local-only route.
  - Allocation by observation (~L2614) and the operator fresh seat insert (~L3411) stay local inserts and author nothing. A pane that never enrolls an occupant costs no journal entries, and an enrolled seat costs one `seat.created`, once.
  - Remote seats appear in the directory and seat-listing reads (participants, thread show, directory, member listings, `seats` without `--panes`), with state `remote` (§14).
- Accepted limit: a seat's published kind is fixed. If its occupant later switches between human and agent, other nodes keep the published kind. Only the ACK expectation of remote recipients depends on it.
- **`seat.created` v1** payload `{seat, kind: "agent"|"human", display_name: null}`. `display_name` is reserved and always `null` in v1, because seats carry no name today. The created time is the envelope HLC, so the payload does not repeat it.

  `materialize_seat_created`:
  - row exists and is local: set `published = 1` (the authoring node);
  - row exists and is remote: no-op;
  - row absent and `home_node ≠ self`: insert `seats(id, instance_id = self node, state = 'remote', role = 'native', home_node = origin, is_local = 0, remote_kind = kind, published = 1, generation = 0, created_at = origin_ms, host_id/target_id/structural columns NULL)`;
  - row absent and home is self (own-old-incarnation pull-back after a wipe): insert a local `unresolved` seat (`unresolved_reason = 'other'`, `published = 1`) on the `herdr_hosts` row with the greatest `last_attached_at`. With no `herdr_hosts` row the handler returns `FactError::Wait { on: Host }`: the entry is `parked_dep` on `wait:host` (not `failed`), and it and the entries parked behind it apply at daemon start when a host row exists and when the daemon's first host attaches (the attach kicks the Apply lane); no operator replay is needed. The seat cannot resume until C1 or an operator rebinds it, as with any unresolved seat.
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
- Publishing only on first reference (the original recommendation): a never-referenced seat on B is invisible on A, so A cannot invite it or open a direct pair with it (coverage R17).
- Authoring a `seat.kind` change fact: outside the root's kind list. The kind affects only ACK expectation.
- Retiring an unrestorable own seat locally: a local-only retirement would diverge from every other node, and retiring on heuristic evidence breaks TRUST-POLICY.

### 5. Threads: `thread.created`, `thread.register`, `thread.direct`

**Recommended.**
- **`thread.created` v1** `{thread, topic, goal, name: Option<String>, creator}`.
  - `materialize_thread_created` inserts `threads(id, instance_id = self, topic, goal, name, created_at = origin_ms, updated_at, journaled = 1)` if absent.
  - It then calls `thread_registers::upsert` for `topic` and, when present, `name`, with the entry's `(hlc, origin.node, ingest_seq)`, and copies the winners. A register can never be ahead of its thread's creation because `thread.register` needs the thread.
  - It appends the derived `create` info event (`append_event_once_with_decision_seq`, key `create:<thread>`) and bumps the directory filter.
  - The creator's membership is the next fact (§2).
- **`thread.register` v1** `{thread, field: "topic"|"name"|"archived", value: Option<String>, actor: Option<SeatId>}`. `archived` values are `"1"`/`"0"`, and a `null` name clears it. `actor` is the acting seat. A node names only a local seat as actor (otherwise the write is refused); on apply of a replicated entry the actor is kept as the derived event's `actor_seat` only when its seat's home node is the origin (`AuthorRule::SeatHome`'s test), otherwise the change is materialized with no actor and the fact is not refused, so convergence is unchanged. It is absent for automatic archival, and omitted from the bytes when absent. The field is additive (unknown fields are ignored), so v1 is unchanged.
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
- **Pair auto-join** (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §2). The `ThreadDirect` handler queues, through `ApplyContext::follow_up`, a `membership.joined{seat, n = 1, actor = creator}` for each pair seat that is local, not retired and has no `memberships` row in the thread; the creating command does the same inline through the one helper `direct::auto_join_local_pair_seats`. No invitation is authored, and joins write no attention. Until the remote peer's join arrives, the creator's node stages that peer in every send's audience (it has no membership row, so its auto-join is pending).
- **Crossed creates.** A and B open the same pair while partitioned. Each node authors `thread.direct` and joins its own seat, and neither invites the other. After exchange each node applies the other's `thread.direct` as a min-register update, its own seat already has a row so it authors nothing more, and the other seat's join arrives in the other's log. The pair ends with one thread, two members joined at `n = 1` and no invitation on every node, and the pairing wakes nobody.

**Considered.**
- Writing the direct default topic as a register at the earliest HLC: root §6 forbids it, and the label is a pure function of the pair.
- Letting `thread.created` also join the creator: it would hide a membership transition inside a thread fact and need the transition ordinal in two kinds.

### 6. Invitations: `invitation.created`, `invitation.answered`, `already_joined`

**Recommended.**
- **`invitation.created` v1** `{invitation, thread, inviter: Option<SeatId>, inviter_kind: "seat"|"service"|"operator", invitee, invitee_n, created_at, duration_ms, deadline_at: i64, actor_label: Option<String>}` (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §1.2, §4). Every invitation is priority 0, wakes the invitee and carries a deadline (main D6): `deadline_at` is required and equals `created_at + duration_ms`; there is no `confirmation` field and no lazy invitation. `inviter` is required for `inviter_kind = seat` and optional (absent in v1) for `service` and `operator`. `actor_label` names the service author or the operator audit label for the derived event. `invitee_n` is the invitee's `memberships.home_transition` for that thread in the inviter's view at authoring (0 without a row); A2 already refuses to invite a seat that view shows joined.
  - Root's "content, required flag" no longer maps to a payload field: every invitation is confirming. An invitation has no free text today: the context an invitee sees is derived from the thread on its own node (`invitation_content`). Service requirement invitations (`requirement_episodes`) are node-local (§2).
  - **`materialize_invitation`:**
    1. No-op if the ID exists.
    2. `episode = service_controls::next_invitation_episode(thread, invitee)`, a local position.
    3. Insert `invitations(…, state, created_at, created_decision_seq = decision_seq, frozen_duration_ms = duration_ms, deadline_at, confirmation = ('require_ack' for a seat inviter, else NULL: main's stored shape), inviter_seat_id = (inviter for a seat inviter, else NULL), inviter_kind, invitee_n, origin_node)`.
    4. If the invitee's seat is `retired`, the row is inserted with `state = 'recipient_retired'` (`retired_at` = the seat's) and nothing else happens. If the invitee's membership row has `last_joined_n > invitee_n` (a joined transition the inviter had not seen, whether still joined or since left), the row is inserted with `state = 'already_joined'` and nothing else happens: no membership change, no DeliveryTail, no deadline. The current membership `state` is never consulted, because it depends on how many of the invitee's concurrent transitions applied first.
    5. Otherwise the membership row becomes or is inserted as `invited` (today's UPDATE/INSERT). The derived `invite` info event (key `invite:<invitation>`) is appended, and the membership revision and directory/inbox filters are bumped. If the invitee is local and `!suppress(answer_key(invitation))`, `DeliveryTail::invitation(tx, invitee)` runs (§10) and always writes the invitee's invitation `wake_work` (every invitation is p0).
  - `invitations.state` CHECK gains `'already_joined'`. The digest and presentation triggers already treat any non-`pending` state as settled. `inviter_seat_id` is `NOT NULL` exactly when `inviter_kind = 'seat'`, on journaled and local-only threads alike; service and operator invitations keep `NULL` and take their authority from the signing origin node (§17). Amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §4.
- **`invitation.answered` v1** `{invitation, outcome: "accepted"|"rejected", reason: Option<String ≤ 256 B>, answered_at, n}`. `n` is the invitee's membership transition ordinal for an accept (§7), and `0` for a reject.
  - **`materialize_answer`:** a no-op unless the invitation is `pending`. Otherwise:
    - **Late overdue.** If `answered_at > deadline_at` and the invitation is timed, call `record_overdue_if_pending` (it acts only on the inviter's node, §13).
    - **Accepted:** `UPDATE invitations SET state='accepted', accepted_at = answered_at, accepted_actor_seat_id = invitee, accepted_generation/accepted_observation` from `attribution::take(answer:<id>)`. Then the membership transition of §7 to `joined` with `n`, and the derived `accept` event. Then `settle_superseded_invitations(thread, invitee, n)`: every other pending invitation of that seat in that thread becomes `already_joined`.
    - **Rejected:** today's `invitation_rejections` insert, the `voluntary_state` update, and the derived rejection warn event (`append_attributed_event_once`, key `reject:<invitation>`).
    - Both clear the invitation's warning condition and bump the filters.
  - `creates = i…/answer`, so a node that ingested its own answer suppresses delivery of that invitation on catch-up.
- **`settle_superseded_invitations(thread, seat, n)`** settles every `pending` invitation of that seat in that thread with `invitee_n < n` as `already_joined`. It runs at the end of every `membership.joined` apply and of every accept (with the accept's `n`). Whichever of "invitee joined" and "invitation created" applies first, the invitation ends `already_joined`, and a join followed by a leave of the invitee, concurrent with the invitation, supersedes it in every order too. This is the deterministic convergence of crossed invites on every node, including relays. An `already_joined` settlement is not an answer: it wakes nobody, records no overdue, and its `deadline_at` stays as stored.
- **Episodes** are local positions. The payload never carries one, and `invitations UNIQUE(thread_id, seat_id, episode)` holds per node.

**Considered.**
- Authoring an explicit settlement fact for `already_joined`: either node could author it, so it would need a tie-break, and a relay could hold the invitation pending forever if that fact were parked. The derived rule needs no fact.
- Rejecting the second invitation on apply: root §6 says receivers accept what the author allowed.

### 7. Membership: `membership.joined`, `membership.left`

**Recommended.**
- Payload `{thread, seat, n, at, actor: Option<SeatId>}`, where `n ≥ 1` is the seat's home-node transition ordinal for that thread. `actor` (joined only, additive, omitted when absent) is the pair creator of a 1:1 auto-join; it only fills the derived join event's `actor_seat` (kept when it is one of the pair's seats, else dropped; the fact is never refused for it). An auto-joined pair seat's read-cursor floor is the pair's first local position, not the join, and its attribution observation is `direct_auto_join` with no binding. Amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §2.2, §2.3.
  - New column `memberships.home_transition INTEGER NOT NULL DEFAULT 0`. The home node computes `n = home_transition + 1` at authoring, inside the transaction.
  - New column `memberships.last_joined_n INTEGER NOT NULL DEFAULT 0`: the highest `n` of an applied joined transition (a join or an accept). It only grows, so it is order-independent, and invitation supersession reads it (§6).
  - Membership transitions of a seat are authored only by its home node and totally ordered in its log (root §6), so a re-issued or duplicate fact has `n ≤ home_transition` and is a no-op.
  - Accept (§6) uses the same counter.
- **`materialize_membership(joined)`** with `n > home_transition`:
  - set `state = 'joined'`, `voluntary_state = 'joined'`, `joined_at = at`, `left_at = NULL`, `home_transition = n`, `last_joined_n = max(last_joined_n, n)`, and a new local `episode` (`next_invitation_episode`);
  - insert a `membership_intervals` row with `joined_seq = decision_seq`;
  - `read_cursors::init_for_join` when the seat is local;
  - the derived `join` info event (payload `{"action":"join","seat","generation","observation"}` from `attribution::take`, key `join:<thread>:<seat>:<n>`);
  - `settle_superseded_invitations(thread, seat, n)`;
  - the membership revision and directory/member/inbox filter bumps.
- **`left`** with `n > home_transition`: `state = 'left'`, close the open interval with `left_seq = decision_seq`, the derived `leave` event, and the same bumps.
- A joined fact for a seat whose membership row is `retired` is a no-op that still records `home_transition`. Retirement is terminal.
- Command checks stay authoring-side A2: `join` refuses with a pending invitation, `leave` requires `joined`, and pair guards apply.

**Considered.**
- Using the envelope counter as the idempotence key: a re-issue under a new incarnation changes the counter.
- Comparing `(hlc, node)`: a home node's HLC is not monotone in counter (ht-5fr.3 §3).

### 8. Messages: `message.sent`

**Recommended.**
- **Payload v1** (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §1.1; `mode` and `nudge` are gone):

  ```text
  { message, thread, author, body,
    priority: 0..=9, require_ack: bool,
    relays_user: bool, user_intent: Option<..>,
    sent_at, duration_ms,
    recipients: [ { seat, receipt: "none"|"ack", deadline_at: Option<i64> } ] }
  ```

  - `priority` is the message's delivery priority exactly as main stores it (`messages.priority`); it is required. Every delivery row of the message is staged at it.
  - `require_ack` is true iff the send named explicit ACK recipients (`--require-ack`); it decides the recorded `delivery_mode` (`ordinary` iff true), which the `[pN]` read annotation depends on. It implies `priority = 0` (D3), and every `receipt: ack` recipient implies it; a human explicit ACK recipient is `receipt: none`. A payload breaking these rules is `Rejected`.
  - `recipients` is the complete audience the author fixed (root §6). It is sorted by seat ID and excludes the author.
- **Author** (`publish_send`, inside the publish closure after the existing revision checks):
  1. Read the sealed preparation's `prepared_recipients` (ack recipients, `frozen_duration_ms`, `eligible_at_snapshot`) and `lazy_recipients`.
  2. Build `recipients` (receipts only for explicit ACK recipients, main D9):
     - `ack` for prepared rows (explicit ACK recipients that are not human), with `deadline_at = sent_at + duration` when `eligible_at_snapshot = 1`, else `None`;
     - `none` for every other audience member, a human explicit ACK recipient included. On a direct thread the audience also holds the other pair seat while its auto-join is pending (§5).

     `priority` is the sealed preparation's priority, and `require_ack` is whether the request named explicit ACK recipients.
  3. `ensure_published` the author and every local recipient, stage `send:<message>` attribution, then author.

  The prep phase (`stage_recipient`) gets one change for remote recipients: `expects_ack = remote_kind != 'human'` (instead of `open_binding_role`), `availability_provenance = 'remote'`, `eligible_at_snapshot = 1`, and no unavailable-warning evaluation, because the sender has no availability evidence for another node's seat.
- **`materialize_message`:**
  1. **No-op** when `messages.id` exists.
  2. `prep = send_preparation_id_for_message(message)` (the inverse of `derived_message_id`, so `m<tag><8>` ↔ `p<tag><8>`).
  3. **Local branch.** When `local_origin` and `send_preparations(prep)` is `sealed` with no manifest, it adopts the staged rows. The staged audience must equal `recipients`: the same seat set, the same ack flags, frozen durations matching `deadline_at − sent_at` for eligible rows, the staged `priority` equal to the payload's, the staged `delivery_mode` matching `require_ack`, and the delivery-row seats equal to the local `none` recipients. Otherwise it is `Rejected(StoreCorrupt "staged audience differs from authored audience")`, which aborts the command. Root walk-through: "reuses a local preparation's staged rows, keyed by message ID".
  4. **Synthesized branch** (remote origin, or own old incarnation without a preparation):
     - insert `send_preparations(id = prep, operation_scope = 'replicated:<origin uuid>', operation_key = message, digest = SHA-256(payload bytes), status = 'sealed', delivery_mode = (require_ack ? 'ordinary' : 'lazy'), priority = priority, origin = 'replicated', captured_* = current values, interval_high_water = current max interval ordinal)`;
     - one `prepared_recipients` row per `ack` recipient, local or remote (`frozen_duration_ms = max(1, deadline_at − sent_at)` and `eligible_at_snapshot = 1` when `deadline_at` is `Some`; else `duration_ms` and `0`; `availability_provenance = 'replicated'`), in payload order as `receipt_ordinal`;
     - one delivery row (`lazy_recipients`, `priority = priority`) per local `none` recipient that is not suppressed (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §1.1);
     - no `prepared_unavailable_warnings`: unavailable warnings are the sender's derived events.

     `send_preparations` gets `origin TEXT NOT NULL DEFAULT 'local' CHECK(origin IN ('local','replicated'))`. The cleanup and abandon paths ignore `replicated` rows, which are born sealed and published.
  5. **Publication** is `insert_publication`, refactored to take `MaterializeInput` and a `PublicationAuthor`:
     - `messages(decision_at = sent_at, decision_seq, sequence = threads.next_sequence, origin_node, priority, delivery_mode = (require_ack ? 'ordinary' : 'lazy'), native_observation`/`sender_binding_id` from `attribution::take`);
     - `send_manifests`;
     - `next_sequence`/`timeline_revision`/filters;
     - then `DeliveryTail::message(...)` (§10).

     A `PublicationAuthor::Native` built from the payload carries `observation` and `binding` from the attribution (`binding = None` for remote). `messages.sender_binding_id` is nullable already.
  6. **Retired recipients.** For each `ack` recipient whose seat is `retired` at apply, `project_receipt` writes `recipient_retired` (`retired_at` = the seat's `retired_at`) in the job instead of `pending` (§4).
- **Remote recipients** get exactly the `prepared_recipients` row (and, through the job, a `receipt_state` row). No delivery row, no attention, no wake (root §6 "remote recipients only get receipt rows"). Whether a local recipient's delivery row wakes it is decided on its own node, against its node-local nudge threshold at wake time; the threshold is never replicated.
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
                   priority: DeliveryPriority, ack_recipients: i64, warnings: i64) -> Result<(), ApiError>;
    /// Invitation to a local invitee that is not suppressed (always a wake source: p0).
    pub fn invitation(tx, invitee: &SeatId) -> Result<(), ApiError>;
    /// The one attention writer: `wake_work` upsert, a no-op for remote seats.
    pub fn attention(tx, seat: &str, reason_bits: i64) -> Result<(), ApiError>;
}
```

- **`message`** writes the `send_attention` work job (`work:send:<message>`, `high_water = ack_recipients + warnings + 1`) for every message with at least one ack recipient or warning, **on every node**. The job projects `receipt_state` for all ack recipients (a replicated table) and calls `attention` per recipient, and `attention` skips remote seats. For every message with delivery rows it runs `lazy_delivery::note_published`, whose `wake_work` insert (main's threshold rule: only seats whose nudge threshold admits the row's priority, through `NudgeThreshold`'s store-side SQL fragment) gains `AND s.is_local=1` (and, through the synthesized branch, holds only local recipients). Amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §1.1.
- **Suppression.** Before staging delivery for a local recipient, `materialize_message` asks `m.suppress(receipt_settle_key(message, seat))`. A suppressed recipient keeps its `prepared_recipients` row (receipt state is replicated) but gets no delivery row. The job's projection for it calls `attention` with a skip flag, so no `wake_work` bump happens. That flag is the new `send_manifests.suppressed_seats TEXT` (a `'\n'`-joined sorted list, NULL when empty), read by `send_unit`. Settled history therefore never wakes anyone (root §7).
- **`invitation`** writes today's `wake_work` reason-bit-1 upsert, through `attention`, for every invitation (main D6: no lazy invitations). The inbox filter bump stays with the materializer.
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
| topic / name / archive / reopen | `register:<thread>:<field>:<uuid8>:<counter>` (event carries `actor_seat` when the payload names an actor) | `thread.register` on `Won` and changed |
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
- A fact to a local seat, from another node or from this node's own old incarnation alike, is suppressed by an own settlement in `journal` or `journal_forked` (ht-5fr.3 §8 (b)). After a wipe or restore those settlements arrive in the node's own old logs, so the Apply lane's own-history hold (root §7; ht-5fr.3 §7) keeps foreign and own-old facts unapplied until the own logs are ingested. A wiped node therefore applies a `message.sent` only after its own old `receipt.settled`, and wakes nobody for it.
- An own-old-incarnation entry is not suppressed for being own history (ht-5fr.3 §8 (a)): an unsettled ACK or invitation obligation in the pulled-back tail writes its lazy row, attention and `wake_work` normally once the hold lifts, so the seat sees what its sender's node will report overdue.

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
- The overdue hold at daemon start and after a sleep/wall-jump is ht-5fr.13's. It consumes this scope change and adds nothing here.

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
  - Participants, thread show, directory, member listings and the `seats` listing (without `--panes`) include remote seats. Their pane columns are NULL, and their state renders as `remote` (one short word, in the existing state column, so there is no new line per seat).
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
- **Accepted race** (root §8, §12). The eligibility probe reads the local membership view. A remote seat's concurrent `membership.joined` or accept that has not applied here yet does not block the archive, so the thread is archived on every node, the joiner's included. Posts into it still apply and deliver (root §6), and an explicit `reopen` restores it. No rule makes a later remote join reopen the thread: the race costs one explicit reopen, while a reopen-on-join rule would add a second writer of the `archived` register.

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
- **an origin that is not the kind's authorized author** (root §6 "Author authority is not author view"). One helper, `materialize::require_author(tx, m: &MaterializeInput, rule: AuthorRule)`, called first by each handler, with the closed `AuthorRule`:
  - `SeatHome(seat)`: `origin_node == seats.home_node` for `seat.retired`, `membership.joined`/`.left`, `receipt.settled` (the receipt's seat) and `invitation.answered` (the invitation's invitee);
  - `SeatHome(inviter)` for an `invitation.created` with `inviter_kind = seat`; `Any` (the signing origin node is the authority) for a `service` or `operator` invitation, which names no inviter seat (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §4);
  - `SeatHome(author)` for `message.sent`, `SeatHome(creator)` for `thread.created`;
  - `EitherSeatHome(seat_lo, seat_hi)` for `thread.direct`;
  - `OwnTag(seat)` for `seat.created`: the seat ID's tag is `tag_for(origin_node, k)` for some `k` from 0 to the origin's current `tag_k` in `node_directory` (the origin's own retags precede the fact in its log, so every node checks against the same `k`);
  - `Any` for `thread.register` (any node whose seat wrote it) and the `node.*` kinds (their log is the node).

  The rejection is `Rejected(InvalidRequest "origin <uuid8> is not the authorized author of <kind> for <id>")`. The check reads only replicated identity, which `needs` guarantees is applied, so every node rejects the same facts. A re-issue keeps the origin node, so re-issued facts pass. The local-only route (`materialize::local_only`) skips it: its origin is self by construction;
- payload decode errors;
- a `thread.direct` ID mismatch;
- an unknown recipient seat (the needs overflow case);
- the local-branch audience mismatch, which only occurs at authoring and so aborts the command;
- constraint violations (`Conflict` from `store_error`).

The one `Wait` case is the own seat restore with no attached host (`seat.created`, section 4).

Store errors pass through as `FactError::Store` via `from_api`. No handler rejects because of the local membership, archive or eligibility view (root §6). The authority check above is not a view check: it compares the origin with replicated identity that every node holds identically, so it never rejects a fact the authorized author allowed.

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
  - **Crossed direct pair:** A and B each run `ensure_direct_thread` toward the other while unsynced. After sync, there is one `d…` thread with the same `created_at` (earliest creator) on both, both seats are joined at `n = 1`, no invitation exists, and neither seat got a `wake_work` from the crossing. A one-sided create: B authors the peer's `membership.joined` as a follow-up of A's `thread.direct` (amended 2026-10-15 (main v1 reconcile): see [main-v1-reconcile](2026-10-15-cross-node-sync--main-v1-reconcile-design.md) §Tests).
  - **Three nodes (A authors, C relays):** C applies B's `membership.joined` before A's `invitation.created`, and the reverse order on a fourth store. Both end `already_joined`.
  - **LWW:** concurrent topic writes on A and B converge to the higher `(hlc, node)` and append exactly one topic event on the losing node's side when the winner arrives.
  - **Archive:** concurrent archive and reopen converge. A message into an archived thread still applies on the receiver.
  - **Retirement:** B retires its published seat while A sends to it concurrently. Both arrival orders end with that receipt `recipient_retired` on both nodes, and only A records overdue.
  - **Membership re-issue:** join, leave, then a re-issued join with `n=1` is a no-op.
- **Catch-up suppression:**
  - B authors `receipt.settled`, B's store is wiped (fresh store with the same seed), B re-ingests its old log and A's log. The message applies with no `lazy_recipients`/`wake_work` for the settled seat, and the settlement then applies.
  - **Wipe, then pull foreign first:** the same setup, but A's log (with the `message.sent`) is ingested on B **before** B's own old log, with the own-history hold set (B's old log advertised beyond B's contiguous). Nothing of A's applies while the hold is set; after B's old log is ingested the hold clears, the message applies with no `lazy_recipients`/`wake_work` for the settled seat, and no wake is dispatched.
  - **Own old unsettled:** B sends a `require_ack` message from one local seat to another, B's store is restored to a copy from before the send (or wiped), and B pulls the old tail back. While the hold is set nothing of the tail applies; after it clears the unsettled message writes its lazy row and `wake_work` for the recipient, while a settled message in the same tail writes neither.
  - **Restore-window authoring hold:** with the own-history hold active, `ack`, `accept`, `join` and `leave` for a local seat fail with retryable `ServiceBusy "own history catching up"` (amended 2026-10-15, main v1 reconcile §5) and write nothing; `ack_displayed` leaves the receipt pending; a send and an invite succeed; after the hold clears the same `ack` succeeds, and when the old tail already held a settlement for that receipt, the restored node authors no second `receipt.settled`.
- **Order permutation** (root §5 invariant; `tests/store/fact_order.rs`): for each row of the §3 audit, the same fact set is applied into two fresh stores in two different causal orders (by ingesting the logs in opposite orders), and `replicated_projection` is identical in strict mode:
  - crossed `thread.direct` (creator fields);
  - `invitation.created` concurrent with the invitee's join, and with the invitee's join then leave (both end `already_joined`, membership `left`);
  - `invitation.created` and `message.sent` concurrent with the invitee's or recipient's `seat.retired` (`recipient_retired`);
  - concurrent `thread.register` writes;
  - an accept concurrent with a second inviter's invitation (the second ends `already_joined`).
- **Origin authority:** a validly signed `seat.retired`, `membership.joined`, `receipt.settled`, `invitation.answered`, `invitation.created` and `message.sent` from a node that is not the authorized author, and a `seat.created` whose ID tag is not one of the origin's, each apply as `failed` with the authority message on every node and change no replicated row; the same facts from the authorized author apply; a re-issued fact passes; a `seat.created` minted under the origin's pre-retag tag passes.
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
- **Fence:** 20 remote messages applied into a thread while a local seat sends 10 priority-1 messages through the prep/publish pair. All 10 publish without a `snapshot changed` conflict. A prep with an unavailable warning still conflicts on a timeline bump.
- **Oracle:** `test_support::journal_oracle::replicated_projection` (ht-5fr.3) gains the converted tables' replicated columns:
  - `threads(id, topic, goal, name, archived, created_at, journaled)`;
  - `direct_threads(thread_id, seat_a, seat_b, created_hlc, created_node)`;
  - `seats(id, home_node, kind = coalesce(remote_kind, published kind), retired = state='retired', retired_at)` (normalizing local versus remote);
  - `memberships(thread, seat, state, home_transition, last_joined_n)`;
  - `invitations(id, thread, seat, state, deadline_at, confirmation, inviter, invitee_n, origin_node, accepted_at)`;
  - `invitation_rejections(invitation_id, reason)`;
  - `messages(id, thread, actor_seat_id, body, decision_at, origin_node, priority, delivery_mode)` for `kind='ordinary'` (priority and delivery_mode amended 2026-10-15, main v1 reconcile §Tests);
  - `prepared_recipients(message, seat, frozen_duration_ms, eligible)`;
  - `receipt_state(message, seat, state, deadline_at, acked_at, settle_batch, retired_at)`.

  Attribution columns, every local position and every delivery table are excluded. A scripted multi-command workload (create, pair, invite, accept, reject, send at priorities 0, 1 and 3 and with `--require-ack`, ACK, topic, archive, retire) replayed into an empty store matches in strict mode.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral` (all new files are core and name no harness), `nice scripts/check-default-features`, and focused `nice cargo test --locked --all-features facts fact_convergence messages receipts control archival invitation wake`.

Suggested task cut (files):
1. Schema additions (`threads.journaled`, `seats.published`, `memberships.home_transition`/`last_joined_n`, `invitations.origin_node`/`invitee_n`/`already_joined`, `messages.origin_node`, `send_preparations.origin`, `send_manifests.suppressed_seats`, `receipt_state.settle_batch`, `direct_threads.created_*`, `retirements.origin`, `fact_attribution_staging`, the trigger and index changes): `migrations/node/0001_node_baseline.sql`, `src/service/kicks.rs` (classification).
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

## Key abstractions

**MaterializeInput** — new struct (`src/store/materialize.rs`), the one seam between "who applied this" and "what rows it writes": every store materializer takes it, whether a `FactKind` handler or a local-only writer called it.
- Owns: the apply instant, `decision_seq`, origin node and ms, `local_origin`, provenance and the suppression query (§2); `materialize::local_only`, the only other constructor (fresh `decision_seq`, self origin, never suppresses).
- Hides: whether the caller is ApplyEngine or a managed-thread writer, so each materializer exists once.
- Must not: allocate positions itself beyond `local_only`'s one `next_decision_seq`, or carry kind data (the payload stays a separate argument).
- Relationships: built from `ApplyContext` by the handlers and by `local_only`; consumed by the materializers, which stay `pub(crate)` in their table-owning store files (`seats`, `control`, `messages`, `receipts`), so `src/sync` holds no SQL and `src/store` never reads `ApplyContext`.
- Lifecycle: a borrowed value for one apply or one local-only write.

**FactRoute** — new closed enum (`src/store/materialize.rs`, `Journal` / `LocalOnly`) plus one generic emit, the one place a command turns a decided payload into rows.
- Owns: picking the route once per command from `threads.journaled`; for `Journal`, the restore-window authoring hold (§2: one-shot same-key kinds for local seats refuse as retryable while the own-history hold is active), publishing every local seat the payload references (`seats::ensure_published`, §4) and then `journal::author` / `author_above`; for `LocalOnly`, refusing any remote seat with `Unsupported` "service threads are node-local" (§2) and calling the kind's materializer with `local_only`.
- Hides: the managed-thread exception, so no converted command branches on `journaled`.
- Must not: skip the journal for a journaled thread, publish seats on the local-only route (§2: managed membership never publishes), or decide anything the command decides (validation, IDs, audience).
- Relationships: commands call it in their fixed per-command order (§2); it depends on Journal and on each kind's materializer entry, which `FactKind` handlers also use.
- Errors: authoring errors pass through unchanged (including ht-5fr.3's `parked_dep` `Conflict`); the remote-seat refusal is a distinct `Unsupported`.

**Attribution** — new module plus struct (`src/store/attribution.rs`), the only owner of `fact_attribution_staging` and of the local-binding columns' values on replicated rows.
- Owns: `stage` by the command and `take` by the materializer under the local staging keys (§3); the fallback observation (`replicated_from:<uuid>`, or `local_journal` for an own-old-incarnation entry with nothing staged).
- Hides: that attribution never crosses the wire, and the provenance strings TRUST-POLICY defines.
- Must not: be keyed by an `EntityKey`, or survive its transaction (a staged row left at commit is a bug, caught by WriteGuard's suites).
- Relationships: per spec §2 `MaterializeInput.provenance` is a string rather than ApplyContext's closed `Provenance`; Attribution is its only reader.
- Lifecycle: staged and taken within one writer transaction.

**Fact payloads (`sync::facts::payload`)** — new module, the shared payload value types and the `EntityKey` spellings for the v1 kinds.
- Owns: `SeatKind`, `RecipientSpec`, `ReceiptObligation`, `SettleHow`, `AnswerOutcome`, `RegisterField`, `SettleBatch` (closed enums or plain structs, snake_case, `serde(default)`-tolerant); the key builders (`seat_key` … `answer_key`); each payload's list of referenced seats, which FactRoute publishes.
- Hides: the composite key spellings (`m…/s…`, `i…/answer`), so ApplyEngine and the suppression query treat them as opaque.
- Must not: carry local positions (`decision_seq`, sequence, episode, revision, `host_id`) or attribution.
- Relationships: used by the kind modules, by command authoring and by the materializers; depends only on `EntityKey` and the ID types.

**WriteGuard** — new test-support module (`src/test_support/write_guard.rs`), a SQLite authorizer that enforces the write-path closure (Testing).
- Owns: the replicated-table list and the rule "writes only inside an apply savepoint or a `local_only` scope".
- Must not: exist outside `test-support` builds or change behaviour when disarmed.
- Relationships: armed around converted commands in the existing store suites; the oracle projection (ht-5fr.3) checks the same table set from the other side.

**FactKind / FactRegistry** — extended: eleven v1 kinds in six entity-grouped modules under `src/sync/facts/`, each `MAJOR = 1`, one `.with::<K>()` each in `builtins()`. Every handler is the same shape (decode, `MaterializeInput` from `ApplyContext`, its one materializer, `FactError::from_api`), and the materializer entry is the one FactRoute's local-only path calls.

**DeliveryTail** — extended (§10): `attention` is the single `wake_work` writer and the one locality guard (a no-op for remote seats); `message` writes the `send_attention` job on every node and suppressed recipients ride `send_manifests.suppressed_seats`; `invitation` replaces `control.rs`'s inline upserts; `materialization::bump` becomes `attention`. Per spec §10, the job's receipt projection stays shared by local and replicated sends, so the root's "DeliveryTail does nothing for a remote recipient" means no attention, not no job.

**seats** — extended: `published` and `ensure_published` (publish at enrollment of the first open binding, with `FactRoute` as an idempotent backstop; kind from the open binding), `materialize_seat_created` / `_retired` with the own-seat restore branch, and remote eligibility in `eligible_delivery_recipient`; callers still test locality only through `LOCAL_SEAT` / `is_local_seat`.

**Store materializers (`control`, `messages`, `receipts`)** — extended: one `materialize_*` per kind (§1), each idempotent by its rule (§3); `settle_superseded_invitations` is the one `already_joined` rule (supersession by `last_joined_n > invitee_n`, never by current state), run by both `invitation.created` and `membership.joined`; `memberships.home_transition` is the membership idempotence counter and `last_joined_n` the supersession counter; `materialize::require_author` with the closed `AuthorRule` is the one origin-authority check every handler runs first; `insert_publication` takes `MaterializeInput` and a payload-built `PublicationAuthor`, after either adopting the sealed local preparation or synthesizing a `replicated` one (a fixed two-case branch on `local_origin`, per root §5/§7); `settle_pending` keeps its decision and the batch event moves to `materialize_settlement` keyed by `settle_batch`.

**Deadline lane / `record_overdue_if_pending`** — extended: the guard resolves the obligation's `origin_node` and writes nothing for another node's obligation; the sparse-due triggers, invitation due scan and extension-lapse scan are scoped to `origin_node = self`; the retirement job also serves `retirements.origin = 'replicated'`.

**Node-local workers and reads** — extended: wake, liveness, observation, archival and pane listings filter `LOCAL_SEAT`; archival defers threads with a live remote member and authors `thread.register{archived}` through `author_above`.

Variation: fact kinds are the open set: one `FactKind` type in its entity module, one registry entry, one materializer in the table-owning store file. The route (journal or local-only) is a fixed two-case enum chosen once from `threads.journaled`. Payload enums (`SeatKind`, `SettleHow`, `AnswerOutcome`, `RegisterField`), preparation origin and retirement origin are closed sets handled with `match` or a CHECK. Who applies a fact (ApplyEngine or the local-only path) varies only as data in `MaterializeInput`, so there is no trait for it. Per spec §2, local attribution travels through a staged row rather than an `ApplyContext` extension, so it is a store module and not part of the handler surface.

Walk-through: an agent on A sends to B's seat in a journaled thread. `publish_send` keeps its revision checks (timeline only when warnings exist, §16), reads the sealed preparation, builds the sorted `RecipientSpec` list with the remote seat as `ack` (expectation from `remote_kind`), and stages `send:<m>` Attribution. `FactRoute::Journal` publishes the author (its first replicated reference) through `seat.created`, then authors `message.sent`. ApplyEngine runs the `MessageSent` handler, which builds `MaterializeInput` and calls `messages::materialize_message`: the local branch adopts the staged rows (audience equal), `insert_publication` takes the attribution and writes the message with `origin_node = A`, and `DeliveryTail::message` writes the `send_attention` job. The job projects B's receipt row, and `attention` skips the remote seat. On B the same handler takes the synthesized branch, Attribution falls back to `replicated_from:A`, the suppression query is false, and the job's `attention` writes `wake_work` for the local seat. A's sparse-due trigger holds the receipt and B's does not, so only A can warn. Likely future change: a `seat.renamed` kind filling the reserved `display_name`. That is one `SeatRenamed` type in `src/sync/facts/seat.rs`, one `.with::<SeatRenamed>()`, and one `seats::materialize_seat_renamed`; its payload lists the seat, so FactRoute publishes it, and MaterializeInput, Attribution, DeliveryTail and the commands that do not rename are untouched.

Digest:
- MaterializeInput: owns the per-apply origin, position, provenance and suppression handed to materializers, plus the `local_only` constructor; new apply-time inputs are fields here;
  materializers never read `ApplyContext` and handlers never write tables.
- FactRoute: owns the journal-or-local-only choice, the restore-window authoring hold, seat publication before authoring and the managed-thread remote-seat refusal; a new kind needs only its referenced-seat list;
  commands never branch on `threads.journaled` or call `ensure_published` themselves.
- Attribution: owns `fact_attribution_staging`, the staging keys and the fallback provenance observation; new attributed kinds add a staging key here;
  materializers never read bindings for attribution and payloads never carry it.
- Fact payloads (`sync::facts::payload`): own the shared payload value types and the v1 `EntityKey` builders; new shared payload types and key spellings go here;
  callers never format `creates`/`needs` keys inline or put local positions in a payload.
- WriteGuard: owns the replicated-table write check for tests; a new replicated table is added to its list and the oracle projection;
  converted commands never write a replicated table outside apply or `local_only`.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
