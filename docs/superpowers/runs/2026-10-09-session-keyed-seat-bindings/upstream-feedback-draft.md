<!-- upstream-feedback draft — PARKED (autonomous run). Not filed. Triage: kept D1–D6, Q-A, Q-B, G1; G2 judged weak (keep or drop at filing). Delivery per user preference: Herdr superpowers workspace agent tab, not gh. Unscrubbed. -->

# 2026-10-09-session-keyed-seat-bindings: test failures introduced inside the run pass as "pre-existing", and the build-only merge gate lets cross-merge test breaks through, so the integration tip stays red across three final reviews

Plugin: superpowers 6.4.2-alepar4.20. Run: super-auto (autonomous after the design phase), 76 beads (1 epic, 60 tasks of which 34 are `review:` split beads, 15 bugs), 41 task merges plus 3 base absorptions, 5 super-code coordinator invocations (one resumed with `resumeFromRunId`), design roast 2 rounds, PR roast 2 rounds plus a post-cap audit, 4 super-code final reviews, 2026-10-09 to 2026-10-11.

## Defects

### 1. A failure that an earlier merge in the same run introduced counts as "pre-existing", so the integration tip stays red and nobody owns the failure
- **Evidence:**
  - code-final-review-1.md: "failing tests on the integration tip get logged as 'pre-existing / fails at base' and are never assigned to anyone. It shows up in ht-54r.4, .5, .23, .11, .17, .9, .12 and .39 … a red integration tip was accepted as the baseline for every later task … All 12 fail, deterministically, 0 of 12 passed."
  - The session had to file ht-54r.39 by hand ("Fix tests failing on the run tip … measured by the .7 …"). The phase-6 sweep still found 4 failing tests (run.md `sweepFix: 4 failing → ht-54r.72 … .75`).
  - implementer-prompt.md:97-102 and task-reviewer-prompt.md:57 accept a test result at `[BASE]` as proof that a failure is pre-existing. `[BASE]` is the integration tip the task was cut from, not the run's fork point.
- **Premise to verify:** `[BASE]` resolves to the current integration tip and never to the run's fork point. No later stage (merge, seam review, coordinator triage) turns a "fails at base" failure into a bead.
- **Suggested fix shape:** when a failure also fails at `[BASE]`, re-run it at the run's fork point. If it passes there, the run caused it: file a fix bead (or a blocker) automatically instead of accepting it as a baseline.

### 2. `assemble-args --script` puts report text into the engine with `String.prototype.replace` and a string replacement, so `$'`, `$&` or `` $` `` in a prior report breaks the engine script
- **Evidence:**
  - friction.md [2026-10-11T01:34:38]: "prior report text containing $' … was substituted via String.replace without escaping $-patterns, so $' expanded to the template tail and the engine script failed to parse (Unterminated string constant). Workaround: zero-width-space between $ and the pattern char in a copy of the prior report."
  - super-roast/scripts/assemble-args.mjs:296: `engine.replace(hook, \`const A = ${JSON.stringify(args)}\`)`. `args` carries `priorReport` (line 274).
- **Premise to verify:** other call sites also pass user-derived text as a replacement string (lines 182 `artifact`/`diffCmd`, 227 `triageInput()`, 269-270 `independence`). `replaceLine`/`replaceAll` split and join, so they are safe.
- **Suggested fix shape:** use a replacer function (`s.replace(tok, () => val)`) at every site whose replacement holds caller text, and add a fixture whose prior report contains `$'`, `$&`, `` $` `` and `$1`.

### 3. A `resumeFromRunId` relaunch re-dispatched a task that had already merged and been cleaned up. It created a spurious review bead and a `→ blocker` merge line and cancelled a dependent
- **Evidence:**
  - ledger:33 `Merge: ht-54r.1 — rebase clean · seam-review none · check none → blocker`
  - ledger:34 "pending retry [cause: already-merged task re-dispatched after coordinator resume] … the branch task-ht-54r.1 no longer exists … Bead ht-54r.1 was still open"
  - ledger:35 "resume replay reopened ht-54r.1 and recreated review bead ht-54r.28 → session closed both"
  - ledger:36 "Task 3 (ht-54r.3): cancelled (parent ht-54r.1 blocked)"
  - ledger:41 "ledger-check landed≠completed: 3 vs 0"
  - friction.md [2026-10-10] "Coordinator should check already-merged before re-splitting on replay."
- **Premise to verify:** `scripts/already-merged INTEGRATION TASK_BRANCH` exits 2 ("unknown ref") once `remove-task-worktree` has deleted the task branch (ledger cleanup line "deleted: task-ht-54r.1"), so the already-merged path cannot fire after cleanup. The replayed pre-fan-out calls include non-idempotent `bd`/review-bead operations.
- **Suggested fix shape:** fall back to the ledger's `Merge: <id> … check pass` line, or a `Merge branch 'task-<id>'` merge subject, when the task branch is gone. Make the replayed bead operations check the current state first.

### 4. The resumed coordinator returned `stopReason: ready-drained` while a task was ready
- **Evidence:** ledger:43 "resumed coordinator stopped ready-drained with ht-54r.3 ready (replay state inconsistent) → fresh relaunch without resumeFromRunId (ledger + bd ready recovery)". friction.md [2026-10-10]: "returned ready-drained with work ready". coordinator-workflow.md:329 sends `ready-drained` straight on to Finish.
- **Premise to verify:** the stop decision trusts in-memory, replay-seeded state and does not re-query `ready-in-tree` before returning.
- **Suggested fix shape:** before returning `ready-drained`, re-run `ready-in-tree`. If anything is ready, return `stalled` (or re-enter the loop) and log an inconsistency line.

### 5. A hung test run inside a task agent was invisible to both the agent and the coordinator for 22 minutes
- **Evidence:**
  - ledger:293 "ht-54r.25 integration test binary hung 22 min (… blocked in lane_wiring::Session::new read; mod_delivery::watch_admission_over_cap_refuses_busy blocked on read) → session stopped the hung binary so the task can report"
  - friction.md [2026-10-10T23:10:59] "its own wait loop could not see it"
  - The same test later hit `TIMEOUT 300s` in the full sweep (bead ht-54r.74).
- **Premise to verify:** neither the implementer prompt nor the coordinator bounds a test command's wall time or reports when an agent makes no progress.
- **Suggested fix shape:** the implementer and sweep prompts require a hard wall-clock bound on every test command (runner-native terminate-after, or `timeout`) and report a hit as TIMEOUT evidence. Optionally the coordinator logs a `Slowness:` line for an agent past N minutes.

### 6. `Metrics: ledger-check` prints a fixed cause that was wrong here, and it never names the mismatched ids
- **Evidence:**
  - ledger:231 `landed≠completed: 24 vs 23 — the ledger shows landings this run did not count (an incomplete Resume read seeds too few completions)`. That invocation was a fresh launch (ledger:112 "Relaunch fresh (no resumeFromRunId)").
  - The same off-by-one persists through ledger:304, :325 and :350 (35/34, 37/36, 41/40).
  - code-final-review-1.md: "The 'landed≠completed 24 vs 23' gap comes from ht-54r.20, which is logged as both 'cancelled' and 'complete'" (ledger:101 and :146).
- **Premise to verify:** the completion count drops a task with a `cancelled` line from an earlier invocation that later completed.
- **Suggested fix shape:** print the ids in the set difference instead of a fixed cause, and count a cancel-then-complete task as completed.

## Run metrics
### Judge panel
- roast-design-1: mode design, iteration 1 of 3, verdict Should-fix (11 confirmed), independence: same-family (Claude) — seat-differentiated panel · rung: Workflow; seat-agreement: panels 50 · rr 0.72 · rg 0.72 · fg 0.60 · unanimous 0.52 · ground-loo 0.72 (n=36) · reproduce 14/33/3 · refute 4/44/2 · ground 22/26/2
- roast-design-2: mode design, iteration 2 of 3, verdict Should-fix (9 confirmed) [converged]; delta vs prior: 7 new confirmed (0 Blocking) · 0 carried · 9 resolved · 2 regressed (0 Blocking) · 0 punch-listed; independence: same-family (Claude) — seat-differentiated panel · rung: Workflow; seat-agreement: panels 15 · rr 0.60 · rg 0.67 · fg 0.33 · unanimous 0.33 · ground-loo 0.56 (n=9) · reproduce 9/5/1 · refute 4/11/0 · ground 14/1/0
- roast-pr-1: mode PR, iteration 1 of 3, verdict Should-fix (11 confirmed); independence: same-family (Claude) — seat-differentiated panel · rung: Workflow; seat-agreement: panels 34 · rr 0.76 · rg 0.74 · fg 0.74 · unanimous 0.62 · ground-loo 0.81 (n=26) · reproduce 10/24/0 · refute 7/25/2 · ground 16/16/2
- roast-pr-2: mode PR, iteration 2 of 3, verdict Should-fix (10 confirmed) [converged]; delta vs prior: 10 new confirmed (0 Blocking) · 0 carried · 6 resolved · 0 regressed · 5 punch-listed (open); independence: same-family (Claude) — seat-differentiated panel · rung: Workflow; seat-agreement: panels 12 · rr 0.83 · rg 0.92 · fg 0.92 · unanimous 0.83 · ground-loo 1.00 (n=10) · reproduce 11/0/1 · refute 9/2/1 · ground 10/1/1
- roast-pr-post-cap-audit: mode PR, iteration post-cap audit, verdict Should-fix (2 confirmed) [converged]; delta vs prior: 1 new confirmed (0 Blocking) · 0 carried · 0 resolved · 1 regressed (0 Blocking) · 9 punch-listed (open); independence: same-family (Claude) — seat-differentiated panel · rung: Workflow; seat-agreement: panels 2 · rr 0.50 · rg 1.00 · fg 0.50 · unanimous 0.50 · ground-loo 1.00 (n=1) · reproduce 2/0/0 · refute 1/1/0 · ground 2/0/0
- Step-backs: design round 1 `patch` (4 clusters, no recurrence); code round 1 `patch` (3 clusters: lazy-unread-per-binding, role-from-resolved-binding, authority-by-role-and-liveness).

### Fix loop
- `Metrics: completions — review clean 36 · after fix pass 5 · parked 3 · re-entry closes 0 · dispatched early 16 · cancelled 5` (ledger:348, last invocation, cumulative)
- `Metrics: fix-pass — entered 7 · FIXED 5 · BLOCKED 2` (ledger:349)

### Merge-back
- `Metrics: merges 41 · merge-failed 5 · rebase-conflicts 5 · seam-reviews 17 (fixed 5) · check-fails 2 (fixed 1)` (ledger:347)
- `Metrics: ledger-check landed≠completed: 41 vs 40 — … · append-failed 0 · append-retried 0` (ledger:350; see Defect 6)
- 46 `Merge:` lines: 5 with a rebase conflict; seam review fired on 17 (cleared 12, fixed 5); 1 check fail→fixed (ht-54r.71); 1 check fail (ht-54r.70, `→ blocker`, landed later with `check pass`); 4 `→ blocker` in all (ht-54r.1 resume artifact, ht-54r.22 twice, ht-54r.70); 1 `→ auth-refused` (ht-54r.7).
- `ledger incomplete: 53 merge commits vs 41 Merge: lines` by the literal rule (`git log --merges --first-parent cb6052fc..HEAD`). On inspection there is no shortfall. The 53 are 41 `task-ht-54r.*` merges, matched one to one by 40 `check pass` lines plus 1 `check fail→fixed`; 3 base absorptions (520a1780, 8d5120ed, 12300b4c); and 9 merges from the absorbed predecessor branch super-auto/hermes-global-enrollment (ht-4ne), which lie inside the range because the run branch was cut from it. See Doc gap 1.

### Coverage
- coverage round 1: `requirements: 16 · mapped: 14 · unmapped: 2 (R15, R16 — appended from r-new)` · not degraded
- coverage round 2: `requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 14 → 17 · novel 16/17 (94%) · widening: yes`
- `scope-filter: 6 in-scope · 5 punch-listed` (code fix round 1)
- Code fix round 2: no scope filter; it converged and ran a regression-only pass (`regressionPass-round-2: [Should-fix] src/service/liveness.rs:72 → ht-54r.70`).

### Timing
none. The ledger has no `Profile:` lines (see Not established). The detector lines stand in:
- `Detector: round 1 — 1 ready · topped-up 2 · cap 8 · peak in-flight 4 · … merge queue peak 1 · idle slots 4 · waiting on deps 22 · runtime slots 10` (resumed invocation)
- `Detector: round 1 — 4 ready · topped-up 10 · cap 8 · peak in-flight 6 · … review re-entries 1 · stacked 4 · cancelled 1 · merge queue peak 3 · idle slots 2 · waiting on deps 1 · hot-file cap raised: src/cli/hook.rs · hot-file deferrals: TRUST-POLICY.md (1), src/cli/hook.rs (2), src/store/bindings.rs (1)`
- `Detector: round 1 — 9 ready · topped-up 2 · cap 8 · peak in-flight 8 · … merge queue peak 2` (fix-loop round 1)
- `Detector: round 1 — 2 ready · … peak in-flight 2 · idle slots 6` and `round 2 — 1 ready · … peak in-flight 1 · idle slots 7` (regression-only + harness-neutral re-entry)
- `Detector: round 1 — 4 ready · … peak in-flight 4 · merge queue peak 2 · idle slots 4` (sweep-fix re-entry)
- `Edge audit: round 2 — open leaves 0, depth 0, achievable width 0 vs cap 8; changes: none`. No `Edge cut:` lines. graph-pass (run.md): depth 7→6 · width 3.6→4.2 · applied 7.
- Every round reports `top-up queries 0/40`.

### Bead graph
76 beads, all closed. Rows ht-54r.26–.38, .40–.51 and .61–.69 are `review: <id>` early-unblock split beads. ht-54r.28 is the spurious one from Defect 3.

| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-54r | epic | Session-keyed seat bindings: many concurrent bindings per pane seat, per-binding cursors, env attribution, liveness, cleanup (absorbs ht-4ne) | Root epic for run docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings (branch super-auto/session-keyed-seat-bindings). |
| ht-54r.1 | task | Seam contract: binding identity, claims and cursors (schema V29, protocol, store stubs) | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §1, §2 (cursor table), §4 (Migration V29, Protocol). |
| ht-54r.2 | task | Env claim derivation and process fingerprint (client, pure module) | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §1 table, decisions 9, 12, 15. |
| ht-54r.3 | task | Daemon: cooperative mapping and check-in by binding identity (enroll, touch, subagent attach, fingerprint refusal) | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §1 (daemon decides), §2 Enrollment, decisions 3, 13. |
| ht-54r.4 | task | Human exclusivity and managed-launch placeholder under multiple bindings | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md decision 14, §3 Managed launch. |
| ht-54r.5 | task | CLI and hooks send env claims; retire the agent OccupantContext journal | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §1 (claims come from the environment; client journal), decision 15. |
| ht-54r.6 | task | Claude PreToolUse --agent-id rewrite and subagent hook events in setup | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §1 table (Claude row), §4 Protocol (setup). |
| ht-54r.7 | task | Per-binding cursors and inbox page | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §2 Cursors, Inbox per binding, ACK. |
| ht-54r.8 | task | First-contact history stats and resume fast-forward | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §2 First-contact stats, Fast-forward; decisions 5, 6. |
| ht-54r.9 | task | Per-binding hook notifications and attention digest | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §2 Hook notifications. |
| ht-54r.10 | task | Binding liveness: connected/disconnected transitions and seat_active predicate | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Liveness, Presence; decision 9. |
| ht-54r.11 | task | Harness end events disconnect bindings (Codex/Claude/Hermes hook wiring) | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Becoming disconnected (end events); §4 Protocol (setup). |
| ht-54r.12 | task | Mod delivery channel per binding | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Wake (mod channel). |
| ht-54r.13 | task | Wake per connected top-level binding; composer wake to the pane's foreground agent | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Wake; decision 8. |
| ht-54r.14 | task | Pane-close retirement within a live Herdr incarnation | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Pane-close retirement; decision 10. |
| ht-54r.15 | task | Archival: inactive members don't block; fix scan-restart churn | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Archival; decision 11. |
| ht-54r.16 | task | Presence display: active vs inactive in participants and seat list | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Presence. |
| ht-54r.17 | task | Hermes on session bindings: drop cross-harness retire, lineage via parent_session_id, punch list | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §4 What changes from ht-4ne; §1 Lineage; ht-4ne report docs/superpowers/runs/2026-10-09-hermes-global-en |
| ht-54r.18 | task | Skill, README and agent docs for session bindings | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §4 Skill and docs; decision 2 (shared-seat warning). |
| ht-54r.19 | task | Configuration smoke: Claude+2 subagents, Codex embedded/daemon, Hermes interactive + hermes -z in a Claude pane, human exclusivity, resume/fast-forward, pane close→retire→archive | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §4 Configurations to exercise (1–6). |
| ht-54r.20 | task | Subagent write refusal, receipt binding attribution and lineage cursor copy | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §2 ACK, decision 13, §1 Lineage. |
| ht-54r.21 | task | Sweep: rewrite remaining 'current binding' readers for multiple bindings | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §1 (`seats.generation` … joins written `b.generation = s.generation`, which mean 'the current binding',  |
| ht-54r.22 | task | Per-seat delivery state onto bindings: lazy_recipients replaced by cursor, catch_up per binding | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §2 'What happens to today's per-seat delivery state'. |
| ht-54r.23 | task | Liveness worker: timed fingerprint re-check, same-pid rule, availability recompute | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §3 Becoming disconnected, Presence (availability); decision 9. |
| ht-54r.24 | task | Upgrade path: stale installed hooks and pre-upgrade callers (actionable refusal, re-enroll) | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md §4 Protocol (setup re-installs; agents mid-session re-enroll), decision 15. |
| ht-54r.25 | task | Integration sweep: session-keyed seat bindings end to end | Spec: docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md (whole). |
| ht-54r.26 | task | review: ht-54r.2 | Review, fix and merge of ht-54r.2, tracked separately so ht-54r.2's dependents can start once it is implemented. |
| ht-54r.27 | task | review: ht-54r.6 | Review, fix and merge of ht-54r.6, tracked separately so ht-54r.6's dependents can start once it is implemented. |
| ht-54r.28 | task | review: ht-54r.1 | Review, fix and merge of ht-54r.1, tracked separately so ht-54r.1's dependents can start once it is implemented. |
| ht-54r.29 | task | review: ht-54r.3 | Review, fix and merge of ht-54r.3, tracked separately so ht-54r.3's dependents can start once it is implemented. |
| ht-54r.30 | task | review: ht-54r.10 | Review, fix and merge of ht-54r.10, tracked separately so ht-54r.10's dependents can start once it is implemented. |
| ht-54r.31 | task | review: ht-54r.21 | Review, fix and merge of ht-54r.21, tracked separately so ht-54r.21's dependents can start once it is implemented. |
| ht-54r.32 | task | review: ht-54r.14 | Review, fix and merge of ht-54r.14, tracked separately so ht-54r.14's dependents can start once it is implemented. |
| ht-54r.33 | task | review: ht-54r.16 | Review, fix and merge of ht-54r.16, tracked separately so ht-54r.16's dependents can start once it is implemented. |
| ht-54r.34 | task | review: ht-54r.4 | Review, fix and merge of ht-54r.4, tracked separately so ht-54r.4's dependents can start once it is implemented. |
| ht-54r.35 | task | review: ht-54r.15 | Review, fix and merge of ht-54r.15, tracked separately so ht-54r.15's dependents can start once it is implemented. |
| ht-54r.36 | task | review: ht-54r.7 | Review, fix and merge of ht-54r.7, tracked separately so ht-54r.7's dependents can start once it is implemented. |
| ht-54r.37 | task | review: ht-54r.22 | Review, fix and merge of ht-54r.22, tracked separately so ht-54r.22's dependents can start once it is implemented. |
| ht-54r.38 | task | review: ht-54r.23 | Review, fix and merge of ht-54r.23, tracked separately so ht-54r.23's dependents can start once it is implemented. |
| ht-54r.39 | task | Fix tests failing on the run tip: topology_handoff live (5), cooperative fence replay, multi-binding fixture | Covers: tests failing at run-branch tip 13f42abe (measured by the .7/.22 rebase pass, failing identically on the run tip, so not caused by those branches): - cli::topology_handoff::tests::live::* (5 tests) - service::dis |
| ht-54r.40 | task | review: ht-54r.39 | Review, fix and merge of ht-54r.39, tracked separately so ht-54r.39's dependents can start once it is implemented. |
| ht-54r.41 | task | review: ht-54r.5 | Review, fix and merge of ht-54r.5, tracked separately so ht-54r.5's dependents can start once it is implemented. |
| ht-54r.42 | task | review: ht-54r.20 | Review, fix and merge of ht-54r.20, tracked separately so ht-54r.20's dependents can start once it is implemented. |
| ht-54r.43 | task | review: ht-54r.13 | Review, fix and merge of ht-54r.13, tracked separately so ht-54r.13's dependents can start once it is implemented. |
| ht-54r.44 | task | review: ht-54r.8 | Review, fix and merge of ht-54r.8, tracked separately so ht-54r.8's dependents can start once it is implemented. |
| ht-54r.45 | task | review: ht-54r.24 | Review, fix and merge of ht-54r.24, tracked separately so ht-54r.24's dependents can start once it is implemented. |
| ht-54r.46 | task | review: ht-54r.17 | Review, fix and merge of ht-54r.17, tracked separately so ht-54r.17's dependents can start once it is implemented. |
| ht-54r.47 | task | review: ht-54r.11 | Review, fix and merge of ht-54r.11, tracked separately so ht-54r.11's dependents can start once it is implemented. |
| ht-54r.48 | task | review: ht-54r.9 | Review, fix and merge of ht-54r.9, tracked separately so ht-54r.9's dependents can start once it is implemented. |
| ht-54r.49 | task | review: ht-54r.12 | Review, fix and merge of ht-54r.12, tracked separately so ht-54r.12's dependents can start once it is implemented. |
| ht-54r.50 | task | review: ht-54r.18 | Review, fix and merge of ht-54r.18, tracked separately so ht-54r.18's dependents can start once it is implemented. |
| ht-54r.51 | task | review: ht-54r.19 | Review, fix and merge of ht-54r.19, tracked separately so ht-54r.19's dependents can start once it is implemented. |
| ht-54r.52 | bug | Fix: V29 migration tests fail on LivenessFingerprintRequired | Covers: store::queries::binding_inbox_tests::no_duplicate_delivery_after_v29_migration; handoff_fences::handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence (r1 [Must-fix] 12 de |
| ht-54r.53 | bug | Fix: contract/ports/sweep/adapter-wiring test failures on run tip | Covers: ports::allocation_tests::ports_have_no_unsupported_defaults_or_admitted_duplicates; contracts::pending_and_retired_rows_preserve_physical_and_provenance_facts; contracts::seat_and_delivery_inspection_are_compound |
| ht-54r.54 | bug | Fix: Hermes enrollment sweep/smoke see 2 bindings, expect 1 | Covers: hermes_enrollment_sweep::hermes_bare_cli_main_flow_enrolls_checks_in_keeps_seat_on_new_and_excludes_children; hermes_enrollment_smoke::hermes_bare_enrollment_configurations_with_daemon_up (r1 [Must-fix] 12 determ |
| ht-54r.55 | bug | Fix: rewrite mod_delivery /clear and /resume tests to session-keyed model (ht-ozo) | Covers: mod_delivery::clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick; mod_delivery::resume_reack_settles; r1 [Must-fix] bead ht-ozo Test-defect bead. |
| ht-54r.56 | bug | Fix: liveness worker must disconnect connected bindings with no fingerprint | Covers: r1 [Must-fix] src/store/liveness worker leaves fingerprint-less connected bindings ht-54r.23 deferred this until ht-54r.5 landed (it has). |
| ht-54r.57 | bug | Fix cluster: lazy unread per binding (inbox v2 lazy source, membership) | Covers: [Should-fix] src/store/queries.rs:6514; [Should-fix] src/store/lazy_delivery.rs:287; r1 [Must-fix] src/store/queries.rs inbox lazy source per binding (F1) Cluster bead: it may change every instance of its rule, c |
| ht-54r.58 | bug | Fix cluster: subagent read-only from resolved binding role; subagent own-stream reads (ht-6v2) | Covers: [Should-fix] src/store/seats.rs:4480; [Should-fix] src/cli/mod.rs:2797; r1 [Must-fix] bead ht-6v2; r1 [Must-fix] src/harness/caller_env.rs CODEX_THREAD_ID malformed silent downgrade; failing test harness::bridge_ |
| ht-54r.59 | bug | Fix cluster: accountable top-level = role+liveness, not cooperative_top_level provenance (display-ACK) | Covers: [Should-fix] src/store/receipts.rs:315; r1 [Must-fix] src/store/receipts.rs display-ACK for cooperative_env_session (F3) Cluster bead: it may change every instance of its rule, cited or not. |
| ht-54r.60 | bug | Fix: apply_liveness Superseded verdict must re-validate claim ordering | Covers: [Nit] src/store/bindings.rs:665 apply_liveness re-validates a Superseded verdict only by binding id + fingerprint; a lifecycle check-in by the judged loser between the unlocked candidate read and the write leaves |
| ht-54r.61 | task | review: ht-54r.54 | Review, fix and merge of ht-54r.54, tracked separately so ht-54r.54's dependents can start once it is implemented. |
| ht-54r.62 | task | review: ht-54r.53 | Review, fix and merge of ht-54r.53, tracked separately so ht-54r.53's dependents can start once it is implemented. |
| ht-54r.63 | task | review: ht-54r.52 | Review, fix and merge of ht-54r.52, tracked separately so ht-54r.52's dependents can start once it is implemented. |
| ht-54r.64 | task | review: ht-54r.60 | Review, fix and merge of ht-54r.60, tracked separately so ht-54r.60's dependents can start once it is implemented. |
| ht-54r.65 | task | review: ht-54r.59 | Review, fix and merge of ht-54r.59, tracked separately so ht-54r.59's dependents can start once it is implemented. |
| ht-54r.66 | task | review: ht-54r.56 | Review, fix and merge of ht-54r.56, tracked separately so ht-54r.56's dependents can start once it is implemented. |
| ht-54r.67 | task | review: ht-54r.57 | Review, fix and merge of ht-54r.57, tracked separately so ht-54r.57's dependents can start once it is implemented. |
| ht-54r.68 | task | review: ht-54r.58 | Review, fix and merge of ht-54r.58, tracked separately so ht-54r.58's dependents can start once it is implemented. |
| ht-54r.69 | task | review: ht-54r.55 | Review, fix and merge of ht-54r.55, tracked separately so ht-54r.55's dependents can start once it is implemented. |
| ht-54r.70 | bug | Fix regression: continuity reattach inserts successor binding without fingerprint; liveness worker disconnects it | Covers: [Should-fix] src/service/liveness.rs:72 Since ht-54r.56 the liveness worker disconnects any connected agent binding with no fingerprint, but the continuity reattach path still inserts its successor binding withou |
| ht-54r.71 | bug | Sweep fix: scripts/check-harness-neutral — move claude/codex/hermes caller shapes and hook tables behind adapters | Covers: scripts/check-harness-neutral (68 violations at HEAD, 0 at main 43cc19cd; required CI step .github/workflows/ci.yml:197, AGENTS.md per-change check). |
| ht-54r.72 | bug | Sweep fix: continuity_successor_survives_a_liveness_pass_before_its_first_touch fails | Covers: store::control::tests::continuity_successor_survives_a_liveness_pass_before_its_first_touch (panics 'the successor is a candidate of the pass: []', tests/store/control.rs:8539) Sweep-fix bead: it may change the t |
| ht-54r.73 | bug | Sweep fix: read-only inbox selector refuses a top-level caller (lazy_display_json_machine_explicit_selector_readonly) | Covers: lazy_display::lazy_display_json_machine_explicit_selector_readonly (tests/cli/lazy_display.rs:259: Role::TopLevel caller gets 'subagents may read and summarize; only top-level agents check in, accept or ACK') Swe |
| ht-54r.74 | bug | Sweep fix: mod_delivery::watch_admission_over_cap_refuses_busy hangs (300s timeout) | Covers: mod_delivery::watch_admission_over_cap_refuses_busy (TIMEOUT 300s in full sweep; also hung >20 min in the ht-54r.25 integration run, blocked on a read) Sweep-fix bead: fixes the code the failing test exercises (o |
| ht-54r.75 | bug | Sweep fix: session_bindings_smoke config9 composer-wake predicate fails under full-suite load | Covers: session_bindings_smoke::config9_foreground_and_background_agents_in_one_pane (tests/integration/session_bindings_smoke.rs:2106 'no match, no composer wake: Some(BindingRow{… sess-bg …})' — passed in focused runs, |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-54r.2 | ht-54r.1 | consumes binding schema V29 + CallerClaim fields + store::bindings/read_cursors API + CLI --agent-id option |
| ht-54r.3 | ht-54r.1 | consumes binding schema V29 + CallerClaim fields + store::bindings/read_cursors API + CLI --agent-id option |
| ht-54r.4 | ht-54r.2 | consumes fingerprint format and process-info port |
| ht-54r.4 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.4 | ht-54r.10 | consumes sole disconnect transition |
| ht-54r.5 | ht-54r.2 | consumes caller_env::derive(), process-info port and refusal codes |
| ht-54r.5 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.6 | ht-54r.1 | consumes binding schema V29 + CallerClaim fields + store::bindings/read_cursors API + CLI --agent-id option |
| ht-54r.7 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.8 | ht-54r.7 | consumes per-binding cursors and inbox page |
| ht-54r.8 | ht-54r.10 | the fast-forward rule fires on a lifecycle reconnect and must never fire on a supersede (acceptance and r2); its tests need ht-54r.10's connect transition and s |
| ht-54r.8 | ht-54r.22 | consumes per-binding catch-up state |
| ht-54r.9 | ht-54r.5 | per-binding notices for a background agent and the foreground agent in one pane need two sessions' hooks to resolve to two bindings, which needs ht-54r.5's payl |
| ht-54r.9 | ht-54r.6 | consumes agent-id claim |
| ht-54r.9 | ht-54r.7 | consumes per-binding cursors and inbox page |
| ht-54r.9 | ht-54r.22 | consumes per-binding lazy/catch-up delivery state |
| ht-54r.10 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.11 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.11 | ht-54r.5 | consumes client env claims |
| ht-54r.11 | ht-54r.6 | consumes Claude hook groups (SubagentStart/SubagentStop) and --agent-id rewrite |
| ht-54r.11 | ht-54r.10 | consumes sole disconnect entry point |
| ht-54r.12 | ht-54r.1 | consumes has_unread signature |
| ht-54r.12 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.12 | ht-54r.5 | src/cli/watch.rs must register with its own session's binding, built with ht-54r.5's agent_claim (env-derived claim construction); without it two sessions in on |
| ht-54r.12 | ht-54r.10 | consumes disconnect side-effect hook |
| ht-54r.13 | ht-54r.1 | consumes binding-keyed mod-channel lookup signature |
| ht-54r.13 | ht-54r.7 | consumes per-binding unread query |
| ht-54r.13 | ht-54r.10 | consumes liveness transitions API and seat_active predicate |
| ht-54r.14 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.14 | ht-54r.10 | consumes liveness transitions API and seat_active predicate |
| ht-54r.15 | ht-54r.10 | consumes liveness transitions API and seat_active predicate |
| ht-54r.16 | ht-54r.10 | consumes liveness transitions API and seat_active predicate |
| ht-54r.17 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.17 | ht-54r.5 | consumes client env claims |
| ht-54r.17 | ht-54r.7 | consumes read_cursors::seed lineage copy |
| ht-54r.17 | ht-54r.10 | consumes sole disconnect transition |
| ht-54r.18 | ht-54r.2 | consumes caller_env::derive(), process-info port and refusal codes |
| ht-54r.18 | ht-54r.3 | consumes binding errors and refusal semantics |
| ht-54r.18 | ht-54r.4 | consumes human exclusivity behaviour |
| ht-54r.18 | ht-54r.5 | consumes client env claims |
| ht-54r.18 | ht-54r.8 | consumes first-contact/fast-forward presentation |
| ht-54r.18 | ht-54r.9 | consumes per-binding notices |
| ht-54r.18 | ht-54r.12 | consumes mod channel behaviour |
| ht-54r.18 | ht-54r.13 | consumes wake behaviour |
| ht-54r.18 | ht-54r.14 | consumes retirement behaviour |
| ht-54r.18 | ht-54r.15 | consumes archival rule |
| ht-54r.18 | ht-54r.16 | consumes presence output |
| ht-54r.18 | ht-54r.17 | consumes Hermes enrollment behaviour |
| ht-54r.18 | ht-54r.20 | consumes subagent read-only and ACK rules |
| ht-54r.18 | ht-54r.23 | consumes liveness/availability semantics |
| ht-54r.18 | ht-54r.24 | consumes pre-upgrade refusal guidance |
| ht-54r.19 | ht-54r.4 | consumes human exclusivity and launch placeholder |
| ht-54r.19 | ht-54r.5 | consumes client env claims |
| ht-54r.19 | ht-54r.6 | consumes Claude hook groups (SubagentStart/SubagentStop) and --agent-id rewrite |
| ht-54r.19 | ht-54r.8 | consumes first-contact/fast-forward presentation |
| ht-54r.19 | ht-54r.9 | consumes per-binding attention digest |
| ht-54r.19 | ht-54r.11 | consumes end-event disconnects, subagent enrollment, child resolution |
| ht-54r.19 | ht-54r.12 | consumes per-binding mod channel registry |
| ht-54r.19 | ht-54r.13 | consumes per-binding wake |
| ht-54r.19 | ht-54r.14 | consumes retirement under multi-binding |
| ht-54r.19 | ht-54r.15 | consumes archival eligibility rule |
| ht-54r.19 | ht-54r.16 | consumes presence display |
| ht-54r.19 | ht-54r.17 | consumes Hermes binding enrollment and lineage |
| ht-54r.19 | ht-54r.20 | consumes ACK settlement, subagent write refusal, lineage copy |
| ht-54r.19 | ht-54r.21 | consumes multi-binding readers |
| ht-54r.19 | ht-54r.22 | consumes per-binding lazy and catch-up state |
| ht-54r.19 | ht-54r.23 | consumes liveness worker and availability |
| ht-54r.19 | ht-54r.24 | consumes upgrade path |
| ht-54r.19 | ht-54r.58 | consumes subagent own-stream display + AdvanceCursors (replaces ht-6v2) |
| ht-54r.20 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.20 | ht-54r.7 | consumes read_cursors::seed |
| ht-54r.21 | ht-54r.3 | consumes daemon binding resolution (store::bindings implementations) |
| ht-54r.22 | ht-54r.1 | consumes read_cursors API and has_unread signature |
| ht-54r.22 | ht-54r.3 | acceptance 'catch-up survives another binding's check-in' needs two bindings on one seat, which is ht-54r.3's register_available rewrite (before it, a second li |
| ht-54r.23 | ht-54r.2 | consumes caller_env::derive(), process-info port and refusal codes |
| ht-54r.23 | ht-54r.10 | consumes liveness transitions API and seat_active predicate |
| ht-54r.24 | ht-54r.1 | consumes REQUIRED_HOOK_EVENTS constants |
| ht-54r.24 | ht-54r.3 | consumes binding resolution and refusals |
| ht-54r.24 | ht-54r.5 | consumes client env claims |
| ht-54r.24 | ht-54r.6 | consumes Claude hook groups |
| ht-54r.24 | ht-54r.7 | consumes read_cursors::seed |
| ht-54r.24 | ht-54r.10 | consumes sole disconnect transition |
| ht-54r.25 | ht-54r.1 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.2 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.3 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.4 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.5 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.6 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.7 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.8 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.9 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.10 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.11 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.12 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.13 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.14 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.15 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.16 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.17 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.18 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.19 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.20 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.21 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.22 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.23 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.24 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.39 | consumes all leaves (integration sweep) |
| ht-54r.25 | ht-54r.52 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.53 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.54 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.55 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.56 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.57 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.58 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.59 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.25 | ht-54r.60 | consumes the round-1 fix-loop tree (sweep runs end to end over the fixed code) |
| ht-54r.26 | ht-54r.2 | unstated |
| ht-54r.27 | ht-54r.6 | unstated |
| ht-54r.28 | ht-54r.1 | unstated |
| ht-54r.29 | ht-54r.3 | unstated |
| ht-54r.30 | ht-54r.10 | unstated |
| ht-54r.31 | ht-54r.21 | unstated |
| ht-54r.32 | ht-54r.14 | unstated |
| ht-54r.33 | ht-54r.16 | unstated |
| ht-54r.34 | ht-54r.4 | unstated |
| ht-54r.35 | ht-54r.15 | unstated |
| ht-54r.36 | ht-54r.7 | unstated |
| ht-54r.37 | ht-54r.22 | unstated |
| ht-54r.38 | ht-54r.23 | unstated |
| ht-54r.40 | ht-54r.39 | unstated |
| ht-54r.41 | ht-54r.5 | unstated |
| ht-54r.42 | ht-54r.20 | unstated |
| ht-54r.43 | ht-54r.13 | unstated |
| ht-54r.44 | ht-54r.8 | unstated |
| ht-54r.45 | ht-54r.24 | unstated |
| ht-54r.46 | ht-54r.17 | unstated |
| ht-54r.47 | ht-54r.11 | unstated |
| ht-54r.48 | ht-54r.9 | unstated |
| ht-54r.49 | ht-54r.12 | unstated |
| ht-54r.50 | ht-54r.18 | unstated |
| ht-54r.51 | ht-54r.19 | unstated |
| ht-54r.61 | ht-54r.54 | unstated |
| ht-54r.62 | ht-54r.53 | unstated |
| ht-54r.63 | ht-54r.52 | unstated |
| ht-54r.64 | ht-54r.60 | unstated |
| ht-54r.65 | ht-54r.59 | unstated |
| ht-54r.66 | ht-54r.56 | unstated |
| ht-54r.67 | ht-54r.57 | unstated |
| ht-54r.68 | ht-54r.58 | unstated |
| ht-54r.69 | ht-54r.55 | unstated |
| ht-54r.70 | ht-54r.71 | Hold until the harness-neutral sweep fix ht-54r.71 merges; the task's work is committed on task-ht-54r.70 (439c37ac) and only scripts/check-harness-neutral fail |

## Design questions

### A. Should the merge gate run the task's own named tests on the merged tree, not only the build-only `mergeCheck`?
- **Evidence:** three final reviews in a row flagged the same class:
  - code-final-review-2.md S2: "ht-54r.56 merged later and makes the liveness worker disconnect any connected binding with no fingerprint … The merge check is clippy only, so this seam went unnoticed" (ht-54r.59's test red on the tip, 5 of 5 runs).
  - code-final-review-3.md: "the same pipeline defect for the third review in a row … ht-54r.70 landed with its own regression test red, and the lazy_display regression went unnoticed. This needs a focused-test merge gate."
  - code-final-review-4.md repeats it: "lazy_display regression went unseen until the sweep".
  - The ht-54r.70 triage (ledger:308) told the re-dispatch to "Re-run the Task 36 focused tests" after rebasing onto ht-54r.71. The task then merged (`ledger:316 Merge: ht-54r.70 … check pass`) with that test red, because nothing at merge time checks it.
  - friction.md [2026-10-11T03:48:59].
- **For a focused-test gate:** the task's own named tests (the ones its report cites), run on the merged tree, are a small and bounded set. They would have caught all three seams here at merge time instead of through three final-review re-entries and a sweep-fix pass (ht-54r.70, .72–.75). The implementer's reported run goes stale exactly when a task is held and rebased (ht-54r.70), and SKILL.md's "re-runs its tests if the tip moved" covers only the unstacked-implementer path.
- **For keeping build-only:** the merge queue is single-flight, so tests in the merge dispatch serialize every landing (merge queue peaks here were 1–3). Flaky or load-sensitive tests (ht-54r.5's 11 Hermes timeouts under load; config9 under load) would turn into merge blockers. Upstream already decided "No per-merge full test suite is introduced" (#12, D5), and the Finish sweep is meant to catch these.
- A middle option: run the task's named tests only when the rebase or merge moved the tip since the implementer's evidence, or only for held or re-dispatched tasks.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### B. After a `[converged]` exit, should confirmed correctness defects, and final-review Must-fix items, have any fix path besides the human re-entering at the finish menu?
- **Evidence:**
  - roast-pr-2 exited `[converged]` with "10 confirmed … 0 Blocking … 5 punch-listed (open)". The punch list kept `[Should-fix] src/store/lazy_delivery.rs:507` (cursor never advances after a mod or hook-pull ACK) and `[Should-fix] src/service/delivery_pull.rs:249` (agy loses hook delivery after the upgrade).
  - code-final-review-3.md and -4.md grade both as Must-fix ("breaks spec §2 … on the main Claude delivery path").
  - Under super-auto SKILL.md:289 a final review from a post-loop re-entry "goes to the punch list, tagged `final review`". Final review 4's 7 Must-fix items went there, and report.md opens with "status: completed with 0 unresolved Blocking".
  - friction.md [2026-10-11T03:48:59].
  - #12 Question A answered "no new severity floor … final review catches misgraded minors". In this run final review did catch them, but only after the loop had exited, so catching them changed nothing.
- **For the current rule:** the loop needs a hard termination guarantee. Punch-listing sub-Blocking findings keeps a 3-iteration cap meaningful, and a human at the finish menu can re-enter.
- **For a fix path:** an unattended run spent two more final reviews (3 and 4) whose Must-fix output nothing consumed. A bounded option exists: one more pass for Must-fix items from a post-loop final review, mirroring the existing regression-only pass with no recursion. Alternatively, `[converged]` could require zero Should-fix in correctness lanes, or report.md's status could say "not ready" whenever the last final review does.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

### 1. The "ledger incomplete" merge-count rule misfires when a run absorbs its base or an unmerged predecessor branch
- **Evidence:** `git log --merges --first-parent cb6052fc..HEAD` gives 53 merge commits against 41 success-path `Merge:` lines, but every task merge has its line (see Merge-back). The extra 12 are 3 `Merge main (…)` absorptions and 9 `task-ht-4ne.*` merges from the predecessor branch the run absorbed. report-template.md also leaves open whether `check fail→fixed` counts as "success-path".
- **Premise to verify:** the rule is meant to count task merges only.
- **Suggested fix shape:** count only `Merge branch 'task-<epic>.*'` subjects on the run branch, starting from the run-start merge rather than the base, and define success-path as `check pass | check fail→fixed`.

### 2. The coverage reviewer template forbids tool calls, which forces a 16 KB prompt inline through the orchestrator
- **Evidence:** friction.md [2026-10-10] "coverage r1: reviewer prompt (16 KB) passed by file path (one Read) instead of inline, to keep orchestrator context; template says no tool calls."
- **Premise to verify:** the no-tool-calls rule exists for independence, not to bar reading the assembled prompt itself.
- **Suggested fix shape:** allow exactly one Read of the assembled prompt file, or state that inline is required and why.

## Already fixed — do not re-litigate
- none known fixed for the defects above in 6.4.2-alepar4.20. Related positions already taken upstream, which this report does not reopen:
  - #12 D5: `mergeCheck` covers default and all-features builds. This run's gate did both: `nice cargo clippy --locked --all-targets --all-features -- -D warnings && nice scripts/check-default-features`.
  - #12 Question B: the one regression-only bead was used here as ht-54r.70.
  - #12 D1: composed seam review exists; 17 seam reviews fired here.
- Design question A asks about the narrower focused-test gate, not the per-merge full suite #12 rejected. Design question B asks about the post-loop path, not a severity floor.

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- `Profile:` lines are absent from the ledger, so Timing is `none`. No critical path, wait causes, merge-lane load or what-if figures exist for this run.
- `Edge cut:` lines are absent (one `Edge audit:` line, no changes).
- How much time a focused-test merge gate would save or cost is unmeasured, and so is how often it would block on load-sensitive tests.
- Only the line-296 call site is confirmed to break the engine (Defect 2). Lines 182, 227 and 269-270 are suspected, not reproduced.
- The 22-minute hang (Defect 5) may be a product or test-fixture hang (isolated-Herdr `Session::new` reads) and not a skill defect. The finding is only that nothing surfaced it.
- The resume defects (3, 4) come from one relaunch after an unplanned, user-authorized mid-round stop (account switch). Behaviour after a planned, quiesced relaunch was not observed.
- Final reviews 1–4 call the ht-54r.7 out-of-pipeline conflict resolution "never seam-reviewed", but ledger:123 records `seam-review cleared` on its relaunch merge. This report makes no claim about it.
- The run's wall-clock duration is not measured; only the dates (2026-10-09 to 2026-10-11) are known.

## Verification bar
- assemble-args: a unit fixture whose `--prior-report` contains `$'`, `$&`, `` $` `` and `$1`. The emitted `--script` file must parse (`node --check`) and must embed the report byte for byte.
- Coordinator replay harness: (a) merge a task, run its cleanup (branch deleted), stop, and relaunch with `resumeFromRunId`. Expect an already-merged close: no new `review:` bead, no `Merge: … → blocker`, no cancelled dependents. (b) Seed replay state with a ready task. The coordinator must not return `ready-drained`.
- Implementer and reviewer contract: a scenario where test T passes at the fork point, a merged task breaks it, and a later task sees T fail at `[BASE]`. Expect a fix bead to be filed, not an accepted "pre-existing".
- A task whose test never terminates must report TIMEOUT within the stated bound and must not wait indefinitely.
- Ledger-check: a task logged `cancelled` in one invocation and `complete` in a later one must yield landed = completed, or print that id.
- For design question A, if adopted: replay this run's three seams (ht-54r.56 vs .59 test, ht-54r.70 own test, lazy_display after .71) through the new gate and confirm each is caught at merge.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
