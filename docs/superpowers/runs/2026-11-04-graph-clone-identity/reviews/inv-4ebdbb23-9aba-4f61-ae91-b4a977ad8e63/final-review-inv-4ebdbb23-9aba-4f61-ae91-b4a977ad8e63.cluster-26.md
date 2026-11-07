# Final review, cluster 26 (tests-protocol), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7. Both SHAs verified with `git rev-parse --verify`. The caller checkout stayed at the head with a clean status throughout.

Inputs: `slices/000122.diff` (`tests/protocol/capabilities.rs`, +23), `slices/000123.diff` (`tests/protocol/error_class.rs`, +1) and `slices/000124.diff` (`tests/protocol/output.rs`, +44). All three are mounted into the library test binary (`src/protocol/capabilities.rs:282`, `src/protocol/output.rs:692`, `src/protocol/results.rs:2558`).

## What the cluster changes

`tests/protocol/capabilities.rs`:

- `capability_constants_are_stable` pins `SEAT_CONTINUITY_KEY_V1` to the literal `seat.continuity_key_v1`, the name spec 4.5 settles. The exact advertised lists there and in `bare_daemon_advertises_legacy_capabilities_without_v2_recorder` gain it at the position `ADVERTISED` has it (`src/protocol/capabilities.rs:228`).
- `every_advertised_capability_has_a_handler` gains a `SEAT_CONTINUITY_KEY_V1` arm, modelled on `SEAT_READONLY_V1`. It decodes a literal `check_in` JSON carrying `continuity_key`, checks the field, `validate()` and the round trip, then calls the handler.
- Three `continuity_key: None` lines on command literals. Mechanical.

`tests/protocol/error_class.rs`: `ContinuityKeyPending` maps to no class. This matches spec 4.2, where the claimless refusal is definitive and so not transient. `ALL.len() == EXPECTED.len()` forces the row, and the production mapping is `src/protocol/results.rs:2445`.

`tests/protocol/output.rs`: `seat_inspect_text_prints_one_key_line_when_a_key_is_held` renders the same fixture with no key, with a key, and with a key plus a conflict.

- The test requires the text with a key to equal the keyless text plus exactly one `key clone-1 graph\n`. It requires the conflict render to equal the key render. It also decodes `bound_by` in both spellings of spec 4.5 and the `key_live_elsewhere` conflict kind through `CommandResult`.
- Its three stated kills hold by reading `src/protocol/output.rs:478-511`. Dropping `continuity_key` from the skip list adds a JSON field and breaks the `replacen` equality. A conflict that changed the text breaks `conflicted == held`. A printed line with no key breaks `!without.contains("key")`.
- The fixture contains no other `key` substring, so that negative check is meaningful.

## Checks run

A review-owned detached worktree at the head, seeded by a copy-on-write clone of the `ht-ma6i.10` target and removed afterwards, ran `nice heavy tb cargo test --locked --all-features --lib -- protocol::capabilities protocol::output protocol::results::error_class_tests seat_inspect`: 83 passed, 0 failed, 7177 filtered out. That covers all five edited tests. No tracked file changed and no mutation was run. No full-suite run was made; the caller owns that.

## Reach of the capability arm

The arm's comment reads "the daemon refuses the claim (no such seat), not the field". `daemon_handler` at `tests/protocol/capabilities.rs:102` wraps `NoDomain`, whose `handle` returns `NotFound "no domain"` for every command (lines 88-95). So no claim is judged and the key is never read, and `assert_ne!(error.code, ErrorCode::InvalidRequest, ..)` cannot fail. The real checks in the arm are the decode, `validate()` and the round trip.

`SEAT_READONLY_V1` has the same shape at the base, so the arm follows existing practice. This is an overstated comment and not a coverage hole, because the behaviour is tested elsewhere:

- A real keyed `CheckIn` is decided in `src/store/seats/cooperative.rs:602,956,1048` and pinned by `first_check_in_binds_with_check_in_provenance` and its neighbours in `tests/store/continuity_key_decisions.rs:1254-1490`.
- The arm decodes only `CheckIn`. The wire presence, absence and round trip of the key on `ResolveSeat` and `ContinuityCheckIn` are pinned by `continuity_key_is_absent_from_the_wire_when_none` (`src/protocol/commands.rs:2033`).
- The other three conflict kinds' snake_case names are pinned by the tests in `src/protocol/service.rs:998-1010`.
- The retirement line, which this cluster's text test does not render, is pinned at `tests/cli/human.rs:958`. The JSON form of the two new `SeatInspection` fields is read at `tests/cli/launch.rs:1418-1430` and `tests/store/continuity_keys.rs:965-968`.

## Baseline rows and paths that leave the cluster

The spec's baseline is B1 to B4. This cluster holds test code only and delivers no row by itself.

- B1 (continuity key): the cluster pins the capability name and advertisement, the no-class refusal code and the `key` line. The path from the hook through the daemon to the C1 transaction leaves the cluster and is the seam reviewer's to trace.
- B2 (pane close reverses by undo): nothing here exercises it, beyond the text test skipping `retirement_state` through the shared branch. The seam reviewer traces it.
- B3 and B4: nothing here touches memberships on agent exit or Herdr.

`SeatInspection` being `deny_unknown_fields` while it gains fields is a skew question for the protocol source, already raised in `final-review-inv-4ebdbb23-9aba-4f61-ae91-b4a977ad8e63.cluster-13.md`, and is not reassessed here.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The literal wire name `continuity_key_pending` is pinned nowhere: `constructed_errors_keep_the_literal_wire_form` computes its expected string from serde (`wire_name`) for every code, so a rename of the variant's wire form passes, and no test or spec row holds the literal. Spec 4.5 lists no wire name for it and its consumers compare the enum, so the exposure is limited to a future rename.

## Deferred OK
- `tests/protocol/capabilities.rs:824` arm comment overstates its reach: `NoDomain` answers `NotFound` to every command, so the `assert_ne!` cannot fail and no claim is refused; the same is true of the `SEAT_READONLY_V1` arm it copies, and the keyed check-in is tested in `tests/store/continuity_key_decisions.rs`. A later cleanup could drop the handler call from both arms or say what it proves.
- Spec 4.7 gives the `key` line as "4 bytes plus the key and owner"; the rendered line is 6 bytes plus both, as `src/protocol/output_compact.rs:927` and this cluster's literal show. Spec wording only; the code doc is correct and the string is operator-facing.
