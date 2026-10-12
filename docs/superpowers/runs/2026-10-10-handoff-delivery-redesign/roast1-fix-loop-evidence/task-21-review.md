# ht-0lo.13 / Task 21 scoped review

Status: CLEAN
Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`
Reviewed HEAD: `2b49940f7530b4995c5f10eeb0337a0682c42370`

## Spec compliance

✅ Current `bd --sandbox --readonly show ht-0lo.13 --json` and `bd --sandbox --readonly comments ht-0lo.13` agree with the frozen Task 21 scope and brief; no amendments or comments exist. The supplied base is an ancestor of HEAD. The package contains two commits and changes only the two authorized paths.

✅ Fresh capture marks the warning source complete only in ordinary-only mode (`src/store/queries.rs:6540`); resumed cursors are normalized before rotation (`:7165`). The warning-only two-slot reservation is disabled only for that mode (`:7296`). The 100-candidate limit and enabled-source work accounting remain intact. Public query still selects ordinary_only=false, and the V1 and origin-warning implementations are untouched.

✅ The regression retains 128 and 8192 closed negative-audience episodes with no recipient projection, verifies positive ordinary lazy items, exact charged visits/scope probes, zero warning audience probes, exhausted private cursors and equal measured VM work. Resumption uses an actual public V2 continuation whose warning phase is unfinished. A positive public/origin control proves the observer sees the negative lookup and preserves both dedicated warning paths.

Combined lint, harness-neutral, default-feature and integration gates remain parent-owned and pending. This verdict is the scoped source/test-evidence review, not a claim that those gates or the whole task are complete.

## Issues

Critical: none.

Important: none.

Minor: none.

## Reachability and focused checks

Concrete risk: the private wrapper could be unused, or a sibling caller could retain warning traversal. Read-only caller search traces `Command::DeliveryPullV2` (`src/service/dispatch.rs:1426`) to service `presentation_pull`, store `presentation_pull` (`src/store/mod.rs:1297`, `:1309`), `invitation_content::pull` (`src/store/invitation_content.rs:2096`, ordinary call at `:2442`), and `ordinary_presentation_page` (`src/store/queries.rs:7443`) to `query_mode(..., true)` (`:7460`). Searches find one production ordinary-page caller and exactly the private true/public false query-mode call sites.

Concrete risk: marking the cursor bit might not prevent SQL traversal. The focused `source_done`/rotation read (`src/store/queries.rs:6546`) confirms Warnings completion is exactly `next_manifest_warning`; the loop rotates completed sources before scanning. Public query still calls `query_mode(..., false)` (`:7114`).

## Test evidence

No tests or Cargo commands were run by this reviewer. The report records causal RED with `nice cargo nextest run --locked --all-features --lib --no-fail-fast -E` selecting the three new fully qualified `store::queries::tests::query_work_q4` tests. Read `/private/tmp/ht-roast1-source-red-c2b0d8fb/run.log`: fresh and resumed both fail specifically at actual warning audience probes 1 versus 0; public/origin passes. Summary: 3 run, 1 passed, 2 failed.

The exact parent GREEN command is recorded in `task-21-report.md` and `/private/tmp/ht-roast1-rust-stage2-corrected-741984b9/execution.json`: `nice cargo nextest run --locked --all-features --lib --test service --no-fail-fast -E` with the three new Q4 selectors, existing `q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers` and `q4_actual_continued_tail_checks_scope_once_and_charges_retirement`, two migration selectors and six owner-budget selectors. Read the actual execution JSON and run log: all five task-relevant Q4 tests pass; overall 13 run, 9 passed, 4 failed, exit 100. The four failures are separately scoped causal owner-wait controls, explicitly identified in dispatch and report, and all fail at `exhausted waiter must return BEFORE owner release: Timeout`. They are not an overall success and do not invalidate these five positive source controls. Leak check exit 0, run ID `ea337674-33a7-4cd0-9b47-18f05aa0b118`.

Read-only `git rev-parse` independently confirms both changed-file blobs at task HEAD equal the parent validation HEAD `046100d70eedc1076279426e64c5949464e09f48`: production `f16f22e4637a104c69d3879e45eafac61b61d173`, tests `51d031da75ac37d408e533c65dde98330c56ec16`. Evidence is adequate for this scope; no passing assertion is inferred from intended values alone.

Test changes commands (both exit 0), run in the task worktree:

```sh
git diff --stat bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Result: one test file, 201 insertions and 3 deletions. The deletions extend the existing observer storage from two counters to three; existing assertions remain unchanged. No test is removed, skipped or weakened.
