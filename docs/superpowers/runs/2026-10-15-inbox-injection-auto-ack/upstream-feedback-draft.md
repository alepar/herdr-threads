# inbox-injection-auto-ack: cleanup confirmation and late-task profiling defects

Plugin: 6.4.2-alepar5.4. Target: alepar/superpowers. Run: 7 implementation/fix leaves, 7 feature merges, 2 design roasts + 2 PR roasts + 1 scoped audit; 2026-10-15–16. Ordinary-subagent coordinator snapshots cover a cumulative 2h25m window.

Local draft, parked for user review: no issue has been filed. Filing and optional scrubbing require review of the final body.

## Defects

### 1. Cleanup reported stopped despite denied inspection/termination
- **Evidence:** friction: `stop-run-processes sandbox process inspection denied ps/kill yet emitted stopped for active compiler PIDs; initial cargo actually finished after12m25s.` The run required approved host-visible inspection for authoritative hygiene.
- **Premise to verify:** confirm this is the upstream-owned helper and inspect how process-list failure, kill failure and post-termination verification affect its output and exit code. No helper source root-cause inspection was done for this draft.
- **Suggested fix shape:** emit stopped only after verified termination; return a distinct blocked/unverified result when permissions prevent confirmation.

### 2. Late-created tasks acquire pre-creation retry wait and become false bottlenecks
- **Evidence:** tasks .9/.10 were created at08:42, but the final cumulative profile starts06:41. It reports `.10 — 2h23m on the critical path (100%)`, with `wait 2h05m`, while its printed implementation/review/merge chain is17m. Coordinator explicitly recorded that the2h05m predates task creation and is not scheduling idle.
- **Premise to verify:** check whether missing task availability defaults to the first invocation timestamp and whether this flows into wait, critical-path and what-if calculations.
- **Suggested fix shape:** bound readiness by evidenced task creation/discovery and dependency satisfaction; mark missing timestamps unknown and recompute dependent metrics.

## Run metrics

### Judge panel

- 2026-10-15-inbox-injection-auto-ack-roast-design-1.md
  mode: design        iteration: 1 of 3
  coverage: scouts 8/8 (premortem, completeness, yagni, failure-mode, feasibility, domain:distributed-systems, domain:message-delivery-and-acknowledgment, domain:agent-hook-and-channel-integration) · raw 2 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
  independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
  seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0

- 2026-10-15-inbox-injection-auto-ack-roast-design-2.md
  mode: design        iteration: 2 of 3
  delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 1 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
  coverage: scouts 9/9 (premortem, completeness, yagni, failure-mode, feasibility, regression, domain:distributed-systems, domain:messaging-delivery, domain:auth-provenance) · raw 0 → deduped 0 → panel 0 · spot 0 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0
  independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
  seat-agreement: n/a — no candidates or panels this round.

- 2026-10-15-inbox-injection-auto-ack-roast-pr-1.md
  mode: pr        iteration: 1 of 3
  coverage: scouts 11/11 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 3 → deduped 3 → panel 2 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
  independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
  seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0

- 2026-10-15-inbox-injection-auto-ack-roast-pr-2.md
  mode: pr        iteration: 2 of 3
  delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 1 resolved · 0 regressed (0 Blocking) · 2 punch-listed (open)
  coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 0 → deduped 0 → panel 0 · spot 0 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0
  independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
  seat-agreement: n/a — no candidates or panels this round.

- 2026-10-15-inbox-injection-auto-ack-roast-pr-post-cap-audit.md
  mode: pr        iteration: post-cap audit
  delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 0 resolved · 0 regressed (0 Blocking) · 2 punch-listed (open)
  coverage: scouts 10/10 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, testing, hygiene-docs) · raw 0 → deduped 0 → panel 0 · spot 0 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0
  independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
  seat-agreement: n/a — no candidates or panels this round.

### Fix loop
Metrics: completions — review clean 6 · after fix pass 1 · parked 0 · re-entry closes 0 · dispatched early 2 · cancelled 0
Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0

### Merge-back
Metrics: merges 7 · merge-failed 0 · rebase-conflicts 1 · seam-reviews 3 (fixed 0) · check-fails 0 (fixed 0)
Metrics: ledger-check ok · append-failed 0 · append-retried 0
Git first-parent feature merges:7; successful ledger Merge lines:7; no shortfall. Rebase-conflict merge:1; seam reviews fired:3; check fail→fixed:0; check fail:0; blocker:0. The aborted pre-fix staged merge never committed.

### Coverage
coverage-round-1: requirements:8 · mapped:8 · unmapped:0; two independent coverage reviews, no tree change.
scope-filter round1:1in-scope ·1punch-listed; a separate spot-only documentation Nit stayed open. Design graph depth3→3,width1.3→1.3,0cuts/0parked.

### Timing
These are raw cumulative profile outputs. Counterfactuals are model estimates, not measured speedups. Snapshot C includes the late-task availability defect above; its latest cap2 also differs from historical cap3→4. Role/cap definitions need verification before interpreting peak5 as any cap violation.

```text
Profile: ht-794l · 1 invocation · 2026-10-16T06:41Z → 2026-10-16T08:05Z · wall 1h25m (task graph 1h23m, after the last landing 2m) · agents 42 (timed 42) · beads dispatched 4, landed 4, planned 4
Profile: shape — 4 beads · 4 edges · depth 3 · width 1.3 · beads per level 2/1/1 · widest fan-out ht-794l.1 (2 direct, 2 transitive dependents)
Profile: bound — graph lower bound 1h58m along ht-794l.1 → ht-794l.2 → ht-794l.4 (unlimited slots and merge lanes) · model of the run 1h58m (cap 4, one merge lane) vs actual 1h23m (143%) → graph-bound: the 3-bead dependency chain set the pace — reshaping it is the lever
Profile: critical path — 1h23m = implement 42m · review 1m · fix 9m · merge 24m · seam 1m · planning 2m · dispatch 3m · of which other beads' merge-lane turns 7m
Profile: critical chain — plan planning 2m → ht-794l.3 implement+review+merge 51m → merge lane ×1 5m (seam 23s) → ht-794l.1 fix+merge 12m → merge lane ×2 9m (seam 56s)
Profile: bottleneck — ht-794l.3 — 51m on the critical path (62%) · implement 42m · review 1m · merge 7m · dispatch 23s · implement on the path 42m: test 16s · poll 17m · build 2s · git 21s · read 2s · edit 10s · shell 1m · other 5s · model 23m (longest: "sleep" 30s, "sleep" 30s) · dependents 1 direct, 1 transitive
Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 59m over 4 of 4 landed beads — planning 2m (1) · retry 57m (3)
Profile: tests — implementers and fixers, summed: 1m running tests and 51m polling background runs, 32% of their 2h45m · 0 Bash call(s) ran into the 10-minute tool limit
Profile: merge lane — busy 25m of 37m (69%) · seam reviews and fixes 1m (5% of busy) · 16 merge dispatches, first try not merged for 4 of 4 · queue wait 7m total, peak 1 waiting · stack-parent wait 23m
Profile: concurrency — 42 dispatches over 1h25m · a dispatch running 1h22m (97%) · dispatches at once: 0 for 3% · 1 for 21% · 2 for 21% · 3+ for 55% of the time · peak 5
Profile: runtime — peak 5 agents at once · slot cap 4
Profile: what-if — cut ht-794l.2 <- ht-794l.1 → 1h19m (−39m, 33%) — payoff if this edge were cut — whether it can be is a design call
Profile: what-if — split ht-794l.1 → 1h27m (−31m, 26%) — its 2 dependent(s) wait only for a contract slice ≈30% of its implement time
Profile: what-if — faster ht-794l.2 → 1h34m (−24m, 21%) — its implement time halved
Profile: what-if — faster ht-794l.1 → 1h36m (−22m, 19%) — its implement time halved
Profile: what-if — cut ht-794l.4 <- ht-794l.2 → 1h42m (−17m, 14%) — payoff if this edge were cut — whether it can be is a design call
Profile: what-if — split ht-794l.2 → 1h42m (−17m, 14%) — its 1 dependent(s) wait only for a contract slice ≈30% of its implement time
Profile: context — agents 19 (measured 19) · peak≥80% 0 · tasks>1 4 · compacted 0 agents, 0 compactions · coordinator 171K/475K (36%), 0 compactions · max code/impl_ht_794l_2 61%
Profile: context agent — code/impl_ht_794l_2 peak 288K/475K (61%) · first 33K · compactions 0
Profile: unmeasured — the schedule model is 143% of actual, so the what-ifs are rough
```

```text
Profile: ht-794l · 1 invocation · 2026-10-16T06:41Z → 2026-10-16T08:32Z · wall 1h51m (task graph 1h50m, after the last landing 1m) · agents 61 (timed 61) · beads dispatched 5, landed 5, planned 5
Profile: shape — 5 beads · 4 edges · depth 3 · width 1.7 · beads per level 3/1/1 · widest fan-out ht-794l.1 (2 direct, 2 transitive dependents)
Profile: bound — graph lower bound 1h58m along ht-794l.1 → ht-794l.2 → ht-794l.4 (unlimited slots and merge lanes) · model of the run 1h58m (cap 4, one merge lane) vs actual 1h50m (108%) → graph-bound: the 3-bead dependency chain set the pace — reshaping it is the lever
Profile: critical path — 1h50m = implement 51m · review 3m · fix 9m · merge 25m · seam 1m · planning 4m · finish 1m · dispatch 4m · wait 13m · of which other beads' merge-lane turns 7m
Profile: critical chain — plan planning 2m → ht-794l.3 implement+review+merge 51m → merge lane ×1 5m (seam 23s) → ht-794l.1 fix+merge 12m → merge lane ×2 9m (seam 56s) → final-review finish 1m → plan planning 2m → ht-794l.8 implement+review+merge 11m
Profile: bottleneck — ht-794l.3 — 51m on the critical path (47%) · implement 42m · review 1m · merge 7m · dispatch 23s · implement on the path 42m: test 16s · poll 17m · build 2s · git 21s · read 2s · edit 10s · shell 1m · other 5s · model 23m (longest: "sleep" 30s, "sleep" 30s) · dependents 1 direct, 1 transitive
Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 2h38m over 5 of 5 landed beads — planning 2m (1) · retry 2h36m (4)
Profile: tests — implementers and fixers, summed: 2m running tests and 52m polling background runs, 31% of their 2h53m · 0 Bash call(s) ran into the 10-minute tool limit
Profile: merge lane — busy 27m of 1h04m (42%) · seam reviews and fixes 1m (5% of busy) · 22 merge dispatches, first try not merged for 5 of 5 · queue wait 7m total, peak 1 waiting · stack-parent wait 23m
Profile: concurrency — 61 dispatches over 1h51m · a dispatch running 1h35m (86%) · dispatches at once: 0 for 14% · 1 for 28% · 2 for 16% · 3+ for 42% of the time · peak 5
Profile: runtime — peak 5 agents at once · slot cap 4
Profile: what-if — cut ht-794l.2 <- ht-794l.1 → 1h19m (−39m, 33%) — payoff if this edge were cut — whether it can be is a design call
Profile: what-if — split ht-794l.1 → 1h27m (−31m, 26%) — its 2 dependent(s) wait only for a contract slice ≈30% of its implement time
Profile: what-if — faster ht-794l.2 → 1h34m (−24m, 21%) — its implement time halved
Profile: what-if — faster ht-794l.1 → 1h36m (−22m, 19%) — its implement time halved
Profile: what-if — cut ht-794l.4 <- ht-794l.2 → 1h42m (−17m, 14%) — payoff if this edge were cut — whether it can be is a design call
Profile: what-if — split ht-794l.2 → 1h42m (−17m, 14%) — its 1 dependent(s) wait only for a contract slice ≈30% of its implement time
Profile: context — agents 24 (measured 24) · peak≥80% 0 · tasks>1 4 · compacted 0 agents, 0 compactions · coordinator 193K/475K (41%), 0 compactions · max code/impl_ht_794l_2 61%
Profile: context agent — code/impl_ht_794l_2 peak 288K/475K (61%) · first 33K · compactions 0
Profile: unmeasured — none
```

```text
Profile: ht-794l · 1 invocation · 2026-10-16T06:41Z → 2026-10-16T09:05Z · wall 2h25m (task graph 2h23m, after the last landing 2m) · agents 84 (timed 84) · beads dispatched 7, landed 7, planned 7
Profile: shape — 7 beads · 4 edges · depth 3 · width 2.3 · beads per level 5/1/1 · widest fan-out ht-794l.1 (2 direct, 2 transitive dependents)
Profile: bound — graph lower bound 1h58m along ht-794l.1 → ht-794l.2 → ht-794l.4 (unlimited slots and merge lanes) · model of the run 2h08m (cap 2, one merge lane) vs actual 2h23m (90%) → graph-bound: the 3-bead dependency chain set the pace — reshaping it is the lever
Profile: critical path — 2h23m = implement 14m · review 49s · merge 2m · dispatch 31s · wait 2h05m
Profile: critical chain — plan planning 0s → ht-794l.10 implement+review+merge 17m
Profile: bottleneck — ht-794l.10 — 2h23m on the critical path (100%) · implement 14m · review 49s · merge 2m · dispatch 31s · wait 2h05m · implement on the path 14m: test 27s · poll 4m · build 5s · git 6s · shell 32s · other 1s · model 9m (longest: "cargo fmt --check" 25s, "git diff --check" 13s) · 4 attempts (?, ?, ?, IMPLEMENTED) · dependents 0
Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 6h49m over 7 of 7 landed beads — planning 2m (1) · retry 6h46m (6)
Profile: tests — implementers and fixers, summed: 2m running tests and 59m polling background runs, 31% of their 3h19m · 0 Bash call(s) ran into the 10-minute tool limit
Profile: merge lane — busy 30m of 1h37m (31%) · seam reviews and fixes 1m (4% of busy) · 27 merge dispatches, first try not merged for 7 of 7 · queue wait 8m total, peak 1 waiting · stack-parent wait 23m
Profile: concurrency — 84 dispatches over 2h25m · a dispatch running 1h56m (80%) · dispatches at once: 0 for 20% · 1 for 25% · 2 for 23% · 3+ for 32% of the time · peak 5
Profile: runtime — peak 5 agents at once · slot cap 2
Profile: what-if — faster ht-794l.2 → 1h46m (−22m, 17%) — its implement time halved
Profile: what-if — faster ht-794l.1 → 1h47m (−22m, 17%) — its implement time halved
Profile: what-if — cut ht-794l.4 <- ht-794l.2 → 1h52m (−17m, 13%) — payoff if this edge were cut — whether it can be is a design call
Profile: what-if — split ht-794l.2 → 1h53m (−15m, 12%) — its 1 dependent(s) wait only for a contract slice ≈30% of its implement time
Profile: what-if — unlimited slots → 1h58m (−10m, 8%) — no slot cap
Profile: what-if — cut ht-794l.2 <- ht-794l.1 → 2h02m (−6m, 5%) — payoff if this edge were cut — whether it can be is a design call
Profile: context — agents 32 (measured 32) · peak≥80% 0 · tasks>1 4 · compacted 0 agents, 0 compactions · coordinator 217K/475K (46%), 0 compactions · max code/impl_ht_794l_2 61%
Profile: context agent — code/impl_ht_794l_2 peak 288K/475K (61%) · first 33K · compactions 0
Profile: unmeasured — none
```

### Bead graph
| id | type | title | what (first sentence) |
| --- | --- | --- | --- |
| ht-794l | epic | Automatic inbox injection coverage and receipt settlement | Goal: eligible hooks and channels inject complete inbox content, direct inbox only for incomplete delivery, and automatically settle existing receipt obligations after complete delivery, including linked nodes. |
| ht-794l.1 | task | Canonical hook-context receipt completion | Implement native-hook receipt settlement as an additive extension of CompleteInboxDelivery, with negotiated capability and truthful cooperative_hook_context provenance. |
| ht-794l.10 | task | Complete additive hook receipt capability test contracts | Covers: capability_constants_are_stable, bare_daemon_advertises_legacy_capabilities_without_v2_recorder, every_advertised_capability_has_a_handler final affected sweep failures. |
| ht-794l.2 | task | Native hook coverage, automatic ACK and conditional inbox guidance | Make native hook injection report coverage explicitly, automatically complete displayed receipt candidates after successful context output, and emit inbox retrieval only for undisplayed/unknown content. |
| ht-794l.3 | task | Bound channel injection and preserve automatic ACK on overflow | Bound push-channel tool-call injection and make delivery continuation request inbox only when a batch or truncation leaves content undisplayed. |
| ht-794l.4 | task | Integration sweep: injected inbox coverage and automatic receipts | Verify the complete local goal flows across standard native hooks, text inbox, mod channel and hook-pull after all feature leaves land, adding integration coverage for seams no leaf already proves and fixing small gaps inline. |
| ht-794l.5 | task | review: ht-794l.1 | Review, fix and merge of ht-794l.1, tracked separately so ht-794l.1's dependents can start once it is implemented. |
| ht-794l.6 | task | review: ht-794l.3 | Review, fix and merge of ht-794l.3, tracked separately so ht-794l.3's dependents can start once it is implemented. |
| ht-794l.7 | task | review: ht-794l.2 | Review, fix and merge of ht-794l.2, tracked separately so ht-794l.2's dependents can start once it is implemented. |
| ht-794l.8 | task | Cover native auto-ACK sibling unread and wake isolation | Covers: [Should-fix] src/cli/hook_inbox.rs:365 |
| ht-794l.9 | task | Update superseded lifecycle guidance assertions and current handoff goldens | Covers: final affected sweep initial failures in sweep-initial-failures.md: lifecycle/shared guidance and current handoff golden expectations. |

All11records are closed:7implementation/fix tasks,3review bookkeeping tasks and1epic.

| dependent | blocker | reason |
| --- | --- | --- |
| ht-794l.2 | ht-794l.1 | consumes negotiated CompleteInboxDelivery receipt extension and canonical hook source (needs: ht-794l.1) |
| ht-794l.4 | ht-794l.1 | consumes all leaves (integration sweep) |
| ht-794l.4 | ht-794l.3 | consumes all leaves (integration sweep) |
| ht-794l.4 | ht-794l.2 | consumes all leaves (integration sweep) |
| ht-794l.5 | ht-794l.1 | unstated |
| ht-794l.6 | ht-794l.3 | unstated |
| ht-794l.7 | ht-794l.2 | unstated |

## Design questions
none — analyst cap-history/fresh-context candidates were not retained as defects: cap role semantics are unverified, and fresh isolated judging was already required and the contaminated attempts were excluded.

## Doc gaps
none

## Already fixed — do not re-litigate
none verified upstream. Local mitigations used authorized OS-visible cleanup, replaced contaminated judges, and preserved the profiler caveat; these are not claims of an upstream fix.

## Not established
- Analyst was fresh GPT-6-astra (high), not Opus. Reviews were same-family GPT with role separation; no cross-family independence is claimed.
- No upstream source root-cause inspection or helper/profiler patch was attempted; premises above must be checked.
- Cap history/role definitions and manual judge isolation enforcement were advisory analyst candidates, not confirmed defects retained here.
- No source-only project build regression or measured benefit from edge cuts/halved implementation times was established. Background polling buckets may include compilation waits.
- By default the ledger append path can lose a line; ledger-derived counts are lower bounds except for the available ledger-check and independent7-merge git cross-check.
- Ledger, detector, Metrics, profile, roast and full --limit0 bead sources were gathered; none missing.

## Verification bar
- Run cleanup against owned private stand-ins while process listing is denied and while termination is denied. Neither case may claim verified stopped; an authorized successful run must prove termination without touching unrelated processes.
- Profile a resumed run with a task introduced later: time before creation must not be retry/ready wait, and printed critical-chain totals must reconcile. Exercise missing creation timestamps and multiple cap epochs; unknown facts must stay unknown.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
