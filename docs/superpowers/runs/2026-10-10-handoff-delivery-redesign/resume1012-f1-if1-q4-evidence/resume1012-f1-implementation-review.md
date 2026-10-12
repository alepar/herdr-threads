# F1 minimal implementation: one focused review

**Scoped source PASS. Must-fix code findings: none.** The sealed implementation patch follows the accepted F1 direction. Runtime GREEN, the separate mixed-page control and required source gates remain parent-owned; this report does not close F1 or certify an applied production tree. One focused review under super-code 6.4.2-alepar5.1; no roast or round count.

## Exact reviewed package

SHA256 verified by direct file reads:

- Implementation patch: `5a3f4bdd2b72c5396e2cecac8988bb8c20652911dcd5eb0f893ace67e10f21c1`.
- Author report: `63392b414bb2efb30758dc44bbf7ab4ec1b179a7906b1517d7e1b37b457f2159`.
- Seal: `2dc2ae288e3f0cd35dfa1251760c45ba4017157797a03b9f6b71d4281dbf90b6`.
- Sizer test patch: `ec9b151c2eb3fd5774b3f9cf76f1ad0c96dff550ccf733611c90368f597eeb07`.

The supplied `.worktrees/upstream-composition-v090` still matches the seal's three baseline hashes: presentation `367e8bc25bca53b00c8c3977b761fd77a1d3663d941fce4c6a98329d587bc276`, invitation_content `07937cfaf68524f5b47f903470c4aa836766eaa0bd51b539c3897ccf90451e87`, TRUST `ea56cf83f3df3f75570bb62242f1d25a782d563c9bf69cad3a7aaa89d759dd48`. Consequently the review is of the sealed implementation delta against this matching baseline, not a claim that these source files already contain it. Proposed hashes in the seal are presentation `e5750172bedca8b57b4cd2744adc5fe7c8714f7fbaf5884e90e4e07e8b5b2483`, invitation_content `014f6829888a976dffc0a937734d9da66f9698b831a5db799fbcea2dc85bd41e`, TRUST `8cf88dfd7ff9312a9e9eccc124321d2ed82365510151b87bca961f0c5dc3bb1e`.

Read proposal2 and the preceding scoped review, patch/report/seal/test patch, relevant baseline admission/carry/measurement code, and the parent's retained causal RED logs. Only this report is written. No runtime, Cargo, SQL execution, Git or Herdr writes, source edits, or nested agents.

## Compliance and focused checks

At `src/store/invitation_content.rs` normal append hunk, deferral is restricted to classified `OutputLimit`, empty `outputs`, and positive remaining `message_slots`. The existing class/source selection supplies normal mode; blocked/known and capacity checks still precede measurement. Only `processed_message_count` increments before continuing. It does not decrement a slot, append output or offered/truncated IDs, record an offer, mutate pending/canonical body/reader/receipt state, clear partial/skip state, or modify an Offer stamp. Draining the processed private prefix reuses existing carry persistence. Nonempty overflow and exhausted allowance break with the candidate retained. Invalid rendering returns StoreCorrupt inside the existing deciding transaction.

The immediate authority check traced was service registered-codec selection and profile validation → canonical store binding/frozen-render checks → normal producer or exact-key carry revalidation → changed append. Existing canonical carry shape/body/truncation checks and frozen prefix remain intact. No weaker public path or new consumer behavior was introduced.

At `src/delivery/presentation.rs` classifier hunks, the counting writer must actually exceed OUTPUT_LIMIT and the profile must validate before the error becomes `OutputLimit`. Invalid structured items and serializer/codec failures without that evidence classify as `InvalidRendering`; 352-pass exhaustion returns before serialization or counter changes. One call runs one actual serializer pass; no second singleton probe appears. Public `measure` and `Sizer::measure` retain their signatures and old numeric errors through the internal conversion. Other callers therefore retain their existing contract, while the changed normal branch can distinguish causes.

No transaction, post-commit transport/writer, explicit confirmation or retry-reset boundary changes. The exact adopted TRUST addition is present in the same implementation patch, including unknown first-discovery IDs, unchanged pending authority, later independently admitted retry, absence of a new negative cache/wake guarantee, and honest pre-commit versus post-commit loss behavior. This remains a current-render limitation rather than successful delivery.

## Test changes and evidence

The reviewed test patch only adds the focused classifier test: real Watch JSON overflow and one counted pass, allowance exhaustion with unchanged counters, invalid control epoch, and invalid codec profile classified separately. It contains no deleted, skipped or loosened existing test. Review used the exact supplied patch; no Git test-diff command was permitted or run.

The retained baseline run at `/private/tmp/ht-C-resume1012-f1-normal2-typed-baseline` selected both causal tests with `nice cargo nextest run --locked --all-features --lib --no-fail-fast -E` and their exact names. Its log shows Lazy reaching the intended `0` versus `4` progress failure, while Ordinary failed earlier at QueryReturnedNoRows in the sparse receipt-state fixture. These are distinct failures; the latter is not evidence of Ordinary causal RED. Summary: two failed tests, 0.610 seconds, leak check exit0. Execution manifest records head `101edb65c161de1cf27a35b728e64c0027549c59` and the actual task-worktree test binary. Parent reports the effective-projection fixture correction applied afterward; no corrected outcome or GREEN is claimed here.

At dispatch, the parent's focused library batch was running and the normal mixed-page control was being authored separately. Their pending evidence is explicitly outside this source-only verdict. The existing causal-control direction includes fitting recapture after independent publication, so later-mail success alone will not satisfy it. No additional broad matrix, repeated whole suite or unchanged-boundary tests are requested by this review.

Minor handoff note: the new refusal diagnostic `invalid normal presentation rendering` is 37 UTF-8 bytes; record that size with the implementation commit/PR per the model-visible-string rule. It is an error diagnostic, not new per-message framing or a retry notice.

**PASS for the sealed minimal implementation delta, with no must-fix source issue.** Parent must attach the focused results and matching applied-source/gate evidence before declaring F1 fixed or merge-ready.
