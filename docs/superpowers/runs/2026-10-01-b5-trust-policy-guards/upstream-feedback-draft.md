# 2026-10-01-b5-trust-policy-guards: planner-noted missing edges are not applied, so a dependent task blocks and is quarantined

Plugin: 6.4.2-alepar4.3 at launch → 4.5 (104b54e) → 4.6 (2bf1d53), switched at round boundaries. Run: 1 epic, 16 merged task beads (9 design + 6 code-roast fixes + 1 sweep fix), 2 design-roast and 2 code-roast rounds, about 14 h wall clock, 2026-10-01.

## Defects

### 1. A missing dependency edge the planner notices is only written into plan prose; the bead graph and dispatch ignore it
- **Evidence:** ledger: planner notes "Missing dependency edge (task 5 -> task 2)… the bead graph does not record this dependency… Prefer dispatching task 5 after task 2 merges"; the coordinator still dispatched the task stacked on a different parent. It blocked twice, was quarantined (`Slowness: … quarantined on a planner-noted missing edge → edge added by session`), cost two blocker beads, one cancelled stacked child and a full coordinator relaunch.
- **Premise to verify:** the planner can emit a missing edge as structured data; today PLANNED's `mapping[].deps` comes only from `scripts/tree-deps` (the bd graph).
- **Suggested fix shape:** add `missingEdges: [{dependent, blocker, reason}]` to PLANNED; apply each with `bd dep add` (verify-then-apply, as for edge cuts) or hold the dependent until the blocker merges; log a ledger line either way.

### 2. A triage RESOLVE that says "wait until X merges" is re-dispatched immediately, so the one-retry bound turns a correct triage into a quarantine
- **Evidence:** first RESOLVE: "COORDINATOR ACTION: add the missing graph edge … Re-dispatch … only after [blocker] has merged … Do not re-dispatch it now, because it would block again." Next ledger line: "BLOCKED — Second RESOLVE … without resolving — escalating per the one-retry bound … because the coordinator re-dispatched before [blocker] merged." `resolveRetryHook` calls `runTask(id)` at once; TRIAGE is only `{decision, detail, cause}`.
- **Premise to verify:** triage can tell "needs a sibling to merge first" from "needs a clarification"; pendingRetry / round-head hold can be reused.
- **Suggested fix shape:** optional `waitFor: <bead>` / `addEdge` on TRIAGE; park the id until that bead merges, without spending the one-RESOLVE budget.

### 3. review-package uses the base captured at workspace setup; after a mid-task rebase it is not an ancestor of HEAD, the script exits 3, and no review runs
- **Evidence:** ledger: "ran review-package with the task's original pre-rebase BASE, 644ed4406c99 ('stack: …'). That commit is not an ancestor of the rebased HEAD, so the script exited 3 and no review took place … with BASE=f6ba883d … wrote a valid package (exit 0)". coordinator.js carries `base` from the workspace step only.
- **Premise to verify:** `git merge-base <integration> HEAD` at review time is the right lower bound, while still excluding stack parents' commits (the `stack:` first-parent rule).
- **Suggested fix shape:** check `git merge-base --is-ancestor $base HEAD`; on failure recompute (latest `stack:` merge, else merge-base with the integration branch) and log the substitution rather than taking the blocker path.

### 4. Processes leaked by task implementers' test runs are never noticed or cleaned, and they contaminate the deferred sweep
- **Evidence:** 244 orphaned (ppid 1) test daemons from task-worktree binaries plus 4 private test host servers accumulated over the run; the final sweep failed once under that load (`FAIL … once under 249 leaked test daemons; 10/10 clean re-runs on quiet machine`). The project fixture leak is the root cause (filed downstream); the upstream gap is that nothing in super-code detected it.
- **Premise to verify:** leaked processes can be attributed to a task worktree (argv path / cwd / process tree) without touching the user's own processes.
- **Suggested fix shape:** at task worktree teardown and before the sweep, count processes whose argv or cwd lies under a task worktree; log `leaked processes N from <task>` and either stop them or mark the sweep load-contaminated.

### 5. super-auto has no rule against running a full-suite pass while a large roast/coordinator Workflow is running on the same host
- **Evidence:** friction: "full-suite preview run concurrently with a 154-agent roast workflow: 13 extra failures (harness `--version could not be observed`) vanished when re-run in isolation".
- **Premise to verify:** super-auto controls when roasts and suite runs start; timing-sensitive suites are common enough for a sequencing rule.
- **Suggested fix shape:** never run the sweep (or any full-suite command) while a roast or coordinator Workflow is in flight; treat failures seen under concurrency as unconfirmed until reproduced alone.

## Run metrics
### Judge panel
- design iteration 1 · same-family (Claude) — seat-differentiated panel · panels 29 · rr 0.83 · rg 0.76 · fg 0.66 · unanimous 0.62 · ground-loo 0.75 (n=24) · reproduce 8/20/1 · refute 3/25/1 · ground 13/15/1
- design iteration 2 · same-family (Claude) · panels 5 · rr 0.40 · rg 0.60 · fg 0.00 · unanimous 0.00 · ground-loo 0.00 (n=2) · reproduce 3/2/0 · refute 0/5/0 · ground 5/0/0
- PR iteration 1 · same-family (Claude) · panels 30 · rr 0.87 · rg 0.77 · fg 0.63 · unanimous 0.63 · ground-loo 0.73 (n=26) · reproduce 13/17/0 · refute 9/21/0 · ground 20/10/0
- PR iteration 2 · same-family (Claude) · panels 3 · rr 0.67 · rg 1.00 · fg 0.67 · unanimous 0.67 · ground-loo 1.00 (n=2) · reproduce 3/0/0 · refute 2/1/0 · ground 3/0/0
### Fix loop
- `Metrics: completions — review clean 15 · after fix pass 1 · parked 0 · re-entry closes 0 · dispatched early 6 · cancelled 1`
- `Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0`
### Merge-back
- `Metrics: merges 16 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 8 (fixed 2) · check-fails 0 (fixed 0)`
- `Metrics: ledger-check ok · append-failed 0 · append-retried 0`
- 16 `Merge:` lines: 0 conflict, 8 seam-review fired, 0 check fail.
### Coverage
- coverage round 1: requirements 16 · mapped 16 · unmapped 0; round 2: requirements 22 · mapped 22 · unmapped 0 (divergence: 14 → 14, novel 100%, widening yes)
- code fix round 1: `scope-filter: 12 in-scope · 1 punch-listed`
### Bead graph
none (omitted: project-specific titles; available on request)

## Design questions
- **Hung control-plane calls.** 4.6 degrades a *null* planner, but a planner call that never returns still stalls the run: "opus planner dispatch produced no response for 15 min, six times in a row … no task started in 91 min" (the session stopped it by hand). For: 91 idle minutes. Against: a deadline may kill a slow but valid planner pass on large epics, and it may belong in the Workflow runtime. If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.
- **Escalation topics vs. a no-pause session.** The user reserved one decision ("ask only on contention about …"); a session Stop hook then forbade pausing, so the recommended option was applied and parked. Should super-auto, when the user names escalation topics, check at launch whether pausing is possible and otherwise ask those questions up front or get consent to auto-apply? Against: hooks may be undetectable; recording the assumption may suffice. If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
- super-design §Adversarial Review Loop step 1 requires one fix bead per confirmed design finding even when the fix only amends spec/bead text; this run applied such fixes inline and recorded them in run.md. An explicit exemption for design-text-only fixes (no code) would match practice.

## Already fixed — do not re-litigate
- `close-in-tree-epics` exit 5 on bd 1.3's object-shaped `epic close-eligible` output — fixed in 2bf1d53.
- A null planner abandoning the round when ready ids are already mapped — fixed in 2bf1d53.

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count above is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- Single-run observations; no timing comparison across skill versions.
- "Reviewers never execute tests" was flagged by every final review; it is consistent with task-reviewer-prompt's "Don't run tests" and is not reported as a defect here.
- Bead-graph tables omitted (project-specific).

## Verification bar
- Defect 1/2: a dryRun scenario where the planner emits a missing edge and triage returns RESOLVE with waitFor; assert no second dispatch before the blocker merges.
- Defect 3: a task whose implementer rebases mid-task; assert review-package runs with a recomputed base.
- Defect 4/5: a live run on a project whose tests spawn background processes; assert the leak count is logged and the sweep does not overlap a Workflow.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
