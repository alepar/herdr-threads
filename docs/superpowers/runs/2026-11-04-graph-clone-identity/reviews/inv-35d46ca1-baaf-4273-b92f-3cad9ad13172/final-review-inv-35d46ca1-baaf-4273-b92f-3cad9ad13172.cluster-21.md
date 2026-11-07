# Final review, cluster 21 (src-sync), epic ht-ma6i

## Source and scope

Base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout read clean at the head throughout. The review source guard bound the invocation before this file was written.

The cluster is two files, slices 000116 and 000117: `src/sync/facts/mod.rs` (two registrations) and `src/sync/facts/seat.rs` (two payloads, two `FactKind` types, the module note). Other files were opened only to check the named risks below: `src/store/seats/publication.rs`, `src/store/seats/reversible.rs`, `src/store/control/retirement.rs`, `src/store/materialize.rs`, `migrations/node/0016_graph_clone_identity.sql`, `TRUST-POLICY.md`, and the test targets.

Evidence method: reading at the head, plus `scripts/check-wire-compat` (exit 0, 0 violations, 0 stale, both payloads' six fields in the baseline). No test or build was run here; the tests were read, not executed.

## Requirement baseline trace

The spec has a `## Requirement baseline`. Rows B1, B3 and B4 do not touch this cluster: B1 (continuity key) has no fact, keys being local by design; B3 is "no threads change"; B4 is met here since nothing in these files reaches Herdr. They are the other reviewers' to trace.

B2 ("Pane close retires immediately; graph undo reverses it") has a cross-node leg that lives in this cluster, and it is delivered from production entry points:

- Author, close: `begin_retirement_authored` (`src/store/control/retirement.rs`) authors `seat.retired_reversible` through `emit_if_published` for a `Closed` retirement of a key holder.
- Author, undo: `ServiceOperation::UnretireSeat` (`src/store/service_controls.rs:206`) reaches `reversible::unretire`, which authors `seat.unretired`.
- Author, final: `FinalizeRetirement` and the operator's `seat retire` reach `reversible::finalize`, which authors the ordinary `seat.retired` with the original `retired_at`.
- Apply: `daemon_registry()` returns `FactRegistry::builtins()` outside the test-only legacy swap, and both kinds are registered there, so a peer's real apply path dispatches them.

The chain from a pane close or a service request to those functions is the seam reviewer's; the leg inside this cluster has no test-only gap.

## Conformance to spec 4.3

Each spec statement about the kinds was checked at the head:

- Names, major and shape: `seat.retired_reversible{seat, reason, retired_at}` and `seat.unretired{seat, retired_at, unretired_at}`, both `MAJOR = 1`, one `FactKind` each in `sync/facts/seat.rs`, one `.with::<K>()` each. `seat_clone_identity_payloads_round_trip` pins the JSON and that an additive field is ignored.
- Version rules from the module header: no `deny_unknown_fields`; `creates` is `None`; `needs` is the seat key only, as `seat.retired`; payloads carry fact ids and origin times, no local positions; no SQL in `src/sync`, the handlers delegate to one materializer each.
- Author rule: `require_author` with `AuthorRule::SeatHome` runs first in both handlers, so a non-home origin is `failed` as `seat.retired` is (`non_home_origin_is_failed`).
- Held kinds: both names are in `HELD_KINDS` and the route check reads `K::NAME` over `referenced_seats()`, which returns the seat for both payloads; `held_kinds_list_matches_the_spec` pins the list.
- Reason column: `RetireReason` serializes to `closed` or `operator`, exactly the `reversible_retirements.reason` CHECK, so no wire value of the enum can fail the row insert.

## Replay and ordering risks traced

The named risk is a fact landing in a state its handler does not expect, since one origin's log is ordered but a node can park, replay, or receive a re-issued copy.

- Parked histories on a node without the kinds: rev/unret/rev, rev/final, rev/unret/final and an obligation after the un-retire all land correctly by reading the materializers: a reversible fact on a retired row is a no-op because `retire_row` updates only `state!='retired'`, and an unretired fact outside the open `reversible` row's episode is a no-op through `frozen_episode`. `a_registry_without_the_kinds_parks_them_and_converges_after_replay` compares the upgraded node against one that knew the kinds across all four.
- Own-log replay across a second episode: rebinding is not a fact, so a replayed local row reaches a second `seat.retired_reversible` while still holding its `unretired` row. No primary-key failure results, because `retire_row` deletes the `unretired` row inside the transition before `insert_fact_row` runs.
- Live echo on the authoring node: the fence and the un-retire transition both run before emission, so the echo finds a retired row or a row no longer `reversible` and changes nothing.
- Re-issued copies (N7): `episode_fact_applied` excludes the fact's own journal row, counts only `applied` rows, and is covered on a peer and on a rebound owner by `s9_reissued_reversible_episode_is_a_noop_on_a_peer` and `s9b_reissued_episode_is_a_noop_on_a_rebound_owner`. Its one false-positive window, a later episode starting in the same millisecond, is stated in its doc comment.
- Restore-window hold: a refused emission returns `ServiceBusy` through `?` before commit, so neither the fence nor the un-retire is half applied.

Nothing in these traces produced a defect in the cluster.

## Must fix before landing

- none

## Untested scope

- The two kinds have no envelope in the frozen peer-wire chain: `UNCHAINED_KINDS` in `tests/sync/peer_wire_fixtures.rs` records this as a decision because the chain must keep reproducing the 2.1.1 fixtures, so their wire shape is pinned by the payload round trip in `tests/sync/facts_seat.rs` and not by a recorded envelope or frame.
- This review ran no tests; `tests/sync/facts_seat_reversible.rs`, `tests/sync/facts_seat.rs` and `tests/sync/incarnations_scenarios.rs` were read at the head, and execution evidence is the caller's suite run.

## Deferred OK

- `src/sync/facts/mod.rs:35` still lists the `seat` module as `seat.created`, `seat.retired`; the diff added the two registrations but not this layout line, so it should name all four kinds at the next edit of that file.
- For the seam reviewer, plausible and not reproduced: a window message whose fact reaches a node after that node applied `seat.unretired` is not shifted there, because the shift ran before its receipt existed, so its deadline stays as sent while an earlier-applying node's copy moved by the window; TRUST-POLICY names only the clock-skew case of this divergence, and the path lives in `src/store/seats/reversible.rs`, outside this cluster.

## Verdict
ready
