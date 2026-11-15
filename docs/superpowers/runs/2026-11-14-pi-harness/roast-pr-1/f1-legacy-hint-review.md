# Legacy recovery-hint delta review

Source verdict: **Approved, conditional on final matching verification**. Spec compliance and code quality: **Approved for this delta**. No Critical, Important or Minor findings. Prior F1 approvals remain applicable.

Reviewed frozen patch SHA256 `9f17462fc23f679718628c604e84dced678cac368cb619c2673a302a8cc5b1f1`; both source files match `f1-legacy-hint-frozen-sha256.json`. Independently verified that all seven files at accepted adoption commit `6dc221c3d` match the prior `080cd0` adoption manifest.

`integrations/pi/extension/herdr-threads.js:506` filters recovered proposals before current-budget accounting, ordinal seeding and watch admission. Retention requires the immutable arena discriminator and current ordinary/protected record shapes; protected hints must have an eligible body kind and opaque 64-hex key. Pre-F1 hints and bridge-defaulted arena flags without current record discriminators are discarded. Current clear controls cannot be mistaken for proposal hints. The recovery registry entry is deleted and the local wrapper released before admission, so discarded legacy arrays are not retained across its asynchronous work. No canonical state migration or authority is introduced.

`integrations/pi/extension/tests/extension.test.mjs:430` constructs eight old-shape proposals containing 256 ordinary records, including a bridge-defaulted arena flag. Its zero-ACK assertion demonstrates that those optional hints were dropped: the causal RED's eight old ACK calls establish unwanted legacy retention, **not false ACKs**, because this fixture freshly observes the exact old accepted frames. The meaningful budget/admission continuation then supplies 224 fresh ordinary records plus 32 protected slots and asserts exact delivery counts, 224 ordinary ACK IDs and one whole-page confirmation. The accompanying focused current-key reload and retained-tie regressions preserve supported hint behavior. Reported targeted GREEN: 3/3, 192.394228 ms.

Dropping hints permits conservative at-least-once canonical ordinary replay. It cannot recover notices already settled under the old implementation; no such recovery is claimed. The filter assumes cooperative locally generated recovery objects, consistent with the existing model, rather than adding adversarial validation.

The accepted adoption checkpoint has its separately recorded 79-test/check/cleanup evidence. The new delta's final 80-case callback/SDK run, required guards and scoped cleanup were active at review time; successful matching results and the eventual commit remain acceptance conditions. Native/provider qualification is separate. No tests, services, credentials or seat writes were performed for this review.

## Final gate closure

**Final source verdict: Approved. Spec compliance: Approved. Code quality: Approved. All source-review conditions are closed.**

Accepted adoption commit `6dc221c3d` matches all seven files of the approved `080cd0` manifest (verified in the original delta review). Independently hashed both legacy-delta files at commit `b5f19ac3f` against the `9f1746` manifest; both match.

Inspected the completed implementation report and final `f1-legacy-hint-final-*` logs: 80/80 callback/actual-SDK tests passed, zero failed/skipped, 806.635205 ms; clippy passed in 19.82 seconds (real 19.86); formatting, neutral and wire guards passed; host and builder run-scoped process/temp leak checks were clean. The implementer reports all owned commands/children finished and no remaining scratch. No tests were rerun during gate closure.

This closes the adoption, re-admission, recovered-order and legacy-hint **source** acceptance chain. Native/runtime qualification remains pending, and the candidate binary must be relinked before qualifying the current source. No feature-wide/runtime approval is implied.
