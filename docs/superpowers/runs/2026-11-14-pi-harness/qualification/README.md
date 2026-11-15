# Pi qualification evidence

Current integration source-accounting anchor: `bad263b15dbbca518ea0509d16f90f9d01c1c602` on main `e60ec5eb5`. [Integration accounting](integration-rebase.md) verifies all 73 rebased patches and the unchanged 33-path outside-Pi inventory. [PR review 2](../roast-pr-2/report.md) concluded clean with zero findings and convergence on pre-rebase source `1dbdce20b` versus `91ce0cdeb`; it does not cover the new base. F1 is closed with scoped source, canonical/SDK and native evidence below. The following runtime results retain their historical candidate anchors; integration checks are recorded separately.

[Final integration gates](integration-gates.md) passed on the v3.2.1-based tree: **9,645 passed, zero failed, 66 skipped**, with formatting, clippy, default-feature, harness-neutral and wire checks passing and clean same-run process/temp checks. Test execution took 1,033.189 seconds; the version rebuild took about 7 minutes 12 seconds separately. Clippy took 62.45 seconds, exceeding its 60-second budget. Main retains independent integration review and merge ownership.

[Corrected full-suite attempt 2](full-suite-attempt-2.md) records the earlier pre-rebase functional pass: **9,645 passed, zero failed, 66 skipped**, 993.332 seconds, with clean same-run process/temp checks. Both runs missed the absolute 300-second budget and `ht-n4xh` remains open. Main accepted the suite miss as nonblocking for merge/release in this epic under the standing host-wide four-thread cap used for training; no suite tuning or cap increase is authorized here. This disposition does not turn the budget into a pass or establish causation through a paired benchmark.

[Scoped live qualification](final-live.md) passed in **39.501 seconds** on the exact `a8aef2df…` candidate at source `d55f7e984`, using `deepseek-v4-pro`. The run recorded exact native context/branch receipt evidence, provider/model continuity, zero extraneous inbox calls and clean owned cleanup. This is the single authorized scoped live run, not a full live matrix. [Exact live identities and evidence](final-live-evidence.json) remain separate from the full-suite test-profile artifacts.

- [Consolidated core changes](core-changes.md): all 33 changed outside-Pi source paths, neutral defaults, compatibility evidence and limitations.
- [Commit reorder](commit-reorder.md): complete 51-entry equivalent mapping and final core-first order.
- [Deferred-notice seam](deferred-notice-seam.md): neutral causal tests and default-mode compatibility.
- [Final compatibility](f1-final-compatibility.md): 12 fresh scripted cases; safety/threshold remain explicitly historical with the footer delta described.
- [F1 native attempts](f1-native-attempts.md), [native HOL progression](f1-native-hol.md), and [notice liveness](notice-liveness.md): separate candidate-bound native, SDK and canonical evidence, with failures retained.
- [F1 driver source evidence](f1-driver-source.md) and [independent final source review](../roast-pr-1/f1-driver-source-review.md): historical frozen source delta, corrected race, 25 self-tests and clean scoped leaks; later native evidence is linked above.
- [Historical integrated qualification](task-3-final.md): candidate-specific native evidence and failed full-suite/budget history.
- [Original Rust qualification](rust.md) and [shared setup extraction](rust-fix1.md): existing-harness evidence cited by the consolidated table.
