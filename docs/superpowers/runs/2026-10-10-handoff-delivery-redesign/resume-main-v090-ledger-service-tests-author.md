# Main v0.9.0 ledger/service test author report

Status: QUIESCENT. Tests authored and statically checked; compilation and runtime validation belong to the parent. No RED/PASS runtime claim.

Checkout: `ROOT/.worktrees/upstream-composition-v090`.
Owned source paths: `tests/delivery/ledger.rs`, `tests/service/delivery_pull.rs`. No source edits elsewhere, Git index operations, commits, Cargo/build/test execution, daemon/process spawning, or subagents. Formatting used `rustfmt --edition 2024 --config skip_children=true` on precisely these two files.

## Decisions

- Read current normative TRUST-POLICY; retained same-user cooperation, canonical binding admission, session identity, connected-state semantics, receipt provenance and explicit physical confirmation boundaries.
- Coordinated final APIs with `/root/impl_watch_recovery`: BindingOffers/binding_mut/get use BindingId; PresentationInput and WatchReplayScope require binding_id; passive refresh/owner-live receive exact binding.
- Canonical test lookup matches the claim's seat, harness, native session and agent id (None maps to the canonical empty top-level DB key), with ended_at IS NULL. It never selects the latest binding of a seat or invents a seat-derived binding alias. Race callbacks capture that actual ID before moving into the callback.
- Adapted every D seat_mut/get, bounded-ledger constant, direct PresentationInput, replay scope, mod registration, passive refresh and owner-live call in the owned files. Existing full offer stamps, 256-slot capacity, full nonzero count domain/count-four causal race, work counts, zero-walk, literal codec and native outlook assertions remain.
- Main preserves the seat mapping generation on a new session. Two inherited D assertions now require equal generation plus distinct canonical bindings; the retained-offer case also proves the same-process predecessor is disconnected and cannot reconcile its old ledger. Publication progress queries select the actual new binding. Historical state snapshots across refresh select the original exact binding so a sibling row cannot replace the oracle.
- Main's digest_notice_offer now stores binding_id/seat_id/offered_ordinal, without duplicated generation/execution. The native frontier test joins that exact binding ID to canonical occupant generation/execution and retains the full positive ordinal advancement, real V2 notice, remaining requirement, actual Attention/controls and no-reoffer assertions.

## Added causal negative

`service::delivery_pull::tests::presentation_two_live_bindings_keep_offer_stamps_replay_and_carry_independent`

Uses real CheckIn for A and B, equal seat/mapping generation, distinct canonical bindings and distinct process fingerprints (existing second_binding helper proves both connected). Actual V2 pulls populate both ordinary ledgers at count one and allocate real priority invitation pages with withheld promoted-lazy carry. It checks complete original stamps, distinct tokens, exact retained consumer rows, carry identity/prefix, A replay after B's allocation, rejection of A's token under B, rejection of A's due snapshot under B, and rejection of a foreign-binding owner hint. Explicit current-token/control confirmations preserve the sibling state and leave ordinary receipts, lazy obligations and all 25 invitations pending. It adds no timing policy, fake canonical row, or production seam.

The existing snapshot validation test also uses a real second binding's ID for the mismatched due-snapshot case rather than a fabricated foreign session string.

## Static verification and retained oracles

- `rustfmt --edition 2024 --config skip_children=true --check tests/delivery/ledger.rs tests/service/delivery_pull.rs`: exit 0.
- Scoped `git diff --check`: exit 0.
- Static inventory versus the captured pre-edit files: no removed test keys; exactly one added case above.
- `presentation_v2_service_requirement_attach_release_fresh_replacement_keeps_exact_commands` is byte-for-byte unchanged, including full literal topic/goal/IDs/revision commands and zero-ACK/accept physical-confirmation checks.
- No legacy seat_mut/SeatOffers/per-seat ledger constants remain in either owned file. No runtime outcomes claimed. Parent must compile and exercise the composed APIs and newly added carry scenario.

## Working file fingerprints

- `tests/delivery/ledger.rs` SHA256 `9a674b11f70d568acf7c55947940e891287f1e2114023516902d1bcd386255e0`; 23 test attributes.
- `tests/service/delivery_pull.rs` SHA256 `149a502c61986fc2d217d9f9be356cb8a54f3e2c05274214a596e19e291eebd3`; 144 test attributes.
