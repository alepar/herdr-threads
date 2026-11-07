# Final review: cluster 9 (src-daemon), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified with `git rev-parse --verify`. The caller checkout was at the head and clean when read. No review-owned worktree was created; everything was read through `git show` and `git diff`.

Cluster input: `src/daemon/transport/service_connection.rs`, slice `slices/000034.diff` (24 insertions, 13 deletions).

## What the change does

It replaces the `session_v2` bool and the `matches!` list with `ServiceSessionLevel`, the abstraction spec section "Key abstractions" names for this file. The session records its level once at registration. Each operation is then compared with `ServiceSessionLevel::required_by(&operation)`, and a session below that level gets `Unsupported` naming the required capability.

## Checks against the spec

- Spec 4.3 "Service v3", new operations require `service_session_v3`: `required_by` is an exhaustive match with no wildcard (`src/protocol/service.rs`), and all five new operations map to V3. A new variant cannot compile without an arm.
- Ordering: the derive on `V1, V2, V3` gives `V1 < V2 < V3`, pinned by `session_level_maps_capabilities_and_operations`. A v3 session therefore still serves v1 and v2 operations, which `v3_session_dispatches_every_generation` pins.
- v1 and v2 behaviour is unchanged: the refusal text for a v1 session is still exactly `operation requires service_session_v2`, pinned by `v1_session_refuses_v2_operations_and_keeps_serving`. The refusal keeps the session connected, as before. `tests/service/graph_d2.rs`, which registers v1, names `EnsureThread`, `Invite`, `Notify`, `SetTopic`, `ReleaseRequirement` and `Archive`; it does not name `Membership` or `Reopen`, whose V1 arms this file's tests do not pin either.
- No silent fallback: an unknown name is refused by `first.validate()` with `Unsupported` before any session exists (`register_accepts_v1_v2_and_v3_only`). An old daemon's validate accepted only v1 and v2, as the removed comment in the slice records, so it rejects v3.
- The new `let Some(level) ... else` sits before `gate.register_session`, so its refusal leaves no registered session. It cannot run after `validate()`, and it answers with the same code and text as `validate()`, so it cannot change an outcome. It is dead but harmless, and it avoids a panic path in the daemon.
- The gate sits before `spawn_blocking` and the handler, so a refused operation reaches no store and writes no journal row. The tests assert an empty recorder.
- "Must not gate peer facts": this file only handles registered service connections, and peer links leave earlier in `src/daemon/transport.rs` through `peer_connection::hand_off`.
- Model-visible text: none added. The refusal text goes only to a registered service over its socket.

## Requirement baseline, within this cluster

- B1 and B2: this file is the transport hop for the operations they need. The traced path is `src/daemon/transport.rs:520`, `serve_registered`, the level gate, `handler.service_operation`, `src/daemon/control.rs:277`, `src/service/dispatch.rs:1977`, then `src/store/service_controls.rs`, which has real arms for all five operations and no remaining `clone_identity_not_served`. On a v3 registration this hop delivers. Who registers v3 in production leaves this cluster; see the seam item below.
- B3 and B4: nothing in this file bears on them. B4 holds here: the file makes no Herdr call.

## Test evidence

Not re-run, to avoid duplicating the owners' runs. `git diff 8945c48ca 0755e609d` over `service_connection.rs` and `tests/daemon/service_session_capability.rs` is empty, and `git log 8945c48ca..0755e609d -- src/protocol/service.rs` lists nothing. The owners recorded passes of these tests: `.superpowers/sdd/ht-ma6i-plan/task-1-report.md` lines 20-23 and `task-6-report.md` line 77. Those runs were on earlier trees, not this head. The caller owns the full-suite run on the head, per the ledger.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Seam reviewer, B1 and B2 producer path, which leaves this cluster: no repository client registers v3 at this head; `PersistentServiceClient` registers v2 (`src/client/service.rs:560`) and its `result_matches` knows no v3 result (`src/client/service.rs:906`), while the only production graph adapter read here is built on that client (`herdr-graph/src/threads/adapter.rs:1`, working tree) and `LiveServiceGate::register_session` refuses a second session with `ServiceBusy` "a service connection is already registered" (`src/service/live_gate.rs:64`), so the raw v3 socket that `docs/programmatic-service-client.md:40` prescribes cannot run beside that adapter and must replace it, journal included; please judge whether this counts as delivery of B1 and B2 or needs v3 in the client.
- `service_connection.rs` and its test file were not re-run at this head; the evidence is the owners' passes on earlier trees plus empty diffs for these files since 8945c48ca, and the caller's full suite is the check at this head.

## Deferred OK
- `let Some(level) = ServiceSessionLevel::from_capability(capability) else` in `service_connection.rs:192` is unreachable after `first.validate()`; it is harmless and the alternative is a panic path in the daemon.
- `serve_registered` keeps the existing rule that an in-session frame failing `ServiceWireRequest::decode` closes the connection, so a v3 frame carrying an invalid `ContinuityKey` drops the session rather than getting a refusal; this matches every other operation's validation failure, and a caller that builds keys with `ContinuityKey::parse` never sends one.
- `reserved_author_id(&instance)` gives every registered session on an instance the same author, so "refused for another service" (spec 4.2) and `Unauthorized` on `UnretireSeat` are not reachable by a second live service, and `another_author_is_refused` seeds its rival author by raw SQL; this fits the cooperative single-service model, so it needs no change here.
