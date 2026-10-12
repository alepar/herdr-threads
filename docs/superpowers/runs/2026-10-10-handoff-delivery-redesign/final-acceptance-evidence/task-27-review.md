# ht-0lo.19 light source/evidence review

Status: CLEAN
Base: 3d23527675170cacaec470aa13b3baf53688d4ba
Reviewed HEAD: c3f9f6b58da36729aaf653b9a9e8ec93b3187260

## Spec compliance

✅ Current `bd --sandbox --readonly show ht-0lo.19 --json` requirements agree with the frozen Task 27 plan and brief; `bd --sandbox --readonly comments ht-0lo.19` reports no comments. No conflicting amendments or triage clarifications were found.

✅ The one-commit package changes only the two named test functions in `tests/hook_entrypoint/agy.rs` and `tests/store/receipts.rs`. The AGY test adapts its oracle to Main's versioned direct-send envelope, preserves the canonical captured thread/invitation/message comparisons, and adds explicit version, kind and typed-result checks. The receipt test advances the genuine historical SQL prefix from 37 to 38, with one explanatory comment. No production, spec, SQL, cap, timeout, downstream hook/body/no-settlement or rollback/reset/warning assertion changes occur in the diff.

✅ Both required independent diagnosis reports were read. Their old/new envelope and physical-migration rationale agree with the patch. A focused check for the risk of adapting the oracle to an invented output shape inspected `src/cli/direct_send.rs:453`: the producer actually serializes `CommandResult::MessageSent` inside `{version,result}` and adds thread and selection there. A focused check for weakening the historical upgrade scenario inspected `tests/store/receipts.rs:4426` and `migrations/0038_receipt_send_time.sql:89`: the helper applies a real sorted SQL prefix, and migration 38 rebuilds receipt state with independently nullable availability/deadline. The original trigger-induced rollback, cursor/completion/derived-row assertions and retry/reopen checks remain intact.

The caller's serial merge and sole final-C rerun remain subsequent integration work, as the task/report explicitly assign them to the caller. This review does not claim that final acceptance sweep has run.

## Issues

Critical: none.
Important: none.
Minor: none.

## Test evidence

No runtime, Cargo, Herdr, tracker mutation, source mutation or delegation was performed by this reviewer.

The report links `/private/tmp/ht-final-regression5/execution.json` and `run.log`. The recorded command is:

```sh
nice cargo nextest run --locked --all-features --lib --test combined --test hook_entrypoint --no-fail-fast --success-output immediate -E 'test(=store::receipts::tests::receipt_discovery_upgrade_combined_reset_warning_and_cursors_roll_back_together) | test(=agy::activation::modern_direct_ensure_lazy_pair_reaches_actual_turn_hook_without_settlement) | test(=direct_send::json_output_uses_the_versioned_send_envelope_with_the_thread) | test(=store::connection::tests::published_v36_upgrade_preserves_exact_in_turn_offers_and_delivery_history) | test(=store::connection::tests::published_v35_upgrade_preserves_exact_ack_bindings_and_delivery_history)'
```

The selected inventory contains exactly those five tests; every test passed, run exit 0, 2.229 s process wall time. The two corrected tests therefore reach their preserved downstream assertions. The independent direct-send envelope and published v35/v36 upgrade controls also pass. Scoped `scripts/check-no-leaked-processes --run-id 0a84a666-26e4-4cb2-9fd0-696587c31e6c` exited 0. Recorded validation HEAD is `9e521e6eb6c091f40362ea80626ac8f7a758d60f`; `git rev-parse` confirmed that each changed file has exactly the same blob at reviewed HEAD and validation HEAD: AGY `0816ae80026c149faf40714b3b05e34bcf3d7aaa`, receipts `bad5e3f951400714b6b8e2059063fca6d41348f9`.

The report's pending-gates note is superseded by the completed `/private/tmp/ht-final-regression19-gates/execution.json`: `cargo fmt -- --check`, `scripts/check-harness-neutral`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and `nice scripts/check-default-features` all exited 0. The staged tree matches the targeted validation tree `fb3f09bcc0349b05351ff308333388cc0cdd0d63`. Neutral and Clippy logs confirm successful checks without warnings.

Test-change commands, run in the designated source worktree:

```sh
git diff --stat 3d23527675170cacaec470aa13b3baf53688d4ba..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff 3d23527675170cacaec470aa13b3baf53688d4ba..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Both commands exited 0: two files, 19 insertions and five deletions. No deleted/skipped test or unjustified weakened oracle was found. Base ancestry check and review-package generation both succeeded; package contains one commit and both required file hunks.
