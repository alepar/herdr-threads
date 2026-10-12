# Q4 body/acceptance baselines and approved IQ1 supplement

**C remains active and NOT READY; formal code-roast counters remain 0.** Three completed baseline packages and the approved IQ1 supplement/TRUST delta are preserved unchanged. Compile-only f0 and original 1c baselines, prior setup failures, causal REDs and reviews remain historical and untouched.

## Completed immutable baselines

### ht-C-v090-q4-body3-private-root-baseline

- HEAD `a6daa3ec644569a5b3c1cdd0a53272a5fb6e083b`; tree `7eb66f3b1e1eebfdd4ce0ac381fddf1b98c2202e`; scoped run UUID `830fcdb4-e68f-49de-b455-a5af74b17813`.
- Summary [   0.349s] 3 tests run: 0 passed, 3 failed, 4901 skipped
- Listing exit 0, 39.452642249991186 seconds; runtime exit 100, wrapper 0.5697159999981523 seconds; scoped leak check exit 0.
- Exactly 3 expected/selected cases; raw individual counts independently verified, with duplicate summary rows counted once.

### ht-C-v090-q4-body-accept7-baseline

- HEAD `25d2c4e67b6e5ec07612fc3a671334a642cf3eee`; tree `78fda2c9b69ed9075fa97bf97ef7564efe33f98f`; scoped run UUID `3ae06b48-df86-44db-ac67-284aae5ae4d5`.
- Summary [   0.517s] 7 tests run: 1 passed, 6 failed, 4901 skipped
- Listing exit 0, 52.821621499955654 seconds; runtime exit 100, wrapper 0.7373937919037417 seconds; scoped leak check exit 0.
- Exactly 7 expected/selected cases; raw individual counts independently verified, with duplicate summary rows counted once.

### ht-C-v090-q4-accept4-codec-baseline

- HEAD `c3fdba6b442c8ea3de3924cd29efd5c1fc175db3`; tree `7df0407f214c6ad47e60912f804a1b9b949987e1`; scoped run UUID `1a9fc9bb-c526-4d45-a47e-71f723465edd`.
- Summary [   1.044s] 4 tests run: 0 passed, 4 failed, 4904 skipped
- Listing exit 0, 57.260945249930955 seconds; runtime exit 100, wrapper 1.259980084025301 seconds; scoped leak check exit 0.
- Exactly 4 expected/selected cases; raw individual counts independently verified, with duplicate summary rows counted once.

## First reached assertions and causal limits

- a6 private-root body3: all three fail. Two stop in UnsafePath setup, supplying no body-display/AdvanceCursors RED. The private continuation control stops at missing first Lazy body, establishing reader eligibility RED only, not tail-resumption behavior.
- 25d body+accept7: one PASS, six FAIL. JSON/machine/partial/failed-output negative control passes. Full flushed body fails with InvalidBudget (`annotated read exceeds byte budget`, required minimum 288 bytes); diagnostic cause remains pending and this outcome is not uniquely attributed to the desired display/Advance change. Private continuation still stops at missing first Lazy body. All four accept controls stop in the wrong Watch `page.text` rendered-body oracle during setup; this baseline alone is not acceptance causal RED.
- c3fd codec-corrected accept4: all four reach actual acceptance causal RED. Framed Watch body is None instead of the newly joined settled ordinary body; private-counter overflow is accepted rather than refusing/rolling back; disconnected/absent/stale fanout and retained carry/outbox controls reach missing one private counter increment. Corrected actual codec framing distinguishes these failures from the previous page.text setup oracle. Preserve their raw first reached assertions and avoid claiming later assertions completed.
- Recorded listing/build and runtime durations are scoped measurements. No controlled production incremental benchmark is inferred and no budget was raised.

## Stable supplement and currently active exclusions

Parent identifies IQ1 supplement SHA-256 `29aa8f134a6fc710317becc6c72f3c8b7843eece7745691d8e121e89d51b19d4` and TRUST delta `06b18289854b632d7478f2f42de3ce753528a2615bf088541a13a05fa885c682` as finalized/approved. Both exact hashes are independently verified and their full bytes copied. Their historical prospective wording remains unchanged; approval here is parent-attributed, not an invented author verdict or runtime result.

The current 107-test writer/budget6 run is RUNNING and was not read or copied. Implementation spec/quality reviewers are actively appending revised code verdicts; updated review snapshots are deferred until parent confirmation of completion. No partial review append was frozen. The parent decision file remains outside this task.

## Byte-exact artifacts and SHA-256

All package logs, inventories, execution/binary provenance and scoped leak evidence are included. Every destination is byte-equal to its preserved source and SHA-256 verified. Destinations below are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q4-body3-private-root-baseline/execution.json` | `ht-C-v090-q4-body3-private-root-baseline/execution.json` | 2448 | `741d17e6fd77555e80fcb26e64a09fec511e461f1ae70b51e03c51ab428f786f` |
| `/private/tmp/ht-C-v090-q4-body3-private-root-baseline/leaks.log` | `ht-C-v090-q4-body3-private-root-baseline/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-body3-private-root-baseline/list.json` | `ht-C-v090-q4-body3-private-root-baseline/list.json` | 869922 | `25f93083520ea3728023634f8f8c2002877564c9a8a867e94e9845c5ce2f78ba` |
| `/private/tmp/ht-C-v090-q4-body3-private-root-baseline/list.stderr` | `ht-C-v090-q4-body3-private-root-baseline/list.stderr` | 220 | `d71b0fa609378564e3af8fd4778e54460754cd1d1315b7ae6355e0e49c10a59f` |
| `/private/tmp/ht-C-v090-q4-body3-private-root-baseline/run.log` | `ht-C-v090-q4-body3-private-root-baseline/run.log` | 4171 | `1827cf1fe21afe3e8abd1079e4d7d504b976a908ef330e5a46da7c4efc8617a1` |
| `/private/tmp/ht-C-v090-q4-body-accept7-baseline/execution.json` | `ht-C-v090-q4-body-accept7-baseline/execution.json` | 4279 | `289e07d5193230d0ab535252e05c0aed645eefa2f00b3b34d4fdbac85c78cf14` |
| `/private/tmp/ht-C-v090-q4-body-accept7-baseline/leaks.log` | `ht-C-v090-q4-body-accept7-baseline/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-body-accept7-baseline/list.json` | `ht-C-v090-q4-body-accept7-baseline/list.json` | 870732 | `f7e9a967e7fd0cde1bc680c6154e684b5d05f41f4fbffa879e496fe99e9db824` |
| `/private/tmp/ht-C-v090-q4-body-accept7-baseline/list.stderr` | `ht-C-v090-q4-body-accept7-baseline/list.stderr` | 220 | `435bb9ef1168f5c0c597759b3bfd0c480ce87fb691290f68d057add5776b416f` |
| `/private/tmp/ht-C-v090-q4-body-accept7-baseline/run.log` | `ht-C-v090-q4-body-accept7-baseline/run.log` | 8091 | `2a4a8b2c683c15a2004087299fcee027fc275caf430323331203f3d753669f24` |
| `/private/tmp/ht-C-v090-q4-accept4-codec-baseline/execution.json` | `ht-C-v090-q4-accept4-codec-baseline/execution.json` | 2894 | `78e5149ceeda44b74319598bccc4a1b4c68b9fe818124620cbdc105f5bf58b15` |
| `/private/tmp/ht-C-v090-q4-accept4-codec-baseline/leaks.log` | `ht-C-v090-q4-accept4-codec-baseline/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q4-accept4-codec-baseline/list.json` | `ht-C-v090-q4-accept4-codec-baseline/list.json` | 870801 | `3d82e77856ec152036812ca44b3b384d964d14bc4ec259967ee19200c06feaba` |
| `/private/tmp/ht-C-v090-q4-accept4-codec-baseline/list.stderr` | `ht-C-v090-q4-accept4-codec-baseline/list.stderr` | 220 | `7adc98a86f5bdb040db7107f842c6f55d0981408499c65651b70638952b0fb61` |
| `/private/tmp/ht-C-v090-q4-accept4-codec-baseline/run.log` | `ht-C-v090-q4-accept4-codec-baseline/run.log` | 5767 | `34792b681a6b47ed3c583e6482bbcc974304a32ffd08b2b0903c0dd11604b2bc` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-v090-q4-lazy-continuation-retirement-refinement.md` | `approved-supplement/resume-v090-q4-lazy-continuation-retirement-refinement.md` | 8747 | `29aa8f134a6fc710317becc6c72f3c8b7843eece7745691d8e121e89d51b19d4` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q4-lazy-continuation-retirement-trust-addendum.patch` | `approved-supplement/resume-v090-q4-lazy-continuation-retirement-trust-addendum.patch` | 1863 | `06b18289854b632d7478f2f42de3ce753528a2615bf088541a13a05fa885c682` |

Only new evidence/documentation files were written. No source/test/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. No runtime loop was performed. Earlier originals, active author/review files and parent decision file remain untouched. QUIESCENT.
