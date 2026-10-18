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

This branch is not pushed by the task agent. Hosted after-change compile timings and cache-hit rates require the coordinator to push the reviewed commits, then a subsequent run to restore a populated cache. Do not treat local warm builds or the first cold cache population as hosted cache-hit evidence. Compare the same steps above and the cache-hit summaries emitted by the updated workflow.

## Implemented cache boundaries

CI now restores the SHA-pinned Swatinem/rust-cache v2.8.2 after selecting Rust, for the four test shards, clippy, all four target builds and the single release-proof job. Keys separate operating system, runner architecture and build target; the action additionally keys actual rustc identity, lockfiles/manifests and Cargo/Rust environment, with dependency-compatible fallback restoration. Test shards share one dependency cache and only shard 1 saves it. Proof caching explicitly includes `target/failpoints-release` and `target/failpoints-control`, alongside the test target directory. Installed tools, workspace crates and incremental state are not cached; this bounds payloads instead of archiving a whole developer target tree (the warm local tree exceeds 100 GiB).

These caches reduce repeated dependency builds; source crate compilation still runs. In the sampled shard-2 log roughly 91 seconds preceded the source crate's approximately 460-second compile, so a dependency hit cannot be described as eliminating the entire 515-second build. Removing identical proof prebuilds from all four shards is a separate change: a dedicated job runs the original exact Rust proof test once, preserving hook-count and output assertions. Hosted speedup and cache-hit rates remain pending the two-run measurement above.
