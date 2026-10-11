> VALIDATION CORRECTION: Earlier shared-target test/lint evidence below is superseded and must not be relied upon. The coordinator found shared artifacts could originate from the behavior-only branch. Use the private-target revalidation section appended below as authoritative current evidence. Earlier RED output remains historical context, not independent current validation.

# Task ht-akx.2 report

Implemented the decided case-sensitive literal thread-name OR topic discovery contract in CLI `--search` help (including list summary), README discovery example (`--search psa-global`), adopted amendment section 4, and `DirectoryQuery.topic_contains` field documentation. The legacy serde spelling and all APIs remain unchanged. README describes selected-instance and actual membership scopes, bounded pages/empty progress and explicit continuation, and distinguishes picker fuzzy filtering. Generic top-level topic/body search and all bounds are preserved.

Additional maintained-document contradictions were declared to the controller before editing and approved: `docs/design/herdr-threads/2026-09-27-herdr-threads--cli-design.md`, `...--store-design.md`, and `...-design.md`. Corrections are limited to current directory discovery terminology.

## Verification

All commands used `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/target` and `HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`.

- `cargo fmt`: exit 0.
- `nice cargo test --locked --all-features --lib directory_search_generated_continuation_preserves_literal_scope_order_and_output`: 1 passed, 0 failed; 0.14s test execution. Exercises real isolated SQLite store query and parses returned `next_argv`; verifies exact literal UTF-8/quotes/punctuation search, all-membership scope, recent ordering, output context/JSON, cursor and bounds.
- `nice cargo test --locked --all-features --lib thread_list_help_explains_literal_name_or_topic_search`: GREEN 1 passed, 0 failed; 0.01s.
- `nice cargo test --locked --all-features --lib continuation_argv_round_trips_context_filter_format_and_bounds`: 1 passed, 0 failed; 0.00s. Existing parser coverage extended with recent ordering and literal punctuation.
- `nice cargo test --locked --all-features --lib picker_directory_wire_keeps_old_directory_shape_and_cursor_only_contract`: 1 passed, 0 failed; 0.00s. Existing compatibility test extended with nonempty legacy `topic_contains` serialization/deserialization.
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: exit 0, finished in 37.63s.
- `nice scripts/check-default-features`: exit 0, silent output.
- `git diff --check`: exit 0.

The harness prints `nice: setpriority: Operation not permitted`; underlying checks execute successfully. Initial path-specific RED compilation reported 1m26s without a lock line. Generated-continuation test invocation reported 3m00s including explicit artifact-lock waiting; active compile and lock durations cannot be separated precisely from available output. Subsequent contiguous validation (coordinated with controller) rebuilt first test in 40.42s, then reused it in 0.04s and 0.03s. No established incremental speed regression; no performance scope expansion.

## TDD evidence

Before editing CLI help, added `thread_list_help_explains_literal_name_or_topic_search` and ran its focused command. RED: 1 failed, 3577 filtered out; assertion for `case-sensitive literal substring` failed. Actual help showed blank `--search` description and `List threads, optionally filtered by membership or topic text`. Added minimal help text naming both fields and literal case sensitivity. GREEN: same focused command passed (1 passed, 3578 filtered out). Serialization/continuation additions characterize preserved behavior, rather than introducing runtime changes. Human documentation earns no separate behavior tests.

## Files changed

- `src/cli/commands.rs`
- `src/protocol/commands.rs`
- `README.md`
- `docs/design/herdr-threads/shared-contract-amendment-adopted.md`
- `docs/design/herdr-threads/2026-09-27-herdr-threads--cli-design.md`
- `docs/design/herdr-threads/2026-09-27-herdr-threads--store-design.md`
- `docs/design/herdr-threads/2026-09-27-herdr-threads-design.md`
- `tests/cli/commands.rs`
- `tests/protocol/capabilities.rs`

## Self-review and concerns

Reviewed full diff and specification acceptance. No store behavior, picker, exact name resolution, protocol version or serde field changes. Real continuation fixture uses topic matches so it is independently testable before Task 1 behavior integration. Default membership README wording handles selected/inferred seat and seatless scope correctly. No current explicit topic-only discovery assertions remain in maintained documentation; historical topic-search references describing generic topic/body search remain appropriate.

No concerns. No daemons, helper child processes, private Herdr servers, real configuration writes, Herdr-thread mutations, push or stash were used. Focused in-process checks only; no full suite run or leaked owned processes.

## Private-target revalidation (authoritative; 2026-10-10)

Source HEAD: `7d4580362c28a3e4654f99ae7088e4ce97a62a52` (rebased docs branch). No product/source changes were needed.

Created a private APFS copy with `cp -cR /Users/alepar/AleCode/herdr-threads/target ./target`, then removed all copied project artifacts using `cargo clean -p herdr-threads --target-dir "$PWD/target"` with `CARGO_TARGET_DIR="$PWD/target"`. Clean reported `Removed 47505 files, 51.3GiB total`. Only the private target was cleaned; shared target was never cleaned, and source timestamps were never forced. Fresh cargo compilation printed this docs worktree's full path. Test executable was this worktree's `target/debug/deps/herdr_threads-a1eddcbd246c5614`.

The following sequential commands ran with `set -e`, `CARGO_TARGET_DIR="$PWD/target"`, `HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`, from the task2 worktree:

- `nice cargo test --locked --all-features --lib thread_list_help_explains_literal_name_or_topic_search`: PASS 1/1, 3578 filtered; 0.01s execution. Fresh package build: 1m32s.
- `nice cargo test --locked --all-features --lib continuation_argv_round_trips_context_filter_format_and_bounds`: PASS 1/1, 3578 filtered; 0.00s execution; cargo 0.04s.
- `nice cargo test --locked --all-features --lib picker_directory_wire_keeps_old_directory_shape_and_cursor_only_contract`: PASS 1/1, 3578 filtered; 0.00s execution; cargo 0.03s.
- `nice cargo test --locked --all-features --lib directory_search_generated_continuation_preserves_literal_scope_order_and_output`: PASS 1/1, 3578 filtered; 0.13s execution; cargo 0.03s.
- Config gate, exact except approved private target substitution: `nice cargo test --locked --all-features --lib directory_`: PASS 38/38, 3541 filtered; 0.96s execution; cargo 0.03s.
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: PASS exit 0, 39.59s.
- `nice scripts/check-default-features`: PASS exit 0, silent output.

Source-matching inventory: freshly compiled private library has 3579 total tests. The 38-test directory_ gate explicitly ran `protocol::capabilities::tests::directory_search_generated_continuation_preserves_literal_scope_order_and_output`, confirming docs additions are present. It ran existing `directory_stale_restart_keeps_selected_context_and_filters`, `directory_cursor_stales_only_on_relevant_member_revision`, and `directory_topic_filter_stales_after_topic_change`; it did not list Task 1's new name matching or name-mutation tests. This independently validates the docs source rather than the behavior-only branch. All focused checks and the config gate pass against this isolated source-matching target.

Harness-only `nice: setpriority: Operation not permitted` remains; underlying commands execute and succeed. Required checks complete, no product changes or additional commit, and no concerns.

Additional read-only process verification: `scripts/check-no-leaked-processes --run-id c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8` initially could not spawn `ps` in sandbox; approved CLI-only escalation rerun exited 0 with `no leaked test processes`. Working tree remains clean at HEAD `7d4580362c28a3e4654f99ae7088e4ce97a62a52`.
