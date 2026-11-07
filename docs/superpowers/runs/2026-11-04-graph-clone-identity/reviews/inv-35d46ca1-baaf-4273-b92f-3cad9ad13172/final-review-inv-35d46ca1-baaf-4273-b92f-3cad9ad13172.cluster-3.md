# Final review, cluster 3 (docs), epic ht-ma6i

## Source and method

Base cf13ed76c1c2871b321ad290d03229cb785c400e and head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout read clean at the head during this review, and the review source guard bound successfully before this file was written.

The cluster is one file, `docs/programmatic-service-client.md` (slice 000004, +47/-1). Two commits touch it in the range: e1e2d27d1 (ht-ma6i.9) and a28bf497b (ht-ma6i.20). This is a reading review of the document against the spec and the code at the head. No test, daemon or client was run.

## Claims that match the head

Each of these was read in the code at the head.

- Registration and gating: `PersistentServiceClient::register` sends `SERVICE_SESSION_CAPABILITY_V3`. `ServiceSessionLevel::required_by` puts the five operations at V3, and `service_connection.rs:268` answers `Unsupported` with "operation requires service_session_v3".
- Client surface: `submit` and `query` route the four mutations and `ContinuityAlerts`, and `result_matches` has arms for all five results.
- `ReserveContinuityKey`: `continuity_keys::reserve` and `require_bindable_seat` match both rows, including the `seat` returned for a bound key, the no-op re-reserve, and the `Conflict`, `NotFound` and `InvalidRequest` cases.
- `ReleaseContinuityKey`: `continuity_keys::release` and `delete_key` match the row.
- `UnretireSeat` and `FinalizeRetirement`: `require_key_owner` answers `Unauthorized`, including when the key is gone. `no_retirement` answers `NotFound`, and `reversible::unretire` and `finalize` match the results.
- `ServiceBusy`: the ht-ma6i.20 change keeps the key pending (`src/client/service.rs:878`), and `service_busy_stays_pending_and_replays_under_the_same_key` pins it against a stand-in listener. This closes the prior final review's refusal-table item.
- Key syntax: the `ContinuityKey` parse tests pin 1 to 128 ASCII-graphic bytes.
- Conflict kinds and holds: `hold_kind` holds on exactly the three kinds the table marks yes. `current_resolution_owner` refuses only a free pane.
- Clearing paths: `rebind_unresolved_seat` clears a pane's row on a successful claim or an operator rebind. The operator fresh-seat branch now clears it too (`src/store/seats/operator.rs:297`), which closes the ht-ma6i.8 gap. `clear_conflicts_absent_from` clears on a coherent publication, and `delete_key` clears on release.
- Caps and alerts: both 256 caps are constants. `record_conflict` returns `TableFull` and writes nothing past the cap, and `record_stashed_conflict` leaves the refusal standing.
- Alert feed: `alerts_page` matches the paging text (oldest first, frozen high water, no cursor on the last page, `next_argv: None`). The `AUTOINCREMENT` in `migrations/node/0016_graph_clone_identity.sql` backs the ordinals claim.
- `SeatInspection`: both fields use `skip_serializing_if`. `reversible::retirement_state` is absent unless the seat reads retired. `inspect_key_line` prints `key <key> <owner>`.
- The spec link and its section references resolve.

## Why line 44 is wrong for this client

Line 44 reads: "Mutations carry `operation` and replay as above: the same key and payload return the first result, a different payload is `OperationPayloadMismatch`, and a refusal journals nothing."

That was accurate when the section told v3 callers to frame `ServiceWireRequest` themselves, because it describes the daemon's `operations` table. ht-ma6i.20 replaced that advice with line 40 ("`submit` carries the four mutations under their `operation` keys, journaled and replayed like any mutation") and left line 44 unchanged. Read against the client the section now prescribes, all three clauses are wrong:

- `submit` records an intent before any bytes go out, so a refused call is journaled.
- The failure branch of `send_saved` then calls `self.journal.complete(key, definitive_response)` for every definitive refusal except `UnknownOutcome` and `ServiceBusy`, so a refusal leaves a durable `.complete` record.
- A second `submit` under that key fails in `ServiceIntentJournal::record` (`src/client/service.rs:172`) with `ServiceCallError::Journal` and `AlreadyExists`, "operation key already completed". The same happens after a success, so the same key and payload do not return the first result through `submit`. The result is in `journal().inspect_completed(&key)`, and `replay` answers `InvalidRequest` "operation already completed" (pinned at `src/client/service.rs:2176`).
- A different payload under a key with a live intent fails locally with `AlreadyExists` (pinned at `src/client/service.rs:1293`), so that caller never sees `OperationPayloadMismatch`.

The "Replay and completion" section at line 86 states the client behaviour correctly, so the document now contradicts itself, and the wrong statement sits in the section herdr-graph codes against.

The failure is reachable through the document's own flow. A service calls `UnretireSeat` before the daemon has observed the close and gets `NotFound`. Line 44 says a refusal journals nothing and the general text asks for caller-stable keys, so the service retries under the same key. Every retry then fails with a local journal error until it calls `forget_completed` or changes key, and the undo does not happen. The failure is loud and recoverable, and the fix is one sentence.

## Requirement baseline trace

The spec has a `## Requirement baseline` with rows B1 to B4 and no human scope change. This cluster describes a surface and delivers no row by itself.

- B1: the service half has a production path, `PersistentServiceClient::submit` to `serve_registered` to `service_controls::operate` to `continuity_keys::reserve`. The agent half (a hook presenting the key, then the deciding transaction) leaves this cluster and is the seam reviewer's to trace.
- B2: `UnretireSeat` follows the same path to `reversible::unretire`, and the required order at line 57 matches spec section 4.3. Pane close reaching `begin_retirement_authored`, and the relaunch that reattaches, are the seam reviewer's to trace.
- B3: the spec records no threads change, and the document contradicts nothing here.
- B4: the document touches no Herdr surface, and its launcher guidance asks for none.

No row is shown undelivered by this cluster. The line 44 defect is a contract inaccuracy, not a missing behaviour.

## Verdict
not ready (line 44 of the service client document misstates replay and refusal journaling for PersistentServiceClient)

## Must fix before landing
- `docs/programmatic-service-client.md:44` says the same key and payload return the first result, a different payload is `OperationPayloadMismatch`, and "a refusal journals nothing", which is the raw daemon behaviour and is false for the `PersistentServiceClient` that line 40 now prescribes: `send_saved` completes every definitive refusal into a `.complete` record and `ServiceIntentJournal::record` (`src/client/service.rs:172`) then fails any new `submit` under that key with `AlreadyExists`, so a service that retries a `NotFound` `UnretireSeat` under its stable key gets a local journal error on every attempt and the undo does not happen; state instead that a definitive refusal completes the key locally, so a retry takes a new key, that a finished key's result is read with `journal().inspect_completed(&key)`, and that only `ServiceBusy` and ambiguous outcomes stay pending for `replay`.

## Untested scope
- The document has no executable check, and no test names it; every claim here was verified by reading the head, and nothing was run for this review, including the line 44 retry sequence, which rests on the code and the two pinned tests cited above.
- `ServiceBusy` on `UnretireSeat` and `FinalizeRetirement` through `PersistentServiceClient` is pinned only against a stand-in listener, not a real daemon session (already recorded for ht-ma6i.20).
- The `Conflict` and `Unauthorized` rows for another service cannot be produced over the wire, because every session registers `reserved_author_id(&instance)` (`src/daemon/transport/service_connection.rs`), so those rows rest on store-level checks.

## Deferred OK
- Line 75 says an alert-only row clears when its owner stops being the pane's resolved owner, but `clear_alert_only_if_owner_changed` has one caller (`src/store/seats/cooperative.rs:1053`), so the row stands until the pane's next check-in, a rebind or a publication without the pane; it holds nothing and only leaves a stale `conflict` on `SeatInspect`, so reword the line or clear eagerly (carried from the prior final review, still present).
- Line 53 lists `InvalidCursor` and `InvalidBudget` for `ContinuityAlerts`, while `alerts_page` answers `InvalidRequest` for an invalid page (`src/store/continuity_keys.rs:488`); a typed caller only reaches this through a programming error (recorded for ht-ma6i.9, still present).
- The general pending text at lines 5 and 36 calls a pending error ambiguous and does not name `ServiceBusy`, where nothing was applied and the session stays; the table rows at lines 51 and 52 cover the two v3 operations, but the client change applies to every journaled mutation (recorded for ht-ma6i.20).
- "Another service" has no production meaning today because every session shares one author, so a second service would silently share keys; the document should say so before a second service appears.
- The required order at line 57 does not say that `UnretireSeat` before the daemon has observed a close answers `NotFound`, or to wait for `retirement_state`; one sentence covers it, and it is the likeliest route into the Must fix retry.
- The section does not say a seat whose restored target never returns also retires reversibly after 10 observed minutes (`src/store/seats/observation.rs:2953`, spec section 4.4), that an operator retire of a live key holder is final, or that threads' own `launch` and `handoff` scrub the variable; the linked spec covers all three.
- The section names `SeatInspection` without saying where a service reads it: `PersistentServiceClient` has no such operation, so a service uses the native `SeatInspect` command; a clause would save the reader a search.
- Text output also prints `state: retired (reversible)` (`src/protocol/output_compact.rs:937`), which the text-output sentence at line 81 does not mention; `--json` carries `retirement_state`, so nothing is wrong.
