# Final review: cluster 13 (src-daemon), epic ht-ma6i

## Review basis

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified as commits. The caller checkout stood at that head with a clean status at the start and at the last read; no change was observed.

Inputs: `slices/000069.diff` (`src/daemon/transport/service_connection.rs`), the digest, and the design at `docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-04-graph-clone-identity-design.md`. Other files were opened only to check a named risk, all read at the head SHA: `src/protocol/service.rs`, `tests/daemon/service_session_capability.rs`, `src/daemon/transport.rs`, `src/daemon/control.rs`, `src/service/live_gate.rs`, `src/client/service.rs`, `docs/programmatic-service-client.md`.

`scripts/check-wire-compat` and `scripts/check-harness-neutral` passed in the caller worktree (0 violations each). No cargo build or test ran here, to avoid testing the same code twice: the task 1 report records `session_capability_tests` passing on source tree 8df9c1832ec2, and the blobs of `service_connection.rs` (aebfa7755) and of the test file (d1cfbf1ac) at that tree are identical to those at the head.

## What the change does

The per-session gate moves from a boolean with a hard-coded list of three operations to `ServiceSessionLevel`, which the spec's key abstractions name as the owner of which session version each operation requires. The session keeps the level it negotiated at registration, and each operation is refused with `Unsupported` when `level < ServiceSessionLevel::required_by(&operation)`, with the session left open.

## Behaviour traced

- v1 session, v2 operation: the same refusal with the same detail, `operation requires service_session_v2`, and a test pins that string. The refusal happens before the handler, and the test pins that the handler saw nothing and the session stayed connected.
- v1 and v2 sessions, v3 operation: `Unsupported` naming `service_session_v3`, no handler call, session kept. Pinned for all five v3 operations on both levels.
- v3 session: admits every generation. The old `capability == V2` comparison would have refused v2 operations on a v3 session; the ordered comparison does not, and a test pins it across v1, v2 and v3 operations.
- The derived `Ord` follows declaration order and a unit test pins `V1 < V2 < V3`, so `level < required` cannot silently invert.
- `required_by` is an exhaustive match with no wildcard, so a new `ServiceOperation` variant fails to compile until it names a level. That meets the key-abstraction rule, and this file no longer lists operations.
- The eight v1 arms in `required_by` are exactly the operations the old gate let through on a v1 session, so no existing v1 client loses an operation.

## Risks checked and cleared

- Bypass of the gate: `serve_registered` is the only production path to `LocalService::service_operation` (`src/daemon/transport.rs:521`; `src/daemon/control.rs:285` only forwards to the domain), and the gate runs on every operation frame in the loop, so no service operation reaches the store unchecked.
- Unknown capability: `first.validate()` runs first and answers `Unsupported`, as before; `is_supported_service_capability` is defined through `from_capability`, so the two cannot disagree.
- Decoding before the gate: `ServiceWireRequest::decode` validates the operation before the level check, so a v1 session sending a v3 `ContinuityAlerts` with an invalid page drops the connection instead of getting `Unsupported`. That is the same treatment an invalid v2 `History` on a v1 session already had, and it needs a malformed request.
- Error text: the detail is built by `format!` from `required.capability()`, a static name with no caller input. It goes to the service client and is not model-visible, so the token rules add nothing here.
- Trust policy and harness neutrality: the cluster adds no provenance, seat decision or harness branch.

## Baseline trace within this cluster

- B1: the gate admits `ReserveContinuityKey`, `ReleaseContinuityKey` and `ContinuityAlerts` on a v3 session and hands them to the handler. Reservation semantics and presentation of the key leave this cluster for the seam reviewer.
- B2: the gate admits `UnretireSeat` and `FinalizeRetirement` on a v3 session and hands them to the handler. The earlier producer gap has closed on the threads side at this head: `PersistentServiceClient::register` sends `service_session_v3` (`src/client/service.rs:560`) and `result_matches` has arms for the v3 results (`:951` to `:960`). The rest of the row leaves this cluster for the seam reviewer.
- B3: nothing in this cluster changes what happens to memberships when an agent exits.
- B4: nothing in this cluster changes a Herdr call or shape.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Seam reviewer, B2 and B1 consumer: the herdr-graph checkout at `/var/home/alepar/AleCode/herdr-graph` has no `UnretireSeat`, `FinalizeRetirement` or `ReserveContinuityKey` in `src/threads/adapter.rs` or `src/ports/threads.rs`, and both still describe `service_session_v2` (`src/ports/threads.rs:37`, `src/threads/adapter.rs:555`); the epic is the threads side and this path leaves the cluster, so this review does not call either row undelivered.
- Seam reviewer, version skew: `PersistentServiceClient` now registers only v3, so a client built at this head against an older daemon is refused at registration for its v1 and v2 work too; `docs/programmatic-service-client.md` states there is no fallback, and whether every consumer can upgrade the daemon first lies outside this cluster.
- The `let Some(level) = ... else` branch in `serve_registered` has no test and cannot run today, because `first.validate()` refuses the same names first through the same function.
- The daemon test exercises only `EnsureThread` and `Notify` of the eight V1 arms, and the unit test asserts only `Send` and the five v3 operations, so a wrong level on `Invite`, `Membership`, `SetTopic`, `ReleaseRequirement`, `Archive` or `Reopen` would refuse those on v1 and v2 sessions and pass both tests; the arms are correct at this head by inspection.
- This review ran no cargo tests; the passing record is the task 1 report on an identical pair of file blobs, and the crate around them has changed since, so a run at this head comes only from the caller's full suite.

## Deferred OK
- The unreachable `let Some(level) = ... else` reply duplicates what `first.validate()` already answers, with the same code and text; it is a total conversion in place of a panic path in the daemon, its comment says why, and removing it would need a validated capability type at the wire boundary.
- The header comment of `tests/daemon/service_session_capability.rs` still describes only the v1 and v2 refusal, while the file now also covers v3.
