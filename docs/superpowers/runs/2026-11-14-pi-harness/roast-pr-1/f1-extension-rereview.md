# F1 extension scoped re-review

Source verdict: **Approved, conditional on final verification**. Spec compliance: **Approved for the reviewed source**. Code quality: **Approved for the reviewed source**. Important I1 is **ADDRESSED**. No new Critical, Important or Minor findings in the fix delta.

Reviewed the exact difference between preserved `f1-extension-frozen-v1.patch` and replacement `f1-extension-frozen.patch`. Replacement SHA256: `6e88afde7cd8c2781006e21c785b9fee825ce9f805c5d61dc4c9d8ad78052b1a`. All seven files match the replacement manifest. Production JS SHA256: `514bd104bf839328c0609e4c5524ff21dd0dda9215d92908959ff8dde17052e7`. The original report remains intact as the record of the rejected snapshot.

## I1 closure

`integrations/pi/extension/herdr-threads.js:179` now checks outstanding confirmation/replacement ownership in the common `startWatch` entry point, before the child-presence check. Consequently both exit-3 re-admission and refusal-status-only re-admission join the same owned replacement; neither can bypass the gate through `admit()`.

`replaceWatch` captures the old confirmation and awaits it after the old watch stop (`:163`). Its final startup passes that replacement's own promise identity (`:173`), so the common gate allows only that completed join to start the successor. An unrelated re-admission sees the existing replacement and returns without starting another. The confirmation completion handler clears its flight and leaves replay to an existing replacement (`:349`), avoiding a self-await or duplicate replay. Shutdown still fences the owner and joins the tracked replacement/confirmation work, so this delayed startup cannot create a post-shutdown child.

`integrations/pi/extension/tests/extension.test.mjs:776` adds the causal refusal regression. It holds an actual adapter confirmation request, exercises exit3, no_binding plus exit3, and no_binding without exit, then invokes genuine-turn admission. It verifies a second SessionStart occurred while the watch count remains one; after releasing confirmation, exactly one successor starts. Finally cleanup resolves the held promise and shuts down the fixture even on assertion failure. The report records the old implementation failing with two watches instead of one and the corrected refusal/exit/ambiguity group passing 3/3 in 191.671006 ms.

## Prior review and remaining gates

The replacement delta changes only this startup gate and its regression. The prior review's accounting closure, opaque receipt/key handling, whole-page proof, revision/reload rules, supported projection recovery, corrected pressure-test interpretation, Rust fixture compatibility and documented cooperative context-observation limit remain applicable. No broader re-review or test rerun was performed.

This is source approval with explicit verification conditions, not final feature acceptance. The recorded 77-test run predates I1's fix. At this review checkpoint, the final 78-case affected callback/SDK run, lint/guard and scoped cleanup checks were active. Acceptance requires their successful results against this exact snapshot, plus confirmation that the eventual commit matches it. Native/provider and end-to-end canonical uptake qualification remain separate gates.
