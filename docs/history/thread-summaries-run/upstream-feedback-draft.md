# 2026-10-02-thread-summaries-compaction-survival: super-code ledger reads via a verbatim LLM echo silently corrupt Resume and Finish metrics once the ledger grows

Plugin: superpowers@superpowers-alepar 6.4.2-alepar4.9. Run: autonomous super-auto, 56 beads (18 designed leaves + review/fix/sweep-fix beads), 3 design-roast rounds, 2 code-roast rounds, 6 super-code invocations, 1 base-branch merge (32 conflicting files), 2026-10-02.

## Defects

### 1. super-code reads its ledger by having an agent echo it verbatim; at ~40 KB the reads failed or came back empty/lossy, corrupting Resume reconstruction and Finish metrics without saying so
- **Evidence:**
  - Invocation 4: `Metrics: UNAVAILABLE (merges) — the Finish ledger re-read returned null`. The read died on an API safeguard false positive, `[reasoning_extraction]` (req_011CfeW7PqVhSatECCNDDdFr; a second one was req_011CfeWnAXPXmqrmLGK1SZKC).
  - Invocation 5: `Metrics: merges 30 … ledger-check M≠completed: 30 vs 1` and `Detector: … waiting on deps 30`. `completed` is seeded from the Resume parse (coordinator.js:378-386), so Resume rebuilt 0 of 29 completions.
  - Invocation 6: `merges 0 … review clean 0` and `ledger-check M≠completed: 0 vs 3`, even though Merge/complete lines for the 3 tasks are present.
  - coordinator.js:1650 asks the agent for the file's "exact, complete contents verbatim". An empty `text` is accepted as "file does not exist yet"; only `null` is treated as unavailable (coordinator.js:360, :1226).
- **Premise to verify:**
  - The Workflow runtime has no direct file-read primitive.
  - An empty or truncated echo is detectable: the `# SDD ledger — plan:` header and the just-appended `Launch:` line must both be present.
- **Suggested fix shape:**
  - The read agent returns a line count and a sha256, and both are validated; on mismatch, missing header or missing current Launch line, report UNAVAILABLE loudly.
  - Read large ledgers in fixed chunks, or project only the line kinds the parsers use (`^Task |^Merge: |^Launch:`).

### 2. The Recurring-cluster detector never fires on a class that appeared in 7+ tasks: exact-text signatures, and state that is in memory per invocation
- **Evidence:**
  - The ledger has 112 `minor (deferred)` lines and 0 `Recurring` lines.
  - The same class appears under different wordings: `full suite not run`, `Whole-suite/integration tests not run`, `Full suite not completed`, `Test evidence ran --lib only`, `hook_entrypoint full target not run`.
  - coordinator.js:303-314 normalises only numbers, hashes, paths and quoting. `minorClusters` is a `new Map()` that is never seeded at Resume.
  - The final-review prompt (coordinator.js:1670) relies on these Recurring lines.
- **Premise to verify:** A reviewer-emitted category token clusters usefully without merging unrelated findings.
- **Suggested fix shape:**
  - The task reviewer emits a short category token per minor, and clustering keys on it.
  - Rebuild the clusters from the ledger's minor lines at Resume.

### 3. Under deferSweep the implementer is told not to run the whole suite, but the task reviewer still files "full suite not run" as a deferred minor
- **Evidence:**
  - implementer-prompt.md:55 says: `Do not run the whole suite; it runs once at the end of the epic.`
  - Reviewers still deferred this as a minor in 7+ tasks.
  - Every final review then echoed it as "not ready … needs green full suite".
- **Premise to verify:** The reviewer prompt can be told the epic's sweep policy.
- **Suggested fix shape:** task-reviewer-prompt.md states that the sweep is deferred, and flags only missing targeted tests.

### 4. Implementers' "pre-existing failure" claims pass review unverified; in this run the claimed pre-existing failure was a branch regression
- **Evidence:**
  - Ledger lines say: `shared_reuse_by_second_seat … pre-existing is inferred, not measured on base`; `claimed identical on base; not independently verified`; `claimed pre-existing on base, not verified by reviewer`.
  - It was later fixed as a branch defect in a dedicated fix bead (catch_up row count, a feature of this epic).
  - Neither implementer-prompt.md nor task-reviewer-prompt.md mentions pre-existing claims.
- **Premise to verify:** Running one named failing test at the task's `base` SHA is cheap enough to require.
- **Suggested fix shape:**
  - A "pre-existing" claim must carry that test's output at the task base SHA. Without it, the reviewer treats the claim as a finding, not a deferred minor.
  - Optionally, the coordinator flags the same test called "pre-existing" by 2 or more tasks.

### 5. super-auto's "append friction the moment it happens" dirties the integration worktree, and super-code's merge refuses a dirty integration worktree
- **Evidence:**
  - The ledger shows: `Merge: ht-1ip.10 — rebase clean · seam-review none · check none → blocker`, then `pending retry — … the integration worktree … had one uncommitted change: docs/superpowers/runs/…/friction.md`.
  - super-auto/SKILL.md:242 says to append "the moment they happen … and commit it with the run.md writes".
- **Premise to verify:** Any run that records friction during phase 3 hits this.
- **Suggested fix shape:** Any one of:
  - commit each friction append immediately;
  - have the merge clean-check ignore the caller-declared run directory, the same way `processRoots` names it;
  - keep the log outside the integration worktree until phase 6.

### 6. Parked `degraded-verdict` records cannot be superseded, so report-status keeps qualifiers whose condition no longer holds
- **Evidence:**
  - The orchestrator had to delete `full-suite sweep deferred … branch unmeasured` by hand after the phase-6 sweep measured the branch.
  - report-status:134-139 adds every parked degraded-verdict qualifier unconditionally.
- **Premise to verify:**
  - Some degraded verdicts are tied to a condition the run later resolves, such as an unmeasured or deferred sweep.
  - Others are permanent history, such as coverage widening, and must stay.
- **Suggested fix shape:**
  - Give parked records an optional `resolves-on:` key (for example `sweep`), or a `superseded` marker that report-status skips.
  - Phase 6 sets that marker once `codeBuckets.sweep` is stamped at the tip.


## Design questions

### A. Should planning flag classifier-sensitive task operations up front?
Pre-flight probes only the standard operation classes. Three tasks hit harness classifier refusals mid-run as BLOCKED-AUTH, which quarantined their dependents and stranded partial work:
- a subagent Write of an evidence deliverable named `findings.md` was refused with "Subagents should return findings as text, not write report files", and the task was hand-merged;
- a `herdr --help` chained with a keychain probe was refused as `[Credential Exploration]`;
- accepting a folder-trust prompt in a spawned agent was refused as `[Create Unsafe Agents]`, and nothing was implemented.

For: 3 of 34 tasks hit this and cost hand-merges and a rescope. Against: probing arbitrary task operations side-effect-free is hard, and quarantine already reports honestly. Cheap partial fixes:
- tell implementers that deliverable files must not be named `findings.md` or `report.md`;
- let BLOCKED-AUTH keep the task's committed WIP branch so the caller can merge it.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### B. Should changes landed after convergence get a bounded roast?
The code roast converged at f8e15115. After that, the base-branch merge (32 conflicting files, migrations renumbered) and three sweep-fix beads landed with per-task review only. One of them changed wake-safety behaviour: `ordinary wake now only excludes Herdr-blocked, no longer composer ActiveTurn`. super-auto/SKILL.md:236 says "after reviewing that combined tree's conflicts and its clean auto-merges" but defines no dispatch or tooling for that review.

For: unroasted behaviour changes otherwise reach the human labelled "converged". Against: it adds a phase, and per-task review plus the sweep may be enough for mechanical conflict resolution. A middle option is a helper that lists the conflict-resolved hunks.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### C. Should coverage run a bounded third round when round 2 widens?
Round 2 reported `findings 14 → 16 · novel 15/16 (94%) · widening: yes` and stopped at the fixed two rounds, so its 17 fixes were never re-reviewed. The design roast that followed found 1 Blocking in its round 2.

For: 94% novelty means coverage had not converged. Against: the design roast runs right after and may cover the same ground more cheaply. Measuring whether the design-roast findings overlap the unreviewed round-2 fixes would settle it.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

### 1. The documented launch paths are refused when skillsRoot is the plugin cache
`Workflow({scriptPath: '<skillsRoot>/super-code/coordinator.js'})` (super-code/SKILL.md:31,265; coordinator-workflow.md:307,1241) was refused with "must be a script path this tool returned, or a file you can already read". The run launched a byte-identical copy (sha 7cc9223f) from the scratchpad instead, and did the same for super-roast through `assemble-args --script`.
- **Premise to verify:** the refusal comes from the harness's readable-path rule.
- **Fix shape:** either document the launch preamble (copy the script, or Read it first, then verify its sha), or ship a `stage-coordinator` script.

### 2. super-auto defines no consumer for super-code's final-review verdict, and the run-state form for it is wrong
- run-state.md:43 gives `review: CLEAN` or `review: <verdict> (<N> confirmed)`. The actual value was `review: not ready (…)`.
- super-auto/SKILL.md never mentions the final review.
- Under deferSweep, the final review cannot say "ready".
- Six invocations each ended in an opus final review. One of them dispatched a single task.
- Smoke-found beads that tasks filed outside the epic's label reached phase 5 only because the orchestrator folded them in by hand.

**Fix shape:**
- define how the final review's Must-fix and Untested-scope sections, and beads filed outside the epic, feed the phase-5 step-back;
- fix the form to `ready|not ready (<summary>)`;
- skip the per-invocation final review under deferSweep in favour of one review after the phase-6 sweep.

## Run metrics
### Judge panel
- 2026-10-02-thread-summaries-compaction-survival-roast-design-1.md: design        iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 66 · rr 0.79 · rg 0.73 · fg 0.58 · unanimous 0.55 · ground-loo 0.69 (n=52) · reproduce 16/50/0 · refute 4/62/0 · ground 32/34/0
- 2026-10-02-thread-summaries-compaction-survival-roast-design-2.md: design        iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 16 · rr 0.69 · rg 0.94 · fg 0.63 · unanimous 0.63 · ground-loo 0.91 (n=11) · reproduce 13/3/0 · refute 8/8/0 · ground 14/2/0
- 2026-10-02-thread-summaries-compaction-survival-roast-design-3.md: design        iteration: 3 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 15 · rr 0.73 · rg 0.60 · fg 0.60 · unanimous 0.47 · ground-loo 0.64 (n=11) · reproduce 7/8/0 · refute 5/10/0 · ground 11/4/0
- 2026-10-02-thread-summaries-compaction-survival-roast-pr-1.md: PR        iteration: 1 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 48 · rr 0.71 · rg 0.71 · fg 0.71 · unanimous 0.56 · ground-loo 0.79 (n=34) · reproduce 20/28/0 · refute 12/36/0 · ground 26/22/0
- 2026-10-02-thread-summaries-compaction-survival-roast-pr-2.md: PR        iteration: 2 of 3 · independence: same-family (Claude) — seat-differentiated panel · seat-agreement: panels 2 · rr 1.00 · rg 0.50 · fg 0.50 · unanimous 0.50 · ground-loo 0.50 (n=2) · reproduce 1/1/0 · refute 1/1/0 · ground 2/0/0

### Fix loop
- Metrics: completions — review clean 14 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 7 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
- Metrics: completions — review clean 17 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 8 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0
- Metrics: completions — review clean 22 · after fix pass 1 · parked 1 · re-entry closes 0 · dispatched early 8 · cancelled 0
- Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0
- Metrics: UNAVAILABLE (completions) — the Finish ledger re-read returned null; no counts derived
- Metrics: UNAVAILABLE (fix-pass) — the Finish ledger re-read returned null; no counts derived
- Metrics: completions — review clean 29 · after fix pass 1 · parked 1 · re-entry closes 0 · dispatched early 8 · cancelled 0
- Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0
- Metrics: completions — review clean 0 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 0 · cancelled 0
- Metrics: fix-pass — entered 0 · FIXED 0 · BLOCKED 0

### Merge-back
- Metrics: merges 14 · merge-failed 1 · rebase-conflicts 3 · seam-reviews 8 (fixed 1) · check-fails 1 (fixed 1)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: merges 17 · merge-failed 1 · rebase-conflicts 3 · seam-reviews 9 (fixed 1) · check-fails 1 (fixed 1)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: merges 23 · merge-failed 1 · rebase-conflicts 3 · seam-reviews 9 (fixed 1) · check-fails 1 (fixed 1)
- Metrics: ledger-check ok · append-failed 0 · append-retried 0
- Metrics: UNAVAILABLE (merges) — the Finish ledger re-read returned null; no counts derived
- Metrics: UNAVAILABLE (ledger-check) — the Finish ledger re-read returned null; no counts derived
- Metrics: merges 30 · merge-failed 1 · rebase-conflicts 3 · seam-reviews 9 (fixed 1) · check-fails 1 (fixed 1)
- Metrics: ledger-check M≠completed: 30 vs 1 · append-failed 0 · append-retried 0
- Metrics: merges 0 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 0 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check M≠completed: 0 vs 3 · append-failed 0 · append-retried 0
- Merge: lines 35 · rebase conflict 3 · seam-review fired 9 · check fail→fixed 1 · check fail 0 · → blocker 1

### Coverage
- requirements: 13 · mapped: 13 · unmapped: 0 
- scope-filter: 8 in-scope · 11 punch-listed

### Bead graph
| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-1ip | epic | Thread summaries for compaction survival + soft-deadline receipt pokes | Root epic for super-auto run 2026-10-02-thread-summaries-compaction-survival. Spec: docs/superpowers/runs/2026 |
| ht-1ip.1 | task | Seam contract: summary schema, wire types, settings, inert hooks and trust-polic | Spec docs/history/thread-summaries-run/2026-10-02-thread-summaries-compact |
| ht-1ip.2 | task | Message authorship: record author_role and send --relays-user | Spec §1. Send path records author_role from the sender's open binding (operator_human->human, cooperative_top_ |
| ht-1ip.3 | task | Summary core: deterministic chunker, rendered sizes, displayed cover | Spec §2,§3. Pure module src/summary/{chunk,cover,render}.rs: render a message as a job-bundle line (header + b |
| ht-1ip.4 | task | Summary ledger: daemon extraction, submission validation, carry-forward merge | Spec §5,§6. Pure module src/summary/{ledger,validate,identifiers}.rs: L0 ledger skeleton pre-fill (user_instru |
| ht-1ip.5 | task | Summary job service: plan, leases, job bundles, submit, store blocks | Spec §4. Store + daemon handlers for Summary/SummaryJob/SummarySubmit: read-visibility check as history; compu |
| ht-1ip.6 | task | Catch-up mode: frontier hold on pushed attention, exit, stall and supersession | Spec §7. Implement the catch-up API behind the contract stubs: enter_or_keep (top-level binding only, frontier |
| ht-1ip.7 | task | Deadline extension: effective deadlines in overdue, warnings, pending receipts a | Spec §8. Implement effective_deadline = max(frozen, extension_until of latest catch-up row) behind the contrac |
| ht-1ip.8 | task | CLI summary commands and escaped rendering | Spec §4,§5. herdr-threads summary <thread> (Ready: cover blocks rendered as level/range header + instructions/ |
| ht-1ip.9 | task | Recovery hook text: hot threads on compact/resume/clear and summary hint on join | Spec §9. Hot-thread query (pending receipt/attention for the seat, or message newer than hot_window), hook lif |
| ht-1ip.10 | task | Claude SessionStart compact: capture native evidence and admit it in the recipe | Spec §9. Capture a real Claude Code SessionStart payload with source=compact for the installed version (docs/e |
| ht-1ip.11 | task | Skill: thread-summary procedure with parallel small-model workers | Spec §4,§9. integrations/skill/SKILL.md gains a 'Thread summaries' section: when to run (hook instruction, joi |
| ht-1ip.12 | task | Spike: composer stash and poke-during-turn native evidence (Claude, Codex) | Spec §10,§12. Native spike under herdr: for each harness, determine whether the composer text can be read (pan |
| ht-1ip.13 | task | Soft-deadline poke: soft point, focus plumbing, eligibility and dispatch | Spec §10. Soft point = effective_deadline - (1 - soft_fraction) * window; scheduler tick selects seats with un |
| ht-1ip.14 | task | Composer stash and poke-during-turn capabilities from spike evidence | Spec §10. Using the spike's findings, add recipe capabilities composer_stash and poke_during_turn per harness/ |
| ht-1ip.15 | task | Configuration smoke: native summary and poke flows on Codex and Claude | Spec §12. On a real daemon under herdr, for each harness (Codex, Claude): trigger the recovery event (Codex co |
| ht-1ip.16 | task | Seam integration: summary flow end to end on a real daemon | Coverage r1 (end-to-end thin path). Integration test against a real daemon with a scripted worker driving the  |
| ht-1ip.17 | task | Seam integration: soft-deadline poke against effective deadlines (fake host) | Coverage r2. Real-store, fake-host integration test of the poke path with the real effective_deadline: a pendi |
| ht-1ip.18 | task | Integration sweep: thread summaries for compaction survival + soft-deadline poke | Root integration sweep (coverage). Verify the goal's main flows end to end: recovery hook -> summary Work -> w |
| ht-1ip.19 | task | review: ht-1ip.1 | Review, fix and merge of ht-1ip.1, tracked separately so ht-1ip.1's dependents can start once it is implemente |
| ht-1ip.20 | task | review: ht-1ip.3 | Review, fix and merge of ht-1ip.3, tracked separately so ht-1ip.3's dependents can start once it is implemente |
| ht-1ip.21 | task | review: ht-1ip.4 | Review, fix and merge of ht-1ip.4, tracked separately so ht-1ip.4's dependents can start once it is implemente |
| ht-1ip.22 | task | review: ht-1ip.8 | Review, fix and merge of ht-1ip.8, tracked separately so ht-1ip.8's dependents can start once it is implemente |
| ht-1ip.23 | task | review: ht-1ip.2 | Review, fix and merge of ht-1ip.2, tracked separately so ht-1ip.2's dependents can start once it is implemente |
| ht-1ip.24 | task | review: ht-1ip.6 | Review, fix and merge of ht-1ip.6, tracked separately so ht-1ip.6's dependents can start once it is implemente |
| ht-1ip.25 | task | review: ht-1ip.5 | Review, fix and merge of ht-1ip.5, tracked separately so ht-1ip.5's dependents can start once it is implemente |
| ht-1ip.26 | task | review: ht-1ip.11 | Review, fix and merge of ht-1ip.11, tracked separately so ht-1ip.11's dependents can start once it is implemen |
| ht-1ip.27 | task | review: ht-1ip.13 | Review, fix and merge of ht-1ip.13, tracked separately so ht-1ip.13's dependents can start once it is implemen |
| ht-1ip.28 | task | review: ht-1ip.9 | Review, fix and merge of ht-1ip.9, tracked separately so ht-1ip.9's dependents can start once it is implemente |
| ht-1ip.29 | task | review: ht-1ip.7 | Review, fix and merge of ht-1ip.7, tracked separately so ht-1ip.7's dependents can start once it is implemente |
| ht-1ip.30 | task | review: ht-1ip.17 | Review, fix and merge of ht-1ip.17, tracked separately so ht-1ip.17's dependents can start once it is implemen |
| ht-1ip.31 | task | review: ht-1ip.16 | Review, fix and merge of ht-1ip.16, tracked separately so ht-1ip.16's dependents can start once it is implemen |
| ht-1ip.32 | bug | Extension-lapse scan never finishes a pass (cursor reset, permanent continuation | Phase-3 final review F2 (must fix). scan_due resets the whole cursor including extension_after whenever it fin |
| ht-1ip.33 | bug | Long open user instruction has no working retrieval command in Ready | Phase-3 final review F3. An open priority instruction over 2 KiB renders as `(long; herdr-threads read THREAD  |
| ht-1ip.34 | task | review: ht-1ip.33 | Review, fix and merge of ht-1ip.33, tracked separately so ht-1ip.33's dependents can start once it is implemen |
| ht-1ip.35 | task | review: ht-1ip.32 | Review, fix and merge of ht-1ip.32, tracked separately so ht-1ip.32's dependents can start once it is implemen |
| ht-1ip.36 | task | review: ht-1ip.14 | Review, fix and merge of ht-1ip.14, tracked separately so ht-1ip.14's dependents can start once it is implemen |
| ht-1ip.37 | bug | Fix failing test shared_reuse_by_second_seat (catch_up row count) | F-A. tests/store/summary.rs:575 `shared_reuse_by_second_seat` fails (left 1, right 0): it asserts zero catch_u |
| ht-1ip.38 | bug | Poke collection: exclude never-reservable seats and back off when reserve_poke r | F-B. poke::collect (src/store/poke.rs) returns human-bound, unresolved and targetless seats that wake::current |
| ht-1ip.39 | bug | Composer read: ambiguous Claude placeholder is Unknown (never poke over a draft) | F-C. src/harness/composer.rs:145 claude_empty treats any single-row `Try "..."` draft as the empty placeholder |
| ht-1ip.40 | task | Spec note: dropped fold transitions are not logged | F-D. Spec §5 says dropped fold transitions are logged; bundle_fold drops them silently (no logging facility in |
| ht-1ip.41 | task | review: ht-1ip.37 | Review, fix and merge of ht-1ip.37, tracked separately so ht-1ip.37's dependents can start once it is implemen |
| ht-1ip.42 | task | review: ht-1ip.39 | Review, fix and merge of ht-1ip.39, tracked separately so ht-1ip.39's dependents can start once it is implemen |
| ht-1ip.43 | task | review: ht-1ip.40 | Review, fix and merge of ht-1ip.40, tracked separately so ht-1ip.40's dependents can start once it is implemen |
| ht-1ip.44 | task | review: ht-1ip.38 | Review, fix and merge of ht-1ip.38, tracked separately so ht-1ip.38's dependents can start once it is implemen |
| ht-1ip.45 | task | review: ht-1ip.15 | Review, fix and merge of ht-1ip.15, tracked separately so ht-1ip.15's dependents can start once it is implemen |
| ht-1ip.46 | bug | Fix: composer classification by rendered output (wrap width, Claude suggestion g | Covers: [Should-fix] src/harness/composer.rs:122; bead ht-jf3 (Claude prompt-suggestion ghost text read as a t |
| ht-1ip.47 | bug | Fix: poke admission uses only spec §10 limits and filters before truncating | Covers: [Nit] src/scheduler/mod.rs:259; bead ht-2i4 (soft poke starved by the wake retry backoff) |
| ht-1ip.48 | bug | Fix: summary identifiers and fold size are bounded and fully counted | Covers: [Should-fix] src/summary/identifiers.rs:105, [Nit] src/summary/fold.rs:202 |
| ht-1ip.49 | bug | Fix: bump PROTOCOL_VERSION for the summary wire commands | Covers: [Should-fix] src/protocol/wire.rs:15 |
| ht-1ip.50 | bug | Fix: agent-facing docs and recovery text name the shipped summary procedure | Covers: [Should-fix] docs/agent-usage.md:73; bead ht-dtq (context-reset hook text names a skill section setup  |
| ht-1ip.51 | bug | Fix: priority citations count only ordinary messages | Covers: [Nit] src/store/summary.rs:1548 |
| ht-1ip.52 | bug | Fix: author_role backfill test covers the binding start bound and multiple bindi | Covers: [Nit] tests/store/schema.rs:3278 |
| ht-1ip.53 | bug | Sweep fix: skew/wire-compat tests after the protocol-3 bump | Covers: daemon_skew::skew_tests_use_the_real_release_pair, wire_compat::old_protocol1_cli_gets_decodable_skew_ |
| ht-1ip.54 | bug | Sweep fix: summary_flow spawn sites owned by the leak guard (and stay within the | Covers: no_leaks::every_test_spawn_site_is_owned |
| ht-1ip.55 | bug | Sweep fix: Codex reattachment wake refused as unsafe after the main merge | Covers: trust_policy::codex_reattachment_without_herdr_hint_then_wake |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-1ip.2 | ht-1ip.1 | boundary contract |
| ht-1ip.3 | ht-1ip.1 | boundary contract |
| ht-1ip.4 | ht-1ip.1 | boundary contract |
| ht-1ip.5 | ht-1ip.1 | boundary contract |
| ht-1ip.5 | ht-1ip.4 | submission validator and carry-forward merge |
| ht-1ip.5 | ht-1ip.3 | chunk boundary function and cover algorithm |
| ht-1ip.6 | ht-1ip.1 | boundary contract |
| ht-1ip.7 | ht-1ip.6 | catch-up lifecycle functions (enter_or_keep, on_ready, on_progress, stall scan) |
| ht-1ip.7 | ht-1ip.1 | boundary contract |
| ht-1ip.8 | ht-1ip.1 | boundary contract |
| ht-1ip.9 | ht-1ip.3 | chunk boundary function (full-chunk test for the join hint) |
| ht-1ip.9 | ht-1ip.1 | boundary contract |
| ht-1ip.11 | ht-1ip.5 | job bundle format and submission acceptance behaviour (validator rules surfaced  |
| ht-1ip.11 | ht-1ip.8 | summary job/submit CLI shapes (JSON bundle on stdout, submission JSON on stdin) |
| ht-1ip.13 | ht-1ip.1 | boundary contract |
| ht-1ip.14 | ht-1ip.12 | poke capability evidence |
| ht-1ip.14 | ht-1ip.13 | poke eligibility and stash hook |
| ht-1ip.15 | ht-1ip.2 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.11 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.6 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.5 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.38 | never-reservable poke filter |
| ht-1ip.15 | ht-1ip.9 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.10 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.8 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.14 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.7 | native-ready implementation of its flow |
| ht-1ip.15 | ht-1ip.39 | safe Claude composer classification |
| ht-1ip.16 | ht-1ip.5 | summary handlers |
| ht-1ip.16 | ht-1ip.8 | CLI summary commands |
| ht-1ip.16 | ht-1ip.6 | catch-up lifecycle and hold |
| ht-1ip.16 | ht-1ip.9 | recovery hook text |
| ht-1ip.16 | ht-1ip.7 | effective deadline and extension |
| ht-1ip.16 | ht-1ip.2 | author_role/relays_user population |
| ht-1ip.17 | ht-1ip.7 | effective deadline and extension hooks |
| ht-1ip.17 | ht-1ip.13 | soft-point scheduling, eligibility and stash hook |
| ht-1ip.18 | ht-1ip.2 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.39 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.7 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.32 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.8 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.15 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.33 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.1 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.11 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.40 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.9 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.10 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.14 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.4 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.38 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.16 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.13 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.12 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.3 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.5 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.17 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.6 | all leaves (integration sweep) |
| ht-1ip.18 | ht-1ip.37 | all leaves (integration sweep) |
| ht-1ip.19 | ht-1ip.1 | unstated |
| ht-1ip.20 | ht-1ip.3 | unstated |
| ht-1ip.21 | ht-1ip.4 | unstated |
| ht-1ip.22 | ht-1ip.8 | unstated |
| ht-1ip.23 | ht-1ip.2 | unstated |
| ht-1ip.24 | ht-1ip.6 | unstated |
| ht-1ip.25 | ht-1ip.5 | unstated |
| ht-1ip.26 | ht-1ip.11 | unstated |
| ht-1ip.27 | ht-1ip.13 | unstated |
| ht-1ip.28 | ht-1ip.9 | unstated |
| ht-1ip.29 | ht-1ip.7 | unstated |
| ht-1ip.30 | ht-1ip.17 | unstated |
| ht-1ip.31 | ht-1ip.16 | unstated |
| ht-1ip.34 | ht-1ip.33 | unstated |
| ht-1ip.35 | ht-1ip.32 | unstated |
| ht-1ip.36 | ht-1ip.14 | unstated |
| ht-1ip.41 | ht-1ip.37 | unstated |
| ht-1ip.42 | ht-1ip.39 | unstated |
| ht-1ip.43 | ht-1ip.40 | unstated |
| ht-1ip.44 | ht-1ip.38 | unstated |
| ht-1ip.45 | ht-1ip.15 | unstated |

## Already fixed — do not re-litigate
none

## Not established
- The coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing. Every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is therefore a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists. Invocations 4–6 also show the corrupted reads from Defect 1, so their Metrics lines are not trustworthy.
- I have not established why the safeguard flagged the read. Size and verbatim reproduction are a hypothesis, not a measured cause.
- Both the analyst and the orchestrator ran on Opus 5.5. Every judge panel was same-family (Claude), so panel agreement is not independent verification.
- These are single-run observations. Defects 1, 2 and 5 reproduce structurally from the code cited. Defects 3 and 4 are reviewer-behaviour patterns seen in one run.
- I make no speed claims. Detector peaks were 7/8, 3/8, 4/8, 7/8, 1/8 and 3/8, the edge audits found the run graph-bound, and no speedup from any suggested fix was measured.

## Verification bar
- Replay a ledger of 40 KB or more through Resume and Finish. Assert that the reconstructed `completed` equals the number of complete lines, and that a truncated or empty echo produces UNAVAILABLE rather than counts.
- In a dryRun with 3+ deferred minors of one class under different wordings, assert that a `Recurring minor:` line is emitted, including across a relaunch.
- Run a super-auto run whose orchestrator appends to friction.md during phase 3, and check that no merge is refused.
- Run report-status on a run.md with a `resolves-on: sweep` record once a stamped sweep exists; the qualifier should be gone.
- Do a live launch of super-code and super-roast from a plugin-cache skillsRoot in Claude Code.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
