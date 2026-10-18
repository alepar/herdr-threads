# CI release lead-time investigation (ht-z70k)

Measured from GitHub job/step metadata on 2026-10-18; both runs built source `9866b834988acda3ad2ca2079e8b6f33929eec73`. Timings exclude queue time.

| Run | Shard | Test compile (s) | Release proof prebuild (s) | Nextest (s) | Job (s) |
| --- | --- | ---: | ---: | ---: | ---: |
| [manual](https://github.com/alepar/herdr-threads/actions/runs/39153538359) | 1/4 | 470 | 448 | 235 | 1199 |
| [manual](https://github.com/alepar/herdr-threads/actions/runs/39153538359) | 2/4 | 515 | 504 | 183 | 1242 |
| [manual](https://github.com/alepar/herdr-threads/actions/runs/39153538359) | 3/4 | 589 | 580 | 246 | 1460 |
| [manual](https://github.com/alepar/herdr-threads/actions/runs/39153538359) | 4/4 | 595 | 578 | 241 | 1449 |
| [push](https://github.com/alepar/herdr-threads/actions/runs/39153539258) | 1/4 | 588 | 572 | 233 | 1435 |
| [push](https://github.com/alepar/herdr-threads/actions/runs/39153539258) | 2/4 | 445 | 435 | 161 | 1077 |
| [push](https://github.com/alepar/herdr-threads/actions/runs/39153539258) | 3/4 | 586 | 577 | 245 | 1445 |
| [push](https://github.com/alepar/herdr-threads/actions/runs/39153539258) | 4/4 | 575 | 571 | 238 | 1424 |

Compilation accounts for 880–1,173 seconds per shard (14.7–19.6 minutes); nextest accounts for 161–246 seconds (2.7–4.1 minutes). The four-shard split already keeps actual tests below five minutes in these runs. Each shard repeats the same two release builds even though only one partition owns the release-absence test.

The completed shard-2 job log (`117751925806`, manual run) independently reports a cold test compilation taking `8m 35s`, then a no-op test build taking `0.04s` when nextest starts. The workflow at this SHA has no Cargo cache restore/save action, so Rust cache hit rate is **not applicable (no cache configured)**. The generic setup-job `Cache mode: write` log line is GitHub runner metadata, not a Rust cache hit. The repository cache inventory API also returned `total_count: 0` before this change.

The same shard log starts compiling the repository crate at `18:21:03.712Z` and completes the test build at `18:28:43.698Z`: about 460 seconds after roughly 91 seconds spent before that point. Dependency-only caches cannot eliminate the repository crate compilation and linking cost; no whole-build speedup is claimed.

## Artifact reuse decision

CI builds the same four target triples as release.yml but retains no executable or archive artifacts. Release compilation also replaces the embedded `docs/compatibility/harness-versions.json` from the moving `harness-manifest` branch before building; ordinary CI does not. Consequently source SHA alone does not establish identical build inputs. Downloading a same-SHA CI binary cannot safely replace the release build as currently implemented. Release packaging and SHA256SUMS/VERSION/TARGET checks remain unchanged. Reliable reuse would first require a shared archive-producing workflow with frozen manifest inputs and explicit artifact provenance; that is a separate change.

## Measurement boundary

The task agent did not push. The coordinator pushed the reviewed initial implementation to `29733a939`; the completed first cold configured run is measured below, including 0/4 test-shard cache hits. Warm-hit speedup remains unmeasured and needs a subsequent comparable run that restores populated caches. Local warm builds and first-run cache population do not establish hosted cache-hit benefit.

## Implemented cache boundaries

CI now restores the SHA-pinned Swatinem/rust-cache v2.8.2 after selecting Rust, for the four test shards, clippy, all four target builds and the single release-proof job. Keys separate operating system, runner architecture and build target; the action additionally keys actual rustc identity, lockfiles/manifests and Cargo/Rust environment, with dependency-compatible fallback restoration. Test shards share one dependency cache and only shard 1 saves it. Proof caching explicitly includes `target/failpoints-release` and `target/failpoints-control`, alongside the test target directory. Installed tools, workspace crates and incremental state are not cached; this bounds payloads instead of archiving a whole developer target tree (the warm local tree exceeds 100 GiB).

These caches reduce repeated dependency builds; source crate compilation still runs. In the sampled shard-2 log roughly 91 seconds preceded the source crate's approximately 460-second compile, so a dependency hit cannot be described as eliminating the entire 515-second build. Removing identical proof prebuilds from all four shards is a separate change: a dedicated job runs the original exact Rust proof test once, preserving hook-count and output assertions. Cold cache-hit rates are measured below (0/4 shards); only warm-hit benefit remains pending a subsequent restoring run.

## First cache-enabled hosted run (cold)

[Run 39158112991](https://github.com/alepar/herdr-threads/actions/runs/39158112991), on the coordinator's pushed `29733a939`, completed successfully across all ordinary CI jobs (optional manual jobs skipped) and populated the new caches. The completed Linux clippy log explicitly says `No cache found.` and saved 58,913,688 bytes. These measurements are cold-run observations; runtime variance is not evidence of a cache-caused speedup. A later successful restore and completed comparable jobs are still needed for warm-cache measurements.

| Clippy runner | Manual baseline default/all (s) | First configured run default/all (s) |
| --- | ---: | ---: |
| Linux x86_64 | 171 / 114 | 138 / 90 |
| macOS arm64 | 206 / 224 | 163 / 182 |

The task agent retained metadata at `/tmp/ht-z70k-ci-after-current.json` and the completed Linux job log at `/tmp/ht-z70k-ci-after-clippy-linux.log` (job `117765502528`). The completed shard/proof observations below supersede the earlier in-progress snapshot; no warm-hit result is claimed here.

All four shard jobs and the dedicated proof job subsequently completed successfully:

| Shard | Compile (s) | Nextest (s) | Job (s) | Manual baseline job (s) |
| --- | ---: | ---: | ---: | ---: |
| 1/4 | 624 | 234 | 902 | 1199 |
| 2/4 | 633 | 259 | 932 | 1242 |
| 3/4 | 620 | 232 | 892 | 1460 |
| 4/4 | 620 | 238 | 896 | 1449 |

The dedicated exact Rust proof test took 803 seconds (job 822 seconds), outside all shard timers. Shard jobs were approximately 25–39% shorter than the manual baseline; this is a shard-job comparison, not total workflow lead time or proof of cache benefit. Cold compilation worsened to 620–633 seconds versus 470–595 seconds in that baseline, and nextest durations were mixed. Every shard log says `No cache found.` with no restored cache (0/4 shard hits). Shard 1 saved 83,181,822 bytes and the proof job saved 191,855,436 bytes for future runs. A second run with successful restores is still required to measure warm-cache behavior.

The retained local trial evidence includes the metadata and five completed shard/proof logs under `/tmp/ht-z70k-qualification-approved/ci/` (job IDs `117765502614`, `117765502556`, `117765502467`, `117765502554`, `117765502560`).

## Local credential-free integration trial

Command: `scripts/release-qualify --self-test --evidence-dir /tmp/ht-z70k-qualification-approved`, run with approval for private sockets/processes. Overall exit **1**: the suite exceeded the unchanged 300-second budget. Every subprocess exited 0; no functional test failed. Nextest reported **7,689 passed, 48 skipped, 370.352 seconds**; driver wall time for that step was **371.45 seconds**. A foreign worktree's compilation was observed on the shared host before/during qualification, but no causal attribution or adjusted budget is claimed. The full suite ran exactly once.

The Python process loaded a pre-review driver snapshot before the later failure-path fixes and help clarification. Rust/product sources are identical across the entire task series and the merged main; the matrix and lifecycle subsequently recorded source `7634df5fd`. The final driver failure paths (candidate failure blocking live wake/binary installers, detector failure/missing output, and cleanup-error summary preservation) were verified separately by focused regression tests. This trial is not represented as execution of the final driver tip verbatim. `--self-test` omitted authenticated wake only: matrix, lifecycle and installers were real. **No live wake qualification is claimed.**

| Driver step | Result | Wall seconds | Subprocess exit |
| --- | --- | ---: | ---: |
| fmt | PASS | 10.46 | 0 |
| neutral | PASS | 1.85 | 0 |
| clippy-all | PASS | 1.99 | 0 |
| clippy-default | PASS | 65.68 | 0 |
| default-features | PASS | 49.45 | 0 |
| prebuild-tests | PASS | 196.46 | 0 |
| prebuild-proof | PASS | 196.48 | 0 |
| suite | FAIL | 371.45 | 0 |
| suite-leaks | PASS | 0.67 | 0 |
| candidate | PASS | 138.58 | 0 |
| upstream | PASS | 1.13 | 0 |
| wake-self-test | PASS | 83.46 | 0 |
| wake-leaks | PASS | 0.45 | 0 |
| matrix | PASS | 670.48 | 0 |
| matrix-leaks | PASS | 0.29 | 0 |
| package | PASS | 693.4 | 0 |
| installer-alias | PASS | 2.22 | 0 |
| installer-integrations | PASS | 3.54 | 0 |
| installer-terminal | PASS | 0.85 | 0 |
| installer-shell | PASS | 67.34 | 0 |
| package-leaks | PASS | 0.44 | 0 |

The wake self-test passed 153 tests (83.274 seconds internally); all five Herdr releases 0.9.1–0.9.5 passed all five matrix categories. The package lifecycle printed `PACKAGE_VALIDATION_PASS` (690.19 seconds internally). Exact CI Python commands passed: alias 5 tests, integrations 14 tests, and terminal `INSTALLER_TERMINAL_PASS`. The shell harness printed `INSTALL_TEST_PASS 296 checks (stub herdr + isolated real herdr matrix)`.

All run-scoped process/temp-file leak checks passed:

| Branch | Run ID | Outcome |
| --- | --- | --- |
| suite | `437966bd-e9a8-4b0b-a6e0-7af6a6734747` | clean |
| matrix | `21d7ca28-2c90-4207-b8f2-174394f264e8` | clean |
| wake | `5643c280-5a7a-4a02-8732-c0136fd44210` | clean |
| package | `94cc7b6a-d152-4c53-8f66-dc047d12b6d4` | clean |

Evidence: `/tmp/ht-z70k-qualification-approved/summary.json`, step logs, `matrix.md`, per-release logs, branch environment records, `ci/` hosted metadata/logs and `reviews/` retained review reports. All five recorded `/tmp/rq.*` scratch roots are absent; the package fixture `/private/tmp/htpv-zd210m8v` was removed. The earlier sandbox attempt was stopped before the suite, its owned children were confirmed gone and its exact scratch root removed; its logs remain under `/tmp/ht-z70k-qualification`.
