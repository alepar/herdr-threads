# Q3 completion-hint scratch controls — REVISE

2026-10-11. Reviewed ignored `q3-completion-hint-controls.patch` against actual candidate fixtures, the public DomainService dispatch/confirmation routes, FairWriter and presentation allocation semantics. Scope is the seven proposed tests in `tests/service/cooperative.rs` and `tests/service/mod_ack_dispatch.rs`. No patch application, source mutation, Cargo, runtime/SQL execution, Git, Herdr or delegation occurred. This is a scoped controls review, not a code roast; counters unchanged. Parent's current gates/baseline remain untouched.

## Required revisions

### HC1 — Successful presentation-confirmation fixture expects a nonexistent ordinary-receipt token

Patch function `q3_completion_hint_successful_presentation_confirm_is_inert` ends with `page.presentation_token.expect("pending receipt Attention has an outbox token")`. Its reused fixture has ordinary pending m1/m2 and no confirming invitation, warning or other nonordinary descriptor. Pending ordinary receipts alone deliberately do **not** manufacture generic Attention or a presentation outbox token. The existing `tests/service/delivery_pull.rs` control around line 10570 explicitly protects that property. Bounded relevance discovers invitation/warning authority, and `invitation_content` allocates an outbox token only for nonempty nonordinary descriptors. Neither the fixture's receipt rows nor its local offer ledger establish such authority.

This test will fail during setup (or at its Page assumption), independently of whether presentation confirmation accidentally issues a completion hint. Do not fix it by restoring generic receipt-derived Attention.

**Minimal correction:** create a real pending confirming invitation/nonordinary item through its canonical public operation, then pull and assert that item's actual outbox token/control. Follow only advertised bounded continuation if needed. Confirm that genuine token and repeat confirmation to check Confirmed/AlreadyConfirmed with no completion hint, receipt delta or Hook channel-health call. Snapshot native/receipt state after the setup operation so the setup's legitimate effects are excluded. Preserve the existing NotCurrentPage negative as a separate case.

### HC2 — Add an actual retained successful operation replay; fresh AlreadySettled is different

Every successful ACK in the patch uses a new operation key (`hint-first` versus `hint-already`, `hint-lazy-first` versus `hint-lazy-already`). `hint-confirm-retained` uses a fresh DeliveryConfirm after a separate explicit ACK while its logical offer remains locally retained. That is a useful authoritative AlreadySettled result, but not replay of the idempotency store's retained successful operation result. No current case verifies the design's allowed conservative callback when the store returns an earlier successful answer without a new durable mutation.

**Minimal correction:** in the Ack/AckDisplayed writer-release test, repeat the exact first request and operation key after its successful return. Assert the same original Acknowledged payload, one additional successful writer-free callback for the same seat, and unchanged exact receipt state/ACK observation/binding/timestamp and retained operation record. Keep the fresh new-key AlreadySettled case too. One shared dispatch replay control suffices; do not force DeliveryConfirm to reach a retained store result by pretending an already-removed offered ID remains naturally admitted. Rename the existing retained-offer case/comment if a report could confuse its local retained offer with a retained operation reply.

## Controls that pass static review

- Successful classification is exercised through public commands and a real SqliteStore, not by calling the private classifier with fabricated CommandResults. The tests inspect exact acknowledged/AlreadySettled/Settled/refused outputs and exact callback seat/count. Lazy-only completion and mixed Lazy/ordinary/refused batches genuinely distinguish the intentionally conservative hint from an ordinary receipt-delta claim.
- The explicit local offer seeding is acceptable for this service-boundary test. It is clearly labeled and only supplies the ledger precondition for public DeliveryConfirm; the canonical store still decides whether each ID exists, is addressed and can settle. It is not evidence of a real model read, physical emission or independent receipt provenance. The unknown-but-offered ID usefully proves that ledger presence alone cannot authorize settlement.
- Foreground re-entry uses the exact FairWriter shared by the real service. A fresh SystemClock and 200ms budget make the callback's wait finite even though the store fixture uses a fixed clock. FairWriter waits in bounded 10ms intervals, releases its mutex during waiting, and removes the waiter on timeout. `map(drop)` releases the successful guard before recording the callback. The expected `Ok(())` and empty writer queues would catch calling this callback while the outer turn is still held. No helper process/thread is created by these tests.
- The rollback trigger targets the second legacy receipt UPDATE, which matches this fixture's no-manifest receipt storage path. Both receipts remaining pending, no retained operation row and no callback are meaningful atomic-outcome assertions. Empty, unknown, stale and wrong-instance requests plus refused/not-offered/retryable routes check inert failure classes. For clearer failure attribution, retain the trigger's diagnostic/error classification rather than relying solely on `is_err()` when reporting the eventual run.
- Existing outer ModACK `record_ack` accounting is preserved and measured separately. Settled and AlreadySettled each increment the expected binding's ACK counter; extra notification/presentation-health counters remain zero. Hook confirmation explicitly expects no mod ACK health. This avoids the earlier blanket no-health oracle that would contradict the real handler.
- Reused fixtures own temporary SQLite state; no daemon or child process is introduced. The callback spy checks dispatch effects, not registry bit scheduling or JS delivery, and should not be presented as coverage of those separately owned boundaries.

## Gate

REVISE HC1 and HC2 before parent application and focused execution. The remaining proposed tests have sound static causal structure for their stated service boundary. This review claims no compilation or runtime PASS and does not alter the preserved baseline.

Reviewed patch SHA-256: `91a72e537c9e32bf7853b57174f14d5647ce6f22f902f775450b8b68d10b78ed`.


## Applied corrections rereview — HC1/HC2 scoped static PASS

2026-10-11. The initial REVISE is preserved above. Reviewed the applied corrections in quiescent `tests/service/cooperative.rs` and `tests/service/mod_ack_dispatch.rs`; no source edits, Cargo, runtime/SQL execution, Git, Herdr or delegation. Other concurrent fixture/query edits are outside this rereview.

**HC1 resolved.** `mod_ack_dispatch.rs:907` now creates a thread and an actual Lazy invitation through public DomainService commands, pulls through the public V2 Hook path, asserts the canonical Invitation item belongs to that thread, and uses its actual advertised presentation token. Lazy invitation presentation is a real nonordinary outbox descriptor; it does not require generic Attention manufactured from ordinary receipts. Receipt/native snapshots occur after invitation setup. The exact confirmation request must return Confirmed then AlreadyConfirmed, with no completion hint, receipt/native change, invitation acceptance, mod ACK or Hook health call. The setup also asserts unrelated operations emitted no completion hint. This removes the nonexistent-token prerequisite without misrepresenting physical presentation as settlement.

**HC2 resolved.** `cooperative.rs:2672` now repeats the exact successful Ack/AckDisplayed operation key and payload, requires the original result including its original acknowledged vector, and checks unchanged receipt state plus ACK observation/binding/timestamp and the exact retained operation scope/key/digest/result/time. It requires a second same-seat callback that successfully reacquires the real foreground writer, then separately retains the fresh new-key already-acknowledged case and third callback. This is actual successful-result replay, not merely fresh AlreadySettled. `mod_ack_dispatch.rs:613` additionally replays the same successful Lazy ModACK key, asserts the same original Settled reply, no receipt delta, one retained operation and one additional harmless hint. It preserves separate existing outer ACK health accounting, including retained and fresh AlreadySettled results. The retained-offer DeliveryConfirm case is now named and described as retained offer, with its fresh AlreadySettled result distinguished from an operation replay.

The earlier writer re-entry, bounded wait, rollback and locally seeded ledger scope assessments remain unchanged. No post-commit ordinary-delta inference is introduced. HC1 and HC2 are cleared for focused execution; static PASS does not claim compilation or runtime GREEN.

Applied patch SHA-256: `ec41f16607e5fcfc3ed9d42159af757b62748bad2980b019357d5070152a17ae`.

Reviewed source SHA-256:

- `tests/service/cooperative.rs`: `c4c69c07a37e7b1e2bfe440c90e58267011a92f4b1213e64bbc1b4af3f002307`
- `tests/service/mod_ack_dispatch.rs`: `7f25c6bfe79acf3f6978991e3abf42a822014563fbd98bbb1cefb596d7506ab8`
