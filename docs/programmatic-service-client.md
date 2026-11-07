# Programmatic service client

`PersistentServiceClient` holds one registered Unix socket and serializes calls on it. Construct it with the instance UUID, socket path, a clock, an optional expected daemon boot UUID, and a private `ServiceIntentJournal` directory under an existing durable parent. Use a separate journal directory per instance and programmatic author. Call `register` before `query`, `submit`, `send`, `history`, `receipts`, or `replay`. `register` sends `service_session_v3`. A daemon that does not know v3 (an older build) refuses the register with "unsupported service capability" and there is no fallback, so this client needs a daemon of this release or later. Upgrade the daemon before the client: this client always registers v3. The daemon advertises `service.send_v1` once it serves send and the two reads. A raw session registered with `service_session_v1` still runs the v1 operations but gets `Unsupported` for `Send`, `History` and `Receipts`. The returned author ID and connection generation are diagnostics; authority remains with the socket. A dropped or invalidated connection needs a new `register` call. An idle connection has no per-call deadline; each budgeted call checks its absolute deadline and cancellation before and after waiting for the session. `registration()` and `disconnect()` are administrative calls without a budget and may wait for an active exchange. Synchronous journal writes and `fsync` cannot be safely interrupted by cancellation, so a budget does not impose a hard bound on a durability operation already in progress.

Use `query(ServiceOperation::Membership(...), budget)` for membership reads. For every mutation, construct the typed `ServiceOperation` with a caller-stable `OperationId`, then call `submit`. The client syncs the exact request envelope, including the operation key and payload, before writing to the socket. `submit` returns the key and result on success. If `ServiceCallError::pending_operation()` returns a key, inspect it with `journal().inspect(&key)` and decide whether to reconnect and call `replay(&key, budget)`. `journal().pending()` lists unresolved keys after a process restart. Registration and queries do not create intent records. A call rejected while waiting for the session does not create a mutation intent or send bytes; a rejected replay leaves its saved intent intact. A `ServiceBusy` refusal leaves the intent pending under its key for every operation, v1 and v2 ones included: replay it under the same key.

```rust,ignore
let journal = ServiceIntentJournal::open(private_state_dir.join("service-intents"))?;
let client = PersistentServiceClient::new(socket_path, clock, instance, None, journal);
client.register(&budget).await?;
let key = OperationId::new(stable_operation_id);
let result = client.submit(ServiceOperation::EnsureThread(EnsureManagedThread {
    thread, topic, goal, operation: key.clone(),
}), &budget).await;
// On a pending error: retain key, reconnect/register, then explicitly replay.
```

## Sending, history and receipts (v2)

A registered service posts a message into one of its own managed threads with `client.send(ServiceSend { .. }, &budget)`. Sends are allowed on managed threads only (an ordinary thread, or a managed thread owned by another author, is refused). The message is an ordinary priority-0 message authored by the service ID, with no seat actor. Every joined member at commit time gets a delivery row that wakes it; only the explicit `recipients` (invited or joined, non-human) owe an ACK and get a receipt with the deadline. A send with no explicit `recipients` creates no receipts (`recipient_count` is 0 and `deadline_millis` is unused); an empty audience is refused (`InvalidRequest`). The service itself is never a recipient. `deadline_millis: None` uses the instance default receipt duration, and `Some(ms)` must be positive. `send` is journaled like any mutation: the operation key in `ServiceSend::operation` is the intent key.

```rust,ignore
let (key, sent) = client.send(ServiceSend {
    thread, body: "Do X, then ACK this message by its exact ID.".into(),
    recipients: vec![worker.clone()], deadline_millis: Some(600_000), operation: OperationId::new("req-1"),
}, &budget).await?;
// sent.recipient_count receipts (the explicit recipients) were created; read their live state with receipts().
let inspection = client.receipts(ServiceReceiptsQuery { message: sent.summary.message.clone(), page }, &budget).await?;
let replies = client.history(ServiceHistoryQuery {
    thread, page, initial: Some(HistoryRange::After { sequence: sent.summary.sequence }),
}, &budget).await?;
```

`MessageSent` is a bounded snapshot: the summary, the author, `recipient_count` and the receipt duration. It carries no per-recipient list. Read live state through `client.receipts(..)` (only for this author's own messages; each recipient is `Pending`, `Acknowledged` with the stored ACK provenance and time, or `Retired` with its retirement cutover) and read the thread through `client.history(..)`. Neither read is journaled, and neither needs a mutation intent.

`Conflict` is definitive: nothing was published (the thread's audience kept changing beyond the daemon's bounded restart allowance), so resubmit under a **new** key. A pending error (`pending_operation()` returns a key) is ambiguous: reconnect, register, and `replay` the same key; the daemon answers the committed result once and never publishes a duplicate. Design: [service-send-amendment.md](design/graph-system-identity/service-send-amendment.md). D9 of the message-priority design (2026-10-14) supersedes that amendment's whole-audience obligation rule.

## Clone identity (v3)

`service_session_v3` adds five operations to v2, for a service that launches agents and keeps their seats across restarts. Registration is the only advertisement: an old daemon rejects the name with "unsupported service capability", and there is no fallback. A session below v3 gets `Unsupported` naming `service_session_v3` for each of them. `PersistentServiceClient` registers v3: `submit` carries the four mutations under their `operation` keys, journaled and replayed like any mutation, and `query` carries `ContinuityAlerts`. Spec: [graph clone identity](superpowers/runs/2026-11-04-graph-clone-identity/2026-11-04-graph-clone-identity-design.md), §4.2 to §4.5.

A key is `HERDR_THREADS_CONTINUITY_KEY`, 1 to 128 ASCII-graphic bytes, in the environment of the agent the service launches. An invalid value counts as absent. An unreserved key is ignored. The key is local to the node and never replicated. A key is cooperative evidence, not a secret or a proof of the caller.

Mutations carry `operation` and replay as above: the same key and payload return the first result, a different payload is `OperationPayloadMismatch`, and a refusal journals nothing.

| Operation | Result | Refusals |
|---|---|---|
| `ReserveContinuityKey { key, seat: None, operation }` | `ContinuityKeyReserved { key, owner, seat, reserved_at }`; idempotent for the owner, and `seat` is the bound seat if there is one | `Conflict`: another service's key |
| `ReserveContinuityKey { key, seat: Some(s), operation }` | The same, with the key bound to `s` at once (`bound_by: service_reservation`). Moves nothing and places nothing. A re-reserve to `s` is a no-op. For a clone already running, or already waiting, with no key | `Conflict`: another service's key, the key bound elsewhere, or `s` holding another key. `NotFound`: unknown seat. `InvalidRequest`: `s` is retired, named, remote or not on a Herdr host |
| `ReleaseContinuityKey { key, operation }` | `ContinuityKeyReleased(key)`; deletes the key and its conflict rows | `Conflict`: another service's key, or a seat holds it. `NotFound`: not reserved |
| `UnretireSeat { seat, operation }` | `SeatUnretired { seat, generation }`; the seat reads `unresolved` with no target, its memberships and requirements are intact, and each pending deadline moves forward by the time that obligation spent frozen (from its creation or the retirement, whichever is later, to the un-retire) | `Unauthorized`: the caller does not own the seat's key, or the key is gone. `NotFound`: not reversibly retired. `ServiceBusy` (retryable, not definitive): the node's own history is catching up and nothing was applied; retry under the same operation key (`replay`: the client keeps the key pending) |
| `FinalizeRetirement { seat, operation }` | `RetirementFinalized(seat)`, for a reversible or unretired seat; releases the key and its conflict rows, and the retirement job settles obligations | `Unauthorized`: as above. `NotFound`: neither reversible nor unretired. `ServiceBusy`: retryable, as above; nothing was applied, retry under the same operation key |
| `ContinuityAlerts { page }` | `ContinuityAlerts(Page<ContinuityAlert>)`; a query: no operation key, nothing journaled | `InvalidCursor`, `InvalidBudget` |

A seat binds to a key at the check-in of an agent presenting it (`bound_by: check_in`) or at a reservation naming the seat. One key has one seat, and one seat one key. A `Closed` retirement of a key-holding seat is reversible: it reads `retired`, nothing is settled, and only what it owes freezes. A keyless seat's retirement stays final. An operator's `seat retire` of a reversible seat finalizes it.

**Required order.**

1. `ReserveContinuityKey` before launch. An unreserved key does nothing.
2. `UnretireSeat` before relaunching a reversible seat's agent. Otherwise the agent stays seatless under a `key_retired_reversibly` conflict and its pane is held.
3. `FinalizeRetirement` when the undo window ends. Threads has no timer: the service owns the window, and a service that disappears leaves its seats retired-reversible, their obligations frozen, until an operator retires them.

A launcher that needs the seat at once starts a harness whose hook waits for a first turn (Codex TUI) with a prompt: with no turn the agent presents no key while idle, and it reattaches on its first turn. A hookless plain CLI caller never reattaches by key.

**Conflicts.** A refused or noted claim records one row per pane. The hook still succeeds, with no new model-visible text, and a later event presents the key again.

| Kind | Meaning | Holds the pane |
|---|---|---|
| `key_live_elsewhere` | The key's seat is resolved at another pane | yes |
| `key_retired_reversibly` | The key's seat is reversibly retired; unretire it first | yes |
| `session_matches_other_seat` | The native session matches another seat | yes |
| `pane_owned_by_other_seat` | The pane has another resolved owner, which keeps working | no |
| `seat_holds_other_key` | The pane's resolved owner holds another key, and this key stays unbound | no |

A holding row refuses ordinary resolution of a pane with no resolved owner, so no new seat appears there. It clears when a later claim from the pane succeeds, when a coherent publication no longer shows the pane, when an operator rebinds or creates a fresh seat on it, or when its key is released. An alert-only row clears when its owner stops being the pane's resolved owner. The table holds 256 rows; past that a claim is still refused but nothing is recorded, so that pane has no hold.

**Alert paging.** `ContinuityAlerts` pages a pull feed, oldest first, with one row for each conflict row that is new or whose key or kind changed: `{ ordinal, key, seat, host, pane, kind, at }`. `seat` is the key's seat, absent for an unbound key. The log keeps the newest 256 rows, and ordinals keep increasing across eviction, so a gap means rows were evicted. Follow `next_cursor` while `has_more`. A cursor is frozen at its first page's `high_water_ordinal`, and the last page has no cursor, so poll from no cursor and skip ordinals already seen. `next_argv` is always absent: the feed has no CLI.

**`SeatInspection` fields.** Both are absent when empty.

- `continuity_key: { key, owner, bound_by, conflict }`: the key this seat holds. `bound_by` is `check_in` or `service_reservation`. `conflict: { host, pane, kind, first_seen_at }` is the most recently updated row naming the key, of any kind. A row naming an unbound key surfaces only in the alert feed. Text output prints `key <key> <owner label>` when a key is held (the label is the owner id before its first `:`, `graph` for `graph:<64 hex>`); `--json` has every field.
- `retirement_state`: `reversible`, present iff the seat is reversibly retired; absent for a seat that is not retired or is finally retired (an older client decodes it unchanged). `summary.continuity` stays `retired` for a reversible seat, and `retirement` (the job status) is `None` until it is finalized. After `UnretireSeat`, `summary.continuity` is `unresolved` with `target: None`.

## Replay and completion

The caller chooses when to replay after an ambiguous outcome. Replaying reads the saved envelope and sends its original request ID, operation key, and payload. A different payload under an existing key cannot replace the saved intent. A definitive daemon response first publishes a durable `.complete` record containing the saved envelope and full correlated response, then removes the `.intent` file. The completed record remains until the caller explicitly calls `journal().forget_completed(&key)`. `journal().completed()` and `journal().inspect_completed(&key)` expose definitive results after restart; `journal().pending()` lists only unresolved intents. Neither reopen nor registration replays automatically, and a completed key cannot be replayed or reused for a different payload.

If local completion fails after a definitive response, `ServiceCallError::Completion` carries the key, typed definitive result or daemon error, and local I/O error. `definitive_completion()` gives access to the key and typed result. Before the completion record is durably published, the original intent remains available. Once the completion record exists, it is authoritative even if intent unlink or directory sync fails; call `journal().retry_cleanup(&key)` to retry that cleanup. Inspect completed state before deciding whether any mutation needs replay. `forget_completed` cleans any retained intent before removing the completed record. Unresolved intents do not expire automatically.

The ordinary `LocalSocketClient` remains a one-shot client. Ordinary CLI invocations close their socket and cannot retain service authority for a later process. This API is for a resident programmatic caller that owns its registration and private intent directory. Service registration is a cooperative same-UID claim, not proof that the process is graph; coordinate ownership and use generation-targeted operator recovery for a hung live connection.
