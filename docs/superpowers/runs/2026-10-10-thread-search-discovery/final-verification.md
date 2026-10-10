# Final post-roast verification

Checked candidate: `4a732c83d005ebfbf622361a3cc2b2534664c773`, combining the code-roasted feature (`840522e5` vs `effa14a2`) with current main `7c40c83c`. The second main merge was conflict-free and only changed canary workflow/Python/tests/docs and release metadata. Reviewed those changes and confirmed zero diff in feature source/tests against the roast input. No feature behavior changed after roast.

All commands ran from the integration worktree with explicit `--manifest-path "$PWD/Cargo.toml"` where applicable and private `CARGO_TARGET_DIR="$PWD/target"`. Tests exported `HT_LEAK_RUN_ID=c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`. No edits or commits overlapped verification. The following source/config tree objects remain the verification identity across later report-only commits (src, tests, Cargo.toml, Cargo.lock in order):

```
062cd8b729e76076d447c14d8d29909834c0f7f8
ef215e17da48ff14a23e4cfee16dd3f33fed14a6
71eca139705ff95887871a77a9ef5d4d61dc464a
be0f5c5251936f39feb3344fa0d711d4c8a44aeb
```

- `cargo fmt --check`: exit 0.
- `nice cargo test --locked --all-features --lib directory_`: exit 0; 42 passed, 0 failed, 3547 filtered; test execution 1.29s. First v0.6.2 library build 1m58s, explicitly printed this integration source path. Log: `/tmp/ht-akx-final-directory.log`.
- `nice cargo test --locked --all-features --test combined directory_search_discovers -- --nocapture`: exit 0; 1 passed, 0 failed, 596 filtered; test execution 1.33s. First v0.6.2 CLI/combined build 1m17s. Isolated instance `1267574a-874a-4f87-8a43-4c991ca55b2f`, private host `/private/tmp/hts-9f2e4b517287/host.sock`, owned daemon 84420 torn down. Log: `/tmp/ht-akx-final-cli.log`.
- Three focused `--lib` checks: `thread_list_help_explains_literal_name_or_topic_search`, `continuation_argv_round_trips_context_filter_format_and_bounds`, `picker_directory_wire_keeps_old_directory_shape_and_cursor_only_contract`: each exit 0, 1 passed / 0 failed; warm invocations 0.04–0.05s. Logs: `/tmp/ht-akx-final-{help,argv,wire}.log`.
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: exit 0, 40.14s, no Cargo warnings. Log: `/tmp/ht-akx-final-clippy.log`.
- `nice scripts/check-default-features`: exit 0, no Cargo warnings. Log: `/tmp/ht-akx-final-default.log`.
- `git diff --check main..HEAD -- src tests README.md docs/design`: exit 0.
- `scripts/check-no-leaked-processes --run-id c494afe8-aa16-4b6b-a2a8-b9bee8e65cb8`: exit 0, `no leaked test processes`, after CLI and again after all tests.

No full-suite run: project instructions prohibit routine full-suite execution until ht-zo4 lands; relevant focused production-path checks are authoritative. Earlier private leaf/integration coverage additionally includes names24/recent24/rejection16; see task reports. Initial compilation after a release-version change is not a measured warm incremental regression. Sandbox `nice` priority denial is environment noise; every underlying check above completed successfully.

Report-only commits after this candidate do not change validated src/tests/Cargo objects; final handback checks compare these objects explicitly. A later material base movement requires new combined-tree verification before merge.
