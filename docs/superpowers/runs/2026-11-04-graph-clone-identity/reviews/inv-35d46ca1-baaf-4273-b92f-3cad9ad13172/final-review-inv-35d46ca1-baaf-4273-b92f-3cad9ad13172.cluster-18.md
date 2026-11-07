# Final review, cluster 18 (src-protocol), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both resolve. The caller checkout stayed at head with a clean tracked status for the whole review.

Files reviewed: src/protocol/authority.rs, capabilities.rs, commands.rs, ids.rs, output.rs, output_compact.rs, pagination.rs, results.rs, service.rs (slices 000078 to 000086).

## Spec 4.5 shape comparison

Every shape spec 4.5 settles for the graph session is present with the exact names and wire spellings.

- Operations: `ReserveContinuityKey { key, seat: Option<SeatId>, operation }`, `ReleaseContinuityKey { key, operation }`, `UnretireSeat { seat, operation }`, `FinalizeRetirement { seat, operation }`, `ContinuityAlerts(ServiceContinuityAlertsQuery { page })`.
- Results: `ContinuityKeyReserved(ContinuityKeyReservation { key, owner, seat, reserved_at })`, `ContinuityKeyReleased(ContinuityKey)`, `SeatUnretired { seat, generation }`, `RetirementFinalized(SeatId)`, `ContinuityAlerts(Page<ContinuityAlert { ordinal, key, seat, host, pane, kind, at }>)`.
- `ContinuityConflictKind` has the five snake_case names, pinned one by one in `clone_identity_results_round_trip`.
- `SeatInspection.continuity_key` and `retirement_state` are optional and absent when None, with `ContinuityKeyBinding` spelled `check_in` and `service_reservation`. The boxed field serializes as the struct (tests/cli/launch.rs:1423).
- `seat` on `ReserveContinuityKey` has no `serde(default)`, but serde derive reads a missing `Option` as None, and the round-trip fixture pins the explicit null.

## Risks traced and cleared

- `ContinuityKey` syntax. The `id!` macro enforces 1 to 128 ASCII-graphic bytes in both `parse` and `Deserialize`, which spec 4.2 requires, and `continuity_key_parses_only_graphic_ascii_up_to_128_bytes` covers the edges. A service operation with an invalid key is refused at decode.
- Keyless request bytes. `continuity_key` on `CheckIn`, `ResolveSeat` and `ContinuityCheckIn` carries `default` and `skip_serializing_if`, so a keyless request serializes as before; `continuity_key_is_absent_from_the_wire_when_none` pins all three. The gate is `SEAT_CONTINUITY_KEY_V1`, in `ADVERTISED`, read by src/cli/site_capability.rs.
- Version gate. `ServiceSessionLevel::required_by` is an exhaustive match. Its V2 arm is the same list as the removed `matches!` in src/daemon/transport/service_connection.rs (`Send`, `History`, `Receipts`), everything else old is V1, and the five new operations are V3. A session below the level gets `Unsupported` naming the capability, and the session stays connected (tests/daemon/service_session_capability.rs, tests/service/continuity_key_session.rs:606).
- Journaling split. `operation_key` is Some for the four mutations and None for `ContinuityAlerts`. `service_controls::operate` returns the query before it requires a key, as spec gap-fill "ContinuityAlerts" states, and the client's result-kind check at src/client/service.rs:951 covers all five pairs.
- Cursor scope. `CursorScope::ContinuityAlerts` is appended last in `SCOPE_CODES` (length 24, existing codes unmoved), as the append-only comment requires. `alerts_page` decodes under that scope with a fixed scope key and direction, and `Cursor` validation still refuses `after_ordinal > high_water_ordinal`. The scope-restricted `inbox` and `attention` cursor payloads refuse a new scope by default.
- `ErrorCode::ContinuityKeyPending`. It is in the `ALL` list and class table, and pinned in tests/protocol/error_class.rs. `cli::retry::is_continuity_refusal` includes it (src/cli/retry.rs:388), so `repair::continuity` does not keep the claim, which is the "definitive" behaviour of the spec 4.2 table. `api_exit_code` takes it through the `_ => EXIT_FAILED` arm with `TargetAlreadyOwned`, its nearest sibling.
- `KEY_CONFLICT_CAUSE`. Both producers (src/store/seats/continuity.rs:495 and src/store/seats/resolution.rs:222) go through `key_conflict_cause`, and src/cli/hook.rs:386 is the only consumer, so a conflict refusal stays quiet in the hook. `shlex` quoting covers the pane text.
- Version skew on `deny_unknown_fields`. `SeatInspection` denies unknown fields and now carries `retirement_state` for every retired seat. An older CLI would reject it, but src/daemon/lifecycle.rs:278 refuses a daemon whose software version differs from the CLI's, so that pair cannot form. `SeatInspection` is not on the peer wire.
- Peer wire. `scripts/check-wire-compat` run at head exits 0 (0 violations, 0 stale); the delta lines are the new `SeatRetiredReversiblePayload` and `SeatUnretiredPayload` fields, outside this cluster.
- Harness neutrality. `scripts/check-harness-neutral` run at head exits 0; nothing in the cluster names a harness.
- Provenance. `cooperative_continuity_key` and `structural_shell_continuity` are constants with TRUST-POLICY rows (lines 717 and 718), as AGENTS.md requires. `service_unretire` and `service_finalize` are defined at TRUST-POLICY lines 714 and 715. `SeatHistoryItem` carries binding and repair rows only, so no closed enum in results.rs has to learn these kinds.
- Text output. `state: retired (reversible)\n` is 28 bytes as its comment says, printed once, and a final retirement's output is byte-identical to before (tests/cli/human.rs:930 for compact and human). `key KEY OWNER\n` is 6 bytes plus the tokens as its comment says, and both tokens are graphic ASCII, so neither line can be split or injected (tests/protocol/output.rs:826, tests/cli/human.rs:913). The field loop skips both JSON fields for `SeatInspect` only.

## Requirement baseline rows at this cluster

- B1 (continuity key): the cluster's part is delivered: the variable's type, the capability, the three command fields, the service operations and results, the refusal code and the alert feed. The production path from the environment and service socket through the store leaves this cluster and is the seam reviewer's to trace. `service_controls::operate` has real arms for all five operations.
- B2 (pane close retires immediately, undo reverses): the cluster's part is delivered: `UnretireSeat`, `FinalizeRetirement`, `SeatRetirementState`, the inspect line and the v3 gate. The retirement and un-retire behaviour leaves this cluster and is the seam reviewer's to trace.
- B3 (agent exit keeps the seat and memberships): no protocol surface is involved.
- B4 (Herdr unchanged): nothing in this cluster touches Herdr.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `ServiceSessionLevel::required_by` has no test pinning the V1 arms `Invite`, `Membership`, `SetTopic`, `ReleaseRequirement`, `Archive` and `Reopen`: `session_level_maps_capabilities_and_operations` asserts `Send` and the V3 operations, and the session tests exercise only `EnsureThread` and `Notify` at V1, so moving one of these arms up would refuse a v1 client with no test failing. The arms are correct at this head by reading.
- `SCOPE_CODES` positions have no test pinning a cursor's wire code to its scope, so the append-only rule for `ContinuityAlerts` and its neighbours rests on the comment; a reorder would only surface as live cursors decoding under the wrong scope.
- This review executed only `scripts/check-wire-compat` and `scripts/check-harness-neutral`; no cargo test ran, because the review is read-only and the ledger records the full-suite measurement as the caller's, so the named tests are cited from reading them at head, not from a pass observed here.

## Deferred OK
- Default text `seat inspect` shows neither a key's `conflict` nor `bound_by`, only `key KEY OWNER`, and tests/protocol/output.rs pins that (`conflicted == held`); spec 4.7 limits the text output to this line and `--json` carries the rest, but an operator reading the default output has to know to ask for JSON or the alert feed to see a conflict on that seat.
- A conflict refusal is recognized by the prefix of its `detail` string (`KEY_CONFLICT_CAUSE`) rather than by an error code, so a reworded cause would turn a quiet refusal into model-visible hook text; the constant is shared by producer and consumer, and the matching CLI and daemon versions rule out skew, so a dedicated code is only worth doing if a third consumer appears.
- Spec 4.7 counts the `key` line as "4 bytes plus the key and owner" and names the state text `retired (reversible)`; the code prints 6 bytes plus the tokens and `state: retired (reversible)`, and the code comments state the true sizes, so only the spec's numbers are stale (the print site itself is already recorded as a deferred minor of ht-ma6i.10).
