# Writer/body-budget RED and b467 production baseline: preserved evidence

**C remains active and NOT READY; formal code-roast counters remain 0.** The completed six-control writer/body baseline, green b467 source gates, failed nineteen-control production run and stable review/design snapshots are preserved. The current 68fab runtime and actively appending Q4 spec successor are excluded. Earlier original failures remain intact.

## Writer/body-budget six-control baseline at 107da32f

HEAD `107da32f07ca90adb8cda96c025b092288fbe7ed`, tree `f1362465754c02bc19652d661e841491a418719a`, scoped UUID `00cfd362-a8a8-4f23-a257-f710991d7911`. Exactly six expected/selected/executed cases: three PASS, three FAIL; nextest 0.584 seconds, 4908 skipped. Listing exit 0 after 46.55468199995812 seconds, wrapper runtime exit 100 after 0.7947809160687029 seconds; scoped leak check exit 0.

- Two service/operator positive controls reach desired canonical reader RED with empty fresh body. Three duplicate/refusal controls PASS. Later unexecuted assertions are not implied by those first failures.
- Actual annotated CLI failure is now diagnostic: step 7 sends server argv `--max-bytes 274` at offset 4298 and receives InvalidBudget with required minimum 288. Parent confirms original caller budget was 2048 and the continuation source shrank it; this is the independently reviewed Q5 causal issue rather than a guessed reader/acceptance failure. No Q5 implementation GREEN is claimed here.

## Green b4671451 source gates

Staged tree `a8a61d810002eb857068a1331ba6d858c70db9cc`, index SHA-256 `45a83df6d3e9e27c0511b99f2fbcd4b0d797a333e4245c48779d23a90cb5c2aa`; every command and aggregate exits 0. HEAD association follows matching b467 runtime tree and parent context; the gate file itself has no HEAD.

- fmt: 4.416580541059375 seconds, exit 0.
- neutral: 1.3142989589832723 seconds, exit 0.
- clippy: 24.787751125055365 seconds, exit 0.
- default: 9.979067624895833 seconds, exit 0.

All embedded log hashes matched before copying.

## b4671451 nineteen-control production baseline

HEAD `b46714512fed8a1ef532e0a17a1771ea92d4e238`, tree `a8a61d810002eb857068a1331ba6d858c70db9cc`, scoped UUID `113b7a7e-9c9c-4507-90ea-8004da0547f6`. Exactly nineteen expected/selected/executed cases: **seven PASS, twelve FAIL**; nextest 0.912 seconds, 4895 skipped. Listing exit 0 after 80.0592000419274 seconds; wrapper runtime exit 100 after 1.1297141669783741 seconds; scoped leak check exit 0.

- Raw log contains exactly ten explicit `SQLite (unclassified, transient): incomplete input` failure blocks. Parent diagnosed a missing closing `)` in `displayed_episode_floor` and committed the one-character 68fab correction. This task read no source and asserts no corrective runtime result.
- Separate sibling-full-body control reaches cursor `None` instead of `Some(3)`; parent reports its quiet Advance error is the same SQL issue. It is retained as its distinct raw failed assertion, not counted as an eleventh explicit SQLite error block.
- Separate annotated full-body failure still shows step 7, reduced 274-byte server budget, offset 4298 and minimum 288: Q5 remains separate from the SQL fix.
- Seven actual passes include initial cursorless body, old-episode private retirement, negative body-output modes, rejection and three writer duplicate/refusal controls. The original private-helper fault/scope test matrix has not thereby been proved; preserve its remaining gates rather than calling the entire private helper qualified.
- Listing/build took 80.0592000419274 seconds; stderr reports compiler completion in 1m 18s. This multi-production-file composed test build is observed speed evidence, not a controlled single-production-file incremental benchmark. No budget was raised.

Actual passing selectors:

- `store::control::tests::direct_threads::reader_controls::q4_initial_published_direct_lazy_is_readable_without_a_recipient_cursor`
- `service::delivery_pull::tests::q4_body_controls::private_old_episode_lazy_tail_retires_and_reaches_fresh_preaccept_body`
- `service::delivery_pull::tests::q4_body_controls::preaccept_body_json_machine_partial_and_failed_stdout_do_not_advance_or_invalidate`
- `store::control::tests::direct_threads::reader_controls::q4_rejection_prevents_cursorless_direct_lazy_revival_or_display_bootstrap`
- `store::control::tests::direct_threads::writer_floor_controls::q4_pair_refuses_managed_ownership_and_required_writer_without_changing_floor_authority`
- `store::control::tests::direct_threads::writer_floor_controls::q4_native_pending_episode_reused_by_native_and_service_writers_keeps_exact_floor_rows`
- `store::control::tests::direct_threads::writer_floor_controls::q4_service_pending_episode_reused_by_service_and_native_writers_keeps_exact_floor_rows`

## Static/design snapshots and active exclusions

Q4 implementation quality review is copied unchanged as its current quiescent static snapshot, preserving initial findings and appended scoped retirement PASS. Q4 implementation spec is excluded pending parent confirmation that the actively appending one-character SQL successor review is done. No partial successor verdict was frozen.

Q5 design SHA-256 `19a6a008fdd9d755869182086e3a68f73fcb33f025747c4197c7fbf637091e80` is independently verified; its finalized spec/quality reviews are copied unchanged. Static design PASS supplies no code/runtime success.

The current 68fab runtime19 remains RUNNING at dispatch and was not read or copied. Parent owns source/runtime/Cargo and the decision file; no parent decision/tracker document was edited.

## Byte-exact copied paths and SHA-256

Full completed packages preserve raw list inventories, binary provenance in execution metadata, run/stderr/leak logs and source-gate logs. Every copy is byte-equal to its preserved source and SHA-256 verified. Paths are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q4-writer-body-budget6-baseline/execution.json` | `ht-C-v090-q4-writer-body-budget6-baseline/execution.json` | 4053 | `979b7dbc6314947c674c543bf14d492bb94fc8cb17a9bf5ae3a771edb26ace12` |
| `/private/tmp/ht-C-v090-q4-writer-body-budget6-baseline/leaks.log` | `ht-C-v090-q4-writer-body-budget6-baseline/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-writer-body-budget6-baseline/list.json` | `ht-C-v090-q4-writer-body-budget6-baseline/list.json` | 872166 | `75dbd2f2dbbd1f5292d5e149cb22908834b0a90d0dccb55e08c49dc7985c1248` |
| `/private/tmp/ht-C-v090-q4-writer-body-budget6-baseline/list.stderr` | `ht-C-v090-q4-writer-body-budget6-baseline/list.stderr` | 220 | `b2ac9ede2389171e58fbc81d4406dd8c2a018f178121f46bf511fcd0829b3d62` |
| `/private/tmp/ht-C-v090-q4-writer-body-budget6-baseline/run.log` | `ht-C-v090-q4-writer-body-budget6-baseline/run.log` | 5288 | `82d21629ad4c1a9ebdc2e1af62fee0e254b77baaf752bd94d2c8a5db55b2f706` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-gates/clippy.log` | `ht-C-v090-q4-production-b4671451-gates/clippy.log` | 219 | `447cbccf89da6a2a552b69918d27081a0d1954a80bd3125ad0d669a8cdb98621` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-gates/default.log` | `ht-C-v090-q4-production-b4671451-gates/default.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-gates/execution.json` | `ht-C-v090-q4-production-b4671451-gates/execution.json` | 1625 | `e06a6fc0d82219cce8c88266b8ee4f14e8d4f24a59e3f4288bf82d5425b709bb` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-gates/fmt.log` | `ht-C-v090-q4-production-b4671451-gates/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-gates/neutral.log` | `ht-C-v090-q4-production-b4671451-gates/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-runtime19/execution.json` | `ht-C-v090-q4-production-b4671451-runtime19/execution.json` | 9893 | `107ccb1e987ec53377918bbb737869bb8b29579e430800aee39935d915fe49c0` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-runtime19/leaks.log` | `ht-C-v090-q4-production-b4671451-runtime19/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-runtime19/list.json` | `ht-C-v090-q4-production-b4671451-runtime19/list.json` | 871867 | `c23383a8bbc3eea9d5aa787fe0a336b1c23464f6b85e1712062359b78eef46ce` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-runtime19/list.stderr` | `ht-C-v090-q4-production-b4671451-runtime19/list.stderr` | 220 | `db6c73f1378b1f8475168f69eb6fd3d186ceace83a8b7287f3e69db0ee76c8ca` |
| `/private/tmp/ht-C-v090-q4-production-b4671451-runtime19/run.log` | `ht-C-v090-q4-production-b4671451-runtime19/run.log` | 18192 | `f897a8e98f42220495516474b40021bbf3c27c2147bf1fe299009518e0f1af4d` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-implementation-quality-review.md` | `reports/resume-v090-q4-implementation-quality-review.md` | 23423 | `334f8ef4472609e151c08fc9468e59f452cb5ce755fd91ad0bdeb58b70af1fc7` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-v090-q5-annotated-continuation-budget-design.md` | `reports/resume-v090-q5-annotated-continuation-budget-design.md` | 10542 | `19a6a008fdd9d755869182086e3a68f73fcb33f025747c4197c7fbf637091e80` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q5-design-spec-review.md` | `reports/resume-v090-q5-design-spec-review.md` | 7447 | `9e05080adde0692e0962738384e53c410d02bf8081c8dc1fbf98344b62bfe18f` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q5-design-quality-review.md` | `reports/resume-v090-q5-design-quality-review.md` | 8173 | `bb2fff126189e08c0756ee8c0584dc2e078ea3de32441b529148a9525bb4e305` |

Only new evidence/documentation files were written. No source/test/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. Earlier artifacts, running execution and active reviewers remain untouched. QUIESCENT.
