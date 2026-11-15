# Corrected unfiltered attempt 2

**Functional PASS; runtime budget FAIL.** On code source `fb7bf028e750a12744397a39b37534d9880555f2`, the single authorized corrected attempt returned exit 0: **9,645 passed, zero failed, 66 skipped**, one slow, across 19 binaries. Compilation took 40.90 s, nextest execution **993.332 s**, and complete command wall time **1,034.38 s**. The 300-second budget remains open as `ht-n4xh`; no third attempt or concurrency increase was performed. This observation does not establish the cause of the timing difference from attempt 1 or the historical baseline.

The warm worktree used `heavy tb`, explicit build/test caps of four, and the unchanged default nextest profile and test groups:

```sh
cargo nextest run --locked --all-targets --all-features
```

Before the suite, format and default-feature checks passed (exit 0, 10.57 s and 25.48 s respectively). No additional clippy run was needed for the unchanged Rust source. The slow test was `manifest::failpoints_are_compiled_out_of_release_builds`, which passed in 172.891 s.

All ten attempt-1 failures now have exact PASS rows in this unfiltered run: registry health wording, launch/handoff registry help, two inherited-name cases, health JSON golden, three bare-setup cases, adapter end-to-end registry wiring, and every-adapter enrollment policy. [The accompanying record](full-suite-attempt-2.json) retains their complete names and PASS rows. Attempt 1 remains a recorded failure: these results do not rewrite its history. The two attempts selected different totals because later regressions were added.

Both same-run builder and host process/temp scans passed, using leak run ID `0f5d1568-cb27-4900-a692-00fff43d620e`. The initial host wrapper failed before starting the checker because `/usr/bin/time` was absent; the direct same-ID checker then passed. The builder consumed the clean scratch ledger before the host check. Nextest's separate internal ID was `6eaaf332-ffe0-41bf-add0-c07eccbf55a8`.

During the gates, HEAD advanced to `d55f7e984c55dd06b805cc7728b9f2ca9985209a` through documentation-only commits. The source delta contains only `docs/` and `integrations/pi/README.md`; the final worktree was clean. Exact paths, wrapper/config hashes, gate results and log hashes are retained in the accompanying JSON and the private `final-gates/full-suite-attempt-2/` evidence directory. Complete suite-log SHA256: `58c5cab1d72b3271db95037d03863f7d894889819fe88a53f40893bae4e8dac8`.

## Native candidate identity after nextest

Nextest placed the normal executable built under the **test profile** at `target/debug/herdr-threads`; its SHA256 is `391499fded00e16de95ed1d4d76708d268c686b05cd5c286b48d4a0b7380b126`. The exact qualified dev-profile executable remained present at `target/debug/deps/herdr_threads-75705d562a6fb0ec`, SHA256 `a8aef2df56e6954fe3c12b7d02841f521712ceb403d13f9b93fabdabe2224289`.

Cargo fingerprints show both executables have `test-support`, the same compiler/configuration, and no extra Rust flags. Their profiles differ: `Cargo.toml` sets test opt-level 1, codegen-units 256 and LTO off, versus dev opt-level 0 and codegen-units 64. The changed common-path hash was not a default-feature binary or a production source change. Between the qualified build anchor `a19fd881e` and the suite source, no Rust, Cargo/configuration or production Pi extension source changed; the Pi SDK test did change.

Root authorized one warm `cargo build --locked --all-features --bin herdr-threads` to restore the dev output path. It completed before the subsequent cancellation of a pending rebuild arrived: exit 0, no compilation, Cargo 0.05 s / wall 0.07 s. The common path again hashes to the exact original `a8aef2df…` candidate. No manual file replacement or test rerun occurred. Native qualification continues to identify the dev candidate by its exact hash; this full-suite result identifies the test-profile artifacts separately.

Skipped/ignored cases are not qualification claims. This gate does not replace the separately recorded native, live, JavaScript/SDK, historical-producer or speed-budget requirements.
