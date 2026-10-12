# Task 25: canonical one-slot carry experiment

Status: source committed; parent-owned runtime evidence and gates pending.

Base: `dc15706ead701e609ce530c69dd170ecb6ad123c`
Initial implementation: `fa044039afb878e52e7c62dd126b6ce2cdedf574`
Head: `f746e5698205e7e2cd08b6b70a538bf919f091a4`
Branch: `super-auto/sweep-ht-0lo-17`
Files changed: `tests/service/q4_accept_controls.rs` only.

## Implementation

The fixture retains all 257 canonical Send IDs and the actual Watch codec-rendered map. Before any corrective mutation it records compact full-capacity counts, the first two unoffered IDs with the complete unoffered count, decoded source/unread/lazy/receipt positions and frozen high waters, carried IDs, partial count and skip ID. It checks the diagnosis's falsifiable geometry: 256 offers, the retained real direct Lazy offer, exactly two unknown ordinary sends, and empty initial carry. A different geometry fails before ACK rather than being forced into the prediction.

It selects the earliest actually rendered nontruncated ordinary in canonical thread-t send order, verifies its actual body/class/ACK/wake tuple, and uses the existing `window.ack(id, true, false, unique_operation)` Context-display route. Bounded real pulls refill exactly one slot, with the completed offer absent and every other original full stamp unchanged. Exactly one previously unknown send is offered and rendered; actual carry must remain nonempty, consist of unoffered canonical tail IDs, and retain a pending ACK-required receipt plus unread own cursor.

The original actual invitation/outstanding-token setup, nonempty-carry/full256 checks, and every acceptance-only state/counter/stamp assertion remain. Additional acceptance-only receipt provenance and existing own cursor snapshots ensure acceptance cannot masquerade as display or ACK. No private state writes, production changes, cap/zero-slot changes, clock changes, or TRUST changes.

Diagnostics produce two compact lines, approximately 1 KiB total, without bodies or ledger dumps. The commit carries the truthful required Outside-adapter trailer.

## Falsifiers

- Pre-action geometry: 255 offers, 0/1/3 unknown sends, missing retained Lazy, or already nonempty carry falsifies this single experiment's proposed boundary.
- Actual display selection: no rendered whole ordinary, truncated stamp, different body/class/ACK/wake flags prevents using its display as evidence.
- Reclamation/refill: completed offer remains, count differs from256, any survivor stamp changes, zero/two formerly unknown admissions, or a rendered ID outside the real newly offered tail fails.
- Carry: empty carry, known/unsent ID, terminal receipt or advanced own cursor fails.
- Acceptance: any reset of original preserved private fields/full offer stamps, wrong counter increments, changed receipt provenance or changed existing own cursor fails.

## Verification

`rustfmt --edition 2024 tests/service/q4_accept_controls.rs` completed successfully. `git diff --check` completed successfully. These are formatting checks, not runtime evidence.

Parent runtime override prohibits all Cargo/tests/runtime/private-process operations in this implementer lane. Parent owns focused success-output capture and required gates. No tests are reported green here yet.

Existing parent actual RED sources: final-C had 771 PASS / 3 FAIL, and the unchanged three reproduced at d842 with 0 PASS / 3 FAIL (`/private/tmp/ht-final-C-three-repro-d8427582/{execution.json,run.log}`). The carry failure was `SETUP: actual unoffered carry at full capacity`, before acceptance. The diagnostic report describes those observations and explicitly leaves detailed boundary geometry as a hypothesis until this experiment runs.

## Test hygiene

No tests, daemons, Herdr instances or background processes were started by this lane. No runtime cleanup was needed; parent owns test process hygiene and gates.

## Assumptions and concerns

Current readonly bead has no comments or amendments. It agrees with the extracted brief and diagnosis. Parent runtime override supersedes template test/cleanup/rebase steps; no automatic rebase was performed. Original design Goal and full accepted refinements were read and remain unchanged. User's checkout, integration source/index, tracker, Herdr and all runtime state remain untouched.

The exact two-unoffered/empty-carry boundary and successful one-slot capture are deliberately unproven until the parent run. If they differ, retain the diagnostics and report the actual reason; do not weaken assertions or fabricate private progress.

Initial sandboxed Git stage/commit attempts executed and returned `Operation not permitted` creating the task worktree index lock. Narrow approved Git-only escalations then staged and committed only this lane's file successfully. No automatic approval rejection occurred.

## Compiler fix

Parent actual 26-selector focused build stopped before any test executed with two E0277 errors: `MessageId` does not implement Display in the new assertion diagnostics at lines860/898. Actual compiler output was read from `/private/tmp/ht-sweep-fix-focused-d044-fa044-04cc/list.stderr`; parent's `execution.json` records 10.174s listing/build time, no tests run and zero leaked processes for run UUID `8b4b1b83`.

Separate commit `f746e5698205e7e2cd08b6b70a538bf919f091a4` changes exactly the two `{id}` formats to `{id:?}`. No fixture geometry, hypothesis, oracle, assertions or production changes. `rustfmt --edition 2024 tests/service/q4_accept_controls.rs` and `git diff --check` completed successfully; branch is clean. Parent retains sole runtime ownership and will rerun the exact26 selectors. Runtime PASS remains unclaimed.

## Parent runtime evidence

Actual combined validation 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2 / tree29f7d42d742fa09a56bf64f9de1648420e2565e3: exact26 tests PASS (13.506s nextest summary, process13.737s; list/build52.117s), scoped leaks0 UUID aa07d15e-4108-41cd-8b65-f16cde5c0b5e. Exact command, selection and output: /private/tmp/ht-sweep-fix-focused-corrected-f746/execution.json and run.log. Original3 now reach full post-setup preservation assertions. Every task-changed file blob independently matches author f746e5698205e7e2cd08b6b70a538bf919f091a4. Combined required fmt/neutral/strictclippy/default PASS, evidence /private/tmp/ht-sweep-combined-gates-0b04/execution.json. No production change, cap/time limit adjustment or failed assertion waiver.
