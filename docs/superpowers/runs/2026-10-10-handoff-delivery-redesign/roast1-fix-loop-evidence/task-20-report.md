# Task 20 / ht-0lo.12 source implementation

Implemented the exact scoped stale-invitation rule. `discardStale` retires an obsolete invitation by queued-object identity, preserving its independently queued successor with the same invitation ID. Ordinary message/attention cleanup retains its existing ID semantics. Queue coalescing and caps remain outside this task.

## Provenance and changes

- Base: `bdae610212ed26a095ca100bd1dc7aa1bd02fac2`.
- Tests-only commit: `2295162f79661c31cf0f44d400330c0fdfab0523`.
- Production commit/head: `ba0585a9422d32df676fa6767be9e8eaae91513b`.
- Branch: `super-auto/roast1-ht-0lo-12`; no rebase performed.
- Files: `integrations/claude/mod/tests/delivery.test.ts`, `integrations/claude/mod/hooks/register.js`.
- Every commit includes its exact `Outside-adapter` trailer.
- Own-worktree `git diff --check` passed and final tracked/untracked status was clean.

## Regression selectors and intended RED/GREEN

- `late stale invitation denial retains replacement passive command context`: A is in-flight, the watch restarts, and B has the same ID with a changed command-context epoch. After late A denial the queue must contain exactly one replacement; after the retry backoff, B's body must append and its exact version must be recorded without ACK or wake.
- `late stale invitation denial retains replacement timed topic context`: the same sequence exercises the waking append path with a changed topic revision. B must append, record the replacement version, and produce one content-free wake with no invitation ACK.
- `an in-flight invitation re-streamed unchanged after restart survives a late denial`: a control that preserves the exact in-flight object when its unchanged context is re-streamed into the replacement run; it retries once and suppresses later completed duplicates.

The first two intentionally fail unchanged production at the queue-retention assertion: expected `["invitation:replacement"]`, actual `[]`. Their later append/version/wake assertions are unmeasured in RED. The unchanged-re-stream control is intended PASS in both epochs. GREEN requires all three selectors and the existing restart/completion/partial-whole controls to pass.

## Parent-owned runtime evidence

The implementer ran no Cargo, Node/plugin runtime, daemon, socket, Herdr, tracker mutation, or delegated agent. Parent owns runtime evidence under the explicit dispatch override.

Parent baseline validation head: `d70c10b215d649c1556347217a0666eadcfb3cda`, tree `3949feec8141fa6b5e6a2b91873bdc32ac1cfe7f`.

Exact commands supplied by parent:

```text
nice claude plugin validate --strict /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/integrations/claude/mod
nice claude plugin test /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/integrations/claude/mod
scripts/check-no-leaked-processes --run-id 94dd6afc-194f-4b08-9fd2-0181f1caf032
```

Strict validation exited 0 in 0.402s. Plugin test exited 1 in 0.747s with both replacement regressions failing at the intended retention assertion, while the unchanged-re-stream control passed:

```text
122 pass
2 fail
Ran 124 tests across 3 files. [0.61s]
```

Parent baseline logs: `/private/tmp/ht-roast1-mod-baseline-2295162f/execution.json`, `/private/tmp/ht-roast1-mod-baseline-2295162f/mod.log`. I read both; source head/tree stayed fixed and clean.

Parent GREEN validation head: `1f0823344ff6267c904a026fb4cfa3f92e8dd8d6`, tree `eb9bd36333e2eb4cd9a5664234454f632cf76606`. Parent reports both changed mod files exactly match production head `ba0585a9422d32df676fa6767be9e8eaae91513b`; I also verified equal Git blob IDs for each file across those commits.

Exact GREEN commands:

```text
nice claude plugin validate --strict /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/integrations/claude/mod
nice claude plugin test /Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/integrations/claude/mod
scripts/check-no-leaked-processes --run-id e2086795-31f1-434d-9757-ce41bb885718
```

Strict validation exited 0 in 0.551s. Plugin test exited 0 in 0.803s:

```text
(pass) late stale invitation denial retains replacement passive command context
(pass) late stale invitation denial retains replacement timed topic context
(pass) an in-flight invitation re-streamed unchanged after restart survives a late denial
124 pass
0 fail
Ran 124 tests across 3 files. [0.65s]
```

Existing in-flight channel-loss behavior, changed complete-context retention, same-revision partial/whole retention, and the rest of the plugin suite passed. GREEN logs: `/private/tmp/ht-roast1-mod-green-ba0585a9/execution.json`, `/private/tmp/ht-roast1-mod-green-ba0585a9/mod.log`. I read both; source head/tree stayed fixed and clean.

## Test hygiene

No processes were started by the implementer. Parent baseline scoped leak check exited 0 in 0.203s and GREEN scoped leak check exited 0 in 0.243s; no leaked processes reported. No survived processes.

## Assumptions and concerns

Current bead and comments were read through `bd --sandbox --readonly`; no comments or conflicting amendments exist. The current bead and frozen Task 20 brief agree, including exclusion of queue coalescing at register.js:296. Setup base was verified clean. Parent corrected the missing task-brief helper line and supplied the exact brief; a temporary own-worktree extraction was removed. No protected trust-policy paths or model-visible strings changed.

Plugin runtime verification is evidenced above. Required integration gates remain parent-owned and pending; this source-only report makes no claim that unrun checks passed.
