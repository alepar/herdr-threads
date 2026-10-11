# Task 15 Watch stall-confirmation proposal review

Spec CLEAN

Quality CLEAN

Reviewed the complete frozen `task-15-stall-confirmation-proposal.md`, SHA-256 `bc8b2fc1681b55d93fbdef28c92755e811a50edf96334c62df473271b67acc23`. This is a bounded design/source audit, not implementation approval or a whole Task 15 verdict. The earlier conditional legacy-port choice is superseded. No Cargo commands, tests, source edits, Git mutations, or Herdr operations were performed.

## Spec compliance

No blocking findings remain in the selected proposal. It unconditionally chooses a new default-inert `record_presentation_attention_push(seat, generation, native_session, lease, now)` method. The real registry must test Live state and the entire exact current owner tuple and write `last_attention_push` under the same registry entry lock. The generation-only legacy method is explicitly unchanged and unused by this new path. This resolves the original ambiguity without extending legacy reconnect-grace authority into V2 confirmation.

The existing owner permit is retained across admission, canonical confirmation, reply validation, and the new method. At the inspected source, register, unregister, and close_inner also take that permit. Unregister releases its initial lookup lock before acquiring the permit, then re-finds the exact channel under the entry lock; there is no inverted lock held across permit acquisition. Sweep directly removes expired grace entries, not Live entries. The proposed atomic method makes its own disconnect outcome local: if disconnect/replacement has won before its entry-lock check, it cannot arm; if it records while the exact owner is still Live, a subsequent disconnect may retain that valid earlier evidence in grace. Same-generation/same-native replacement still has a different lease and cannot inherit a stale confirmation write. The implementation must not recursively acquire the owner permit from the new method while Confirm already holds it.

The proposed service predicate is correctly limited to Watch, fresh `Confirmed`, and at least one validated acknowledged applicable Attention stamp. Hook, AlreadyConfirmed, empty acknowledgments, clear-only acknowledgments, passive output, Pull/replay/Unchanged, refusal, and failed write/flush cannot arm it. Canonical result validation precedes the write, so malformed or foreign acknowledgments cannot create local health evidence. This is distinct from `record_ack`, receipt settlement, invitation acceptance, or delivery authority.

I checked the subtle fresh-status dependency against the current canonical producer. `invitation_content::confirm` can acknowledge an already-confirmed control while deciding status from any newly retired slot. However, the actual allocator freezes at most one control per page (`Controls::pending`, followed by a single control insertion). Every non-control descriptor is retired in the first confirmation call. Consequently an already-confirmed Attention cannot accompany a second fresh change on that same canonical page: after Attention is confirmed, that page is retired; subsequent confirmation is AlreadyConfirmed. Withheld Attention followed by its first explicit confirmation can legitimately be fresh even if non-controls were retired earlier. The selected predicate is sound for this producer; future multi-control allocation must re-evaluate this dependency rather than assume the status alone proves a newly confirmed Attention.

The lost-response rule is also sound: canonical commit plus the local write can occur before the response is lost; the retry returns AlreadyConfirmed and does not move the cutoff. A process failure between canonical commit and the local write can omit this process-local health evidence. The proposal does not claim durable atomicity between those two effects, and does not introduce it.

## Code quality and policy implications

No design quality findings. The new operation is a bounded registry mutation, not a source walk or a call to legacy `mod_stall_oldest` from Confirm. The existing worker remains responsible for stall judgment. That worker's historical pending-receipt scan remains outside this amendment; this review does not certify its boundedness.

The final proposal correctly states that the push timestamp is a publication cutoff, not a new ten-minute timer. `stalled_judgement` checks registration/real ACK age and qualifying receipt age; it does not wait ten minutes after push. An already old registration and old pending ordinary receipt may therefore become stall-eligible immediately after valid Attention confirmation. Ordinary-only, body-at-most-8-KiB, receipt-publication-at-or-before-push, grace handling, cooldown, and actual ACK resets remain the existing policy. Presentation Confirm must not update ACK age. No new durable counter, wake predicate, harness branch, model-visible string, or capability advertisement is authorized by this proposal.

## Required implementation evidence

The planned real-registry/public-handler fixture is appropriate and must avoid `registry.pass`/legacy notify before the assertion. Pull alone must leave the push absent; fresh Watch confirmation must arm existing stall detection for an older supported pending receipt without settling it. Strict ten-minute boundary and unsupported lazy/oversized receipt exclusions remain meaningful existing policy checks.

The implementation review must inspect and the tests must cover:

- Exact generation, native-session, lease, and Live checks at the new registry method, including default-inert behavior. Wrong tuple, replacement, reconnect grace, rebind grace, and absent entry must not write. A deterministic ordering test should distinguish disconnect-before-check rejection from valid confirmation-before-disconnect evidence.
- Public V2 Hook, lazy invitation/message, passive ModRefresh, clear-only, empty/obsolete acknowledgments, invalid acknowledgments, Pull/replay, and AlreadyConfirmed negatives. Consumer write/flush/refusal tests supplement, rather than replace, real registry assertions.
- Lost response by discarding an actual successful Confirm result, then retrying later. Publish a second receipt after the initial cutoff and demonstrate that retry does not make that receipt eligible; merely observing an unchanged boolean stall result is insufficient. Isolate or settle any earlier qualifying receipt so it cannot mask a moved cutoff. Keep the independent observer inactive in this fixture.
- Confirmation before registration replacement and confirmation after replacement/disconnect; same generation/native values alone must not suffice. Registration must retain its existing reset behavior. Ordinary pending receipt state must remain unacknowledged after presentation confirmation.

No implementation/test execution is claimed here. Production remains held for parent authority, actual D implementation, focused execution, source/interface rereview, and whole D/Task 15 gates.

## Point-in-time source provenance

Read-only source inspection used the active D workspace `ROOT/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`. D is concurrently working in disjoint areas; these are file hashes, not a claim of a frozen whole workspace. Relevant source hashes were rechecked before writing this review:

| Path | SHA-256 |
| --- | --- |
| `src/ports.rs` | `afbe8ae35b1792219fdaaf77da6feb0901cb364d4fd6d18e4c64f9d13963a1b6` |
| `src/service/mod_channels.rs` | `6eea923909421fd55259377ee8b0e189176823231128aaa2125c92e827c4b9f7` |
| `src/service/delivery_presentation.rs` | `7a7a3149c708bd464479be1651ca6ad36f6ed29c3d1a1eb8fd39d79bb4a94ad8` |
| `src/store/invitation_content.rs` | `cf9ce73d8f43d2d55c9492df7c11b7cffae5d27f6c7fa02117eea1b17082bd3b` |
| `src/store/seats.rs` | `aa4bf9d178a700e0ce17a1701345cd36bec21c66c7f243f8d2964ed1cf3fd49d` |
| `tests/service/mod_channels.rs` | `60b9974065e2586a6969e104e25761e18a4910af0be25947cd46e6eb6985f8a9` |

Concrete inspected seams: `mod_channels.rs` close_inner/350, stalled_judgement/410, notify_at/496, exact owner checks/514, register/521, unregister/582, legacy push/646 and ACK/659; `ports.rs` owner/default methods/3560 and legacy push/3624; `delivery_presentation.rs` retained permit and canonical confirmation/74 onward; `invitation_content.rs` confirmation/249, Controls/395, single-control allocation/648; `seats.rs` existing stall receipt lookup/4679. No unexecuted source fixture is represented as passing evidence.
