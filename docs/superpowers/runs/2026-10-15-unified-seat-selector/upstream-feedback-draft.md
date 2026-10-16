# 2026-10-15-unified-seat-selector: inline roast fixes leave superseded spec text behind, so the next round re-finds it

Plugin: superpowers 6.4.2-alepar5.3. Run: super-auto, autonomous. 10 task beads, 2 design-roast rounds, 2 code-roast rounds plus a post-cap audit, 3 super-code invocations. About 3h20m on the critical path for the main invocation. 2026-10-15/16.

## Defects
### 1. Inline spec/bead roast fixes have no completeness sweep, so superseded mentions survive and come back as regressions
- **Evidence:**
  - The roast-design-2 header reports "3 regressed". Its entries say:
    - "the fix pass rewrote D3, D6 and the bead but left line 376 … fixed incompletely";
    - "the superseded 'clearing' text was left in the stage's Owns entry and leaf 3";
    - "the fix text itself is now the defect".
  - The round-1 step-back had already written sweep rules, for example: "State the renderer contract once, in D6, and have every other mention … point to it."
  - One converged punch-list item about the order of the session refusal was applied inline without a re-roast. It came back as a Must-fix in the final code review (M1), which used up the run's one post-loop fix round.
  - super-design step 1 carries a cluster `rule:` "for all its instances, cited or not" only for fix *tasks*. The inline fixes of step 2 have no equivalent.
- **Premise to verify:** the superseded wording can be found mechanically (quoted evidence plus the cluster rule name the old phrases), and the regressions came from missed occurrences, not from wrong fixes.
- **Suggested fix shape:** for inline fixes, grep the spec tree and bead descriptions for each quoted superseded phrase, and record zero residual hits (or each kept hit) in the run-state line. Apply the same check to converged punch-list items.

### 2. Under deferSweep, the final review still has to give ready/not-ready, so every review says "not ready: unmeasured"
- **Evidence:**
  - `coordinator.js` `finalReviewPrompt` says: "no full-suite measurement of this branch exists yet — say so in your verdict … Verdict (ready / not ready)".
  - Re-entry 1 returned "Verdict: not ready … the branch has not had a full-suite run yet".
  - Re-entry 2 invented "Ready, conditional on the full-suite sweep".
  - super-auto routed the untested item as a Must-fix step-back key that only phase 6 can clear.
  - super-auto SKILL.md already calls these reviews "informational when it returns".
- **Premise to verify:** under deferSweep the caller always sweeps before the verdict counts.
- **Suggested fix shape:** when the sweep is deferred, grade the code only and list the deferred sweep once under Untested scope, or add a `ready-pending-sweep` verdict. super-auto's final-review routing should drop the deferred-sweep item.

## Run metrics
### Judge panel
- 2026-10-15-unified-seat-selector-roast-design-1.md: mode: design        iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 26 · rr 0.81 · rg 0.77 · fg 0.58 · unanimous 0.58 · ground-loo 0.71 (n=21) · reproduce 9/17/0 · refute 4/22/0 · ground 15/11/0
- 2026-10-15-unified-seat-selector-roast-design-2.md: mode: design        iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 6 · rr 0.50 · rg 1.00 · fg 0.50 · unanimous 0.50 · ground-loo 1.00 (n=3) · reproduce 6/0/0 · refute 3/3/0 · ground 6/0/0
- 2026-10-16-unified-seat-selector-roast-pr-1.md: mode: PR        iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 15 · rr 0.87 · rg 0.80 · fg 0.80 · unanimous 0.73 · ground-loo 0.85 (n=13) · reproduce 2/13/0 · refute 2/13/0 · ground 5/10/0
- 2026-10-16-unified-seat-selector-roast-pr-2.md: mode: PR        iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
- 2026-10-16-unified-seat-selector-roast-pr-post-cap-audit.md: mode: PR        iteration: post-cap audit · independence: same-family (Claude) — seat-differentiated panel · rung: Workflow · 
### Fix loop
- Metrics: completions — review clean 8 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 4 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
- Metrics: completions — review clean 9 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 4 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
- Metrics: completions — review clean 10 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 4 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
### Merge-back
- Metrics: merges 8 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 2 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: merges 9 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 2 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: merges 10 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 2 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Merge: lines 10; first-parent merge commits since fork (incl. 1 main absorption) 11
### Coverage
- requirements: 11 · mapped: 11 · unmapped: 0
- scope-filter: 2 in-scope · 1 punch-listed
### Timing
- Profile: bound — graph lower bound 3h20m along ht-u3ym.1 → ht-u3ym.3.2 → ht-u3ym.3.3 → ht-u3ym.4 → ht-u3ym.6 (unlimited slots and merge lanes) · model of the run 3h20m (cap 8, one merge lane) vs actual 3h20m (100%) → graph-bound: the 5-bead dependency chain set the pace — reshaping it is the lever
- Profile: critical path — 3h20m = implement 3h05m · review 2m · merge 3m · planning 7m · ready 8s · startup 3m · dispatch 30s
- Profile: bottleneck — ht-u3ym.3.2 — 1h51m on the critical path (55%) · implement 1h47m · review 1m · merge 2m · dispatch 7s · implement on the path 1h47m: test 10m · poll 1h11m · build 9m · git 5s · read 46s · shell 2m · model 14m (longest: "python3 - <<'EOF'" 2m, "cargo fmt" 2m) · dependents 2 direct, 4 transitive
- Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 3m over 1 of 8 landed beads — stack-conflict 3m (1)
- Profile: merge lane — busy 33m of 2h51m (19%) · seam reviews and fixes 6m (18% of busy) · 10 merge dispatches, first try not merged for 2 of 8 · queue wait 5m total, peak 1 waiting · stack-parent wait 1s
- Profile: bound — graph lower bound 22m (unlimited slots and merge lanes) · model of the run 22m (cap 8, one merge lane) vs actual 22m (100%) → bead-bound: ht-u3ym.11 alone set the pace — split it or make it faster
- Profile: critical path — 22m = implement 12m · review 49s · merge 39s · planning 5m · ready 11s · startup 3m · dispatch 8s
- Profile: bottleneck — ht-u3ym.11 — 14m on the critical path (61%) · implement 12m · review 49s · merge 39s · dispatch 6s · implement on the path 12m: test 5m · build 3m · read 6s · shell 10s · model 4m (longest: "cargo test --locked --all-features --lib cli::cooperative_t…" 2m, "for f in "--lib cli::cooperative_tests" "--lib cli::command…" 1m) · dependents 0
- Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 0s over 0 of 1 landed beads
- Profile: merge lane — busy 39s of 39s (100%) · seam reviews and fixes 0s (0% of busy) · 1 merge dispatches, first try not merged for 0 of 1 · queue wait 2s total, peak 1 waiting · stack-parent wait 0s
- what-if lines are schedule-model estimates, not measurements (see the bound line's model-vs-actual).
### Bead graph
| id | type | title |
| --- | --- | --- |
| ht-u3ym | epic | Unified seat selector: names for every CLI seat argument |
| ht-u3ym.1 | task | Seat selector resolver module (pure) |
| ht-u3ym.11 | task | Fix: seat stage allocates last (after all slots resolve and after caller/capability refusals) |
| ht-u3ym.12 | task | Fix: session refusal before seat stage; TRUST-POLICY allocation claim matches code |
| ht-u3ym.2 | task | Seat argument slots in the CLI parser (SeatSlot, SeatMapping) |
| ht-u3ym.3 | epic | Runtime seat resolution stage resolve_cli_seats |
| ht-u3ym.3.1 | task | Move-only: extract parent-narrowing and seat-allocation helpers |
| ht-u3ym.3.2 | task | Seat resolution stage core + parser flip |
| ht-u3ym.3.3 | task | Seat resolution: command-path integration |
| ht-u3ym.4 | task | Configuration smoke: seat selector on every seat-taking command |
| ht-u3ym.5 | task | Docs, skill and trust policy for seat selectors |
| ht-u3ym.6 | task | Integration sweep: unified seat selector |

| dependent | blocker | reason |
| --- | --- | --- |
| ht-u3ym.10 | ht-u3ym.4 | unstated |
| ht-u3ym.9 | ht-u3ym.5 | unstated |
| ht-u3ym.3.6 | ht-u3ym.3.3 | unstated |
| ht-u3ym.3.5 | ht-u3ym.3.2 | unstated |
| ht-u3ym.8 | ht-u3ym.1 | unstated |
| ht-u3ym.7 | ht-u3ym.2 | unstated |
| ht-u3ym.3.4 | ht-u3ym.3.1 | unstated |
| ht-u3ym.6 | ht-u3ym.2 | consumes all leaves (integration sweep) |
| ht-u3ym.6 | ht-u3ym.3.2 | consumes all leaves (integration sweep) |
| ht-u3ym.6 | ht-u3ym.4 | consumes all leaves (integration sweep) |
| ht-u3ym.6 | ht-u3ym.5 | consumes all leaves (integration sweep) |
| ht-u3ym.6 | ht-u3ym.1 | consumes all leaves (integration sweep) |
| ht-u3ym.6 | ht-u3ym.3.1 | consumes all leaves (integration sweep) |
| ht-u3ym.6 | ht-u3ym.3.3 | consumes all leaves (integration sweep) |
| ht-u3ym.3.3 | ht-u3ym.3.2 | consumes resolve_cli_seats stage and parser flip |
| ht-u3ym.3.2 | ht-u3ym.3.1 | consumes parent-narrowing and seat-allocation helpers |
| ht-u3ym.3.2 | ht-u3ym.2 | consumes SeatSlot/SeatMapping/ParsedCli.seat_selectors |
| ht-u3ym.3.2 | ht-u3ym.1 | consumes seat_selector resolver API |
| ht-u3ym.5 | ht-u3ym.3.3 | consumes final seat-selector behaviour (resolved: line format, narrowing, nickname, conflict rules) |
| ht-u3ym.4 | ht-u3ym.3.3 | consumes resolve_cli_seats stage with command-path integration |

## Design questions
### Parallelism pass and edge audit treat every bead as one round of equal cost
In this run one oversized bead held 55% of the critical path (1h51m). Profile what-ifs, which are model estimates: split it → −41m (21%), make it faster → −54m (27%). The graph pass applied nothing, and its deepest proposal folded a move-only bead *into* the bottleneck.
- For: an edges-only, uniform-cost model optimised a chain that was not the real critical path.
- Against: bead-size estimates before implementation are unreliable, and a split is a scope change that §Splitting a Bead already owns.

Should the parallelism pass weight the critical path by an estimated size and park split proposals for a heavy bead?

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps
### 3. The ledger path is not named where the session is told to watch the ledger
super-code's "Watch for slowness" text and super-auto's watch sentence never name `.superpowers/sdd/<plan>/progress.md`. That path appears only in coordinator-workflow.md:598 and coordinator.js:85, and the session gets it only in the return value, after the run. This session's watcher matched on "ledger", missed `progress.md` for the first 30 minutes, and recorded no session-side `Slowness:` lines. Fix: name the file in both watch paragraphs, and optionally log the ledger path at launch.

### 4. A NEEDS-SPEC request cannot express "this is undecided"
A coverage reviewer used NEEDS-SPEC for two questions the spec had never decided. Re-sending the spec cannot answer those. The orchestrator turned them into spec amendments, a path the skill does not define. Fix: the reviewer prompt should say that an undecided question is a GAP. The orchestrator should convert any NEEDS-SPEC whose reason names undecided questions into GAP findings.

## Already fixed — do not re-litigate
none

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- Every observation here comes from a single run. The what-if savings are schedule-model estimates.
- The finding 3 symptom (the watcher missing the file) was reported by the session and is not in friction.md.

## Verification bar
- Finding 1: replay a design roast fix round on a spec with duplicated decision text, and check that round 2 reports zero regressed findings.
- Finding 2: run a dryRun or replay of the coordinator with deferSweep and assert that the final-review verdict is not "not ready" solely for the deferred sweep.
- Findings 3 and 4: text checks on the skill files.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
