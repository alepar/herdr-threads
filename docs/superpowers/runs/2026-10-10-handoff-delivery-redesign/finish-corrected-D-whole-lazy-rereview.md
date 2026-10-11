# Corrected D lazy-only activation test rereview

**Spec: ADDRESSED. Quality: ADDRESSED.** D-WHOLE-4 / I4 is corrected by the reviewed source delta. This closes the prepared-test defect at the scoped successor, not at the unchanged cumulative D HEAD, and does not claim active execution success.

Reviewed exact helper delta `0007ada15cdd131d4a358f8929ccd18cfe8dd6a0` → `c4f0eb00cb7b5b943aae5fbd85172c3dc382f7a0` in ROOT/.worktrees/finish-watch-activation. Verified HEAD `c4f0eb00cb7b5b943aae5fbd85172c3dc382f7a0` and tree `d8458c1406e8d3b7d7aa01ec69b85a1ae7690655`. The delta changes only `tests/integration/mod_delivery.rs` (33 insertions, 14 deletions). Reviewed the full delta and surrounding relay logging/lifetime and production token handling. No source, index, HEAD, tracker or Herdr changes; no Cargo or native execution.

The corrected test at `tests/integration/mod_delivery.rs:2182` locates the actual `delivery_pull_v2` / `delivery_pulled_v2` Page containing the lazy ID, including the valid case where an already in-flight initial Pull selects it. It checks the actual owner lease, one-item payload, absent serialized presentation token, empty emitted controls and absent pending outbox. Token absence matches `src/protocol/delivery.rs:306` (None is omitted), store creation only for nonempty frozen descriptors, and the Watch consumer's token-conditional Confirm call. It no longer waits for an impossible Confirm.

The original observable assertions remain: actual lazy stdout item, no ACK requirement, canonical lazy recipient still pending, no mod ACK, no attention/nudge, and a live channel across the health interval. It then stops its owned Watch child, waits for canonical reconnect grace, and checks the post-registration relay log for absence of Confirm, ACK, acceptance or lazy-completion RPCs. Rechecking canonical pending/outbox state and prompt count after child shutdown strengthens the negative assertion without changing production behavior.

No additional finding in this narrow correction. Its real capability-discovery precondition remains intact; the inactive baseline cannot establish that the revised active test passes. Integration into D, successful execution against the separately activated exact source, and actual production-consumer evidence via captured stream remain parent-owned gates. The cumulative report/coverage for `31e5879781904b9ec20ee5ea3cec7517856260f3` intentionally continue to describe I4 as open at that earlier pin.

**COMPLETE + QUIESCENT.** No processes started or left running.

