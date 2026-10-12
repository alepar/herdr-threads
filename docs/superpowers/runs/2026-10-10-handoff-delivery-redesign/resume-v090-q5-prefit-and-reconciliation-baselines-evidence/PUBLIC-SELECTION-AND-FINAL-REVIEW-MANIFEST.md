# Public selection correction and finalized review: immutable addendum

**C remains NOT READY; formal code-roast counters remain 0.** This addendum preserves the stopped prefix-selection attempt, actual corrected-selection run and the now parent-confirmed finalized/quiescent public-test review. Earlier files remain unchanged.

## Review-cutoff correction

The earlier `reviews/resume-v090-q5-public-tests-review.md` copy was byte-verified before the parent notified this task that the report was still being appended. Its “finalized” characterization in the initial manifest should be read only as a historical cutoff, not confirmed final-review quiescence. That historical copy remains immutable. The new `reviews/finalized-resume-v090-q5-public-tests-review.md` was copied only after parent confirmation of final quiescence; it preserves original public-test REVISE, subsequent public-control scoped PASS and a separate unit-patch REVISE. A verdict for one patch is not transferred to another.

## Stopped prefix attempt: no runtime

HEAD `94fde7aeed80d3febbd1cee86c0184fbcb161133`, tree `840a7daedad3c041f40c89f634511d20356711b2`, scoped UUID `ec21f18d-8705-4222-bd83-51d74b7dbef1`. Listing compilation succeeded (exit 0, 10.43413116701413 seconds); execution has no runtime/selected/binary section. Scoped leak check exit 0.
Parent reports the harness stopped at an empty-selection assertion: it used an incorrect `cli::` prefix while compiled test keys were `lazy_markers::q5_continuation_public::...`. This is a parent selection mistake, not a product test failure or completed runtime. The execution metadata does not serialize that assertion; supplied attribution is kept explicit.

## Actual corrected-selection public six-control run

HEAD `94fde7aeed80d3febbd1cee86c0184fbcb161133`, tree `840a7daedad3c041f40c89f634511d20356711b2`, scoped UUID `5f6293f4-cbc8-422e-b213-c9d0510e1bbf`. Exactly six expected/selected/executed tests: **two PASS, four actual causal FAIL**. Nextest summary duration 1.135 seconds, 729 skipped; listing exit 0 in 0.22346745908726007 seconds; wrapper runtime exit 100 in 1.3659008749527857 seconds; scoped leak check exit 0.
Parent identifies the four reached failures as public reduced-bound byte-exact output mismatches. Full raw byte arrays are retained in `run.log`; they are not truncated or reproduced in the manifest. No continuation/output repair is inferred from the two passing controls.

Raw case outcomes:

- PASS: `lazy_markers::q5_continuation_public::q5_public_insufficient_original_bound_writes_nothing_and_performs_no_display_effect`
- FAIL: `lazy_markers::q5_continuation_public::q5_search_public_refits_preserve_2048_byte_filter_route_actor_and_cursor`
- FAIL: `lazy_markers::q5_continuation_public::q5_search_public_reselected_no_lazy_page_still_restores_original_bound`
- FAIL: `lazy_markers::q5_continuation_public::q5_history_public_refits_preserve_1400_byte_and_row_bounds_through_completion`
- FAIL: `lazy_markers::q5_continuation_public::q5_history_public_reselected_no_lazy_page_still_restores_original_bound`
- PASS: `lazy_markers::q5_continuation_public::q5_public_json_and_unadvertised_modes_keep_initial_serialization_unchanged`

## Current pending work (parent supplied)

Parent records a new production candidate `ccf658fa` from reviewed `8f4`, both implementation reviews scoped PASS. Root production remains unpromoted. New public6 runtime 75752 is active and was not read or copied. Source review PASS does not replace its pending execution. Earlier compile-only and reconciliation failures remain preserved.

## Exact new copies and SHA-256

Every destination below is byte-equal to its preserved source and SHA-256 verified. Both complete packages include raw inventories, logs and scoped leak proof; current-review copy is a new immutable path. Destinations are relative to this evidence directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q5-public6-fixture-corrected/execution.json` | `ht-C-v090-q5-public6-fixture-corrected/execution.json` | 1232 | `5ec98e5c9e0a41f532778ff600debbf7c3ec65c7d68b6c88dc541d2255d31645` |
| `/private/tmp/ht-C-v090-q5-public6-fixture-corrected/leaks.log` | `ht-C-v090-q5-public6-fixture-corrected/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q5-public6-fixture-corrected/list.json` | `ht-C-v090-q5-public6-fixture-corrected/list.json` | 126828 | `05a7d27da13564c741283e6663b024caf61712c5094f91e5115de299c80ccac2` |
| `/private/tmp/ht-C-v090-q5-public6-fixture-corrected/list.stderr` | `ht-C-v090-q5-public6-fixture-corrected/list.stderr` | 219 | `2c3b700bd4631f2b16cb3b65f1bb399c1918672b913bfbe10e0d7ab4df0e75cd` |
| `/private/tmp/ht-C-v090-q5-public6-corrected-selection/execution.json` | `ht-C-v090-q5-public6-corrected-selection/execution.json` | 3543 | `ecb5d0ef2b028421b3fb9a36da7e48e7d61c2c8a58798223fa0a243e781a0292` |
| `/private/tmp/ht-C-v090-q5-public6-corrected-selection/leaks.log` | `ht-C-v090-q5-public6-corrected-selection/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q5-public6-corrected-selection/list.json` | `ht-C-v090-q5-public6-corrected-selection/list.json` | 126690 | `0780b2011675f94e7bc928b22bbebed6646e702e6007ed690b8aed1d7561fbda` |
| `/private/tmp/ht-C-v090-q5-public6-corrected-selection/list.stderr` | `ht-C-v090-q5-public6-corrected-selection/list.stderr` | 73 | `eace1726b7004b659d109fe8d7d692015b8d9b90310f4aaac3afa5f19beb649c` |
| `/private/tmp/ht-C-v090-q5-public6-corrected-selection/run.log` | `ht-C-v090-q5-public6-corrected-selection/run.log` | 51251 | `ae6598344e0ff473618c24ac55b32b3e03e0161d3062e77225af06f0da6d19da` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q5-public-tests-review.md` | `reviews/finalized-resume-v090-q5-public-tests-review.md` | 15686 | `a7b0cd533ed911823a6439ab75ef54ae3de54515ee58b2a3a53ed785ef82d253` |

Only new evidence/documentation files were written. No parent/source/test/index/Git mutation, Cargo/runtime test, Herdr write, subordinate agent or process launch occurred. Earlier snapshots and active public runtime remain untouched. QUIESCENT.
