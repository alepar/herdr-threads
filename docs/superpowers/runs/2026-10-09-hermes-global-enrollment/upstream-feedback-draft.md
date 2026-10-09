# 2026-10-09-hermes-global-enrollment: super-auto expects super-code's Finish to write the run profile, but super-code assigns that step to the invoking session, so no profile was recorded

Plugin: 6.4.2-alepar4.20. Run: super-auto, autonomous, Workflow mechanism; 12 beads under the epic (5 planned tasks, 4 review beads, 3 fix beads), depth 4, three super-code invocations, 8 task merges, two design-roast and two code-roast rounds; about 3 h by file mtimes, 2026-10-09.

**Status:** parked draft. Not filed, not scrubbed. The proposal goes to the user at the finish menu.

## Defects

### 1. super-auto and super-code each say the other one writes the run profile, so neither ran `run-profile` and per-bead timing was lost
- **Evidence:**
  - super-auto `SKILL.md:315` says to copy "that invocation's run profile (`profile-<runId>.md` and `.json`, **which `super-code`'s Finish writes beside its ledger**) into `<run-dir>/`".
  - super-code `coordinator-workflow.md` §Finish (around line 1207) says the opposite: "**The run profile is the invoking session's job too.** The script has no clock … run `bash <this skill's dir>/scripts/run-profile --workflow <dir> … --out <workspace>/profile-<runId>`". super-code `SKILL.md:292` says the same: "When a Workflow run returns with a `stopReason`, record its run profile … on the ledger before any upstream-feedback analysis".
  - When super-auto runs phase 3, super-auto's session *is* the invoking session. Its own skill text points the step back at super-code, so the step has no owner.
  - Friction log: "super-code Finish wrote no `profile-<runId>.md`/`.json` beside the ledger (`.superpowers/sdd/ht-4ne-plan/`), so super-auto's 'copy the run profile into the run dir' step had nothing to copy; per-bead timing for phase 3 is lost with the worktree."
  - Ledger: three `Launch:` blocks (three coordinator invocations, each ending in `Process sweep:`) and zero `Profile:` lines, not even `Profile: unavailable — …`. The Workflow journals for all three runs still exist (`wf_dd8c08d3-372`, `wf_e7f5a2b7-be9`, `wf_15d04b9c-113`), so the profile could have been computed.
- **Premise to verify:** that the intended owner is the invoking session (super-auto's orchestrator) and not a step inside `coordinator.js`'s Finish. The super-code text says the script "has no clock", which supports the invoking session.
- **Suggested fix shape:** in super-auto `SKILL.md:315` and the phase-3 / fix-loop re-entry return handling, say that super-auto runs `scripts/run-profile` itself after each coordinator return (one `--workflow` per launch, appending stdout to the ledger), and then copies the output files. Drop the phrase "which super-code's Finish writes".

### 2. upstream-feedback's `ledger incomplete` check counts super-auto's base-absorption merge as a missing coordinator merge
- **Evidence:** `git log --merges --first-parent main..super-auto/hermes-global-enrollment` lists 9 merge commits, while the ledger has 8 success-path `Merge:` lines. The extra commit is `2160dd08 Merge main into super-auto/hermes-global-enrollment (absorb base drift before phase-6 sweep)`, which super-auto's phase 6 made (run.md: `baseAbsorbed: 3264587b → 2160dd08, 0 conflicted files`). Applied literally, upstream-feedback §Gather would report `ledger incomplete: 9 merge commits vs 8 Merge: lines` and hand it to the analyst as a missing-source finding. That is a false positive.
- **Premise to verify:** that every super-auto run that absorbs base drift creates a first-parent merge commit on the run branch. super-auto §Gates (base drift) suggests it does.
- **Suggested fix shape:** in upstream-feedback §Gather, leave out of the count any first-parent merges whose second parent is reachable from `<base>` (or the merge named by run.md's `baseAbsorbed:` line), or compare only against merges of `task-*` branches.

## Run metrics

### Judge panel
- roast-design-1: mode design, iteration 1 of 3. `independence: same-family (Claude) — seat-differentiated panel · rung: Workflow`. `seat-agreement: panels 17 · rr 0.94 · rg 0.82 · fg 0.76 · unanimous 0.76 · ground-loo 0.81 (n=16) · reproduce 4/13/0 · refute 3/14/0 · ground 7/10/0`
- roast-design-2: mode design, iteration 2 of 3, same independence line. `seat-agreement: panels 4 · rr 0.75 · rg 0.75 · fg 0.50 · unanimous 0.50 · ground-loo 0.67 (n=3) · reproduce 3/1/0 · refute 2/2/0 · ground 4/0/0`
- roast-pr-1: mode pr, iteration 1 of 3, same independence line. `seat-agreement: panels 15 · rr 0.93 · rg 0.80 · fg 0.87 · unanimous 0.80 · ground-loo 0.86 (n=14) · reproduce 5/10/0 · refute 4/11/0 · ground 6/9/0`
- roast-pr-2: mode pr, iteration 2 of 3, same independence line. `seat-agreement: panels 1 · rr 1.00 · rg 0.00 · fg 0.00 · unanimous 0.00 · ground-loo 0.00 (n=1) · reproduce 0/1/0 · refute 0/1/0 · ground 1/0/0`
- Verdicts: design `Should-fix (4 confirmed)` → `Should-fix (3 confirmed) [converged]`; pr `Should-fix (4 confirmed)` → `clean (0 nits) [converged]`.

### Fix loop
The final invocation (counts are cumulative across invocations):
- `Metrics: completions — review clean 8 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 4 · cancelled 0`
- `Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0`

### Merge-back
- `Metrics: merges 8 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 0 (fixed 0) · check-fails 0 (fixed 0)`
- `Metrics: ledger-check ok · append-failed 0 · append-retried 0` in all three invocations.
- 8 `Merge:` lines, all `rebase clean · seam-review none · check pass`. None had a conflict, a seam review, a check failure or `→ blocker`.
- First-parent merge commits: 9 against 8 `Merge:` lines. The difference is the base-absorption merge `2160dd08`, not a lost ledger line (Defect 2).

### Coverage
- coverage round 1: `requirements: 10 · mapped: 10 · unmapped: 2 (R6, R9)`
- coverage round 2: `requirements: 12 · mapped: 12 · unmapped: 0`, with `divergence: findings 13 → 15, novel 13/15 (87%), widening: yes (cap holds at two rounds …)`
- code fix round 1: `scope-filter: 2 in-scope · 2 punch-listed`, plus cluster dropped `hermes-hook-cap-isolation`

### Timing
none. No `Profile:` lines were recorded for any of the three coordinator invocations (Defect 1). The other evidence available:
- Detector lines (each invocation counts from round 1):
  - `Detector: round 1 — 1 ready · topped-up 4 · cap 8 · peak in-flight 3 · top-up queries 0/40 · stacked 4 · merge queue peak 1 · idle slots 5 · runtime slots 10`
  - `… 2 ready · topped-up 0 · cap 8 · peak in-flight 2 …`
  - `… 1 ready · topped-up 0 · cap 8 · peak in-flight 1 …`
- `Edge audit: round 1 — open leaves 4, depth 3, achievable width 2 vs cap 8; changes: none`
- run.md `slowness: round 1: graph-bound — 5 open beads, depth 4, achievable width 2 vs cap 8`
- The peak-vs-cap gap is explained by graph shape, not by the scheduler.

### Bead graph
| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-4ne | epic | Hermes: global enrollment (enroll bare Hermes sessions via hooks, not launch-only) | Make bare Hermes CLI sessions in Herdr panes enroll through herdr-threads' own Hermes hooks |
| ht-4ne.1 | task | Seam contract: Hermes enrollment envelope, adapter enrollment policy and bridge refresh | Adds the behavior-preserving seam both sides consume |
| ht-4ne.4 | task | Hermes CLI enrollment behavior and TRUST-POLICY | Makes a top-level Hermes CLI on_session_start enroll the pane through the ordinary guarded resolver without a check-in |
| ht-4ne.2 | task | User-facing docs: Hermes enrolls through its CLI hooks | Rewrites user-facing Hermes text from launch-only to global enrollment, with its limits |
| ht-4ne.3 | task | Configuration smoke: bare Hermes bridge enrollment in an isolated Herdr session | Runs the real Hermes bridge module with synthetic callbacks against an isolated named Herdr session and daemon, once per configuration |
| ht-4ne.5 | task | Integration sweep: Hermes global enrollment | Verifies the goal's main flow end to end after every leaf lands |
| ht-4ne.6 | task | review: ht-4ne.1 | Review, fix and merge of ht-4ne.1, tracked separately so its dependents can start once it is implemented. |
| ht-4ne.7 | task | review: ht-4ne.4 | (same, for ht-4ne.4) |
| ht-4ne.8 | task | review: ht-4ne.2 | (same, for ht-4ne.2) |
| ht-4ne.9 | task | review: ht-4ne.3 | (same, for ht-4ne.3) |
| ht-4ne.10 | task | Fix: stale launch-only text after Hermes hook enrollment | Covers: [Nit] docs/compatibility/README.md:40, [Must-fix] src/harness/adapter.rs:326, [Should-fix] run.md:27 |
| ht-4ne.11 | task | Fix: cover untested Hermes enrollment scope (retire re-enroll, stale-hint absence, visible smoke skip) | Covers three final-review `[Must-fix] untested:` items |
| ht-4ne.12 | task | Sweep fix: own the spawn sites in hermes_enrollment_smoke.rs | Covers: integration no_leaks::every_test_spawn_site_is_owned |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-4ne.4 | ht-4ne.1 | consumes Hermes enrollment envelope fields and enrollment policy seam |
| ht-4ne.2 | ht-4ne.4 | consumes Hermes enrollment behavior (wording must match shipped behavior) |
| ht-4ne.3 | ht-4ne.4 | consumes Hermes enrollment behavior (SessionStart and UnownedTurn handling) |
| ht-4ne.5 | ht-4ne.1 | consumes all leaves (integration sweep) |
| ht-4ne.5 | ht-4ne.2 | consumes all leaves (integration sweep) |
| ht-4ne.5 | ht-4ne.3 | consumes all leaves (integration sweep) |
| ht-4ne.5 | ht-4ne.4 | consumes all leaves (integration sweep) |
| ht-4ne.6 | ht-4ne.1 | unstated |
| ht-4ne.7 | ht-4ne.4 | unstated |
| ht-4ne.8 | ht-4ne.2 | unstated |
| ht-4ne.9 | ht-4ne.3 | unstated |

The review beads ht-4ne.6–.9 are created by the coordinator's early-unblock mechanism, so `unstated` is expected for them.

## Design questions

### A. Should super-code's final whole-epic review get the run's roast reports (their `## Rejected` sections) and the scope-filter punch list when a caller has them?
- **Evidence:**
  - `coordinator.js:1866` `finalReviewPrompt(epicId, integrationBranch, integrationWorktree, ledgerPath, planPath, sweepSummary, metrics)` takes no roast or punch-list input.
  - Across three invocations, the final reviews kept raising items a roast panel or the scope filter had already settled:
    - Review 1 graded the resume-wording item must-fix. roast-pr-1 had rejected it 3/3 as a misread.
    - Review 1 also graded the concurrent-test retry must-fix. roast-pr-1 had spot-rejected it.
    - Reviews 2 and 3 raised the cross-harness retire. roast-pr-1 had rejected it 3/3; roast-pr-2 rejected a related one-shot point 2/3.
    - Review 3 raised the nextest serializing override, which scope-filter round 1 had punch-listed.
  - The orchestrator re-routed these by hand under super-auto §Final-review items (run.md: "final-review retry item and TRUST-POLICY resume item stay rejected (roast r1 panels)").
  - The last review's verdict stayed `not ready (…)` for reasons that were mostly roast-rejected or punch-listed. The step-4 rewrite to `ready` applies only when the sole reason is the unmeasured sweep, so report.md ends `status: clean [degraded: final review: not ready …]`.
  - Friction log: "each re-entry's review re-raised items a roast panel had already rejected … Passing the run's roast-report paths to the final reviewer would cut that churn."
- **For passing them:**
  - super-auto's own rule is that "an item a roast panel already rejected stays rejected, unless the final review cites evidence that panel's report did not have".
  - The reviewer is the only party that can say whether it has new evidence. Without the reports, the orchestrator has to judge "new evidence" after the fact, by hand, on every re-entry.
  - The headline verdict ends up carrying settled items.
- **Against:**
  - The final review is valuable because it is independent. Here review 2 did add something: it argued the roast's round-1 rejection rested on a same-harness premise that the cross-harness retire removed, and that the roast's own suggested doc note was never written. That argument is what put the decision in front of the human in report.md.
  - Priming the reviewer with rejections could suppress that kind of catch.
- **Middle option:** pass the reports, and require any re-raised item to be tagged `previously rejected (<report>): new evidence: <…>`, so §Final-review items can be applied mechanically.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### B. Should a pre-flight-resolved `mergeCheck` prefer the project's declared per-change lint when it strictly includes the build check?
- **Evidence:**
  - `Launch:` `"mergeCheck":"nice cargo check --locked --all-targets && nice cargo check --locked --all-targets --all-features"`.
  - The project's agent guide declares the per-change gate as `cargo clippy --locked --all-targets --all-features -- -D warnings` ("clippy already type-checks every target; a separate `cargo check` is redundant") plus a default-feature check script.
  - Final review 2: "The run's merge check only ran `cargo check`, so this is the first clippy run on the merged branch." Final review 1 listed "No lint run recorded" under untested scope.
  - Clippy came back clean when the reviewers ran it, so this run lost nothing.
- **For:** at about the same cost, a lint gate catches warning-level breaks at the merge that introduced them. Today nothing in the pipeline runs the lint unless a reviewer chooses to. The sweep runs tests, not lint.
- **Against:** the skill deliberately keeps `mergeCheck` "BUILD-ONLY". A `-D warnings` lint fails merges on style drift, which feeds the merge-check-fix and blocker path with non-seam failures. The caller can already declare `mergeCheck` explicitly.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
none beyond Defect 2 (the upstream-feedback gather rule).

## Already fixed — do not re-litigate
none known.

## Not established
- Analyst: steps 2–3 (Analyze, Triage) ran in one fresh-context subagent on Opus 5.5 (the strongest model available), acting as both analyst and triager, as instructed by the invoking session. They were not split into a separate dispatched analyst plus an invoking-session triage.
- Per-bead timing, critical path, wait causes and what-ifs: not established. No `Profile:` lines exist (Defect 1).
- Design question A does not claim the extra final-review churn cost a measurable amount of wall time. Profile data is missing, and the churn was mainly orchestrator re-routing and a degraded report status, not extra dispatches.
- Design question B is a single-project observation with no defect caught.
- Considered and dropped, recorded so they are not re-reported:
  - The default `testPaths` miss Rust inline `#[cfg(test)]` tests. The skill's guidance already covers this (declare `testPaths` or note it in friction), and the run did note it. The vacuity probe only fires when zero files match, so mixed layouts (some `tests/**`, some inline) stay partly blind without a warning. This run showed no consequence.
  - A sweep failure found only at the phase-6 sweep (a project test guard on spawn sites). This is the intended `deferSweep` trade-off.
  - The real-binary probe abort (project-specific).
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.

## Verification bar
- Defect 1: run super-auto through phase 3 on a small epic under the Workflow mechanism. Confirm the ledger gains `Profile:` lines (or `Profile: unavailable — …`) after each coordinator return, re-entries included, and that `profile-<runId>.md`/`.json` land in `<run-dir>/` in the same commit as that phase's `run.md` write.
- Defect 2: on a branch with a base-absorption merge and N task merges, upstream-feedback's gather should report no `ledger incomplete` line.
- Design question A, if adopted: replay a fix-loop re-entry whose final review re-raises a roast-rejected item. The item should come back tagged with its prior rejection, and §Final-review items should route it without hand judgement.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
