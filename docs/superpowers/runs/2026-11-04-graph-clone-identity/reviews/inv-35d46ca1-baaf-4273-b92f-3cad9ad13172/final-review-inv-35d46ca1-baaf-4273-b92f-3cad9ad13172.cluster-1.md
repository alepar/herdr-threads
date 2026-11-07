# Final review, cluster 1 (config), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout read clean at that head at the start and end of this review. The source binding exited 0.

The cluster is one file, `.config/nextest.toml` (slice 000001, +7 lines): the test group `clone-identity-sweep` (`max-threads = 2`) and one override, `binary(clone_identity_sweep)` at priority 60. The slice matches `git diff base..head -- .config/nextest.toml` exactly.

## Method

Reading, plus one independent evaluation of the head config. No real test binary was built or run for this review.

The evaluation used a scratch crate carrying head's `[[test]]` and `[[bin]]` target names, head's `.config/nextest.toml`, and stand-in shell scripts that answer `--list`, driven through `cargo nextest show-config test-groups` and `cargo nextest list` (0.9.146, the version CI pins). The stand-ins listed the real test names parsed from head's `tests/clone_identity_sweep.rs`, `tests/service/continuity_key_session.rs` and `tests/service/reversible_session.rs`. The scratch files have been removed.

## Trace of the change

- The config parses, and every override filter compiles. Nextest refuses a `binary()` that matches no target, so `binary(clone_identity_sweep)` agrees with the `[[test]]` name head's `Cargo.toml` adds.
- All 10 sweep tests land in `clone-identity-sweep`, and no earlier override claims them. The earlier unscoped name filters require `::` in the test name, and the sweep's tests are top-level.
- The override sets only group and priority, as its neighbours `config_smoke`, `named_seats` and `integration_sweep` do. No `threads-required` or timeout changes.
- The group comment is true. `Matrix::new` builds an `IsolatedHerdr` per test, and the helper's own header says each test owns a private root and the tests can run in parallel, so two at a time shares no state.
- The default guard (60 s x 5) fits. The sweep's wait bound is 60 s per wait, and task-11 recorded 44 s, 50 s and 48.9 s for the whole binary at two at a time.
- The sweep runs in CI. `ci.yml` provisions Herdr and `--all-targets --all-features` satisfies `required-features`, so the end-to-end proof is not a skipped or manual step there.
- `#[path]` pulls the six `#[ignore]` matrix tests into the sweep binary too. `scripts/herdr-release-smoke` selects `--test integration` and `scripts/flake-hunt` drops ignored names, so neither runs them twice.
- `AGENTS.md`'s rule for a suite running a daemon (own `[[test]]` entry, plus a nextest group for a test that must not overlap its kind) is met for the sweep.

The same evaluation confirmed one gap beside the change: the 12 new tests in `continuity_key_session::` and `reversible_session::` take `IN_PROCESS_DAEMON` and land in no group. It is listed at the end with its reasoning.

## Requirement baseline trace

The spec's `## Requirement baseline` has rows B1 to B4 and no human scope change. This cluster schedules tests and delivers no product behaviour on its own.

- B1, B2, B3: the sweep this group schedules is their end-to-end proof, and the config keeps it in the default run. Tracing the production paths leaves this cluster and is the seam reviewer's.
- B4 (Herdr unchanged): nothing here touches Herdr.

No row is shown undelivered by this cluster.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The group has not been observed in a full-suite run: task-11's evidence is `--test clone_identity_sweep` alone, and `progress.md` records the full-suite measurement as left to the caller, so its effect on the latency-asserting chains beside it and on the 300 s budget (full suite and CI shards) is unmeasured at this head.
- This review evaluated group assignment with stand-in binaries carrying head's target and test names; it did not build the real binaries or run the sweep, and relies on task-11's recorded runs for the sweep's pass and leak-check results.

## Deferred OK
- `.config/nextest.toml:203` limits `service-daemon` to `resolution|composition|operator|graph_d2`, so the 12 new `continuity_key_session::` and `reversible_session::` tests, which take `IN_PROCESS_DAEMON`, run ungrouped, against the `AGENTS.md` note that a mutex does not span processes; nothing shows they need to be alone (each has its own root, an earlier review recorded ten of them at 2.19 s together, waits are 5 s and 10 s, no latency assertion), and `graph_send` already runs this way, so add both names to the filter if either flakes under load.
- `.config/nextest.toml:19-20` puts the new group between `integration-daemon` and `service-daemon`, which share the comment at line 14, so the new comment now reads as if it also described `service-daemon`; moving the two lines below `service-daemon` fixes it, and it has no runtime effect.
- `clone-identity-sweep` allows two at a time where the other single-Herdr suites (`session-bindings-smoke`, `named-seats`, `config-smoke`, `hermes-smoke`) allow one, the latter to avoid starving the latency-asserting chains; `sync-daemons` is a precedent for two and the sweep's own waits are generous, so lower it to 1 (about 90 s of serial time) only if the first full-suite run shows starvation.
- `shift_of_1000_pending_rows_finishes_inside_the_call_budget` (`tests/store/reversible_reads.rs:617`) asserts a real 3 s budget with no test group, as task 10 already recorded; add a group if it flakes under starvation.
