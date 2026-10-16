---
super-roast verdict: Should-fix (2 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): Production local cooperative messaging plugin. False receipt completion can hide a pending obligation and missed delivery can stall collaboration. Same-user local inputs are cooperative; assess canonical ownership and output completion rather than adding adversarial caller verification.
inputs: super-auto/inbox-injection-auto-ack@aba89049b367c22ddb3330b16ecfa14415b96b7a vs main@3ff902c77c8df3dbcadf02119bacd9e344f7e59b
coverage: scouts 11/11 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 3 → deduped 3 → panel 2 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 1/1/1/0 · hygiene-docs 2/1/1/0

All 11 scouts returned. Both full panels unanimously confirmed their findings; the spot check confirmed one Nit with lower verification strength. Ordinary manual fan-out used GPT-family seats in place of the skill's fable reporter. This local review does not establish final combined cross-node readiness; the caller has deferred that proof.

## Confirmed findings

- [Should-fix] src/cli/hook_inbox.rs:365 — The native-hook receipt settlement path lacks the required regression proving that settlement by one binding preserves a sibling binding's unread delivery and independent wake eligibility. [lanes: testing]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: Design §5 explicitly requires sibling unread delivery and independent delivery-row wake after seat-level receipt settlement; §8 includes same-seat siblings in acceptance coverage.
  evidence: LazyCommit::submit now sends receipt_candidates through CompleteInboxDelivery before advancing the displaying binding's cursors. Native regressions at tests/hook_entrypoint.rs:2644 and :7617 use one recipient binding.
  evidence: The refute seat found no test combining nonempty native receipt completion with sibling unread delivery and wake assertions. tests/store/receipts.rs:5324 covers AckDisplayed first-settler attribution; tests/integration/lazy_attention.rs:1222 covers invitation service. Neither exercises this composition.
  evidence: This is missing required regression coverage, not a proven runtime defect. The new seat-level settlement makes the gap material in this messaging plugin.
  fix-shape hint: Add a native-hook regression with same-seat sibling bindings that settles the receipt through one binding and verifies the sibling's unread delivery and independent delivery-row wake.

- [Nit] TRUST-POLICY.md:1702 — The normative policy retains the superseded mod-only auto-ACK exception and body-plus-ack truncation recipe. [lanes: hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: TRUST-POLICY.md:1702–1706 still restricts delivery auto-ACK to the bundled Claude mod and describes body followed by ack. Updated operative sections at :538 and :927–940 authorize native-hook completion; src/protocol/watch.rs:108–126 now emits only an inbox continuation.
  evidence: Design §4 explicitly requires updating the old mod-only wording. The paragraph was accurate on base; the new behavior makes it contradictory. Its dated decision-record context and the corrected operative sections limit the impact to Nit.
  fix-shape hint: Amend or explicitly mark the old decision paragraph as superseded, including its truncation instructions.

## Not verified (beyond panel cap)

- none

## Not verified (dedupe failed or judge lost)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- none

## Unverified nits (spot-checked)

- [Nit] src/protocol/commands.rs:114; src/protocol/commands.rs:1588 — CompleteInboxDelivery documentation still promises presentation bookkeeping only and never an ACK, although receipt_candidates now authorizes receipt settlement.
  evidence: The refute spot check confirmed that both variant comments retain the old authority description. Field documentation at :948–951 and the trust policy describe the new authority; src/store/control.rs:2807 calls complete_hook_context, which settles pending receipts in src/store/receipts.rs:119.
  verification: One refute-seat CONFIRM; no full panel. Correct field documentation and canonical validation limit this to Nit.
  fix-shape hint: Update both operation comments to distinguish presentation bookkeeping from authorized hook-context receipt completion.

## Escalations (need human)

- none
---
