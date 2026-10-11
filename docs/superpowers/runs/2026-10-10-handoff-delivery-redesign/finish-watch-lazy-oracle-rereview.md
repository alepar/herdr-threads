# Watch lazy-only oracle scoped rereview

2026-10-11. **I4 ADDRESSED IN SOURCE. Spec PASS; Quality PASS for this test-only correction.** Active execution remains pending.

Independently inspected helper HEAD `c4f0eb00cb7b5b943aae5fbd85172c3dc382f7a0` and its exact delta from `0007ada15cdd131d4a358f8929ccd18cfe8dd6a0`. The sole changed path is `tests/integration/mod_delivery.rs`, blob `2edc9a22a5a2c7ef0f58006163c7c007ecebe88e`. Read the appended author report. This is separate from the cumulative review of D `31e58797`, where I4 remains correctly recorded as open; the parent has not yet applied this helper correction.

The unchanged test name `mod_delivery::activation_v2_watch_lazy_only_page_is_passive` now locates the actual selected PullV2 reply by its real lazy message ID (`tests/integration/mod_delivery.rs:2195`). It allows an initial in-flight Pull to return that message rather than incorrectly requiring the request to start after send. It verifies the actual child's lease, exactly one item, omitted presentation token, empty emitted controls and no canonical outbox. These assertions match the actual message-only allocation and token-gated Watch Confirm behavior.

The correction removes the impossible wait for a matching Confirm. It retains actual lazy stdout/no-required-ACK, pending canonical lazy state, no mod ACK, no Attention, no native prompt and live channel after the existing injected eleven-minute advance. It then stops the owned Watch child and waits for canonical reconnect grace before checking the recorded exchanges for no Confirm, ACK, acceptance or lazy completion. This gives the negative request observation an explicit child EOF boundary. The initial request may precede the log marker, but any prohibited post-output completion would follow the new lazy publication and be inside the observed segment. Final canonical pending/outbox/no-wake checks remain.

No production behavior, capability advertisement, page/token construction, fixture content or capture is changed. No new source defect was found in the scoped delta. A passive invitation/Notice confirmation positive remains separately covered; this test correctly proves tokenless lazy-message passivity rather than inventing presentation authority for ordinary output.

No Cargo, format, lint, native execution, fixture process, source/Git mutation, Herdr/tracker action or subagent was run. Parent must transfer/map this exact correction, compile it and execute the active selected test; no GREEN or activation clearance is claimed. The parent's five meaningful dormant missing-advertisement RED results predate this correction and do not execute its later assertions.

**COMPLETE + QUIESCENT.** Only this report was written for the scoped rereview. No owned processes were started.
