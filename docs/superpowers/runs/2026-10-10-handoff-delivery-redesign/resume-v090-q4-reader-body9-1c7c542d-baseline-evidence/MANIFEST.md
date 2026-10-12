# Q4 reader/body baseline: reached RED, setup limits and approved-design evidence

**C remains active and NOT READY; formal code-roast counters remain 0.** This checkpoint preserves completed Q3 source gates, Q4 compile failure and exact nine-control baseline, plus finalized design and design/test reviews. All earlier original failures and historical verdicts remain intact. No source change or new GREEN is asserted.

## Completed source gates at 3dd448ef

Staged tree `804e9284e76c2f9a00cc0fb87dcc0982faf81fea`, index SHA-256 `4797d90a7220329672af5562118aa2a4da38e22829a94b2f0b45fab266ae165b`; every command and aggregate exits 0. HEAD association with 3dd448ef is parent-provided; gate metadata records tree/index identity only.

- fmt: 4.345005249953829 seconds, exit 0.
- neutral: 1.3101029170211405 seconds, exit 0.
- clippy: 20.839205291937105 seconds, exit 0.
- default: 7.6067659999243915 seconds, exit 0.

Embedded command-log hashes were matched before copying.

## Compile-only six-control attempt

HEAD `f0cc8d396404bdb9d61c7e4304936186872c6e2c`, tree `77d4d6644b0c042122bc00d3a9df8f3f40f7bc8d`, scoped run UUID `ce1cf05e-cc57-4753-a8a6-296d42a0be75`. Six cases expected; compilation/list exit 101 after 7.067951999953948 seconds because `LocalService` was absent from trait scope (E0599, `DomainService::handle`). No runtime executed, no selected/binary evidence produced; raw list JSON is empty. Scoped leak check exit 0.

## Actual nine-control baseline

HEAD `1c7c542d2955cadb905d9aa0bd41685828e43690`, tree `16bdd39da9be9be204781c9a86f2ce9e98354786`, scoped run UUID `b17475c8-d5a2-46e6-bcf8-3e640570f627`. Exactly nine expected and selected tests. Individual raw statuses independently verify **one PASS, eight FAIL**, no SIGINT. Nextest summary duration 0.375 seconds, 4895 skipped; wrapper runtime 0.592704416019842 seconds, exit 100. Listing/build exit 0 after 56.83834712498356 seconds; stderr records 54.96 seconds compiler duration. Scoped leak check exit 0.

### Reached evidence and explicit limits (parent classification)

- Five store positive controls reach desired reader/acceptance causal RED: cursorless initial readability, independent initial bindings, initial acceptance preserving unread gaps, newly enrolled post-join binding versus still-unread sibling, and fresh second-episode direct-Lazy eligibility. Preserve the exact raw first failing assertions; later assertions in each control were not reached.
- The rejection negative control PASS preserves refusal against cursorless direct-Lazy revival/display bootstrap in this scope.
- Private continuation FAIL occurs at the first expected truncated Lazy body being absent. It is reader-eligibility RED only; it does not yet prove a continuation-after-sibling-display defect.
- Two CLI body controls fail during `ContextJournal` root setup with ENOENT before their decisive display/Advance assertions. They supply no actual AdvanceCursors RED yet. Parent reports the author is correcting only private-path creation; that correction has no new result in this checkpoint.

Exact raw outcomes:

- FAIL: `store::control::tests::direct_threads::reader_controls::q4_direct_lazy_completion_is_independent_for_two_initial_bindings`
- FAIL: `store::control::tests::direct_threads::reader_controls::q4_initial_accept_preserves_each_bindings_unread_body_and_display_gap`
- FAIL: `store::control::tests::direct_threads::reader_controls::q4_initial_published_direct_lazy_is_readable_without_a_recipient_cursor`
- FAIL: `store::control::tests::direct_threads::reader_controls::q4_binding_enrolled_after_direct_join_keeps_main_seat_maximum_no_backlog`
- FAIL: `store::control::tests::direct_threads::reader_controls::q4_episode_two_fresh_direct_lazy_is_admitted_without_old_backlog_before_or_after_accept`
- FAIL: `service::delivery_pull::tests::q4_body_controls::private_captured_lazy_tail_resumes_preaccept_after_sibling_full_text_display`
- FAIL: `service::delivery_pull::tests::q4_body_controls::preaccept_utf8_body_final_flushed_text_advances_only_its_binding_and_invalidates_its_consumer`
- FAIL: `service::delivery_pull::tests::q4_body_controls::preaccept_body_json_machine_partial_and_failed_stdout_do_not_advance_or_invalidate`
- PASS: `store::control::tests::direct_threads::reader_controls::q4_rejection_prevents_cursorless_direct_lazy_revival_or_display_bootstrap`

## Frozen design and reviews

The current episode design matches approved SHA-256 `472159f833dfb65ba4b5096b69fc87fe461ec82c63864dc794e9f2eef7d9f1ca`. The full file, two design assessments and two test reviews are copied unchanged. Design assessments preserve initial REVISE history and appended exact-design scoped PASS; test reviews pass the nine authored controls while explicitly leaving the complete Q4 verification matrix open. These source/design verdicts do not upgrade runtime setup failures or establish full design completion.

The parent-owned `resume-v090-q4-parent-decision.md` was neither copied, read nor edited by this task. Current root phase/counters and all source/runtime ownership remain with the parent.

## Byte-exact copied files and SHA-256

All supplied completed package files are copied, including raw listing inventories, execution/binary provenance, logs and scoped leak results. Every destination was verified byte-equal to its preserved source and SHA-256 equal. Paths are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q3-current-3dd448ef-gates/clippy.log` | `ht-C-v090-q3-current-3dd448ef-gates/clippy.log` | 219 | `8b18e7c51c6beb1782e36e7b50f6df12c1ae07dfce6b25f65014e03596878566` |
| `/private/tmp/ht-C-v090-q3-current-3dd448ef-gates/default.log` | `ht-C-v090-q3-current-3dd448ef-gates/default.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-current-3dd448ef-gates/execution.json` | `ht-C-v090-q3-current-3dd448ef-gates/execution.json` | 1614 | `7d682b547df79a940a2ca37f420762b95a47ed58906519e2a878615c5bc2281b` |
| `/private/tmp/ht-C-v090-q3-current-3dd448ef-gates/fmt.log` | `ht-C-v090-q3-current-3dd448ef-gates/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-current-3dd448ef-gates/neutral.log` | `ht-C-v090-q3-current-3dd448ef-gates/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/private/tmp/ht-C-v090-q4-reader6-baseline-f0cc8d39/execution.json` | `ht-C-v090-q4-reader6-baseline-f0cc8d39/execution.json` | 1302 | `03ba1d3851b00b406f0098b223375cf9ac54a567d080bf9ce9ee84f0038a86ac` |
| `/private/tmp/ht-C-v090-q4-reader6-baseline-f0cc8d39/leaks.log` | `ht-C-v090-q4-reader6-baseline-f0cc8d39/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-reader6-baseline-f0cc8d39/list.json` | `ht-C-v090-q4-reader6-baseline-f0cc8d39/list.json` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q4-reader6-baseline-f0cc8d39/list.stderr` | `ht-C-v090-q4-reader6-baseline-f0cc8d39/list.stderr` | 2155 | `3b48f4c22cb7cc63347bb0d224b911f8bdc8305429073524bf5410b5231d2313` |
| `/private/tmp/ht-C-v090-q4-reader-body9-baseline-1c7c542d/execution.json` | `ht-C-v090-q4-reader-body9-baseline-1c7c542d/execution.json` | 5144 | `2d2826299689c2f6df61e50b4c2e1103cb15fbd3092e3708af2a217693e897fc` |
| `/private/tmp/ht-C-v090-q4-reader-body9-baseline-1c7c542d/leaks.log` | `ht-C-v090-q4-reader-body9-baseline-1c7c542d/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-reader-body9-baseline-1c7c542d/list.json` | `ht-C-v090-q4-reader-body9-baseline-1c7c542d/list.json` | 869784 | `fd9951cb391c8054ac8fab0296e27f9c5a6bbc47cc16c8eb6a33aa1c08064838` |
| `/private/tmp/ht-C-v090-q4-reader-body9-baseline-1c7c542d/list.stderr` | `ht-C-v090-q4-reader-body9-baseline-1c7c542d/list.stderr` | 220 | `b241152a33e367d9b95a7aef07177645bcdcec15706ce474b6db79fc02620a5b` |
| `/private/tmp/ht-C-v090-q4-reader-body9-baseline-1c7c542d/run.log` | `ht-C-v090-q4-reader-body9-baseline-1c7c542d/run.log` | 10615 | `d6ce0bdc72df184d3b1b17386343885499de46aedbc6dd660276e3165c62bc08` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-v090-q4-direct-lazy-reader-episode-design.md` | `reports/resume-v090-q4-direct-lazy-reader-episode-design.md` | 31567 | `472159f833dfb65ba4b5096b69fc87fe461ec82c63864dc794e9f2eef7d9f1ca` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-spec-assessment.md` | `reports/resume-v090-q4-spec-assessment.md` | 20254 | `2fa826c3b646a35a92bf950cb492efde285085167850a1b4827da62c4baa6a6d` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-quality-assessment.md` | `reports/resume-v090-q4-quality-assessment.md` | 30210 | `4de3c5f3c4e2803b5ce38319c1d89b22735c63f6ae620a7365bff76683f25461` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-test-spec-review.md` | `reports/resume-v090-q4-test-spec-review.md` | 9438 | `dc74dddb8f3b37fb4a6d9c52777fc7a6aca85e4ab2b5fd1faec8436c6bd02382` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-test-quality-review.md` | `reports/resume-v090-q4-test-quality-review.md` | 9436 | `c24e462ed66f8fe28786c283979078b202c08b0f21a1c347cf5bacd0460a307b` |

Only new evidence/documentation paths were written. No source/test/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. No runtime loop was performed. Historical artifacts and the parent decision file remain untouched. QUIESCENT.
