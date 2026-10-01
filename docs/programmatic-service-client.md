# Programmatic service client

`PersistentServiceClient` holds one registered Unix socket and serializes calls on it. Construct it with the instance UUID, socket path, a clock, an optional expected daemon boot UUID, and a private `ServiceIntentJournal` directory under an existing durable parent. Use a separate journal directory per instance and programmatic author. Call `register` before `query`, `submit`, or `replay`. The returned author ID and connection generation are diagnostics; authority remains with the socket. A dropped or invalidated connection needs a new `register` call. An idle connection has no per-call deadline; each budgeted call checks its absolute deadline and cancellation before and after waiting for the session. `registration()` and `disconnect()` are administrative calls without a budget and may wait for an active exchange. Synchronous journal writes and `fsync` cannot be safely interrupted by cancellation, so a budget does not impose a hard bound on a durability operation already in progress.

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

The caller chooses when to replay after an ambiguous outcome. Replaying reads the saved envelope and sends its original request ID, operation key, and payload. A different payload under an existing key cannot replace the saved intent. A definitive daemon response first publishes a durable `.complete` record containing the saved envelope and full correlated response, then removes the `.intent` file. The completed record remains until the caller explicitly calls `journal().forget_completed(&key)`. `journal().completed()` and `journal().inspect_completed(&key)` expose definitive results after restart; `journal().pending()` lists only unresolved intents. Neither reopen nor registration replays automatically, and a completed key cannot be replayed or reused for a different payload.

If local completion fails after a definitive response, `ServiceCallError::Completion` carries the key, typed definitive result or daemon error, and local I/O error. `definitive_completion()` gives access to the key and typed result. Before the completion record is durably published, the original intent remains available. Once the completion record exists, it is authoritative even if intent unlink or directory sync fails; call `journal().retry_cleanup(&key)` to retry that cleanup. Inspect completed state before deciding whether any mutation needs replay. `forget_completed` cleans any retained intent before removing the completed record. Unresolved intents do not expire automatically.

The ordinary `LocalSocketClient` remains a one-shot client. Ordinary CLI invocations close their socket and cannot retain service authority for a later process. This API is for a resident programmatic caller that owns its registration and private intent directory. Service registration is a cooperative same-UID claim, not proof that the process is graph; coordinate ownership and use generation-targeted operator recovery for a hung live connection.
