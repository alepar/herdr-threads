# Reviewed bounded invitation/warning patch

Scope: relayed invitation relevance request mWRYhdpzB and warning replay request mt6OsTsap in tbMYU5Ter, plus empty-inbox corroboration assigned in taQXwBH7X. The broader tab-creating handoff design remains pending; no topology creation, human-namespace grammar or installer permissions are implemented here.

Verified source base: 36d86ad9d69472d5c29ed4d827e858f67ea35975. Source fingerprint inventory: source-inventory.json. Independent review approved the exact tracked patch SHA256 `1ede646d41fe6c19ecb8c36a4f79dab0043302928d9ef6e3508c77186f87ccd9` plus migration SHA256 `be0ff728fb17015517364604d2d2fe55a5a826665fb1f34e23ecf2f854431417`; both initial Minor findings were resolved and re-reviewed. Migration25 is provisional until actual-main landing order is reconciled with the separate adapter/lazy lanes.

Verification against this source:

- `nice cargo nextest run --locked --all-features --lib` with the recorded relevant-test expression: 414 passed, 2255 skipped, 35.047s. This is a focused selection, not the full suite. Raw output: focused-final.log.
- CLI hook target originally exposed two stale unconditional-accept expectations and one obsolete quiet-between-notice-pages expectation. All three updated behavior regressions passed in 5.497s; raw recheck and original failures retained. Final complete target: 45 passed, 11 skipped, 56.743s (hook-entrypoint-final.log).
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`: exit0, 21.68s (clippy.log).
- `nice scripts/check-default-features`: exit0; successful command is silent (default-features.log).
- `cargo fmt --check` and `git diff --check`: exit0.
- Nine real warning regressions include canonical creation dedup, clear/new events, exact16/1carried pages, late projection/fanout, independent recipients, successor occupants, physical/manifest history, v24 upgrade/tamper detection, 1001delivered events with flat pending work, and late recipient coverage after projection cleanup. Raw meaningful RED/GREEN logs retained; authority/replay suites are in the focused selection.
- Five BEFORE and five AFTER pressure evaluations are scored in assessment.md, with the reported repro and simulation limits distinguished.

Owned process/topology inventory:

- Worker pane w4:pD0 in tab w4:t85 and worktree .worktrees/handoff-workflows remain owned implementation topology. Keep until independently verified DONE+MERGED; no other pane/workspace was closed.
- Tests used owned temporary SQLite databases, local mock/socket fixtures and nextest test processes. Existing fixture helpers spawn owned CLI children and reap them; this patch adds no bare test child spawn, native model launch, shared Herdr restart, or real harness/config write.
- Leak scope `4408334c-9d92-4a88-9a8c-f189370ded99`; scripts/check-no-leaked-processes with that run ID and this worktree root reported no leaked test processes. Final target cleanup is checked again before freeze.
- Pending broader design and its five original handoff baseline reports are retained under ignored `target/pending-handoff-workflows/docs/`, outside this bounded commit. No evidence is deleted.

Main was independently clean at ada2f3767867096d9821db450bd0f6e8eb633d6e during preparation. Its v0.2.12 release CI repair/freeze is still separate. No main mutation, push, release, tag or local installed-package change belongs to this evidence checkpoint. Integration requires the actual coordinated main window and current-base checks; main owns integrated gates/release/local update.
