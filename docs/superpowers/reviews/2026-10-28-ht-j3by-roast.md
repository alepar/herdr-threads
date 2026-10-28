super-roast verdict: clean (2 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling (local daemon coordinating coding agents on one machine, same-user cooperative trust model). A wrong wake costs model context and attention, not data; delivery-contract invariants are the stated core purpose of this code path.
inputs: bead/ht-j3by@69d5baa84 vs 6e918785b
coverage: unavailable — inline run, no coverage object; lanes reviewed inline: correctness, contract (delivery rules 2/4/5), test causality, trust-policy sync
independence: none (inline) · rung: inline

## Confirmed findings

None.

## Rejected (with reason)

- [Should-fix] src/store/wake.rs `none_offered` — "an empty pending set may be a pending item the walk did not see, so a legitimately owed wake is refused." Rejected: the reservation's `pending_receipt` / `pending_invitation` reasons come from the same walks (`pending_receipts(db, seat, None)` and `pending_confirming_invitations`, attention.rs:1750-1764, `has_pending_* = !items.is_empty()`), so an empty unsaturated walk is exactly the candidate's own "nothing pending"; a saturated walk still counts as pending.
- [Should-fix] early `owes_wake` call in src/notification/dispatch.rs now refuses as `Refused(Unavailable)` where `is_current` used to report `Unsafe`. Rejected: `is_current` runs first and still catches an ACK committed before it (load_candidate has no actionable work); the new branch only fires for an ACK landing between the two checks, where Unavailable (pre-send, ladder restored) is the intended outcome, as for an in-turn-served item (ht-78f6).
- [Should-fix] "ACK replaced by a newer receipt before the prompt is now suppressed." Rejected: a newer pending receipt makes the set non-empty, so the wake is still owed (rule 5 holds).

## Unverified nits (spot-checked)

- [Nit] tests/store/wake.rs `an_item_resolved_before_the_prompt_is_not_typed` resolves the item with a raw `UPDATE` rather than the store's ACK/accept path. Acceptable for a store unit test (`effective_receipt` reads the physical row state; the test fails without the guard and passes with it); an integration-level ACK race would need a real daemon and is not deterministic.
- [FYI] Lazy-reason recheck before the prompt is decided by `LAZY_FRONTIER_UNREAD_SQL` (row displayed or not), not by re-evaluating `p <= T` against a threshold lowered after reservation. Out of scope for ht-j3by (ACK race); pre-existing.

## Escalations (need human)

None.
