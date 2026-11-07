# Final review, cluster 29 (tests-support)

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified with `git rev-parse --verify`. Caller checkout observed clean at the head throughout. Input: `slices/000167.diff`, the single 14-line addition to `tests/support/peer_wire_freeze.rs`.

## Analysis

### What the change does

It adds `UNCHAINED_KINDS`, listing `seat.retired_reversible` and `seat.unretired`, so `generator_kind_table_covers_builtins` still accounts for all 15 builtins (13 in `KINDS`, 2 here) while the envelope chain stays equal to the 2.1.1 golden.

### The overlay contract

The generator file is not an ordinary test helper. `scripts/freeze-peer-wire` copies it from the current checkout into a worktree of `--at <rev>` and builds it against that revision's crate (script lines 74-87). The file's own header (lines 4-14) states the consequence: it must compile against every release that has a fixture directory, and "Never use an API added after the oldest fixture release".

`UNCHAINED_KINDS` names `seat::SeatRetiredReversible` and `seat::SeatUnretired` with no cfg gate. Neither symbol exists at any released tag: `git grep 'SeatRetiredReversible\|SeatUnretired' v3.0.0 -- src tests` returns nothing, and `src/sync/facts/seat.rs` at v2.0.4 and v3.0.0 has zero matches. An unresolved type path in a `const` initializer is an unconditional compile error, so the overlay fails to build at v2.0.4, v2.1.1, v2.4.0 and v3.0.0, which are exactly the four committed fixture directories. This was established statically; the build was not run, because no warm `target/freeze-peer-wire` exists and a fresh build target would break the SSD rule for a result name resolution already fixes.

### Effect on the release process

The forward path survives: `RELEASE.md` freezes and checks at the candidate SHA and at the new tag, both of which will carry the kinds. What breaks is any revision before this merge: re-verifying an existing fixture directory at its own release, and freezing a maintenance patch cut from a pre-merge commit using a current checkout's generator. The generator also stops being a stable file for the first time as a compile-time matter, and the `combined` module cannot catch this, since it only compiles at head (the header's "so it cannot rot" covers head alone).

The fix is small. `UNCHAINED_KINDS` is consumed only by `generator_kind_table_covers_builtins` in `tests/sync/peer_wire_fixtures.rs`, which is head-only and never overlaid. Moving the constant there restores the contract and returns the generator to the bytes recorded in the 2.4.0 and 3.0.0 manifests (sha256 8c8fd574), with no change to any test's coverage.

### What was checked and holds

- The stated reason for not chaining is accurate: `current_encoders_reproduce_2_1_1_fixtures` asserts equal envelope and frame counts against the 2.1.1 golden, so a chained kind would fail it.
- The comment's claim that the payload round trip lives with the kinds' own tests holds: `tests/store/reversible_facts.rs` pins both payloads as exact JSON from the real transitions, and `tests/sync/facts_seat.rs` exercises the typed payloads.
- `write_fixture` and `kinds.txt` read `KINDS` only, so a future freeze at a candidate still emits a MANIFEST that agrees with its `envelopes.jsonl`, which `every_fixture_dir_decodes` requires.
- At a candidate that has the kinds, the overlaid `harness = false` target would see `UNCHAINED_KINDS` as unused: a dead-code warning, not an error, and gone with the move.

### Requirement baseline

Rows B1 to B4 have no production path in this cluster: the file is test support. The peer application of the two kinds (the cross-node half of B2) belongs to the sync and store clusters and the seam reviewer; nothing here adds or removes delivery.

## Verdict
not ready (UNCHAINED_KINDS in the overlaid freeze generator names post-3.0.0 types, so scripts/freeze-peer-wire no longer compiles at any existing fixture release)

## Must fix before landing
- tests/support/peer_wire_freeze.rs:103 `UNCHAINED_KINDS` references `seat::SeatRetiredReversible` and `seat::SeatUnretired`, violating the header rule at lines 9-13 ("Never use an API added after the oldest fixture release"); `scripts/freeze-peer-wire 3.0.0 --at v3.0.0 --check` (and the same for 2.0.4, 2.1.1, 2.4.0, or a maintenance patch from a pre-merge commit) overlays this file onto a tree without those types and fails with an unresolved-type compile error before generating anything; move the constant to `tests/sync/peer_wire_fixtures.rs`, its only consumer, which is head-only, and the generator returns to its recorded bytes.

## Untested scope
- No frozen envelope pins the canonical signed bytes of `seat.retired_reversible` or `seat.unretired`; the JSON payload shape is pinned in `tests/store/reversible_facts.rs`, but a canonical-encoding change to these kinds would pass every fixture test.
- `scripts/freeze-peer-wire <previous> --at <candidate> --check` stays byte-identical across this epic because the chain omits the kinds, so a release note's "no peer wire change" evidence from that command would not see the two new held kinds; `every_fixture_dir_decodes` and the 2.1.1 shadow decoder are equally blind to them, leaving `tests/sync/facts_seat.rs` and the multi-node apply tests as the only guard.
- The compile failure above was established by symbol search at the release tags, not by running the overlay build (no warm freeze target exists and a fresh one is a full rebuild).

## Deferred OK
- The freeze chain cannot take a new kind while `current_encoders_reproduce_2_1_1_fixtures` compares whole-chain counts; the task report records this as a follow-up, but no bead tracks it (`bd list` shows none), so it should get one when this lands.
- Stale wording once the kinds are unchained: the `KINDS` doc comment at tests/support/peer_wire_freeze.rs:42 ("Every builtin") and the `peer_wire_fixtures.rs` header ("every builtin fact kind is present") both overstate coverage; correct them alongside the follow-up or the move.
