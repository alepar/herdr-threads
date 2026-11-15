# PR1-F2 scoped re-review

Spec compliance: PR1-F2 ADDRESSED. Code quality: Approved for this scoped driver/fixture fix. No new Critical, Important or Minor finding in the frozen diff. This does not approve PR1-F1 or the final integrated candidate.

Scope: `final-gates/f2-frozen.diff`, coordinator-supplied SHA-256 `7c8bfada260aaf1cbdb25c30f63bffb39e517980b792e69ffb92726d0199c1a9`. Compared against the confirmed PR1-F2 packet/judges: the previous negative case could pass without any target draft reaching the transformer, then recover successfully on the later user turn.

- `scripts/pi-tui-qualify.py:594` now requires a transformer record for the current native session and exact target metadata, with the target message ID in the frame header and the complete unique body line in the pre-transform frame. The corresponding returned entry must retain the custom type and contain the replacement body. An empty transform, another session/ID/body, or an unchanged returned entry cannot satisfy it.
- The fixture logs the actual `result.entries` object that it returns, together with the observed before entries and current public session state (`scripts/pi_tui_fixtures/provider.mjs:93`). The new self-test imports the real fixture and verifies the logged after array equals the returned entries; it also exercises empty and mismatching target evidence (`scripts/pi-tui-qualify.py:889`).
- The driver verifies that the replacement with that exact custom type reaches actual native context, rejects original-body observation before the pending assertion, and requires the canonical receipt to remain pending (`scripts/pi-tui-qualify.py:628`). It evaluates this evidence immediately after the rejected run settles and before removing the transformer or starting recovery. A late-watch interleaving now fails there; a later successful retry cannot repair or mask the missing negative proof.
- Recovery uses a separately captured observation boundary and requires the original unique full body plus an acknowledged canonical receipt. These checks remain after the negative proof rather than being used as evidence that a transformation must have occurred. The extra public state observation before releasing the provider is explicitly only a scheduling yield, not falsely asserted as recipient watch readiness (`scripts/pi-tui-qualify.py:621`).

Evidence inspected:

- `final-roast-pr-1/packets.json` contains the confirmed causal gap and its requirement for an exact target proposal/transformation witness; this patch adds that missing prerequisite while retaining pending/recovery checks.
- `f2-driver-red.log` reproduces the entire old-case false PASS: the test expecting rejection reports that no assertion was raised when watch delivery happens only at retry. `f2-driver-green.log` reports sixteen tests passing, including actual fixture return/log agreement and mismatched session/ID/full-body/replacement refusals.
- `f2-native-negative.log` now reports `negative: pass` and `cleanup: clean`. Per the coordinator's supplied scope, this ran against the owned frozen older `198e` candidate and is evidence for the corrected driver path only; it does not qualify the separate F1 production fix or a later integrated binary.

No tests/builds rerun, source/index mutation, credentials, services, thread operations or subagents. Only this report was written. Root retains committed-patch identity, complete retained native artifacts and final integrated qualification/cleanup verification.
