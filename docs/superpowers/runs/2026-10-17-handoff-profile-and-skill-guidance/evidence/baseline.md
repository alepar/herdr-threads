# Baseline

Source code: main b4d5f9628 (v2.0.3); only design documentation changed during measurement.

`HT_LEAK_RUN_ID=handoff-profile-baseline-99bf1225 nice cargo nextest run --locked --all-targets --all-features -E 'test(handoff)'`

494 tests passed, 7176 skipped, 27.440 seconds. See baseline-nextest.log.

`scripts/check-no-leaked-processes --run-id handoff-profile-baseline-99bf1225`: no leaked test temp files; no leaked test processes.

Initial sandboxed cargo test run: compile 2m36; 342 passed/5 failed. Three failures were socket EPERM; two lifecycle tests interfered when run in one process. Correct privileged nextest run above uses repository-defined test groups and passes all five. This initial run is an environment/runner mismatch, not evidence of a product regression.
