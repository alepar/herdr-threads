# Gathered inputs
plugin version: 6.4.2-alepar4.3 at launch → 4.5 (104b54e) → 4.6 (2bf1d53) mid-run

## friction.md
- 2026-10-01 · super-design · design-roast fix tasks: findings were pure design-text edits (spec + bead descriptions); applied inline rather than filing one fix bead per finding as §Adversarial Review Loop step 1 prescribes — the beads would be created and closed in the same minute with no implementation.
- 2026-10-01 · super-design · parallelism pass: graph-shape printed 4 candidates all "depth 4→4"; the judge dispatch was skipped since no single removal can shorten the critical path.
- 2026-10-01 · super-auto · user asked to be consulted only on ht-rzi.2 contention; a session Stop hook (goal) then forbade pausing, so the contention was resolved by applying the recommended option and parking it.
- 2026-10-01 · super-code · planner noted a missing edge (ht-rzi.3 needs ht-rzi.1's migration 0010) but the coordinator dispatched .3 stacked on .9 anyway; it blocked twice and was quarantined. Resolved by hand after .1 merged (edge added, blockers ht-8rx/ht-ka1 closed). Planner-detected missing edges should be applied (or the task held) rather than only noted.
- 2026-10-01 · super-auto · full-suite preview run concurrently with a 154-agent roast workflow: 13 extra failures (harness `--version could not be observed`) vanished when re-run in isolation (lib 1/1 pass; hook_entrypoint 28/29, the remaining `ten_thousand_…` scale test is load-sensitive like main's pre-existing `twenty_thousand_…` and `three_thread_startup_…` failures). Phase-6 sweep must run with no concurrent workflow.
- 2026-10-01 · super-code (4.3) · fix-loop re-entry: opus planner dispatch produced no response for 15 min, six times in a row (interrupted each time); no task started in 91 min. Stopped and relaunched on the 4.5 coordinator.js.

## Ledger Detector/Slowness/Edge/Merge/Metrics lines
Launch: args {"epicId":"ht-rzi","integrationBranch":"trust-model-invariants","integrationWorktree":"~/AleCode/herdr-threads/.worktrees/trust-model-invariants","skillsRoot":"~/.claude/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar4.3/skills","deferSweep":true,"mergeCheck":"cargo check --locked --all-targets --all-features","config":{"concurrency":4,"runtimeSlots":
Merge: ht-rzi.6 — rebase clean · seam-review none · check pass
Merge: ht-rzi.9 — rebase clean · seam-review none · check pass
Merge: ht-rzi.4 — rebase clean · seam-review none · check pass
Merge: ht-rzi.5 — rebase clean · seam-review cleared · check pass
Detector: round 1 — 4 ready · topped-up 2 · cap 4 · peak in-flight 4 · top-up queries 0/40 · stacked 2 · merge queue peak 2 · waiting on deps 3 · hot-file cap raised: src/ports.rs · hot-file deferrals: src/ports.rs (2) · runtime slots 10
Edge audit: round 2 — open leaves 5, depth 4, achievable width 2 vs cap 4; changes: none; The graph is the binding constraint, not the cap: only ht-rzi.1 and ht-rzi.3 are ready against a cap of 4, and the critical path ht-rzi.1 → ht-rzi.2 → ht-rzi.7 → ht-rzi.8 sets the length. Neither candidate edge should change. ht-rzi.2 needs ht-rzi.1's B5 migration (the cooperative_continuity kind and the cont
Merge: ht-rzi.1 — rebase clean · seam-review fixed · check pass
Slowness: ht-rzi.3 quarantined on a planner-noted missing edge → edge added by session, blockers closed; .3 expected to re-dispatch next round or on relaunch
Merge: ht-rzi.2 — rebase clean · seam-review cleared · check pass
Detector: round 2 — 1 ready · topped-up 1 · cap 4 · peak in-flight 2 · top-up queries 0/40 · stacked 1 · merge queue peak 1 · idle slots 2 · waiting on deps 2 · runtime slots 10
Metrics: merges 6 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 3 (fixed 1) · check-fails 0 (fixed 0)
Metrics: completions — review clean 6 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 3 · cancelled 0
Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
Metrics: ledger-check ok · append-failed 0 · append-retried 0
Launch: args {"epicId":"ht-rzi","integrationBranch":"trust-model-invariants","integrationWorktree":"~/AleCode/herdr-threads/.worktrees/trust-model-invariants","skillsRoot":"~/.claude/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar4.3/skills","deferSweep":true,"mergeCheck":"cargo check --locked --all-targets --all-features","config":{"concurrency":4,"runtimeSlots":
Edge audit: round 1 — open leaves 3, depth 3, achievable width 1 vs cap 4; changes: none; The graph is the binding constraint, not the cap: the open chain ht-rzi.3 -> ht-rzi.7 -> ht-rzi.8 runs one bead at a time against a cap of 4. Dropping ht-rzi.7 <- ht-rzi.3 is the only change that would shorten it (depth 3->2), but I kept that edge. ht-rzi.7 documents the me init refusal error text and the ove
Edge audit: round 1 — open leaves 3, depth 3, achievable width 1 vs cap 4; changes and summary elided
Merge: ht-rzi.3 — rebase clean · seam-review none · check pass
Merge: ht-rzi.7 — rebase clean · seam-review none · check pass
Merge: ht-rzi.8 — rebase clean · seam-review none · check pass
Detector: round 1 — 1 ready · topped-up 2 · cap 4 · peak in-flight 3 · top-up queries 0/40 · stacked 3 · cancelled 1 · merge queue peak 1 · idle slots 1 · runtime slots 10
Metrics: merges 9 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 3 (fixed 1) · check-fails 0 (fixed 0)
Metrics: completions — review clean 9 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 6 · cancelled 1
Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
Metrics: ledger-check ok · append-failed 0 · append-retried 1
Launch: args {"epicId":"ht-rzi","integrationBranch":"trust-model-invariants","integrationWorktree":"~/AleCode/herdr-threads/.worktrees/trust-model-invariants","skillsRoot":"~/.claude/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar4.3/skills","deferSweep":true,"mergeCheck":"cargo check --locked --all-targets --all-features","config":{"concurrency":4,"runtimeSlots":
Launch: args {"epicId":"ht-rzi","integrationBranch":"trust-model-invariants","integrationWorktree":"~/AleCode/herdr-threads/.worktrees/trust-model-invariants","skillsRoot":"~/AleCode/superpowers/skills","deferSweep":true,"mergeCheck":"cargo check --locked --all-targets --all-features","config":{"concurrency":4,"runtimeSlots":10,"hotFileCap":3,"topUpQueryCap":40,"earlyUnbloc
Launch: args {"epicId":"ht-rzi","integrationBranch":"trust-model-invariants","integrationWorktree":"~/AleCode/herdr-threads/.worktrees/trust-model-invariants","skillsRoot":"~/AleCode/superpowers/skills","deferSweep":true,"mergeCheck":"cargo check --locked --all-targets --all-features","config":{"concurrency":4,"runtimeSlots":10,"hotFileCap":3,"topUpQueryCap":40,"earlyUnbloc
Merge: ht-rzi.20 — rebase clean · seam-review none · check pass
Merge: ht-rzi.21 — rebase clean · seam-review cleared · check pass
Merge: ht-rzi.22 — rebase clean · seam-review cleared · check pass
Merge: ht-rzi.19 — rebase clean · seam-review cleared · check pass
Merge: ht-rzi.18 — rebase clean · seam-review fixed · check pass
Merge: ht-rzi.23 — rebase clean · seam-review cleared · check pass
Detector: round 1 — 6 ready · topped-up 0 · cap 4 · peak in-flight 4 · top-up queries 0/40 · merge queue peak 1 · runtime slots 10
Metrics: merges 15 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 8 (fixed 2) · check-fails 0 (fixed 0)
Metrics: completions — review clean 14 · after fix pass 1 · parked 0 · re-entry closes 0 · dispatched early 6 · cancelled 1
Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0
Metrics: ledger-check ok · append-failed 0 · append-retried 0
Launch: args {"epicId":"ht-rzi","integrationBranch":"trust-model-invariants","integrationWorktree":"~/AleCode/herdr-threads/.worktrees/trust-model-invariants","skillsRoot":"~/AleCode/superpowers/skills","deferSweep":true,"mergeCheck":"cargo check --locked --all-targets --all-features","config":{"concurrency":4,"runtimeSlots":10,"hotFileCap":3,"topUpQueryCap":40,"earlyUnbloc
Merge: ht-rzi.24 — rebase clean · seam-review none · check pass
Detector: round 1 — 1 ready · topped-up 0 · cap 4 · peak in-flight 1 · top-up queries 0/40 · merge queue peak 1 · idle slots 3 · runtime slots 10
Metrics: merges 16 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 8 (fixed 2) · check-fails 0 (fixed 0)
Metrics: completions — review clean 15 · after fix pass 1 · parked 0 · re-entry closes 0 · dispatched early 6 · cancelled 1
Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0
Metrics: ledger-check ok · append-failed 0 · append-retried 0

## Roast verdict / delta / seat-agreement / independence lines
### 2026-10-01-b5-trust-policy-guards-roast-design-1.md
super-roast verdict: Blocking (7 confirmed)
mode: design        iteration: 1 of 3
coverage: scouts 8/8 (premortem, completeness, yagni, failure-mode, feasibility, domain:identity-and-authorization, domain:distributed-systems, domain:database-migrations) · raw 100 → deduped 42 → panel 29 · spot 13 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 29 · rr 0.83 · rg 0.76 · fg 0.66 · unanimous 0.62 · ground-loo 0.75 (n=24) · reproduce 8/20/1 · refute 3/25/1 · ground 13/15/1
### 2026-10-01-b5-trust-policy-guards-roast-design-2.md
super-roast verdict: Should-fix (3 confirmed) [converged]
mode: design        iteration: 2 of 3
delta vs prior: 2 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 6 resolved · 1 regressed (0 Blocking)
coverage: scouts 9/9 (premortem, completeness, yagni, failure-mode, feasibility, regression, domain:identity-and-trust-model, domain:sqlite-state-machine-transactions, domain:agent-ipc-protocol) · raw 16 → deduped 5 → panel 5 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 5 · rr 0.40 · rg 0.60 · fg 0.00 · unanimous 0.00 · ground-loo 0.00 (n=2) · reproduce 3/2/0 · refute 0/5/0 · ground 5/0/0
### 2026-10-01-b5-trust-policy-guards-roast-pr-1.md
super-roast verdict: Should-fix (13 confirmed)
mode: PR        iteration: 1 of 3
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, api-contract, observability, testing, hygiene-docs, deploy-safety) · raw 108 → deduped 80 → panel 29 · spot 49 · promoted 1 · judge completion 100% · remainder-capped: 1
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 30 · rr 0.87 · rg 0.77 · fg 0.63 · unanimous 0.63 · ground-loo 0.73 (n=26) · reproduce 13/17/0 · refute 9/21/0 · ground 20/10/0
### 2026-10-01-b5-trust-policy-guards-roast-pr-2.md
super-roast verdict: Should-fix (3 confirmed) [converged]
mode: PR        iteration: 2 of 3
delta vs prior: 3 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 13 resolved · 0 regressed (0 Blocking)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, hygiene-docs) · raw 9 → deduped 4 → panel 3 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel
seat-agreement: panels 3 · rr 0.67 · rg 1.00 · fg 0.67 · unanimous 0.67 · ground-loo 1.00 (n=2) · reproduce 3/0/0 · refute 2/1/0 · ground 3/0/0
### roast-design-context.md

## run.md requirements / scope-filter lines
- coverage-round-1 · canonical R-list: R1-R16 (coverage-round-1-requirements.md; R17-R22 appended from r-new) · requirements: 16 · mapped: 16 · unmapped: 0 · auto 14 applied (C1-C14 in coverage-ledger.md): edge ht-rzi.2<-ht-rzi.1; amended .1 .2 .3 .4 .5; new leaf ht-rzi.7 docs; integration sweep ht-rzi.8
- coverage-round-2 · canonical R-list: R1-R22 (coverage-round-2-requirements.md) · requirements: 22 · mapped: 22 · unmapped: 0 · divergence: findings 14 → 14, novel 100%, widening: yes · auto 14 applied (C15-C28): seam contract ht-rzi.9 (pane-agent port) with .2 .3 .4 depending; amended .1 .2 .3 .4 .5 .7 .8
scope-filter: 12 in-scope · 1 punch-listed

## graph shape
shape: leaves 0 · depth 0 · width n/a · critical path: none
summary: edges 0 · exempt 0 · candidates 0 (critical 0, epic-level 0)
