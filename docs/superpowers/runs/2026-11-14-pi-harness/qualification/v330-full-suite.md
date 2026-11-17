# v3.3.0 final local full-suite attempt

**FAILED: 9,853 passed, 2 failed, 67 skipped** across 19 binaries; all 9,855 selected tests completed. This was one unfiltered attempt with no retry. Both scoped process/temp scans passed.

Rust source: `9f69b8d02b1244a7e21be8df9d368b380f887764`. Start HEAD: `ef8b36efee85cfec95baa9e36479aff39bab33c6`; end HEAD: `59686a8f9e9831dc36f9092d4f017ca9e1147451`. Only native qualification documentation was committed during the run. The later test-only corrections are not included in this result.

Command: `cargo nextest run --locked --all-targets --all-features`, through the shared `heavy`/`tb` wrappers with `CARGO_BUILD_JOBS=4`, `NEXTEST_TEST_THREADS=4`, and `RUST_TEST_THREADS=4`, reusing the warm worktree target. Leak run ID: `66baa96c-e392-4ba5-89bc-07292da03187`. Nextest run ID: `a187b8fe-d312-4745-b680-1eb81604baed`. Exit: **100**.

| Measurement | Result |
| --- | --- |
| Cargo test-profile compilation | 7m 18s (Cargo-reported) |
| Test execution | 927.245s |
| Total command wall time | 1,366.38s |
| Slow release failpoint-build check | PASS, 327.023s |
| Full-suite budget | 300s; exceeded |

Timing remains an explicitly accepted nonblocking cap4 training-host budget miss (`ht-n4xh`). These measurements do not isolate a code-caused performance regression.

The failures were:

- `service::loop_inventory::every_timed_wait_is_on_a_pacer_or_allowlisted_with_a_reason`: two one-shot, two-second `cfg(test)` rendezvous in the native check/type-gap characterization lacked exact justified inventory entries.
- `golden_surfaces::help_texts`: the handoff help fixture retained three old lines where the current source uses four. The corrected text has identical normalized words.

The reviewed correction touches only the inventory allowlist and that golden fixture. Main explicitly accepted (`mnSGlopMCqRw`) focused inventory/characterization and golden-surface validation, plus required guards, instead of a second local full run, provided these remained the only failures and no production fix was needed. CI's sharded full suite remains the merge full gate. This report does **not** claim a passing full suite; the focused follow-up below is separate evidence.

The native-qualified dev binary remains `target/debug/deps/herdr_threads-ee429b642d86e164`, SHA256 `a63ac7a7629f29c117d1968523d65f8253aaaa1a1bfbe5e66b29e0b81ac06ea4`. Nextest replaced `target/debug/herdr-threads` with the test-profile artifact `target/debug/deps/herdr_threads-2042840da67c16bb`, SHA256 `0d084c5f9356b81e382e28c5422e76b5fb02817dd913da0c55e2c253679e0810`. Both report 3.3.0; the differing build profiles are recorded in the [machine-readable evidence](v330-full-suite-evidence.json).

Both host and builder ran `scripts/check-no-leaked-processes --run-id 66baa96c-e392-4ba5-89bc-07292da03187` after the suite, exiting 0 with no leaked processes or temporary files. After those scans, completed secondary directories `target/failpoints-control` (3.4G) and `target/failpoints-release` (1.8G) were removed; main `target/debug` and evidence were preserved. Filesystem availability increased from 382G to 385G; `/tmp` remained 14G/32G (42%). Reported directory sizes are not a claim of uniquely reclaimable physical blocks.

Raw output remains in private `final-gates/v330-final-gates/full-suite.log`; SHA256 `43139b81d47d80de8d8d9f6b843eda89b070381f0e69b5ee5d10e5dd321343d4`. The JSON records both scan-log hashes and exact counts, source identities, profiles, and disposition.

## Focused correction validation

Commit `55401e27c56a9e3f990206c550ec6f42ea61e510` changes only the two reviewed test/fixture files; production source is unchanged. The exact inventory snippets and word-identical golden rewrap passed independent review.

One focused command selected all three loop-inventory tests, the native check/type-gap characterization, and all 15 golden-surface tests across the library and combined binaries: **19/19 passed**, with 9,110 tests unselected/skipped in those binaries. Cargo compilation took 33.45s, tests 5.958s, total 39.54s. The complete filter and both run IDs are in the JSON.

Required gates passed: fmt 6.56s, clippy 20.22s, default features 13.06s, harness neutrality 1.31s, and wire compatibility 0.09s. Both same-run host and builder cleanup scans exited 0. The frozen patch, file hashes, and all eight validation log hashes were independently verified. No second local full suite was run; the original failed attempt above remains unchanged, and CI's sharded full merge gate is still outstanding.
