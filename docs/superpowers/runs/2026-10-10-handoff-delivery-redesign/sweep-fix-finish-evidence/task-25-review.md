# Task review: ht-0lo.17

Status: CLEAN
Base: `dc15706ead701e609ce530c69dd170ecb6ad123c`
Reviewed author HEAD: `f746e5698205e7e2cd08b6b70a538bf919f091a4`

## Spec compliance

✅ Current readonly bead description, frozen Task25 section and brief agree; there are no comments or amendments. Only the named defective test in `tests/service/q4_accept_controls.rs` changes. The setup first records and asserts actual full-capacity geometry, selects an actually codec-rendered whole ordinary message, uses the existing Context delivery-ACK route to reclaim one slot, and proves full256 refill with another real unoffered canonical pending/unread carried ID. Original nonempty-carry, outstanding-token, exact reader-state/counter and complete-offer-stamp preservation assertions remain; receipt and cursor preservation assertions add coverage. No production/cap/deadline/history or Goal changes occur.

The two-unoffered/empty-carry expectations have independent experimental support: parent success output at `run.log:276–277` records before `offers=256 sent_offers=255 lazy=true rendered=255 unoffered=2 carry=[]`, and after actual display settlement `offers=256 rendered=1 unoffered=1 carry=[real ID]`. This correction establishes the task's required acceptance-state precondition rather than weakening its oracle.

## Issues

Critical: none.
Important: none.
Minor: none.
⚠️ items: none.

## Focused checks

Concrete risk: the new ACK could be a manual acknowledgement or the selected body could merely be a fabricated store item. Read `collect_page`/`drain_watch` at `tests/service/q4_accept_controls.rs:83–205` and `OfferWindowRig::ack` at `tests/service/delivery_pull.rs:9087`. The bounded drain actually serializes and decodes production Watch lines, checks output size and body/flag equality, and collects real rendered messages. `ack(..., true, false, ...)` reaches `service.handle(Command::AckModDelivered)` with `ModDeliveryVia::Context` and requires every outcome Settled. Combined with the selected nontruncated offer and exact full body tuple, this supplies the intended whole display evidence. No shared boundary or production behavior is changed, so sibling production migration tracing is not applicable.

Concrete risk: parent evidence could describe a different fixture. Author HEAD and validated parent HEAD `0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2` both resolve the changed file to blob `534d9354eb3525c82d344eb3d2bc12f1005ec2a7`; validated tree is `29f7d42d742fa09a56bf64f9de1648420e2565e3`.

## Test evidence

The implementer report honestly predates runtime GREEN and records the initial E0277 build failure plus its isolated Debug-format correction. Per the dispatch override, supplement it with actual parent evidence in `/private/tmp/ht-sweep-fix-focused-corrected-f746/execution.json` and `run.log`. Its exact command is recorded in `run.argv`: `nice cargo nextest run --locked --all-features --lib --no-fail-fast --success-output immediate -E` followed by the recorded26 exact-name selectors, including this test and the relevant carry/reclamation/acceptance controls. The list and run exited0; actual output reports26 passed, with the named regression reaching its original final acceptance assertions. Scoped leak check `scripts/check-no-leaked-processes --run-id aa07d15e-4108-41cd-8b65-f16cde5c0b5e` exited0. This is meaningful coverage, with no failed runtime result being glossed over.

Required combined gates in `/private/tmp/ht-sweep-combined-gates-0b04/execution.json` all exited0: `cargo fmt -- --check`, `scripts/check-harness-neutral`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and `nice scripts/check-default-features`. Reviewer ran no tests, Cargo, Herdr, or tracker mutations.

Test changes commands (both successful):

`git diff --stat dc15706ead701e609ce530c69dd170ecb6ad123c..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'`

`git diff dc15706ead701e609ce530c69dd170ecb6ad123c..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'`

Result: one test file,172 additions/9 deletions; no deleted/skipped/loosened tests. Review package is nonempty (two commits) and base ancestry check passed.
