# Repaired fba17e52: green gates, 56/60 controls and completed static rereviews

**C remains NOT READY for final qualification.** The completed corrected gates pass and all parent-identified reached production REDs pass in this focused run. Four remaining fixture/oracle failures are preserved. Earlier REVISE review copies, failures, compile-only attempts and interrupted outcomes remain unchanged.

Candidate HEAD `fba17e52b5a607c081ffd9e1e78c51f79eec4acf`, tree `68fd0e88cad46f89d270287b938701f7e2c27ee3`, scoped run ID `c0c30643-7f37-4497-bf23-5a9c400e4af2`. Exactly 60 cases expected and selected. Listing exit 0; runtime exit 100; scoped leak check exit 0.

## Corrected green source gates

Staged tree `68fd0e88cad46f89d270287b938701f7e2c27ee3`, index SHA-256 `c6fd4f4e8b65d314ed4bb6e532198f12e4ff018c48edabf2b95b13dac7d5f7d6`; all gate and aggregate exits 0. Execution file carries tree/index identity, not HEAD. All embedded command-log hashes were verified.

- fmt: 4.3624958749860525 seconds, exit 0.
- neutral: 1.3010608749464154 seconds, exit 0.
- clippy: 21.950192208052613 seconds, exit 0.
- default: 9.68688370892778 seconds, exit 0.

## Actual focused outcome

- Raw nextest summary: 60 tests run, **56 passed, four failed**, 4835 skipped, 2.358 seconds. No SIGINT interruption appears in this run. Individual result lines independently verify these counts; duplicate final-summary result lines were not counted twice.
- Wrapper duration 2.583871249924414 seconds. Listing/build duration 67.38033908291254 seconds; compiler stderr reports 1m 05s. Preserve this multi-file test-build measurement above one minute; it is not a controlled single-production-file incremental benchmark and does not establish a production regression mechanism.
- Parent-identified demonstrated production REDs now PASS in this scope: both active final-send fences, independent direct-store-ACK ordinary scanner, logical virtual Notice same-token replay, and actual query plan/VM controls. This scoped GREEN does not establish complete expanded-matrix, registered-hook or final qualification success.

## Remaining four fixture/oracle failures (parent diagnosis)

- Fast-forward final cursor oracle compares the allocator `threads.next_sequence` against published-head coverage; its coalesced counter-increment assertion already passes. Correct the final-cursor oracle, then rerun.
- Two Lazy fixture setups violate the preparation/message-ID CHECK relation; they fail before their completion-hint behavior can be assessed.
- Presentation fixture uses generation 0, an invalid V2 position. Its setup must become valid before inert successful-confirm behavior can be assessed.

Exact failing selectors:

- `service::delivery_pull::tests::presentation_q3_fast_forward_coalesces_every_gap_beyond_description_cap`
- `service::dispatch::mod_ack_dispatch_tests::q3_completion_hint_delivery_confirm_lazy_mixed_and_retained_offer`
- `service::dispatch::mod_ack_dispatch_tests::q3_completion_hint_mod_ack_lazy_mixed_and_already_settled`
- `service::dispatch::mod_ack_dispatch_tests::q3_completion_hint_successful_presentation_confirm_is_inert`

## New immutable review snapshots and continuation

Registry-quality and completion-hint review files now contain their appended **scoped static PASS** rereviews. Full files are copied to new paths, retaining historical REVISE findings and later closure details. Earlier frozen REVISE versions are not overwritten. Static PASS remains distinct from this runtime result and from complete integration readiness.

Parent reports root tracker C was appended with current evidence while retaining its phase and code-roast counters 0. This evidence task did not edit the tracker. Registered-hook six-test execution is RUNNING at dispatch and was not read or copied. Parent retains sole source/Cargo/runtime ownership.

## Byte-exact copied paths and hashes

Every destination was verified byte-equal to its preserved source and SHA-256 equal. Full completed packages include raw inventories, execution/binary provenance and logs. Destinations below are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q3-repaired-gates-lint-corrected/clippy.log` | `ht-C-v090-q3-repaired-gates-lint-corrected/clippy.log` | 219 | `47c8e9117c35b44b428512125e8174fa31b73916aa0ff1b819d3dd42ec6a3ccd` |
| `/private/tmp/ht-C-v090-q3-repaired-gates-lint-corrected/default.log` | `ht-C-v090-q3-repaired-gates-lint-corrected/default.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-repaired-gates-lint-corrected/execution.json` | `ht-C-v090-q3-repaired-gates-lint-corrected/execution.json` | 1641 | `35b6ae92f1ba04df7b8c7469519971953c02f9323ae462e0fc895cd5361d492d` |
| `/private/tmp/ht-C-v090-q3-repaired-gates-lint-corrected/fmt.log` | `ht-C-v090-q3-repaired-gates-lint-corrected/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-repaired-gates-lint-corrected/neutral.log` | `ht-C-v090-q3-repaired-gates-lint-corrected/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/private/tmp/ht-C-v090-q3-repaired-fba17e52-runtime60/execution.json` | `ht-C-v090-q3-repaired-fba17e52-runtime60/execution.json` | 21692 | `a65573c3679c224b61d7997b563fc0352998845538852251b1a2b255eb6a9d98` |
| `/private/tmp/ht-C-v090-q3-repaired-fba17e52-runtime60/leaks.log` | `ht-C-v090-q3-repaired-fba17e52-runtime60/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q3-repaired-fba17e52-runtime60/list.json` | `ht-C-v090-q3-repaired-fba17e52-runtime60/list.json` | 866594 | `c3b6c0bce3071807349fb66ee136dca401810459925d5578dca4cd6ed9db981e` |
| `/private/tmp/ht-C-v090-q3-repaired-fba17e52-runtime60/list.stderr` | `ht-C-v090-q3-repaired-fba17e52-runtime60/list.stderr` | 220 | `82302003673de96ce9c6633a1fe7c33db0a4ba145f0ce491dbefa612e2312357` |
| `/private/tmp/ht-C-v090-q3-repaired-fba17e52-runtime60/run.log` | `ht-C-v090-q3-repaired-fba17e52-runtime60/run.log` | 13547 | `aa29a00e2191e8721602917bc2c04d3fc480fad469ade92718e36cd6a61dcf88` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-registry-quality-review.md` | `reviews/resume-v090-q3-registry-quality-review.md` | 12184 | `8ad1df6f2f64ecd100959fc0adced108224e95ac1d70bc645b354fb5619f0a77` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-hint-controls-review.md` | `reviews/resume-v090-q3-hint-controls-review.md` | 9487 | `f606b3f034ade4759bd1897e5a7caf6d1f903e15d07f2229e462f76197d6fe65` |

Only new evidence/documentation paths were written. No source/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. Running registered-hook package and author files were untouched. QUIESCENT.
