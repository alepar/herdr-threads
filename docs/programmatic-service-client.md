# Programmatic service client

`PersistentServiceClient` holds one registered Unix socket and serializes calls on it. Construct it with the instance UUID, socket path, a clock, an optional expected daemon boot UUID, and a private `ServiceIntentJournal` directory under an existing durable parent. Use a separate journal directory per instance and programmatic author. Call `register` before `query`, `submit`, `send`, `history`, `receipts`, or `replay`. `register` sends `service_session_v2`; there is no fallback to v1. A daemon that does not know v2 (an older build) refuses the register with "unsupported service capability", and the daemon advertises `service.send_v1` once it serves send and the two reads. A raw session registered with `service_session_v1` still runs the v1 operations but gets `Unsupported` for `Send`, `History` and `Receipts`. The returned author ID and connection generation are diagnostics; authority remains with the socket. A dropped or invalidated connection needs a new `register` call. An idle connection has no per-call deadline; each budgeted call checks its absolute deadline and cancellation before and after waiting for the session. `registration()` and `disconnect()` are administrative calls without a budget and may wait for an active exchange. Synchronous journal writes and `fsync` cannot be safely interrupted by cancellation, so a budget does not impose a hard bound on a durability operation already in progress.

Use `query(ServiceOperation::Membership(...), budget)` for membership reads. For every mutation, construct the typed `ServiceOperation` with a caller-stable `OperationId`, then call `submit`. The client syncs the exact request envelope, including the operation key and payload, before writing to the socket. `submit` returns the key and result on success. If `ServiceCallError::pending_operation()` returns a key, inspect it with `journal().inspect(&key)` and decide whether to reconnect and call `replay(&key, budget)`. `journal().pending()` lists unresolved keys after a process restart. Registration and queries do not create intent records. A call rejected while waiting for the session does not create a mutation intent or send bytes; a rejected replay leaves its saved intent intact.

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

A registered service posts an ACK-required request into one of its own managed threads with `client.send(ServiceSend { .. }, &budget)`. Sends are allowed on managed threads only (an ordinary thread, or a managed thread owned by another author, is refused). The message is an ordinary receipt-bearing message authored by the service ID, with no seat actor. Obligations follow native semantics: every joined member at commit time, plus any explicit `recipients` that are invited or joined; the service itself is never a recipient. `deadline_millis: None` uses the instance default receipt duration, and `Some(ms)` must be positive. `send` is journaled like any mutation: the operation key in `ServiceSend::operation` is the intent key.

```rust,ignore
let (key, sent) = client.send(ServiceSend {
    thread, body: "Do X, then ACK this message by its exact ID.".into(),
    recipients: vec![], deadline_millis: Some(600_000), operation: OperationId::new("req-1"),
}, &budget).await?;
// sent.recipient_count obligations were created; read their live state with receipts().
let inspection = client.receipts(ServiceReceiptsQuery { message: sent.summary.message.clone(), page }, &budget).await?;
let replies = client.history(ServiceHistoryQuery {
    thread, page, initial: Some(HistoryRange::After { sequence: sent.summary.sequence }),
}, &budget).await?;
```

`MessageSent` is a bounded snapshot: the summary, the author, `recipient_count` and the receipt duration. It carries no per-recipient list. Read live state through `client.receipts(..)` (only for this author's own messages; each recipient is `Pending`, `Acknowledged` with the stored ACK provenance and time, or `Retired` with its retirement cutover) and read the thread through `client.history(..)`. Neither read is journaled, and neither needs a mutation intent.

`Conflict` is definitive: nothing was published (the thread's audience kept changing beyond the daemon's bounded restart allowance), so resubmit under a **new** key. A pending error (`pending_operation()` returns a key) is ambiguous: reconnect, register, and `replay` the same key; the daemon answers the committed result once and never publishes a duplicate. Design: [service-send-amendment.md](design/graph-system-identity/service-send-amendment.md).

## Replay and completion

The caller chooses when to replay after an ambiguous outcome. Replaying reads the saved envelope and sends its original request ID, operation key, and payload. A different payload under an existing key cannot replace the saved intent. A definitive daemon response first publishes a durable `.complete` record containing the saved envelope and full correlated response, then removes the `.intent` file. The completed record remains until the caller explicitly calls `journal().forget_completed(&key)`. `journal().completed()` and `journal().inspect_completed(&key)` expose definitive results after restart; `journal().pending()` lists only unresolved intents. Neither reopen nor registration replays automatically, and a completed key cannot be replayed or reused for a different payload.

If local completion fails after a definitive response, `ServiceCallError::Completion` carries the key, typed definitive result or daemon error, and local I/O error. `definitive_completion()` gives access to the key and typed result. Before the completion record is durably published, the original intent remains available. Once the completion record exists, it is authoritative even if intent unlink or directory sync fails; call `journal().retry_cleanup(&key)` to retry that cleanup. Inspect completed state before deciding whether any mutation needs replay. `forget_completed` cleans any retained intent before removing the completed record. Unresolved intents do not expire automatically.

The ordinary `LocalSocketClient` remains a one-shot client. Ordinary CLI invocations close their socket and cannot retain service authority for a later process. This API is for a resident programmatic caller that owns its registration and private intent directory. Service registration is a cooperative same-UID claim, not proof that the process is graph; coordinate ownership and use generation-targeted operator recovery for a hung live connection.
