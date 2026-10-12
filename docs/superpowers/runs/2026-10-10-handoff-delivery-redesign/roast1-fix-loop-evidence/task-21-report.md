# Task 21 / ht-0lo.13 implementation evidence

Status: IMPLEMENTED, source-only; actual parent-owned focused GREEN supplied.
Combined required gates remain parent-owned and pending.

## Implementation

Private ordinary discovery marks its warning phase complete during fresh capture
and normalizes resumed cursors the same way before rotating sources. Its work
guard no longer reserves two candidate slots for the excluded warning phase.
Public V1/V2 warning discovery and the independent origin-warning path retain
their existing implementation. Candidate limit 100 and all enabled-source work
charging remain unchanged. No canonical history or wire shape changed.

Regression fixtures retain 128 and 8192 closed membership episodes for the
selected seat, all preceding an unprojected negative-audience warning. Positive
ordinary-kind messages with lazy delivery exercise actual private discovery.
The resumed witness uses a real public V2 continuation with unfinished warning
discovery, not a fabricated cursor. A separate control proves public V2 checks
the negative audience and exposes the positive warning, and that the dedicated
origin source exposes it to its canonical original inviter.

## Provenance and scope

- Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`.
- Tests-only commit: `06060869280e675323b26055b87440e336fcb618`.
- Production commit / head: `2b49940f7530b4995c5f10eeb0337a0682c42370`.
- Files: `tests/store/query_work_q4.rs`, `src/store/queries.rs`.
- Both commits have truthful Outside-adapter trailers. No rebase performed.
- Current bead and comments were read with `bd --sandbox --readonly`; no comments
  or amendments exist. The current bead and exact Task 21 brief agree.
- Parent supplied the initially absent brief using the shipped task-brief helper.

## Tests and TDD

Runtime ownership stayed with the parent as dispatched. This agent did not run
Cargo, a binary, a daemon, a socket probe, Herdr, or a test process.

Parent supplied actual causal RED evidence at validation HEAD
`c2b0d8fbda2f1ff39cd2aaf513cd14ecc85c7c22`, whose tests include the tests-only
commit against unchanged production. Artifact directory:
`/private/tmp/ht-roast1-source-red-c2b0d8fb/` (`execution.json`, `run.log`).

Exact RED command:

```sh
nice cargo nextest run --locked --all-features --lib --no-fail-fast -E 'test(=store::queries::tests::query_work_q4::q4_private_fresh_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_private_resumed_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_public_and_origin_warning_sources_remain_enabled)'
```

Actual relevant output:

```text
FAIL q4_private_resumed_excludes_warning_history
assertion `left == right` failed: private resumed read skips warning audience SQL
  left: 1
 right: 0
FAIL q4_private_fresh_excludes_warning_history
assertion `left == right` failed: private fresh read skips warning audience SQL
  left: 1
 right: 0
PASS q4_public_and_origin_warning_sources_remain_enabled
Summary [0.298s] 3 tests run: 1 passed, 2 failed, 4939 skipped
```

Both failures are the intended actual warning-audience traversal defect. All
assertions after those failures, including 8192 history iterations and flat-VM
comparisons, are unmeasured in RED. Command exit 100; recorded command duration
0.523s; test selection/build listing 56.208s. Parent leak check exit 0 with run ID
`71e746eb-890d-406a-823d-4161338e45d9`.

Intended GREEN values, not claimed measured until parent evidence arrives:

- Fresh: warning audience probes 0, actual eligibility traces `[2,0]`, visits 2,
  lazy scope probes 6, exactly `msg-q4-0` and `msg-q4-1`, exhausted cursor;
  actual VM cost equal at 128 and 8192 retained episodes.
- Resumed: warning audience probes 0, actual eligibility traces `[1,0]`, visits 1,
  lazy scope probes 3, exactly remaining `msg-q4-1`, exhausted cursor;
  actual VM cost equal at 128 and 8192 retained episodes.
- Public control: negative frozen audience probe 1, public warning exactly
  `q4-origin-warning`; dedicated origin warnings for canonical inviter
  `a-q4-0` contain exactly that warning.

Precise new selectors are the three fully qualified names in the RED command.
Additional relevant existing selectors:

```text
store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers
store::queries::tests::query_work_q4::q4_actual_continued_tail_checks_scope_once_and_charges_retirement
```

Source checks completed: changed-file rustfmt and `git diff --check`. These are
format/diff checks, not runtime claims. Task worktree is clean.

## Test hygiene, assumptions and concerns

No processes were started by this agent; no process cleanup was needed. Parent
owns validation processes and leak checks. Causal RED parent leak check found
none. No tracker writes, Herdr mutations, delegation, or changes outside the
assigned source/test paths and this authorized report.

Assumption: ordinary positive means the ordinary message kind as consumed by
the private ordinary source, including lazy delivery. The new tests assert those
positive LazyMessage items; existing unread/receipt tests remain relevant caller
coverage for their unchanged enabled sources.

Concern: required combined gates remain pending parent execution. No
untested passing claim is made. No model-visible text was added or changed.

## Supplied actual GREEN and blob equivalence

Parent supplied and this agent read actual stage-2 validation artifacts at
`/private/tmp/ht-roast1-rust-stage2-corrected-741984b9/` (`execution.json`, `run.log`).
The recorded validation HEAD is
`046100d70eedc1076279426e64c5949464e09f48`. `git rev-parse` proves its production
`src/store/queries.rs` blob and task production commit
`2b49940f7530b4995c5f10eeb0337a0682c42370` both equal
`f16f22e4637a104c69d3879e45eafac61b61d173`. This is exact production-file
equivalence, not a claim that the combined validation tree equals the isolated
task tree. The corresponding `tests/store/query_work_q4.rs` blobs also match
exactly at `51d031da75ac37d408e533c65dde98330c56ec16`.

Exact parent GREEN command (a combined stage also contains owner-budget RED
controls, so the overall command exits 100):

```sh
nice cargo nextest run --locked --all-features --lib --test service --no-fail-fast -E 'test(=store::queries::tests::query_work_q4::q4_private_fresh_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_private_resumed_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_public_and_origin_warning_sources_remain_enabled) | test(=store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers) | test(=store::queries::tests::query_work_q4::q4_actual_continued_tail_checks_scope_once_and_charges_retirement) | test(=store::effective::tests::receipt_send_time_upgrade_preserves_populated_v28_and_v29_graph_and_restarts_once) | test(=store::effective::tests::receipt_send_time_upgrade_failure_rolls_back_constraint_and_cutover_then_retries) | test(=presentation_owner_budget_controls::presentation_pull_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_pull_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_cancelled_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_confirm_expired_owner_waiter_returns_before_release_without_effects) | test(=presentation_owner_budget_controls::presentation_live_owner_waiters_serialize_and_succeed_after_release) | test(=presentation_owner_budget_controls::presentation_poisoned_owner_recovers_for_pull_and_confirm)'
```

Actual task-relevant output:

```text
PASS [0.351s] q4_public_and_origin_warning_sources_remain_enabled
PASS [0.963s] q4_actual_continued_tail_checks_scope_once_and_charges_retirement
PASS [1.080s] q4_private_fresh_excludes_warning_history
PASS [1.093s] q4_private_resumed_excludes_warning_history
PASS [10.577s] q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers
Summary [10.592s] 13 tests run: 9 passed, 4 failed, 5028 skipped
```

All five task-relevant controls pass. The new tests therefore reach all literal
visit/scope/output assertions, both retained-history populations, and the actual
equal-VM assertions listed above; those are now measured GREEN. The passing
nextest output does not print the VM values themselves, so no numerical VM count
is asserted here. Parent identifies the four service owner-wait failures as
expected causal RED controls for the separate owner-budget task; the actual log
shows all four fail at `exhausted waiter must return BEFORE owner release:
Timeout`. No overall suite-success claim is made from this mixed stage.

Recorded command duration 10.819s, selection/build listing 15.179s. Parent leak
check `scripts/check-no-leaked-processes --run-id
ea337674-33a7-4cd0-9b47-18f05aa0b118` exits 0 (0.194s), with no leaked processes.
Required lint, neutral-core, default-feature and combined gates await parent
execution. No source files changed when recording this evidence.

Source-only handoff: base `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`; tests commit
`06060869280e675323b26055b87440e336fcb618`; production commit
`2b49940f7530b4995c5f10eeb0337a0682c42370`; intended RED/GREEN values and exact
selectors above; runtime verdicts belong to the parent evidence.
