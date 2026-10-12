# Finalized Q4 68fab47e checkpoint: focused results, gates and static reviews

**C remains NOT READY; no waiver or final clearance. Formal code-roast counters remain 0.** The SQL-corrected nineteen-control run has eighteen PASS and one explicit known Q5 budget FAIL; registered-Hook6 passes all six. Physical Q4 SQL proofs, complete matrix and formal code roast remain pending. Earlier original failures and historical review cutoffs are preserved unchanged.

Runtime candidate HEAD `68fab47e7b0aa80449b9db0e9afe9974ab2cd4c7`, tree `3074af207bd6842ee5d4e569f94b7e49d273413e`. Both completed runtime execution records agree on those identities. This evidence task did not independently invoke Git or attest live source cleanliness.

## Completed runtime scopes

### ht-C-v090-q4-production-sql-corrected-runtime19

- Scoped run UUID `897416c5-6687-4ae3-945f-541e612186ab`; exactly 19 expected, selected and executed cases; 18 PASS, 1 FAIL, no SIGINT. Individual raw results independently verify counts.
- Listing exit 0 after 21.297782249981537 seconds; wrapper runtime exit 100 after 1.4391570000443608 seconds; scoped leak check exit 0.
- Nextest summary duration 1.223 seconds (4895 skipped).

### ht-C-v090-registered-hook6-q4-68fab47e

- Scoped run UUID `0e568cf5-826e-4606-a384-e49cd85f9c49`; exactly 6 expected, selected and executed cases; 6 PASS, 0 FAIL, no SIGINT. Individual raw results independently verify counts.
- Listing exit 0 after 21.319928749930114 seconds; wrapper runtime exit 0 after 5.87619950005319 seconds; scoped leak check exit 0.
- Nextest summary duration 5.668 seconds (77 skipped).

## Sole unresolved known failure

`service::delivery_pull::tests::q4_body_controls::preaccept_utf8_body_final_flushed_text_advances_only_its_binding_and_invalidates_its_consumer` remains the sole runtime19 failure. Its raw diagnostic retains the step-7 annotated body budget shrink to 274 bytes at offset 4298, with InvalidBudget minimum 288. SQL correction does not resolve this Q5 envelope defect. It is not waived, reclassified as PASS or hidden by the eighteen Q4 successes.

Registered-Hook6 now passes its direct-Ensure Lazy control along with all five other controls. This is named registered-hook coverage, not full matrix qualification. Eighteen passing Q4 controls are scoped evidence; comprehensive private-retirement refusal/rollback and measured SQL bounds are still pending.

## Completed source gates

Staged tree `3074af207bd6842ee5d4e569f94b7e49d273413e`, index SHA-256 `2541f0f1b4b814cb3069667471b7b13db85eb2822a098ce1b67c41851ba3d08b`; every command and aggregate exits 0. The gate metadata has no HEAD field; matching tree and parent context associate it with 68fab47e. Embedded command-log hashes were independently matched.

- fmt: 4.448485874920152 seconds, exit 0.
- neutral: 1.3107847089413553 seconds, exit 0.
- clippy: 20.512648665928282 seconds, exit 0.
- default: 7.557052707998082 seconds, exit 0.

## Finalized static snapshots and remaining gates

Q4 implementation specification and quality reviews are copied with their finalized exact bytes, including earlier findings, refinement assessments and the spec SQL-correction appendix. The spec’s historical runtime-active cutoff remains unchanged; this checkpoint separately records the now-completed actual result. Static review does not replace runtime, physical SQL plans/VM/counter work, complete expanded matrix or formal roast.

No separate bookend file was present in supplied packages. Available provenance is preserved in full execution metadata (HEAD/tree or staged tree/index, selected inventory, binary hashes, argv and elapsed timings), with listing stderr/run logs and scoped leak proof. No bookend or clean-status evidence was fabricated.

Parent decision, root run/tracker, source and index files were outside this task and untouched.

## Byte-exact sources, copied paths and SHA-256

Every completed package file and both review snapshots are copied byte-equal to their preserved source and SHA-256 verified. Destinations are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q4-production-sql-corrected-runtime19/execution.json` | `ht-C-v090-q4-production-sql-corrected-runtime19/execution.json` | 9895 | `98048eda9a741522c0bebd57b6db3e4b88d727496425bbb7338431a1b383d54a` |
| `/private/tmp/ht-C-v090-q4-production-sql-corrected-runtime19/leaks.log` | `ht-C-v090-q4-production-sql-corrected-runtime19/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-production-sql-corrected-runtime19/list.json` | `ht-C-v090-q4-production-sql-corrected-runtime19/list.json` | 871867 | `c23383a8bbc3eea9d5aa787fe0a336b1c23464f6b85e1712062359b78eef46ce` |
| `/private/tmp/ht-C-v090-q4-production-sql-corrected-runtime19/list.stderr` | `ht-C-v090-q4-production-sql-corrected-runtime19/list.stderr` | 220 | `bc812115b18ba0318f0abca4e65b9ccb948bcb295a561f3f037f5339049191e7` |
| `/private/tmp/ht-C-v090-q4-production-sql-corrected-runtime19/run.log` | `ht-C-v090-q4-production-sql-corrected-runtime19/run.log` | 5179 | `1961263a4c3e3f6f9013143635b7a47d52c0b2556b27b20268585705b6cbd8c1` |
| `/private/tmp/ht-C-v090-registered-hook6-q4-68fab47e/execution.json` | `ht-C-v090-registered-hook6-q4-68fab47e/execution.json` | 3007 | `7f15f5b84f280c1831bc407592f394191447ccce7e5fa0585155cf478f1291dc` |
| `/private/tmp/ht-C-v090-registered-hook6-q4-68fab47e/leaks.log` | `ht-C-v090-registered-hook6-q4-68fab47e/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-registered-hook6-q4-68fab47e/list.json` | `ht-C-v090-registered-hook6-q4-68fab47e/list.json` | 15096 | `26d5ddb2e7768ce4d455e781d5be866fdd8ffc3b36407c599f47337536789f52` |
| `/private/tmp/ht-C-v090-registered-hook6-q4-68fab47e/list.stderr` | `ht-C-v090-registered-hook6-q4-68fab47e/list.stderr` | 220 | `483d62387e09be1e46fcaa49537869b934006c1cd98e96ecccf2feac8fb83ce7` |
| `/private/tmp/ht-C-v090-registered-hook6-q4-68fab47e/run.log` | `ht-C-v090-registered-hook6-q4-68fab47e/run.log` | 1202 | `f32524b7d097c64afe4403094c950cf5bafe48f90dc946bc86f0d8976462d8a0` |
| `/private/tmp/ht-C-v090-q4-68fab47e-gates/clippy.log` | `ht-C-v090-q4-68fab47e-gates/clippy.log` | 219 | `bf3ade448fce19a22a2842da3a7167c33fa5eeb7f34c21c30699c54728e55409` |
| `/private/tmp/ht-C-v090-q4-68fab47e-gates/default.log` | `ht-C-v090-q4-68fab47e-gates/default.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q4-68fab47e-gates/execution.json` | `ht-C-v090-q4-68fab47e-gates/execution.json` | 1581 | `477a3624c01d925eada1d79a18e5f544fdc937e7d47174d85e6483e316d26ce6` |
| `/private/tmp/ht-C-v090-q4-68fab47e-gates/fmt.log` | `ht-C-v090-q4-68fab47e-gates/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q4-68fab47e-gates/neutral.log` | `ht-C-v090-q4-68fab47e-gates/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-implementation-spec-review.md` | `reviews/resume-v090-q4-implementation-spec-review.md` | 31580 | `480abb471c3cdfc4fd1724e27e56c11ae0867af95c0e85f0d50a5aa3b4778500` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-implementation-quality-review.md` | `reviews/resume-v090-q4-implementation-quality-review.md` | 23423 | `334f8ef4472609e151c08fc9468e59f452cb5ce755fd91ad0bdeb58b70af1fc7` |

Only this new evidence directory was written. No source/test/index/Git mutation, Cargo/runtime test, Herdr write, subordinate agent or process launch occurred. Historical files remain intact. QUIESCENT.
