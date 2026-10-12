# Task 22 / ht-0lo.14 source-only implementation report

Status: IMPLEMENTED (source committed; parent runtime verification pending).

## Implementation

Changed only `tests/store/effective.rs`, in
`receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once`.
The historical receipt catalog comparison retains the existing eight explicitly
named migration42 exclusions and now compares object names alongside their exact
stored SQL definitions. The six migration45 presentation fence trigger names are
partitioned separately and compared against independent literal names, tables,
events and expected SQL bodies, including missing/exhausted fence guards and the
version increment. An unexpected additional object remains in the historical
comparison and fails it; the six intentional additions are checked, not silently
excluded.

All original downstream assertions and fixtures are unchanged: genuine v28 and
v36 origins, populated row snapshots, sequence 99, child FK graph, enabled FK
setting, immutable receipt ordinal, absent availability with deadline 1300,
reinitialization with clock 9999, frozen historical decision sequence 10, and
v36 invitation provenance/confirmation/deadline/index preservation.

## Requirements and assumptions

Read current `bd --sandbox --readonly show ht-0lo.14 --json` and comments (none).
No current amendment conflicted with frozen Task 22. Read the parent's extracted
brief and independently extracted Task 22 using the shipped `task-brief` helper
to `.superpowers/task-22-brief.md` inside the task worktree, respecting the rule
that only this report may be written outside it. Base/branch were verified clean
before editing; no worktree recreation or rebase occurred.

The explicit parent runtime override governs generic implementer/TDD/verification
templates: this task is a tests-only oracle correction with an existing causal
FAIL, one source commit, and all actual Cargo/runtime/gates owned by the parent.
No production, trust-policy, wire, cap, design or tracker changes were made.

## Verification evidence

Source-only checks executed in this worktree:

```text
rustfmt --edition 2024 --check tests/store/effective.rs
exit 0; no output
git diff --check
exit 0; no output
git status --short
empty after commit
```

These checks verify formatting/whitespace and committed source state only; they
do not verify compilation or runtime assertions.

Existing parent-provided causal RED evidence was read from
`/private/tmp/ht-roast1-migration-preservation-control/execution.json` and
`run.log`, recorded at head `3abee5a6add69443e49322e96f9130b834686235`:

```text
nice cargo nextest run --locked --all-features --lib --no-fail-fast -E
test(=store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once)

Summary [0.192s] 1 test run: 0 passed, 1 failed, 4938 skipped
FAIL receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once
exit 100
```

The failure is the old object equality at `tests/store/effective.rs:2145`:
the current catalog contains the six intentional migration45 receipt triggers
in addition to the unchanged historical catalog. No historical objects were
removed. Assertions below that failure were not measured in the RED run; the
v36 iteration was not reached. The prior leak check recorded exit 0, but it is
parent evidence, not a check run by this implementation agent.

Intended GREEN: the same exact selector passes once (one test body), completing
both genuine v28 and v36 loop iterations and reaching every original downstream
assertion. Parent must run the selector on the committed correction and supply
actual logs before review/integration. No GREEN/runtime result is claimed here.

Falsifying values for the changed assertions:

- Historical equality fails if an old object such as
  `receipts_ordinal_immutable` is removed/renamed/modified, or an unlisted trigger
  is added to either receipt table.
- Migration45 equality fails if any of the six literal trigger names is absent,
  its event/table differs, its missing/exhausted fence guard is removed, or its
  increment becomes a no-op. Expected SQL is specified independently of current
  migration source and `schema::initialize` output.

## Test hygiene and concerns

No tests, Cargo/build, Node/plugin runtime, product binaries, sockets, daemons,
private Herdr servers, helper processes or background commands were started.
No new process leak check was needed or run. No Herdr or tracker mutations,
delegation, targets, ROOT source/index epoch changes or real-home writes occurred.

Runtime validation, parent gates, one light task review and serial integration
remain pending by contract. Scope is one test-only file; no production commit
is applicable. Git add/commit required approved narrow escalations because the
shared worktree metadata is outside the filesystem's writable worktree root;
both commands completed successfully.

## Source-only handoff

- Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`
- Tests commit / head: `538bfd7c43d9778aa847e1c7028563b84b396a8c`
- Production commit: none (tests-only task)
- Branch: `super-auto/roast1-ht-0lo-14`
- Changed paths: `tests/store/effective.rs`
- Intended RED: one test fails at historical catalog equality because six
  migration45 triggers are added and no historical objects removed; downstream
  preservation checks unmeasured.
- Intended GREEN: one test passes, exercising both v28/v36 origins and every
  downstream preservation assertion.
- Exact selector:
  `store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once`
- Parent command:
  `nice cargo nextest run --locked --all-features --lib --no-fail-fast -E 'test(=store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once)'`
- Runtime claims: none.

## Parent actual GREEN evidence (stage2)

This appendix supersedes the earlier pending-GREEN/runtime-claims statements
for the two migration selectors only. The source commit remains
`538bfd7c43d9778aa847e1c7028563b84b396a8c`; no source edits accompanied this update.

Read parent logs at
`/private/tmp/ht-roast1-rust-stage2-corrected-741984b9/execution.json` and
`run.log`. Actual validation head:
`046100d70eedc1076279426e64c5949464e09f48`.
Direct `git rev-parse` checks confirmed identical `tests/store/effective.rs`
blobs at the task commit and parent validation head:
`fc495dc2725591f995c866233e93a654ecad6c12`.
This establishes source equivalence for this tests-only task despite the parent
validation branch's additional independent regression changes.

Parent's exact runtime command (from `execution.json`):

```sh
nice cargo nextest run --locked --all-features --lib --test service --no-fail-fast -E 'test(=store::queries::tests::query_work_q4::q4_private_fresh_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_private_resumed_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_public_and_origin_warning_sources_remain_enabled) | test(=store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers) | test(=store::queries::tests::query_work_q4::q4_actual_continued_tail_checks_scope_once_and_charges_retirement) | test(=store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once) | test(=store::effective::tests::receipt_send_time_upgrade_failure_rolls_back_constraint_and_cutover_then_retries) | test(=presentation_owner_budget_controls::presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_pull_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_live_owner_waiters_serialize_and_succeed_after_release) | test(=presentation_owner_budget_controls::presentation_poisoned_owner_recovers_for_pull_and_confirm)'
```

Relevant actual output:

```text
PASS [0.466s] herdr-threads store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once
PASS [0.868s] herdr-threads store::effective::tests::receipt_send_time_upgrade_failure_rolls_back_constraint_and_cutover_then_retries
Summary [10.592s] 13 tests run: 9 passed, 4 failed, 5028 skipped
```

The populated preservation selector passed its full test body, including both
genuine v28/v36 origins and all downstream assertions. Existing rollback/retry
coverage also passed. The combined process exited 100 because the four
owner-waiter regression selectors were still intentional causal RED controls
for another task at this validation stage; this is not an overall green batch.

Parent timing: listing/build step 15.179s; runtime process 10.819s.
Parent leak check:
`scripts/check-no-leaked-processes --run-id ea337674-33a7-4cd0-9b47-18f05aa0b118`
exited 0 (0.194s); no leaked processes were reported.

All-four-task combined gates, light review and serial integration remain parent
work. This agent ran no Cargo/runtime processes and made no new source commit.
