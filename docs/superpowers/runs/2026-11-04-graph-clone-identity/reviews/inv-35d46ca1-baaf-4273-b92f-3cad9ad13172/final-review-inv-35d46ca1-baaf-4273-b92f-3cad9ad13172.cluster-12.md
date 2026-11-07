# Final review, cluster 12 (src-client), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. Cluster input: `src/client/service.rs` (slices/000068.diff). The caller checkout stayed clean at the head throughout. One commit changes this file on the branch: a28bf497b.

## What the cluster changes

`PersistentServiceClient` registers `service_session_v3` only, `result_matches` gains the five v3 pairs, and a `ServiceBusy` answer to a journaled mutation keeps the intent pending and the session open, where it used to complete as a definitive refusal.

## Checks against the spec

- §4.3 Service v3 (no silent fallback): `register` sends one capability and returns a refusal unchanged. `v3_refusal_is_typed_with_no_fallback` checks for no second frame and no second connection.
- §4.5 protocol shapes: each `result_matches` arm pairs the operation with the result the spec names. `operation_key` is `Some` for the four mutations and `None` for `ContinuityAlerts`, so `submit` and `query` route them as the spec says.
- Journal reads: `complete` and `inspect_completed` both use `result_matches`, so a completed v3 record reads back after a restart. `persistent_client_runs_the_v3_operations` pins this through `completed()` on a real daemon.
- Key syntax: `validate` has no `ContinuityKey` check, which is safe. `new` panics and `parse` and `Deserialize` refuse an invalid key, so an invalid key cannot reach the journal and poison `pending()`.
- Key abstractions, `store::seats::reversible` ("Callers must tell that last one apart"): delivered at `src/client/service.rs:878`, before the journal completion.

## Is "nothing was applied" true on the real path

The comment at line 876 holds for v3. `store::service_controls::operate` opens one `ServiceDecisionTransaction`, and the `operations` row is written only after the operation succeeds. `OWN_HISTORY_HOLD_DETAIL` is returned from `FactRoute::emit` inside that transaction, and `held_kinds_refused_while_hold_active` shows no journal row is written. So a replay under the same key gets a fresh decision. For `Send` I did not trace every transaction, but staying pending is the safe direction: if an earlier step had committed under the key, the replay returns that stored result.

## External consumer of the wider behaviour change

ServiceBusy-stays-pending applies to every journaled mutation, including v1 and v2 ones (noted by task 12). I read the only consumer outside this repo, `herdr-graph/src/threads/adapter.rs`. `call_error` records the pending key and maps the error to `ThreadsError::ServiceBusy`, which the reconciler retries. `mutate` and `replay_pending` then replay that key rather than minting a new one, so no duplicate mutation arises. No in-repo production code calls this client; only tests do.

## Requirement baseline, this cluster's part

- B1: the client side of key reservation is delivered. `submit` carries `ReserveContinuityKey` and `ReleaseContinuityKey` to `service_controls::operate`, exercised on a real daemon by `tests/service/reversible_session.rs`.
- B2: the client side of graph undo is delivered in the same way for `UnretireSeat` and `FinalizeRetirement`. What happens after the operation reaches the store belongs to the seam reviewer and the store clusters.
- B3, B4: this file has no part in them.

## Test evidence

I ran no tests, to avoid repeating the owner's work. Task 12's report records `cargo test --locked --all-features --lib client::service` passing on source tree 8f9299fe9ddc448b65a72a35591a2fbcc5ab13be, with RED output for three of the four new unit tests. The fourth, `v3_refusal_is_typed_with_no_fallback`, guards against adding a fallback and passed at RED by design. The ledger defers the full-suite measurement at the head to the caller.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `ServiceBusy` followed by `replay` is pinned only against a stand-in listener in `service_busy_stays_pending_and_replays_under_the_same_key`; no test holds a real daemon's own-history hold through `PersistentServiceClient`, so the daemon half rests on the store-level hold tests and the transaction trace above.
- A second `ServiceBusy` answering a `replay` has no direct test; it takes the same branch at `src/client/service.rs:878`, so the key should stay pending.

## Deferred OK
- Other transient refusals still complete as definitive at `src/client/service.rs:884`: `StoreBusy` and the daemon's `DeadlineExceeded "service store budget exhausted"` write a `.complete` record, so a caller with a stable key gets `operation key already completed` on `submit`. This predates the branch and the spec names only `ServiceBusy`; nothing was applied, so a new key decides safely. Follow-up: consider keeping these pending too, and note that graph's adapter maps `StoreBusy` to `Rejected`.
- The new tests use the file's existing `temp()` helper (`/tmp/hsc-<uuid>`, line 1021), not `test_support::scratch::temp_path` as the epic's constraints require; a panicking test leaves its directory behind, outside the leak ledger. Tests that pass remove it, and swapping the helper is a one-line follow-up.
- The general pending paragraph in `docs/programmatic-service-client.md` calls a pending error ambiguous and does not mention `ServiceBusy` as a source; the v3 table does state it. This is doc drift outside this cluster and misleads no caller into an unsafe action.
