# ht-akx.3 integration sweep — 2026-10-10

Status: DONE. No implementation correction or additional regression was necessary. Added the dated integration verification note to the binding design; no behavior changes.

## Exact source and provenance

Stable checked HEAD: `65a67dcf6f7bd63d0b0fd8b2665af896bc8ca95f`, branch `task-ht-akx.3`. Workdir/explicit manifest root: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/thread-search-discovery/.worktrees/super-auto-thread-search-discovery--task-ht-akx.3`.

Every build/test used `CARGO_TARGET_DIR="$PWD/target"`; every test exported `HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`. Set up task-private dependency cache with `cp -cR /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/thread-search-discovery/target ./target`, then `cargo clean -p herdr-threads --target-dir "$PWD/target"` removed 1040 files/2.7GiB only from this private target. Shared caches were not cleaned. Fresh compilation explicitly printed `herdr-threads v0.6.1 (TASK3_WORKDIR)`. Fresh library executable: `target/debug/deps/herdr_threads-d0733bebb75f2cd3`; combined executable: `target/debug/deps/combined-ed122a5292bb844d`. No source mutation overlapped checks. Final spec-only note was appended after all checks/process cleanup.

Read both leaf reports; their earlier shared-target evidence is superseded. This report's fresh combined-source checks are acceptance evidence. Initial fresh library compilation took1m42s; combined compilation/lock elapsed2m11s. These are cleaned-package builds, not established incremental regressions. Rejection selection waited for this task's own private combined artifact lock (59.12s cargo elapsed). No other worktree target was used.

## Goal and contract trace

- `src/store/queries.rs::directory`: ordinal and both indexed-recent candidate SELECTs project nullable name, apply one literal case-sensitive name OR topic predicate before summary admission, retain existing indexed seek/high-water/work/row/byte accounting. No independent name scan, per-candidate name query or duplicate admission.
- Name-only/topic-only/both/unnamed/no-match, empty/None, UTF-8, case and literal punctuation, selected-instance isolation, Default/Joined/Invited/All membership and archive semantics: `directory_name_or_topic_literal_matching_and_scopes`.
- Both traversal orders follow zero-match Work pages to exactly one name-only hit without skips/duplicates: `directory_name_hit_after_empty_work_page_has_no_skips_or_duplicates`. Existing directory budget/envelope/high-water tests remain in the42 gate.
- Sole production name UPDATE is `control::set_thread_name`; dedicated `filter_revisions(directory,name/all)` bump shares its canonical mutation transaction and actual-change branch. Replay/unchanged name bypass writes. Create inserts a new generated ID with initial name; service ensure returns existing row unchanged or inserts without name. No unwired production existing-thread name writer found.
- Filtered opaque keys bind selected-instance dedicated name revision (missing row zero); existing topic revision remains. Unfiltered key spellings remain unchanged. `directory_filtered_name_revision_is_selected_and_rejects_legacy_keys` proves other-instance and unrelated directory/all revisions do not newly stale and old filtered keys do stale.
- Production cooperative permit/StorePort dispatch set, rename-out/in and clear exercise both orders and first/later candidates: `directory_name_mutations_stale_filtered_cursors_but_replay_and_noop_do_not`. Checks fresh result membership, CursorStale/restart search, no-op/replay stability, ordinal unfiltered stability, existing recent unfiltered staleness, unrelated Join stability and topic-edit staleness.
- Actual isolated CLI fixture creates named `psa-global` with topic `Important system wide announcements`, then invokes exact `thread list --search psa-global` and requires canonical ID/name. Existing fixture owns isolated fake host, daemon/configs and teardown.
- Public help names case-sensitive literal name/topic semantics; parser continuation keeps literal context/scope/order/format/bounds; legacy `topic_contains` serde shape unchanged. Generated continuation directory regression also runs in42 gate. README and adopted amendment agree; generic message search/exact resolution/picker are untouched.

## Refreshed-main merge seams

Compared exact candidate against `effa14a2`. Rejection function remains present and its source diff relative main is empty; only the separate name setter's dedicated revision publication changes in control.rs. CLI retains full Reject help/argv alongside search help. All appended invitation rejection tests precede the separate name-mutation regression and remain intact;16 rejection checks pass. Specs INDEX retains both thread-search-discovery and invitation-rejection-policy rows. No merge defect found, no trust-sensitive edits performed, no policy weakening/amendment required.

## Fresh verification commands/results

Commands below run from task3 workdir with exports above and explicit manifest:

- `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --lib directory_ -- --nocapture`: exit0,42 passed/0 failed/3547 filtered,1.46s tests. Log `/tmp/ht-akx-3-directory.log` includes all four name regressions and generated continuation test.
- Approved outside sandbox for private Unix sockets: `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --test combined directory_search_discovers -- --nocapture`: exit0,1 passed/0 failed/596 filtered,3.67s. Log `/tmp/ht-akx-3-cli.log`; isolated instance `f5cb3f20-670d-46ff-8ff0-2309b4f05b47`, private host `/private/tmp/hts-6fa2e28d8371/host.sock`, fixture daemon3091 torn down.
- `nice cargo test --manifest-path "$PWD/Cargo.toml" --locked --all-features --lib invitation_rejection_ -- --nocapture`: exit0,16 passed/0 failed/3573 filtered,3.99s. Log `/tmp/ht-akx-3-rejection.log`.
- Same test command prefix with `--lib thread_list_help_explains_literal_name_or_topic_search`, `--lib continuation_argv_round_trips_context_filter_format_and_bounds`, and `--lib picker_directory_wire_keeps_old_directory_shape_and_cursor_only_contract`: each exit0,1 passed/0 failed/3588 filtered,0.00s. Logs `/tmp/ht-akx-3-{help,argv,wire}.log`.
- `cargo fmt`: exit0, source status remained clean.
- `nice cargo clippy --manifest-path "$PWD/Cargo.toml" --locked --all-targets --all-features -- -D warnings`: exit0,50.48s, explicit task3 source check, no cargo warnings/errors; `/tmp/ht-akx-3-clippy.log`.
- `nice scripts/check-default-features --manifest-path "$PWD/Cargo.toml"`: exit0, script silent; `/tmp/ht-akx-3-default.log`.
- Approved outside sandbox after all checks: `scripts/check-no-leaked-processes --run-id c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`: exit0, `no leaked test processes`. An earlier premature check during active clippy listed only that still-running compiler tree; no process was killed. Final check confirms complete cleanup.

Sandbox `nice` emitted its known setpriority denial; cargo commands executed normally. Full suite intentionally omitted per ht-zo4/project/task instructions. No shared service/config writes, Herdr-thread mutations, push, stash, unowned process termination or nested agents.

## Self-review and changes

Reviewed exact binding spec and integrated source/test diff, merge seams, name-write inventory and all check outputs. No meaningful uncovered gap; adding duplicate tests would mirror existing coverage. TDD not applicable because this sweep changes only a verification note. Only tracked change: `docs/superpowers/runs/2026-10-10-thread-search-discovery/2026-10-10-thread-search-discovery-design.md`. Report is controller-owned SDD metadata outside task worktree. Final diff whitespace and status reviewed before commit. No outstanding concerns. Controller owns review, integration and bead close.
