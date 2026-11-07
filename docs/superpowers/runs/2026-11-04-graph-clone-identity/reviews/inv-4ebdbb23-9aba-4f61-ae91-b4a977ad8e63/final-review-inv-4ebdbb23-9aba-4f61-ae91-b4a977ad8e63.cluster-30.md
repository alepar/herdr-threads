# Final review, cluster 30 (tests-sync), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7. Both resolve. The caller checkout stayed at the head with a clean status throughout.

Cluster files: `tests/sync/facts_seat.rs`, `tests/sync/facts_seat_reversible.rs`, `tests/sync/peer_wire_fixtures.rs` (slices 000168 to 000170).

## Evidence gathered

A review-owned detached worktree at the head, seeded copy-on-write, ran `cargo test --locked --all-features --test combined -- facts_seat_reversible seat_clone_identity held_kinds_list peer_wire_fixtures`. All 21 tests of the cluster passed. `tests/combined.rs` registers the new module, so these tests run in the suite.

One probe test, added only in that worktree, failed as predicted. It is the basis of the one landing item below. The probe is a copy of the mixed-version test's setup, with one change: the invitation is created after the un-retire instead of before the freeze.

```rust
rev(&home, &seat, T1);
unret(&home, &seat, T1, T1 + WINDOW);
sync_all(&[&home, &knows, &old]);                 // old parks both
let invitation = home.invite_fact(&thread, &creator, &seat);
sync_all(&[&home, &knows, &old]);                 // old and knows hold the same deadline
// upgrade `old` through open_restored + replay_startup, as the shipped test does
assert_eq!(invitation_row(&upgraded, &invitation).1, invitation_row(&knows, &invitation).1);
// left: 1700000120000, right: 1700000060000: the upgraded node shifted it by WINDOW
```

## Why the probe fails

`unretire_by_fact` calls `shift_pending_deadlines`, which moves every pending deadline the seat owes on that node, with no notion of when the obligation appeared. On a node that knew the kinds, the shift runs at the un-retire, so an obligation created later is never touched. On a node that parked the fact, the shift runs at upgrade, over everything still pending, including obligations created after the un-retire on the home node's timeline.

The shipped test `a_registry_without_the_kinds_parks_them_and_converges_after_replay` cannot see this, because in all three histories its only obligation is authored before the first step. Its "double-shifts" kill claim holds for that shape only.

This contradicts two statements. Spec 4.3 says an upgraded peer replaying parked facts in order "lands correctly". `TRUST-POLICY.md` line 2179, which is normative, says it "lands in the state of a node that knew them", with a single exception that does not cover this case.

Impact is narrow: mixed versions only, once per un-retire replayed at upgrade, and it delays a warning rather than settling anything wrongly. The window has no timer, though, so the delay is as long as the service kept the seat frozen. The same class exists without parking (a window fact shifts by the whole window, recorded in the task report), so documenting it would be coherent. It still has to be written down, because AGENTS.md requires an accepted limit to reach TRUST-POLICY in the change that adds it.

## Review of the rest of the cluster

- Spec fit. The tests cover each cross-node rule of 4.3 and the ht-ma6i.7 acceptance rows that belong here: peer reversible with no job, peer unretired restoring `remote` and shifting, final on a reversible row with `retired_seq` advanced, out-of-episode no-ops on peer and local rows, reversible on a finally retired row, own-log replay, non-home origin `failed`, window message and invitation on home and peer, peer refusals with a post-un-retire control, and the parked and replayed histories. Store-side rows (authoring, restore hold, keyless restored seat) are in `tests/store/reversible_facts.rs`, outside this cluster.
- Starvation test. Its claim is sound: `receipt_due_sparse` and the invitation scan both page by ordinal, the frozen rows are authored first, and the page size is 1, so dropping the frozen fragment would use the page on a row that yields no warning.
- Payload tests. The JSON shapes are pinned exactly and an additive field is tolerated, as the wire-compat rule asks.
- `peer_wire_fixtures.rs`. The table check now counts `KINDS` plus `UNCHAINED_KINDS` against the builtins, so a new unregistered or unlisted kind still fails. The two new `decode_payload` arms are not exercised by any fixture yet.
- Project rules. Temp paths go through `TestIsolation` and no child processes are spawned.
- Baseline rows. B2's cross-node half is exercised here at the fact level, with one test going through the real fence and operator finalization. The path from a real pane close and from `UnretireSeat` into these facts leaves this cluster and is the seam reviewer's to trace. B1, B3 and B4 are not touched by these files.

## Verdict
not ready (a node that parked seat.unretired over-shifts obligations created after the un-retire at upgrade, against spec 4.3 and TRUST-POLICY line 2179, and the convergence test cannot see it)

## Must fix before landing
- Parked `seat.unretired` replay moves deadlines of obligations created after the un-retire (`unretire_by_fact` to `shift_pending_deadlines` in `src/store/seats/reversible.rs`), so the upgraded node differs from one that knew the kinds by the whole window (probe: 1700000120000 against 1700000060000), contradicting spec 4.3 "lands correctly" and `TRUST-POLICY.md` line 2179; either bound the replayed shift to obligations that existed by `unretired_at`, or add this exception to the TRUST-POLICY limit, and in both cases add this history to `a_registry_without_the_kinds_parks_them_and_converges_after_replay` so the behaviour is pinned.

## Untested scope
- The mixed-version test holds no receipt, only an invitation, although the `snapshot` doc comment speaks of "a receipt or invitation deadline"; a receipt's shift on replay is not compared, and the comment overstates what is checked.
- A window message from a sender that really parks the kinds: `old_node_message_stays_frozen_with_the_rest` uses a sender that knows the kinds and has not heard yet; the home-side apply is the same on the wire, but the sender's own view is not asserted.
- A logical (undriven) `prepared_recipients` receipt shifted on a peer: `obligations_on_peer` always drives the send, and only the home-node store test `deadlines_keep_remaining_time_including_logical_receipts_and_invitations` reaches that arm of the shared SQL.

## Deferred OK
- `UNCHAINED_KINDS` leaves the two new kinds with no frozen wire envelope and their `decode_payload` arms unused until a later freeze; already recorded in the ledger as [freeze-chain-gap], and a search of open beads for "freeze" and "unchained" found no follow-up bead for it, so one should be filed by whoever owns the run.
- `window_message_settles_through_a_real_close_and_operator_retire` seeds `continuity_keys` with raw SQL instead of a reservation; the fence and finalization under test are real and the store suites cover reservation.
- `non_home_origin_is_failed` iterates over `(kind, label)` pairs whose two values are identical; a plain list of kinds would do.
