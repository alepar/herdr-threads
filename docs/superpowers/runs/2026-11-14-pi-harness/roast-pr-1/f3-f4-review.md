# Independent F3/F4 scoped review

Verdict: **Approved**. Spec compliance: **Approved**. Task code quality: **Approved**. F3 and F4 are **ADDRESSED**. No Critical, Important, or Minor findings.

Reviewed commit `d108f38dc341ce6b5cf6723dad16e60909596655` and the supplied two-file patch. The committed diff and frozen patch both hash to `9fd9116787567152c1a645258d02596e28cce59a95f0f16be9112f461cb65164`. Source references below identify this immutable commit.

## F3: bounded watcher fixture teardown

- `integrations/pi/extension/tests/extension.test.mjs:358` places watch creation, first-row wait and connected-state assertion inside one `try/finally`. A deadline rejects missing output, an early exit rejects before any row, and the finally clears the timer and awaits the owned watch stop on success or failure. The outer scratch cleanup therefore runs after watcher teardown.
- `integrations/pi/extension/herdr-threads.js:413` supplies the existing concrete stop contract: TERM followed by KILL after 250 ms, returning the exit promise. Its close handler resolves that promise after exit notification (`:431`). The new helper uses that contract rather than leaving an unawaited stop.
- `integrations/pi/extension/tests/extension.test.mjs:387` exercises real Node children for wrong-state assertion, exit-before-row and no-row timeout. Each failure must propagate only after exit is observed. The independent five-second guard has a distinct error, so its firing cannot satisfy the expected rejection; backup teardown still awaits stop. These assertions retain the original success-path check.

## F4: missing-mode UI warning

- `integrations/pi/extension/herdr-threads.js:310` checks forbidden inherited attribution and another owner's registry claim before the new warning path.
- `:312` admits only `undefined` and `null` to the warning branch. It calls a callable public `ctx.ui.notify` with the existing warning text, catches unavailable/throwing notification, and returns. It neither invokes the raw diagnostic/stderr fallback nor reaches owner/environment claims, lifecycle enrollment or watch creation.
- `:318` keeps every explicit non-TUI mode inert, including future mode strings. No mode inference or activation fallback was introduced.
- `integrations/pi/extension/tests/extension.test.mjs:468` covers both missing values, native severity, no stderr/enrollment/environment/registry effects; `:480` covers explicit modes, forbidden callers and duplicate ownership; `:498` covers missing, noncallable and throwing notification. The harness's default notify records into diagnostics, so the quiet assertions also detect unwanted UI notifications.

## Evidence and limits

Inspected `final-roast-f3-f4-report.md`: causal F3 RED failures cover all three failure paths, followed by five passing focused cases; F4 missing-mode RED is followed by five passing focused cases. The recorded combined extension/SDK run reports 60 passed, zero failed, with successful formatting, neutral/wire guards, scoped leak check and clippy. No tests were rerun for this review; no concrete unresolved doubt required one.

This approves only F3/F4. It does not resolve F1 informational-notice settlement or qualify a later integrated candidate. Real TUI/provider and final integration gates remain separately owned. No source/index changes, services, credentials or seat writes were used in this review.
