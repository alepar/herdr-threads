# Rebased v3.2.1 integration gates

**Functional PASS.** The additional authorized merged-tree gate ran once on clean code source `bad263b15dbbca518ea0509d16f90f9d01c1c602`, based on `e60ec5eb5e9746f82b399127838dc9515c7cc712` (v3.2.1). It is separate from the pre-rebase corrected attempt 2. The [rebase audit](integration-rebase.md) records source equivalence; this gate did not rerun native or live qualification.

All prerequisite checks returned exit 0 before the suite started:

| Check | Wall time |
| --- | ---: |
| `cargo fmt --all -- --check` | 6.09 s |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | 62.45 s |
| `scripts/check-harness-neutral` | 1.23 s; zero violations |
| `scripts/check-wire-compat` | 0.09 s; zero violations/stale entries |
| `scripts/check-default-features` | 53.82 s |

The single unfiltered `cargo nextest run --locked --all-targets --all-features` returned **exit 0: 9,645 passed, zero failed, 66 skipped**, one slow, across 19 binaries. The existing warm target was reused through `heavy tb`, with `CARGO_BUILD_JOBS=4`, `NEXTEST_TEST_THREADS=4`, `RUST_TEST_THREADS=4`, the unchanged default nextest profile/groups, and no inherited overrides.

The version rebuild took **7m 12s**, as rounded by Cargo. Bounded worktree-scoped observations confirmed four active compiler workers during that phase. Test execution then took **1,033.189 s**; total command wall time was **1,465.37 s**, user 5,161.84 s / system 304.51 s. These are separate rebuild and execution observations, not a one-file incremental-build benchmark. The slow release failpoint test passed in 214.970 s. All ten historical attempt-1 failure names also have exact passing rows in this integration run, retained in [the machine-readable evidence](integration-gates.json).

The suite exceeded its 300-second budget and clippy exceeded 60 seconds. Both misses remain explicit under `ht-n4xh`. Main explicitly accepted the suite overrun as nonblocking under the cap-4 shared-training constraint; the clippy overrun is reported separately. No timing pass, causal regression attribution, raised cap or additional full run is claimed.

Both mandatory same-run builder and host process/temp scans returned exit 0 and found no leaks. The builder consumed the clean scratch ledger before the host's no-ledger result. Leak run ID: `23661afb-9299-4ce3-b479-62e522e48a04`. Nextest internal run ID: `64092aa3-32f1-44c8-9bdc-09bdaff61228`.

HEAD advanced only through the rebase evidence publication to `9820c9ca6752ef72a3a56d06b66bf55f8bbdd4af`; the worktree was clean after the gates. The three changed paths are qualification documentation, listed in the accompanying JSON. No production, test, Cargo or configuration delta occurred during the gate.

## Binary profile and evidence limits

The resulting `target/debug/herdr-threads` reports **herdr-threads 3.2.1** and hashes to `3e3508042f751e1c6710e18da4ac6c1270ffcfedf2428df0b001b5f28b0ef4d9`. It is the same inode as `target/debug/deps/herdr_threads-89936ad1028d57ca`: the ordinary executable built by nextest under the **test profile**, with `test-support`, no extra Rust flags, and profile fingerprint `3057891792897344596`. Cargo configures test opt-level 1, codegen-units 256 and LTO off; the dev profile uses opt-level 0 and codegen-units 64.

The historical qualified v3.2.0 dev artifact remains intact at `target/debug/deps/herdr_threads-75705d562a6fb0ec`, SHA256 `a8aef2df56e6954fe3c12b7d02841f521712ceb403d13f9b93fabdabe2224289`. It is not a v3.2.1 artifact. No new dev-profile candidate was built or installed in this gate, and no native/live runtime claim is transferred by binary filename. Skipped/ignored cases remain outside this result.

Complete logs and metadata are retained in `.superpowers/sdd/implementation-plan/final-gates/integration-gates/`. The public JSON contains commands, source/tree and wrapper/config hashes, environment facts, timings, cleanup, profile mapping and individual log hashes. No source edits, extra full run, native/live rerun or real-service mutation was performed.
