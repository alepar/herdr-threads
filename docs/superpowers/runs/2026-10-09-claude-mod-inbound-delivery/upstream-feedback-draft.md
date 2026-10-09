# 2026-10-09-claude-mod-inbound-delivery: run profile owned by no one in Workflow mode; post-loop fix chains run unreviewed

Status: sent 2026-10-09 to the superpowers workspace agent tab (wB:p8) per the standing preference; not scrubbed.

Plugin: superpowers 6.4.2-alepar4.20. Run: 34 beads (the epic, 26 work and fix leaves, 6 review beads and 1 gate). There were 8 super-code invocations. The design roast took 2 rounds; the code roast took 2 rounds plus a post-cap audit. Autonomous, about 9h15m.

## Defects

### 1. In Workflow mode, no one runs `run-profile`
- **Evidence:**
  - The ledger has 0 `Profile:` lines across every invocation, and no `Profile: unavailable` line either.
  - super-auto SKILL.md §Phase 6 says the profile is the one "which `super-code`'s Finish writes beside its ledger".
  - super-code coordinator-workflow.md says the opposite: "The run profile is the invoking session's job too. The script has no clock".
  - run.md: `codeMechanism: Workflow`.
- **Fix shape:** add an explicit `run-profile --workflow <dir>` step to super-auto after each super-code return, and correct the SKILL.md clause.

### 2. A super-roast PR round detached the caller's integration worktree
- **Evidence:**
  - Ledger: `Merge: ht-j16.19 — … → held: integration worktree … not on <branch> (detached at cb56e49f)`.
  - code-final-review-2 records the same detached state.
  - Friction log #3.
  - Premise: the friction log, not a transcript, says a roast agent ran the checkout.
- **Fix shape:** PR mode reviews in its own detached worktree. The caller asserts that HEAD is attached after each roast.

### 3. Free-form `[class]` tags defeat recurrence clustering
- **Evidence:**
  - 63 deferred minors carry 63 distinct tags, and 0 `Recurring` lines were produced.
  - The RED-evidence class appears under about 10 tags, across 9 tasks.
  - Final reviews 3–8 each flagged it again by hand.
  - coordinator.js keys clusters on the exact `classTag`.
- **Fix shape:** a seeded tag vocabulary in task-reviewer-prompt.md, or a merge of near-synonym tags at Finish.

### 4. A gated evidence leaf that ran only its fallback counts as "review clean"
- **Evidence:**
  - Ledger: `Human action: ht-j16.9 — No signed-in Claude profile was usable`, then `Task 9 (ht-j16.9): complete … review clean`.
  - The real live run then happened outside super-code and found 3 product defects (D1 0/3, D2 1/3, D3 0/3).
- **Fix shape:** a `fallback` completion outcome that super-auto treats as unmet final-SHA evidence.

### 5. `assemble-args --topic` doubles the date when given a dated run slug
- **Evidence:**
  - The produced path was `2026-10-09-2026-10-09-…-roast-pr-post-cap-audit.md`.
  - assemble-args.mjs builds `${date}-${topic}-roast-…`.
- **Fix shape:** strip a leading date equal to `--date` from `--topic`, or document that `--topic` takes the bare slug.

## Design questions

### 1. After the loop exits, should final-review must-fix items go to the punch list or get a bounded fix?
- **Evidence:**
  - SKILL.md §Final-review items sends them to the punch list.
  - The run deviated 4 times (`postLoopFix:` lines in run.md): an invariant break; live D1–D3; regressions from the D2 fix (submit inside an open turn; ack without submit, a data-loss path); and a double submit confirmed by a repro.
  - Two post-loop fixes passed task review as "review clean" and were found defective by the next final review.
  - About 4h45m of the run came after the loop converged.
- **For the punch list:** it bounds cost and guarantees the loop ends.
- **For fixing:** followed literally, the rule ships known loss and duplicate-delivery defects.
- **Middle option:** fix only loss or invariant-break items, cap post-loop re-entries, and require a regression-lane check on fix-of-fix chains.
- If upstream decides otherwise, please state the position explicitly.

### 2. What should happen when gated final-SHA evidence finds product defects?
- **Evidence:** §Final-SHA evidence says only "run the gated leaves, then go on to phase 6". Live stress found D1–D3. The fixes plus a live re-run went 13/14 scenarios at 3/3, with the remaining scenario at 2/3.
- **Options:** one bounded fix-and-rerun pass, mirroring the sweep-fix pass, or report-only.

### 3. Under `deferSweep`, can a final review ever say "ready"? (low)
- All 8 final reviews read "not ready". code-final-review-6 found "no new code defect that blocks landing". Splitting the code verdict from pending caller gates would keep the signal.

## Doc gaps
- upstream-feedback's `ledger incomplete` check counts base-drift and orchestrator merges. Here there were 26 `task-*` merges against 26 success `Merge:` lines, so the ledger was complete, yet the extra first-parent merges made it look short.
- super-code's `testPaths` is path-glob only, so inline `#[cfg(test)]` Rust tests are invisible to the Test-changes check (friction #2).
- super-design's graph pass has no rule for a graph with a single candidate edge (friction #1).

## Run metrics (abridged)
- **Roasts:**
  - design 1: Should-fix 11.
  - design 2: Should-fix 8 [converged].
  - pr 1: Should-fix 8.
  - pr 2: Should-fix 2 [converged].
  - post-cap audit: clean (1 nit) [converged].
  - All were same-family seat-differentiated panels (rung: Workflow).
- **Metrics (super-code's ledger):**
  - merges 26, 0 failed, 0 rebase conflicts, 7 seam reviews, 0 check fails;
  - completions: 24 review clean, 2 after a fix pass;
  - fix passes: 3 entered, 3 FIXED.
- **Timing:** none, because no profile was written (Defect 1). Detector: peak in-flight 6 of 8; the edge audit found the graph, not the cap, binding.

## Not established
- Per-bead timing: no profile exists.
- Who ran the checkout in Defect 2: stated only by the friction log.
- The post-cap audit's sensitivity: it raised 2 raw findings over 31 post-loop commits, a single observation.
- Whether a stricter post-loop rule would have saved wall time: unmeasured.
