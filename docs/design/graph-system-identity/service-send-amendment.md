# herdr-threads amendment: service-authored receipt-bearing messages and service reads

Status: accepted 2026-10-02 (user-approved herdr-graph request); implemented under epic ht-5nb.

Requested by herdr-graph `implementor` (`threads-amendment-request.md`). Designed against
herdr-threads main `a4a4d4a4` (B5 trust model merged, schema v10). Implementation landed **after**
`super-auto/remaining-herdr-threads-findings` merged (see "Sequencing").

## 1. Verdict: accept, with six adjustments

No conflict with TRUST-POLICY or the graph-system-identity contract. The amendment extends the
service's *author* role from `system_notify` events to ordinary messages. It does not touch the
service's *recipient*, *acceptance* or *ACK* roles:

- **The service can never be a receipt recipient.** `receipts.seat_id`, `prepared_recipients.seat_id`
  and `receipt_state.seat_id` are FKs to `seats`, and the service author is not a seat (design §2).
  This holds structurally, so it needs no new guard.
- **No fabricated ACKs.** ACK stays the native `Command::Ack` with a `CallerClaim`, and
  `receipts.rs` is author-agnostic. No service operation writes receipt state. ACK provenance
  values (A3) are unchanged.
- **Attribution stays explicit.** The message row carries `author_kind='programmatic'` and
  `author_service_id`, and `actor_seat_id` is NULL. This is the claim-as-claim the existing
  notifications already use, and readers already render it through `MessageSummary.event_author`.
- **Wake and deadlines are the native machinery.** Publication enqueues the same `send_attention`
  work job. Overdue warnings are built-in `warn` events in the thread, with
  `source_message = <the service message>`. The service is never a wake target.

Docs to amend: TRUST-POLICY A5 (add a row: "service-authored send / notify / managed-thread
controls: the registered service connection"). In graph-system-identity `design.md` §2, change the
capability list (add send and reads) and add a §4 note that service *messages*, unlike
notifications, carry ordinary receipt obligations. "No new service operation forcibly accepts, ACKs,
retires…" still holds verbatim.

Adjustments to the request as written:

1. **Recipient set uses native semantics. It is not "exactly these seats".** A native ordinary
   message creates an obligation for every effectively joined member at decision time, plus each
   explicit recipient. `prepare_send_step` walks `membership_intervals`, then
   `invited_recipients`, and each explicit recipient must be effectively `invited|joined`.
   `--require-ack` *adds* invitees to the joined snapshot. The service send does exactly the same,
   with no sender exclusion because the service is not a member. Consequence for graph: every
   joined clone in the request thread gets its own obligation and must ACK independently. To
   address one occupant, use a managed channel whose joined set is that audience, for example one
   channel per summarizer seat or clone. A pending *required* invitee is a valid explicit recipient:
   a required invite writes `memberships.state='invited'`, `service_controls.rs:350-360`.
2. **Managed threads only.** The service's counterpart of "sender must be joined" is "thread is
   managed by this service author" (`require_owner`). An ordinary or foreign-owned thread returns
   `IncompatibleOwnership`. Notify stays allowed on any thread. Send is stricter because it creates
   obligations.
3. **A zero-obligation send is rejected.** The operation exists to create obligations, so a sealed
   preparation with `recipient_count == 0` is discarded and returns
   `InvalidRequest("no receipt recipients")`. Native sends keep allowing zero.
4. **The service session capability bumps to `service_session_v2`.** An old daemon cannot decode an
   unknown `ServiceOperation` variant. `service_connection.rs` maps the decode failure to
   `InvalidData` and **terminates the registration**. So the client registers with v2. An old
   daemon then rejects cleanly at register ("unsupported service capability"). A new daemon accepts
   v1 and v2 and records the negotiated capability per session. New operations on a v1 session
   return `Unsupported` and the session survives. If the in-flight branch's
   `protocol/capabilities.rs` (`ADVERTISED`) lands, also advertise `service.send_v1` for discovery.
5. **The send result is bounded and stable under replay.** It has no per-recipient list. The
   recipient count is joined members plus up to 100 explicit recipients, which can be large. The
   result is also stored in `operations.result_json` and replayed later, by which time states have
   moved. The obligation identity is `(message_id, seat_id)`. Every obligation is `Pending` by
   construction at commit. Live per-recipient state comes from the paginated Receipts read.
6. **Reads already exist as claimless public reads, and the service variants add scoping.** Native
   `Command::History`, `Command::DeliveryInspect`, `Command::Recipients` and `Command::Message`
   need no claim: `service/dispatch.rs` sends them straight to `store.query`. A same-UID graph
   process can call them today through `LocalSocketClient`, or through the CLI
   (`history THREAD --after N`, `delivery inspect MSG`). That is confirmed safe and is a usable
   interim path for graph's fake and real integration before this lands. The service-session reads
   add one-connection convenience and author scoping for receipts. They reuse the native wire
   types and cursors unchanged.

Not added (possible follow-ups, not requested):

- An author-scoped "all my pending obligations" list. The manifest-backed effective receipts make
  an author filter non-trivial to bound, and graph already records its own message ids.
- Per-recipient `deadline/overdue` on `Recipient`. Overdue is visible today as warn events in
  history.

## 2. Operation and result shapes

All of these go in `src/protocol/service.rs` unless noted. Existing types are reused wherever
they fit.

```rust
pub const SERVICE_SESSION_CAPABILITY: &str = "service_session_v1";      // unchanged
pub const SERVICE_SESSION_CAPABILITY_V2: &str = "service_session_v2";   // v1 ops + Send/History/Receipts

pub enum ServiceOperation {
    /* existing: EnsureThread, Invite, Notify, Membership, SetTopic, ReleaseRequirement, Archive, Reopen */
    Send(ServiceSend),             // mutation, journaled (operation_key = Some)
    History(ServiceHistoryQuery),  // query, never journaled (operation_key = None)
    Receipts(ServiceReceiptsQuery),// query, never journaled
}

#[serde(deny_unknown_fields)]
pub struct ServiceSend {
    pub thread: ThreadId,              // must be managed by this service author, not archived
    pub body: String,                  // 1..=MAX_BODY_BYTES (64 KiB) and <= instance body limit; same as native
    pub recipients: Vec<SeatId>,       // explicit additions to the joined snapshot; <=100 (MAX_BATCH_ITEMS); no duplicates
    pub deadline_millis: Option<u64>,  // >0; None = instance default receipt duration (native frozen_duration)
    pub operation: OperationId,        // exactly-once key in scope "service:<instance>:<author>"
}

#[serde(deny_unknown_fields)]
pub struct ServiceHistoryQuery {      // field-for-field the native HistoryQuery
    pub thread: ThreadId,
    pub page: PageRequest,
    pub initial: Option<HistoryRange>, // Recent{count} | After{sequence} | Before{sequence}; conflicts with page.cursor
}

#[serde(deny_unknown_fields)]
pub struct ServiceReceiptsQuery {
    pub message: MessageId,            // must be authored by this service author
    pub page: PageRequest,
}

pub enum ServiceResult {
    /* existing variants … */
    MessageSent(ServiceMessageSent),
    History(Page<MessageSummary>),     // == native CommandResult::History payload
    Receipts(DeliveryInspection),      // == native CommandResult::DeliveryInspect payload
}

/// Mirrors ServiceNotification: native-compatible summary plus the exact durable author.
#[serde(deny_unknown_fields)]
pub struct ServiceMessageSent {
    pub summary: MessageSummary,       // kind=Ordinary, author=None, event_author=Some(Programmatic(author)),
                                       // actor_label=Some("herdr-graph"), sequence, created_at
    pub author: ServiceAuthorId,
    pub recipient_count: u64,          // obligations created = send_manifests.recipient_count (>=1)
    pub receipt_duration_millis: u64,  // frozen duration actually applied
}
```

Receipt state comes from the reused `DeliveryInspection { message, delivery: DeliveryAggregates
{committed, acknowledged, ..}, recipients: Page<Recipient> }`. Each `Recipient` row maps as follows:

| graph needs | field |
|---|---|
| pending | `effective_status == Pending` |
| acked + ack time | `effective_status == Acknowledged`, `ack_provenance.decided_at` (plus actor/generation/observation) |
| recipient_retired (+ when) | `effective_status == Retired`, `retirement_cutover` |

Cursors are the native `CursorScope::History` / `CursorScope::DeliveryInspect` cursors. The service
reads call the same query functions, so page sizing and `stop_reason` and `consistency` semantics
are identical (including whatever the in-flight branch's PageFit does). `next_argv` keeps pointing
at the CLI equivalents, which is harmless.

`ServiceOperation::operation_key()` returns `Some` for `Send` and `None` for `History`/`Receipts`.
`PersistentServiceClient::query` already refuses keyed operations, and `submit` refuses keyless
ones. Add `result_matches` pairs `Send→MessageSent`, `History→History` and `Receipts→Receipts`.

`validate()` additions:

- `Send`: body non-empty and at most 64 KiB, at most 100 recipients, no duplicates,
  `deadline_millis != Some(0)`.
- `History`: the native rules (cursor/initial conflict, `Recent.count` in 1..=100,
  `page.validate()`).
- `Receipts`: `page.validate()`.

Exactly-once:

- Replay digest = canonical JSON
  `{"kind":"service_send", thread, body, recipients: <sorted set>, deadline_millis}`. Recipients are
  order-insensitive, as in the native non-claim path.
- The operation key shares the service-wide namespace (`operations.actor_scope =
  "service:<instance>:<author>"`). Reusing a key across operation kinds, or with a different
  payload, returns `OperationPayloadMismatch`.
- `send_preparations.operation_scope` uses the same scope string. That column is free text with
  `UNIQUE(instance_id, operation_scope, operation_key)`.
- After response loss the client re-registers and calls `replay(key)`, which returns the stored
  `MessageSent` without a second message. This is the existing `ServiceIntentJournal` flow,
  unchanged.

Decision path:

- Preparation steps and the publish transaction both run under `ServiceDecisionTransaction`
  (DB write lock, then the authority guard held through commit). This is the same fence notify
  uses, in place of `execute_accountable_transaction` and the native claim.
- At publish the transaction re-checks owner, not-archived and the captured revisions.
- Audience drift is handled like notify: the daemon restarts internally within the request budget,
  up to `MAX_AUDIENCE_RESTARTS`, and drives the discarded generation's bounded cleanup inline
  before rebuilding. `send_preparations` allows one row per key, so rebuilding needs that cleanup
  first.
- If the restarts or the budget run out, the result is `Conflict`. That is definitive: nothing was
  published. Graph resubmits under a **new** key, because the journal completed the old one.

Error cases:

| Case | Code |
|---|---|
| thread unknown | `NotFound` |
| thread in another instance | `Unauthorized` (as notify) |
| thread not managed / managed by another owner | `IncompatibleOwnership` |
| thread archived (prepare or publish) | `Archived` |
| recipient seat unknown / other instance | `NotFound` ("recipient seat unknown: <id>") |
| recipient retired | `InvalidRequest` ("recipient retired: <id>") |
| recipient not invited or joined in thread | `InvalidRequest` ("recipient not a member or invitee: <id>") |
| zero resulting obligations | `InvalidRequest` ("no receipt recipients") |
| body empty / over limit, deadline 0, >100 or duplicate recipients | `InvalidRequest` |
| key reused with different payload or other op | `OperationPayloadMismatch` |
| audience drift beyond restart allowance | `Conflict` (definitive, unpublished) |
| budget exhausted / cancelled | `DeadlineExceeded` / `Cancelled` (pre-decision: unpublished; post-commit loss: `UnknownOutcome` → replay) |
| Receipts: message unknown | `NotFound` |
| Receipts: message not authored by this service | `InvalidRequest` ("message is not authored by this service"; scoping only, the same data is a public native read) |
| History: thread unknown | `NotFound`; cursor errors as native (`CursorStale`, `InvalidCursor`) |
| Send/History/Receipts on a v1 session | `Unsupported`, session kept |
| connection revoked / generation stale | existing `ServiceNotRegistered` / `StaleServiceGeneration` |

Read execution:

- `History` and `Receipts` run on the reader path (`store.query` with a `ReadContext`). They take
  no writer, no lane turn and no authority guard, because they mutate nothing.
- The transport only dispatches them on a live registered connection.
- The Receipts author check is one indexed lookup (`message_author()` == `Programmatic(author)`),
  done before delegating to the native DeliveryInspect query.
- History may target any thread in the instance (history is public), not only managed ones.

Wire fix needed for native readers (part of this amendment): `PendingReceipt.sender: SeatId` is
built by `queries.rs` (~4364), which returns **`StoreCorrupt("receipt sender missing")`** when
`actor_seat_id` is NULL. That is exactly the case for a service-authored message. So without the
fix, a recipient's `pending-receipts` would break. Change it to mirror `MessageSummary`:

```rust
pub struct PendingReceipt { …, pub sender: Option<SeatId>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub sender_author: Option<EventAuthor>, … }
```

Then update its two renderers (`cli/human.rs:~355`, `protocol/output_compact.rs:~308`) to print
the service label. History, `message`, delivery-inspect and `message_summary` already tolerate a
NULL seat author, and the wake, attention, materialization and ACK paths never read the sender.
I checked this.

## 3. Store and schema impact: none, so no v12

The v2 and v5 schema already admit everything this needs:

- `messages.author_kind IN ('native','programmatic','built_in')` and `author_service_id`. The
  `messages_author_shape_insert` trigger requires that programmatic rows have `author_service_id`
  set to an instance-matched `service_authors` row and `actor_seat_id IS NULL`. It does **not**
  constrain `kind`, so `kind='ordinary'` with a body and no `event_key` is legal.
- `receipts`, `receipt_state` and `prepared_recipients` have no sender column. The sender was only
  ever derived from `messages.actor_seat_id`.
- The `send_manifests` trigger requires only `m.kind='ordinary'`, `event_offset=0`, and a matching
  instance, thread and decision_seq.
- `send_preparations.operation_scope` and `operations.actor_scope` are free text.

So the service publish inserts:

```
messages(id, instance_id, thread_id, sequence, kind='ordinary', actor_seat_id=NULL, actor_label='herdr-graph',
         body, decision_at, decision_seq, author_kind='programmatic', author_service_id=<author>)
```

It then writes the same `send_manifests` row and `send_attention` work job as native publish. No
`verify_existing*` change is needed.

The only schema-worthy option is a defense-in-depth trigger: "ordinary programmatic messages only
on threads whose `managed_owner_author_id = author_service_id`". **I do not recommend it.** Under
the cooperative model the code check is sufficient, and the existing trigger already pins the
author's existence and instance. If the coordinator wants it anyway, it would be **v12**: main is at
v10 from B5, and the in-flight branch's `0010_cooperative_only.sql` will be renumbered to v11 on
merge. It would also need the usual `verify_existing_v12` and a v11-to-v12 migration test.

## 4. Sequencing against `super-auto/remaining-herdr-threads-findings`

Satisfied: the remaining-findings branch (ht-p03) is merged; the notes below record why.

Every leaf touches files that branch rewrites, so all of them land after it merges:

- `src/store/messages.rs`: the branch deletes the native-permit arm of `publish_send` and the
  `resolve_*_seat`/`assert_actor_current` helpers (-146 lines). The service send generalizes
  prepare/publish, so it must start from the cooperative-only version.
- `src/store/mod.rs` (`service_operation` routing, ±1148 lines on the branch) and
  `src/store/queries.rs` (page sizing/PageFit, ±1051).
- `src/protocol/results.rs` (`ApiError::new`, typed constructors and `ErrorClass`: new error sites
  must use those) and `src/protocol/output_compact.rs`.
- `src/daemon/transport/service_connection.rs` (-241 on the branch) and `src/client/service.rs`
  (`budget.cancellation.cancelled()`).
- `src/protocol/capabilities.rs` (new on the branch; optional `service.send_v1` advertisement).
- `src/ports.rs`: **no change expected.** `StorePort::service_operation(ServiceOperation, …)` is
  already generic over the enum. If an implementor finds it needs a new port method, it goes on top
  of the collapsed ports.

## 5. Bead plan (definitions for the coordinator to create)

**Epic: "Service-authored receipt-bearing messages and service reads (herdr-graph amendment)"**

- Goal: the registered herdr-graph service can post an ordinary message into a thread it manages.
  The message carries native receipt obligations: ACK by exact id, never implied,
  recipient-retired on retirement, and native wake and deadlines. The service can read the receipt
  state of its own messages and thread history after a sequence. The service is never a recipient
  and never ACKs. No schema migration.
- Design: this note (`docs/design/graph-system-identity/service-send-amendment.md`).
- **Dependency: every leaf is blocked until `super-auto/remaining-herdr-threads-findings` is merged
  to main.** Rebase onto that main and use `ApiError::new` and the typed constructors.
- Consumer: herdr-graph `implementor` (pane w8:pA) builds against §2 with an in-process fake. Its
  real integration test gates on leaf 4.

**Leaf 1: Contract: service send and read wire types, v2 session capability, sender attribution**

- Files: `src/protocol/service.rs`, `src/protocol/results.rs` (`PendingReceipt.sender` becomes
  `Option`, plus `sender_author`), `src/client/service.rs` (register v2, `result_matches`, typed
  `send`/`history`/`receipts` helpers), `src/protocol/capabilities.rs` (advertise
  `service.send_v1` if present), `docs/design/graph-system-identity/service-send-amendment.md`,
  `docs/design/graph-system-identity/design.md` (§2/§4 note), `TRUST-POLICY.md` (A5 row).
- Acceptance:
  - The types match §2 exactly, with `deny_unknown_fields`.
  - `operation_key()` is Some only for `Send`.
  - `validate()` rejects an empty or oversized body, deadline 0, more than 100 or duplicate
    recipients, a History cursor+initial conflict, and a bad `Recent` count.
  - Round-trip tests for the new results. A forged `author` or `connection_generation` field in a
    Send frame is rejected.
  - The client refuses `query(Send)` and `submit(History)`.
  - The register validator accepts v1 and v2 and rejects anything else.
  - Serde tests for `PendingReceipt` with a native sender (unchanged JSON) and with a programmatic
    sender.
  - Docs updated.
- Plan decisions: the PendingReceipt query and renderer fix (sender attribution in
  `src/store/queries.rs`, `src/cli/human.rs`, `src/protocol/output_compact.rs`) landed with leaf 1,
  and `service.send_v1` is advertised by the leaf that lands the last handler, not by leaf 1.
- Depends on: branch merge.

**Leaf 2: Store: service-authored send with native receipt obligations**

- Files: `src/store/messages.rs` (generalize prepare/publish over a sender: native claim or service
  decision), new `src/store/service_send.rs` (or `service_events.rs`), `src/store/mod.rs`
  (`service_operation` routing for `Send`, like Notify), `src/store/queries.rs` (PendingReceipts
  sender attribution), `src/cli/human.rs`, `src/protocol/output_compact.rs`, and tests under
  `tests/store/`.
- Acceptance (real SQLite):
  - A message row has `author_kind='programmatic'`, NULL seat and label `herdr-graph`.
  - Obligations = joined snapshot ∪ explicit `invited|joined`, including a pending required
    invitee.
  - Native ACK by exact id settles it, and repeat ACK is idempotent.
  - History reads, check-in and notify create zero ACKs.
  - Retirement settles to `recipient_retired`.
  - The deadline and overdue warn event come from the frozen duration.
  - The `send_attention` job is enqueued, and pending-receipts lists the item with
    `sender_author=Programmatic`.
  - Every §2 error case is covered: not managed, foreign owner, archived at prepare and at publish,
    unknown, retired or non-member recipient, zero obligations, payload mismatch.
  - Commit followed by response loss, then exact-key replay, returns the stored result with no
    second message.
  - Audience drift restarts internally. Exhausting the allowance gives a definitive `Conflict`.
  - Authority revoked before the decision publishes nothing.
  - An audience larger than one preparation quantum works.
  - The service author can never appear in any receipt table.
- Depends on: leaf 1.

**Leaf 3: Service reads and session-capability routing**

- Files: `src/store/mod.rs` (route History and Receipts through `store.query` with a
  `ReadContext`, no writer), `src/store/queries.rs` (the author check for Receipts reuses
  `service_substrate::message_author`), `src/daemon/transport/service_connection.rs` (record the
  negotiated capability per session; v2 operations on v1 return `Unsupported` without dropping the
  connection), `src/service/dispatch.rs` if routing lives there, and tests.
- Acceptance:
  - History `After{sequence}` pages are byte-identical to native `Command::History` for the same
    thread, and cursors interoperate.
  - Receipts equals native DeliveryInspect for a service-authored message and shows pending, acked
    with `decided_at`, and retired with `retirement_cutover`.
  - A Receipts read on a native or notify message returns `InvalidRequest`. An unknown message
    returns `NotFound`.
  - Reads take no writer or lane turn: they make progress while the writer is held.
  - A v1 session sending Send gets `Unsupported` and its next v1 operation succeeds.
  - An old-style v1 client is unaffected.
- Depends on: leaf 1. Can run in parallel with leaf 2. A test fixture can seed a service-authored
  row directly.

**Leaf 4: Integration: graph request-queue flow end to end, plus native ACK proof**

- Files: `tests/service/graph_send.rs` (new, real daemon plus `PersistentServiceClient`), native
  validation harness fixtures under `tests/native/`, `docs/agent-usage.md` (agents ACK
  service-authored requests exactly like native ones, after dispatch, never on read).
- Acceptance (one real daemon flow):
  1. Register v2, ensure a managed thread, and send a required invite.
  2. The native seat accepts explicitly.
  3. The service sends with a deadline. An idle recipient gets the native wake prompt.
  4. The native seat ACKs by exact id. Receipts shows acked(at).
  5. The native seat replies. Service History `After{seq}` sees the reply.
  6. A second send goes to a seat that is then retired. Receipts shows `recipient_retired`.
  7. Disconnect between commit and response, re-register, and replay: no duplicate.
  8. A v1-registered client cannot send.
  9. Codex and Claude each ACK a service-authored message through their real integration. The
     evidence is the stored ACK provenance (`cooperative_top_level`), not prompt submission.
- Depends on: leaves 2 and 3.

## 6. Reply to graph (summary)

Accepted, with adjustments 1–6 above. Graph-side implications:

- Recipients follow native semantics (the joined snapshot plus explicit invitees), so pick a
  channel whose joined set is the audience.
- Send works only on managed channels.
- Register `service_session_v2`.
- Read per-recipient state through `Receipts`, not from the send result.
- Interim reads are available today through claimless native History and DeliveryInspect over
  `LocalSocketClient`.
- The operation is `ServiceOperation::Send`, returning `ServiceResult::MessageSent`. The reads are
  `ServiceOperation::History`, returning `ServiceResult::History(Page<MessageSummary>)`, and
  `ServiceOperation::Receipts`, returning `ServiceResult::Receipts(DeliveryInspection)`.
- Bead ids will come from the coordinator.
