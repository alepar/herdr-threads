# ht-0lo.12 / Task 20 review

Status: CLEAN
Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`
Reviewed head: `ba0585a9422d32df676fa6767be9e8eaae91513b`

## Spec compliance

✅ Current `bd show ht-0lo.12 --json`, `bd comments ht-0lo.12` (no comments), frozen Task 20 plan and brief agree. No conflicting amendments. Their explicit exclusion of register.js:296 queue coalescing/global caps is applied; that follow-up is outside this review.

✅ `discardStale` removes obsolete invitation objects by identity while retaining ordinary-item ID cleanup. The independently queued successor survives late denial; unchanged re-streaming refreshes the exact in-flight object's run, so it remains retryable. Both listed files have changes, and no foreign paths or model-visible strings changed.

✅ Added regressions exercise actual late stale denial followed by replacement append, exact version recording, passive and waking paths, no invitation ACK, and unchanged re-stream retry/completed suppression. Existing controls were not altered.

## Issues

Critical: none.
Important: none.
Minor: none.
Unverified spec items: none within this source-review scope. Required integration gates remain the parent's separately assigned pre-integration work.

## Focused reachability check

Concrete risk checked: a shared cleanup helper could fix only one delivery path or invalidate the exact in-flight object refreshed by a replacement run. One focused read/search traced the real stream adapter's `cb.line(JSON.parse(line))` (register.js:939), `startChild` callback (190), and `onLine` queueing/deduplication (221, 292–320) to both `pump` and `pumpLazy`. Both late-denial paths call the same `discardStale` (498, 572). The duplicate path updates `dup.run` without replacing the object. Searching `discardStale` found these two callers and no sibling late-denial cleanup still using the superseded ID-only invitation removal.

## Test evidence

The report contains actual parent-owned baseline RED and GREEN evidence, with verified source-blob equivalence between parent validation commits and this task's test/production commits. The dispatch explicitly assigns runtime testing to the parent; this reviewer ran no tests, Cargo or Herdr.

Recorded baseline commands:

```text
nice claude plugin validate --strict /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/integrations/claude/mod
nice claude plugin test /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/integrations/claude/mod
scripts/check-no-leaked-processes --run-id 94dd6afc-194f-4b08-9fd2-0181f1caf032
```

Baseline strict validation passed; plugin run: 122 pass, 2 fail, 124 tests across 3 files. Both new replacement regressions fail at the intended queue-retention assertion; unchanged re-stream control passes. Scoped leak check passes.

Recorded GREEN commands use the same validation/test paths and `scripts/check-no-leaked-processes --run-id e2086795-31f1-434d-9757-ce41bb885718`. Strict validation and plugin run pass: 124 pass, 0 fail across 3 files, including the three new selectors and existing restart/completion/partial-whole controls. Scoped leak check passes. This is plausible and directly relevant coverage; no failure is glossed over.

Test-change commands run successfully in the designated worktree:

```text
git diff --stat bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff bdae610212ed26a095ca100bd1dc7aa1bd02fac2..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Result: 60 added lines in delivery.test.ts; no deleted, skipped, loosened or rewritten existing assertions. Review-package generation succeeded with two commits and two changed files; base ancestry check passed.

## Strengths

The change is confined to the shared cleanup rule, and the regressions distinguish successor-context preservation from unchanged-context retry behavior.
