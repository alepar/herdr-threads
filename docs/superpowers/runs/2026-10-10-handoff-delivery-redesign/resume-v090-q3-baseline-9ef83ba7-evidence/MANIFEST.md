# Q3 baselines: recoverable failed checkpoint

**Status: actual RED retained; production work remains incomplete.** This checkpoint preserves both completed baseline packages, raw listing inventories, execution metadata and logs, plus the two fresh REVISE reviews and production interface plan. No readiness or new GREEN is asserted.

## Completed baselines

- Initial tests-only baseline: HEAD `33c74793a369fad377036bf182b832c8cb05bac7`, tree `0c1922d5432308ce41e15d277211892f4d4ded32`, run ID `fc297edd-f7af-4640-b64d-2967d2c610b8`. Listing/compile exit 101 after 15.571007999940775 seconds: E0277, `usize: FromSql` unsupported. No runtime was executed, no selected/binary evidence was produced, and the raw listing JSON is empty. Scoped leak check exit 0.
- Compile-corrected baseline: HEAD `9ef83ba79c598c36d3693b0139b026d170cdfa5e`, tree `8d49e081f8968b6de8da68641a5a37f06bbfcab2`, run ID `7275a27f-d923-4f3e-9d72-0c35e34149f8`. Listing exit 0; exactly 26 selected and executed tests; nextest reports 7 passed, 19 failed, 4841 skipped in 1.023 seconds. Wrapper duration 1.2439173749880865 seconds, exit 100; scoped leak check exit 0.
- Preserve the speed evidence: corrected listing/build took 75.5717944590142 seconds, and `list.stderr` reports the test build finished in 1m 13s. This is an observed one-file tests-build/listing duration above one minute, not a controlled production-source incremental-build benchmark or proof of a production regression mechanism. No budget was raised.

The corrected compiled lib binary provenance is preserved in execution metadata: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target/debug/deps/herdr_threads-bc80e388954f6954`, SHA-256 `f151fe90937b08fb192b4b4e4f9f6cb7f3b66b22f0d6aef151a2af280b96d460`. The original expected/selected inventories, HEAD/tree/run IDs, binary hash and argv are copied unchanged. No standalone inventory/selection-seal files existed in either supplied package; the raw `list.json` and `execution.json` preserve available enumeration and provenance. No live binary or Git seal was recomputed.

## Continuation state (parent-supplied coordination)

- Wire negotiation and parked-reader fixes have actual RED. Shared captured-body and coalescing test corrections are underway. An ordinary-scanner extension is not uniquely proved yet; do not treat the broad 19 failures as unique evidence for that extension.
- Production authors are active. Parent applied the reviewed persistent-harness fixture after the Notice causal RED and the two-statement DirectSend fresh-claim fix supported by the actual 318 full-claim RED. Neither change has new GREEN at this checkpoint.
- Both fresh controls reviews remain REVISE. Their shared-capture/coalescing objections and scope limits are retained unchanged. Interface plan is a byte-exact snapshot; it does not upgrade review or runtime status.
- Earlier expanded 657-test failure, Notice replay failure, failed/compile-only repair attempts and installation-pending historical reports remain preserved. Current Main565 publication/install coordination was recorded in the preceding checkpoint and is not reassigned by this task.

## Exact copied paths and hashes

Every copy below was verified byte-equal to its preserved source and SHA-256 equal. Destinations are relative to this checkpoint directory. Full completed packages, including raw listing JSON, are included at the parent’s request.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q3-baseline-33c74793/execution.json` | `ht-C-v090-q3-baseline-33c74793/execution.json` | 3075 | `64b57c9c9a4dbe9b1cfcfa00e53134fc8cd415457b04e8c54f70e39b4e13bb39` |
| `/private/tmp/ht-C-v090-q3-baseline-33c74793/leaks.log` | `ht-C-v090-q3-baseline-33c74793/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q3-baseline-33c74793/list.json` | `ht-C-v090-q3-baseline-33c74793/list.json` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-baseline-33c74793/list.stderr` | `ht-C-v090-q3-baseline-33c74793/list.stderr` | 1335 | `e738d7ac29730323722d29f0b692c25334b56045c891ced48ad12b22c453e72f` |
| `/private/tmp/ht-C-v090-q3-baseline-compile-corrected/execution.json` | `ht-C-v090-q3-baseline-compile-corrected/execution.json` | 9857 | `98ea9e6899b1911f1b09a735efa1bc7a9a04577205f54241b27df4914ff4dbea` |
| `/private/tmp/ht-C-v090-q3-baseline-compile-corrected/leaks.log` | `ht-C-v090-q3-baseline-compile-corrected/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-q3-baseline-compile-corrected/list.json` | `ht-C-v090-q3-baseline-compile-corrected/list.json` | 862015 | `c5729986a3412dd2b6e643ec311f4bf075fb2c7ed4e1af4371c4d51a89f86a63` |
| `/private/tmp/ht-C-v090-q3-baseline-compile-corrected/list.stderr` | `ht-C-v090-q3-baseline-compile-corrected/list.stderr` | 220 | `e3fd38199f2dc71277d0e4af916202bb89d744efd1f68a969c62abef2cda51ff` |
| `/private/tmp/ht-C-v090-q3-baseline-compile-corrected/run.log` | `ht-C-v090-q3-baseline-compile-corrected/run.log` | 22335 | `6521404b081043cd2f3db4764d52fa9e98adbcdf664542cbf284cc98541a2919` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-controls-spec-review.md` | `resume-v090-q3-controls-spec-review.md` | 12819 | `c974731aedfa4a027998e3e244141cc6d3ca33ee9433959d7c5da710ff787efe` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-controls-quality-review.md` | `resume-v090-q3-controls-quality-review.md` | 12179 | `53b33a004fc36594fe62fe675bf7626623b8cbbc5ee62f3fd3a719ab9a0d6187` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-v090-q3-production-interface-plan.md` | `resume-v090-q3-production-interface-plan.md` | 26303 | `d7e137b4d385d2be239126b57a5ed42f54f3488f487d2b825e84bb4b9f85dfe4` |

Only new documentation/evidence files were written. No source/index/Git mutation, Cargo/runtime, Herdr operation, subagent or process launch occurred. Production authors’ files were untouched. QUIESCENT.
