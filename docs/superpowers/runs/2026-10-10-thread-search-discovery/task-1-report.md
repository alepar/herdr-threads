# Verification provenance correction

All earlier shared-target results below are historical and **superseded as acceptance evidence**: the coordinator observed Cargo reuse of a sibling worktree binary, so shared-cache provenance was not reliable. Fresh verification below uses only task-private target after cleaning that package in the private clone. No source edits or new commit were made during revalidation. The prior rebase occurred while this completed agent was idle, before revalidation started; no accepted check overlaps a source mutation.

Stable source HEAD: `d61b47204a1770b91582f945b2c784e35c796f47`.
Explicit manifest: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/thread-search-discovery/.worktrees/super-auto-thread-search-discovery--task-ht-akx.1/Cargo.toml`.
Private target: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/thread-search-discovery/.worktrees/super-auto-thread-search-discovery--task-ht-akx.1/target`.
All new commands export `CARGO_TARGET_DIR=$PWD/target` and `HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8` in this workdir. APFS `cp -cR` copied the existing dependency cache; `cargo clean -p herdr-threads --target-dir "$PWD/target"` removed 47505 package files/51.3GiB ONLY from private target before the fresh rebuild. Root cache was never cleaned or mutated by this setup. Source timestamps were not touched.

## Fresh private evidence

- `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --lib directory_ -- --nocapture`: exit0, explicit `Compiling herdr-threads v0.6.0 (TASK_WORKDIR)` path, fresh build 1m59s, private `target/debug/deps/herdr_threads-a1eddcbd246c5614`; **42 passed,0 failed,3541 filtered out,in5.27s**. Inventory includes docs `protocol::capabilities::tests::directory_search_generated_continuation_preserves_literal_scope_order_and_output`, plus behavior `directory_filtered_name_revision_is_selected_and_rejects_legacy_keys`, `directory_name_or_topic_literal_matching_and_scopes`, `directory_name_hit_after_empty_work_page_has_no_skips_or_duplicates`, and control `directory_name_mutations_stale_filtered_cursors_but_replay_and_noop_do_not`. Full log `/tmp/ht-akx-1-private-directory.log`.
- `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --lib recent_picker_ -- --nocapture`: exit0, **24 passed,0 failed,3559 filtered out,in0.12s**; `/tmp/ht-akx-1-private-recent.log`.
- `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --lib directory_search_generated -- --nocapture`: exit0, **1 passed,0 failed,3582 filtered out,in0.09s**; `/tmp/ht-akx-1-private-generated.log`.
- Approved outside-sandbox `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --lib thread_names_ -- --nocapture`: exit0, private binary, build0.07s, **24 passed,0 failed,3559 filtered out,in0.50s**. All24 names listed in tool output; all three private-socket scoped_runtime checks passed.
- `cargo fmt --check`: exit0/no output.

Remaining private CLI, lint/default and process cleanup results are appended after completion.

---

# ht-akx.1 implementation report

## Changes

Pending final verification. Directory candidates project nullable canonical name in the existing ordinal and both recent indexed SELECTs. Literal case-sensitive substring matching admits a candidate once if either topic or optional name contains the needle. None and empty-string semantics remain unchanged. No ID matching, per-candidate name lookup, new scan, protocol shape, picker, message-search or authority changes.

## SQL name-write inventory

- `src/store/control.rs::set_thread_name`: sole production `UPDATE threads SET name` writer. Actual changes execute in the accountable canonical deciding transaction; exact replay returns the retained operation, unchanged values bypass the update/events/revisions.
- `src/store/control.rs::create_thread`: initial name is in an INSERT with a newly generated public thread ID. It cannot rename an existing row. Initial names remain directory high-water controlled.
- `src/store/service_controls.rs::ensure_thread`: reads existing thread instance/owner and returns `ThreadEnsured` without modifying fields when present; absent rows INSERT without name. No managed/service existing-thread name mutation exists.
- `migrations/0019_thread_names.sql`: adds nullable column/index, no existing name change. `0020_recent_activity.sql`/`0023_channel_archival.sql` observe name updates, never write names. Other thread INSERTs under store modules are test fixtures without names. No additional production writer requires file-scope expansion.

## Test scope

`tests/cli/read_cost_names.rs` is an in-process fake history/name renderer unit harness, not an isolated real CLI fixture. Controller approved the additional existing `tests/handoff_topology_cli.rs` fixture, which owns an isolated fake host, daemon, configs and child CLI processes with teardown. Added the actual `thread list --search psa-global` regression there without new infrastructure.

## TDD evidence

Initial RED command (with CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/target and HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8):

`nice cargo test --locked --all-features --lib directory_name -- --nocapture`

Finished test profile in 2m 34s (first worktree build). Two expected behavioral failures, 0 passed: name-only hit absent (`left: ["both"]`, `right: ["named", "both"]`); hit after empty work page absent (`left: []`, `right: ["candidate-150"]`). Predicate implementation followed this RED.

A grouped `--lib directory_` RED attempt exposed a private `PermitMutation` import; corrected to `protocol::commands::PermitMutation`. This compile error is not claimed as behavioral RED. The next lock-wait attempt was interrupted to give the other leaf contiguous target validation, avoiding repeated manifest rebuilds. No result claimed for interrupted commands. A mistaken standalone target invocation reported no `handoff_topology_cli` test target; the file is mounted in `combined`, which is the final command target.

All nice invocations inside the sandbox print `nice: setpriority: Operation not permitted`; cargo itself executes normally. Remaining exact outputs and verification results follow below.

## Self review

Pending final diff review and verification.

## Design deviation resolved by controller

The design requests `filter_revisions(instance, 'name', 'all')` and states the table accepts arbitrary kinds. Runtime RED disproved that assumption: `CHECK constraint failed: scope_kind IN ('directory', 'inbox', 'topic')`. Controller approved dedicated `filter_revisions(instance, 'directory', 'name/all')`, separate from `directory/all`, retaining no schema migration and all transaction/filter semantics. Integration owns a dated spec correction.

Grouped RED `nice cargo test --locked --all-features --lib directory_ -- --nocapture` finished build in 1m 17s; 39 passed/2 failed in 1.15s. Name matching tests passed after predicate fix. Production SetThreadName test failed as expected because continuation returned `Ok(Directory(...))` instead of CursorStale. Legacy/isolation fixture initially failed the discovered SQL CHECK before its cursor assertion; rerun uses approved storage key.

Approved-key RED rerun (`nice cargo test --locked --all-features --lib directory_ -- --nocapture`) built in 15.71s; 39 passed/2 failed in 1.14s. Product mutation regression again correctly returned Ok rather than CursorStale. The legacy test now reached its compact-cursor re-encoding fixture but needed original binding strings restored (`cursor binding unavailable`); this is fixture repair, not product RED. After name binding/publication implementation, grouped run produced 40 passed/1 fixture failure; production mutation regression passed. Re-encoding repair restores the original instance/scope/filter binding before replacing only the legacy opaque key; a focused binding-disabled RED replay follows to establish that repaired regression detects the missing binding.

Focused repaired legacy RED: temporarily disabled only filtered name binding; `nice cargo test --locked --all-features --lib directory_filtered_name_revision -- --nocapture` built in 15.36s and failed 0 passed/1 failed in 0.13s with `unwrap_err()` receiving `Ok(Directory(...))` for the legacy key. Restored name binding immediately afterward.

Final directory GREEN: `cargo fmt` followed by `nice cargo test --locked --all-features --lib directory_ -- --nocapture`: exit 0, **41 passed; 0 failed; 3540 filtered out; finished in 1.18s**. Includes all four new directory regressions, existing bounds/high-water/byte-envelope/membership/topic stale tests and picker checks.

Files changed: src/store/queries.rs, src/store/control.rs, tests/store/queries.rs, tests/store/control.rs, tests/handoff_topology_cli.rs. No protocol edits.

Isolated CLI sandbox attempt: `nice cargo test --locked --all-features --test combined directory_search_discovers -- --nocapture` built in 1m 19s, then failed at private Unix socket bind with PermissionDenied/Operation not permitted (0 passed/1 failed). This is an environment refusal, not behavioral RED. Requested and received CLI-only escalation for the same isolated test, retaining CARGO_TARGET_DIR and HT_LEAK_RUN_ID values; no shared service touched.

Real CLI GREEN (approved outside sandbox): `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/target HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8 nice cargo test --locked --all-features --test combined directory_search_discovers -- --nocapture`: exit 0, build/lock elapsed 24.30s, **1 passed; 0 failed; 596 filtered out; finished in 1.12s**. Fixture used instance 5987bfae-b3e3-455c-bac8-ba078a49b6b3 and private temp state/socket; fixture Drop stops daemon and joins host. No shared server changed.

Existing recent checks: `nice cargo test --locked --all-features --lib recent_picker_ -- --nocapture`: exit 0, **24 passed; 0 failed; 3557 filtered out; finished in 0.08s**, warm cargo startup/build 0.03s.

Existing name checks sandbox attempt: `nice cargo test --locked --all-features --lib thread_names_ -- --nocapture`: **21 passed; 3 failed** in 0.18s, cargo build/lock wall 26.49s. The three existing scoped_runtime tests failed at tests/cli/cooperative.rs:2103 private socket bind with PermissionDenied/Operation not permitted. These are environment failures; rerun outside sandbox follows. All existing store/name journal/parser/renderer checks in that selection passed.

## Final self-review

Reviewed production and test diff: candidate work cap, SQL indexed seeks/high water, fit/cut behavior, admitted summaries and protocol shape unchanged. Topic revision remains filter_revision; dedicated selected-instance name revision joins only filtered opaque keys, which rejects legacy filtered keys even when revision absent/zero. None keys retain original spelling byte-for-byte. Actual name changes publish dedicated revision inside deciding transaction; replay/no-op skips writes. Production dispatch tests verify entering/leaving/clear fresh result sets, stale continuation/restart argv, replay/no-op stability, existing recent unfiltered staleness, ordinal unfiltered stability, unrelated Join stability and SetTopic staleness. Matching table tests verify UTF-8/literal whitespace and metacharacters/case, unnamed/topic-only/name-only/both once, None/empty, archive/membership/order and foreign instance exclusion; work test follows all cursors after zero-match Work page. Existing files are large; changes stay localized and reuse their established fixtures without restructuring.

No unresolved correctness concerns. Full suite intentionally not run while ht-zo4 remains open. Final mandated lint/default checks and commit status recorded below.

## Final required verification

All build/test commands inherited CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/target and HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8.

- `cargo fmt`: exit 0; final diff whitespace check `git diff --check`: exit 0/no output.
- Approved outside-sandbox `nice cargo test --locked --all-features --lib thread_names_ -- --nocapture`: exit 0, build 0.06s, **24 passed; 0 failed; 3557 filtered out; finished in 0.33s**. All previously socket-denied existing regressions pass.
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: exit 0, `Finished dev profile [unoptimized + debuginfo] target(s) in 35.55s`, no cargo warnings/errors.
- `nice scripts/check-default-features`: exit 0. Script succeeds silently and emitted no cargo warning/error (sandbox nice warning only).
- Approved outside-sandbox `scripts/check-no-leaked-processes --run-id c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`: exit 0, `no leaked test processes`.

Final successful focused coverage: 41 directory checks + 24 name checks + 24 recent checks + 1 actual CLI discovery check. Shared Cargo target lock waits were observed independently of active coordinator/docs jobs and were allowed to finish; no unowned process was stopped. Warm library rebuilds were ~15s; all-feature lint was 35.55s. No shared Herdr mutation, user configuration write, push, stash or full-suite run occurred.

Status: DONE; controller-approved storage-key deviation recorded above, no outstanding concerns. Commit is the final task mutation; controller owns task close, review and integration.

## Fresh private CLI evidence (supersedes historical CLI result)

Command with private target/run ID exported: `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --test combined directory_search_discovers -- --nocapture`, approved outside sandbox for isolated sockets. Explicit task1 source `Compiling herdr-threads v0.6.0 (/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/thread-search-discovery/.worktrees/super-auto-thread-search-discovery--task-ht-akx.1)`; package-cache lock messages only (no shared target lock). `Finished test profile ... in 2m05s`; `Running tests/combined.rs (target/debug/deps/combined-ca453c538c6f6d21)`; **1 passed,0 failed,596 filtered out,in1.57s**, exit0. Inventory: `handoff_topology_cli::directory_search_discovers_canonical_name_with_unrelated_topic ... ok`. Owned fixture instance4b9e0d11-8f1a-483f-8b50-94a831af35e5, daemon49314, private host `/private/tmp/hts-ed7094923595/host.sock`; Drop stops/joined fixture children.

## Fresh private mandatory checks and completion

Private target/run ID remained exported throughout. `nice cargo clippy --manifest-path "$PWD/Cargo.toml" --locked --all-targets --all-features -- -D warnings` freshly checked the explicit task1 source path and completed with `Finished dev profile ... in 56.65s`; no cargo warning/error. `nice scripts/check-default-features --manifest-path "$PWD/Cargo.toml"` completed silently with no cargo warning/error. Sequential validation shell exited0. Logs: `/tmp/ht-akx-1-private-clippy.log`, `/tmp/ht-akx-1-private-default.log`; both include full validated manifest/private-target/SHA metadata. Sandbox nice emitted its known setpriority warning only.

Approved outside-sandbox `scripts/check-no-leaked-processes --run-id c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`: exit0, exact output `no leaked test processes`.

Final `git rev-parse HEAD`: d61b47204a1770b91582f945b2c784e35c796f47. `git status --short`: empty. `cargo fmt --check` and `git diff --check`: exit0/no output. No source changes or new commits during revalidation. No source mutation overlapped validation. Acceptance evidence is now the fresh private42 directory/24 names/24 recent/1 generated/1 actual CLI plus private mandatory checks and cleanup; earlier shared-target results remain historical only. Status DONE; no outstanding concerns.
