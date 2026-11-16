# Frozen pre-integration-correction suite

Exact frozen source15fd697ba980420ac4ef812b3de072b6af045409, HEAD/actual main4dab merge tree5ab1a672cd78dbaf41e2dd1854be8156d5d06f52. This run began after clean formal round3 and became historical when main independent integration review found further defects. It does not clear those defects or qualify the later correction tree.

Command: `heavy tb cargo nextest run --locked --all-targets --all-features --profile ci -j 4`, nice19, CARGO_BUILD_JOBS/NEXTEST_TEST_THREADS/RUST_TEST_THREADS=4, unchanged nextest groups, existing owned target. Fresh HT_LEAK_RUN_ID be8fd356-e625-48a5-aa37-d9386358c2fb.

Exit100:9,750 tests run,9,749 passed,1 failed,64 skipped. Execution930.812s; build0.03s; command wall932.366s. The failure is the static no_leaks::every_test_spawn_site_is_owned scanner: tests/harness/codex_wake_reap.rs line41 uses command.spawn after constructing the command through test_support::spawn::command, but lacks the required local leak-guard annotation. The test-owned exact Child pool and failure Cleanup hold ownership; the checker is not weakened. The later correction will add the explicit tag/owner explanation and verify the scanner.

Scoped process and temporary-file checks both passed after completion. The result remains FAIL; no timing-budget pass or cache speed-causation claim is made. Main independent review imposes a new correctness hold and requires final fixed-tree unfiltered qualification after corrections/review. No additional live run occurred.
