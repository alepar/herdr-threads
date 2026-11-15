# F1 recovered-order delta review

Source verdict: **Approved, conditional on final matching verification**. Spec compliance and code quality: **Approved for this delta**. No Critical, Important or Minor findings. Prior ownership/accounting closures stand.

Compared approved `f1-extension-frozen-v2.patch` against replacement frozen patch SHA256 `080cd052f4caad277785bbf8b44a82d038e541d75d075eb6dd895bcd55f95602`. All seven files match the updated manifest. The only semantic changes are the bounded recovered-ordinal seed and its regression test.

`integrations/pi/extension/herdr-threads.js:510` now raises the factory's sequence above each recovered record's existing ordinal before admission starts the watch. It preserves retained ordinals and the existing priority/deadline comparator. Fresh records cannot accidentally sort ahead of requeued retained records merely because the successor factory began at zero. The loop uses the already bounded recovery records; it introduces no timestamps, history cache or wire dependency.

`integrations/pi/extension/tests/extension.test.mjs:410` advances the predecessor's observation order, retains an accepted notice across reload, admits a new tied row, and uses actual-context absence to requeue the old hint. It asserts the old notice precedes the new row and still obtains exactly one whole-page confirmation after observation. This exercises the intended ordering failure without changing delivery priority, deadline or receipt requirements. The report records causal RED (new row before old slot) and the ordering/reload group GREEN, 3/3 in 108.060747 ms.

The prior 78-case affected suite, clippy (25.64 seconds), guards and scoped cleanup results apply to the pre-order snapshot and are explicitly labeled that way. Final matching 79-case callback/SDK, required checks and cleanup were active at this review checkpoint; they and eventual commit-to-snapshot matching remain acceptance conditions. Native/provider and canonical uptake qualification remain separately owned. No tests, services, credentials or seat writes were used in this review.
