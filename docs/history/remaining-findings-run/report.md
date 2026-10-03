# Report: remaining herdr-threads findings (B1–B4, B6–B10)

status: completed with 0 unresolved Blocking, 0 escalations [degraded: code findings parked, final review: final reviews of launches 3–7 "not ready" at their tips, every must-fix since merged (detached-HEAD ref, default-feature build, setup help, Codex trust screen, .141), sweep: PASS @a039f948 (pre-sweep: full serial suite all-targets all-features + default-feature clippy); per user decision 2026-10-02 no later full-suite run — commits after a039f948 (build-speedup, ht-p03.20 runner/recipe rows/evidence) covered by clippy, default-feature guard, fmt and focused tests (version/admission 50, combined 162) at 23236aaa]
metrics: docs/history/remaining-findings-run/upstream-feedback-draft.md (parked draft, not filed; findings also relayed live to the maintainer)

Abbreviations: `RD/` = `docs/history/remaining-findings-run/` (the run directory `docs/superpowers/runs/2026-10-01-remaining-herdr-threads-findings/`, trimmed at merge; the specs are in `docs/design/herdr-threads/remaining-findings/`, and working files not kept here are archived in tag `archive/remaining-findings-run-2026-10-01`, path unchanged there). "Ledger" = `.superpowers/sdd/ht-p03-plan/progress.md`, git-ignored in the run worktree. Base = `main@826a8804`. Status line produced by `super-auto/scripts/report-status` on run.md (escalation count = the one parked design-roast escalation; codeBuckets.escalated and pendingRetry are none).

## Decision summary for the merger

- 0 Blocking findings open. Both roasts converged at round 2: design (10 Should-fix applied) and PR (5 confirmed; the one fix-regression fixed by ht-p03.141). — run.md `roastDesignExit`, `fixLoopExit`
- One open escalation is parked for you: wake-lane durable commits while Herdr is down (see Remaining). — run.md `parked`
- Full-suite evidence is the pre-sweep PASS at `a039f948` only. By your decision of 2026-10-02, later commits had clippy, the default-feature guard, fmt and focused tests only. — run.md `sweepDecision-2026-10-02`, `codeBuckets.sweep`
- Native evidence: `docs/validation/report.md` verdict FAIL with one failing cell (`codex-no-initial-prompt`); 15/16 cells PASS on `62949fe5`. — docs/validation/report.md; RD/native-rerun.json
- Findings closure: 150 findings, of which 137 are closed, 5 deferred and 8 open (G0/B2 and Wave 15[1]–[7], all held by that one Codex cell). — RD/findings-closure.md (`python3 findings-closure-check.py` reports OK)
- CI still runs `--test-threads=1`. The 10-unpinned-runs gate moved to the non-gating side quest ht-zo4, which is in progress. — .github/workflows/ci.yml; bd ht-zo4; RD/findings-closure.md §Deferrals

## Implemented

Every descendant of ht-p03 is closed: 199 beads. 80 are `sp:review` bookkeeping beads, not listed here. The other 119 are work, epic and gate beads. Commit ranges come from the ledger's `complete (commits …)` lines unless another source is named. — `bd list --label sp:ht-p03 --all --json`; run.md `codeBuckets.completed`, `epicClosed`; ledger

**B4: removal of the pre-cooperative verification layer.**
- ht-p03.2 deleted the layer plus the B5-decided P10/W5-1 paths (3b833b2..c4b5edd). The acceptance rg finds nothing at the tip. — ledger; RD/findings-closure.md §B5 non-interference
- ht-p03.3: ports collapsed (f7485dd..9b77987).
- ht-p03.4: ErrorClass plus per-code constructors (e86a8cd..1dc8768).
- ht-p03.22: literal sites converted (36c0691..dfd97e7).
- ht-p03.5: design docs describe the cooperative reality (1d551c7..2cf40e4). — ledger

**B1: retention and live-row discovery.**
- ht-p03.12.1: live and retention indexes, plus `work_jobs.completed_at`. The migration was renumbered `0011_cooperative_only.sql` (v11) at the main merge. — ledger; RD/combine-main-record.md
- ht-p03.12.2–.12.9: linear page fitting, CLI seat page (W6-R1), observation/work discovery on live rows, wake discovery from pending projections (human seats excluded), the retention lane on the Pacer, batched human read (Wave 27), history `full_bodies` with an older-daemon fallback, reserved-only recovery walk. — ledger
- Seams: ht-p03.12.10/.11 (page cursor) and .12.12/.13 (counting fake client). — ledger

**B2: the shared Pacer.**
- ht-p03.7: the Pacer itself.
- ht-p03.9.1–.9.6: lane registry and commit kicks, event-driven cancellation, W9-1 backoff that doesn't climb the ladder, deadline and wake lanes, observation backoff (at most 1 commit per backoff step), the admission observer.
- ht-p03.104: a kick during backoff now triggers observation.
- Seams: ht-p03.39/.40 and .41/.42. — ledger

**B3: diagnostics.**
- ht-p03.10: error taxonomy, skew detection and `remedy()`.
- ht-p03.11: startup log sink and rate-limited lane logs.
- ht-p03.27: truthful Health/doctor and the line budget.
- ht-p03.107: exit-3 remedy chosen by error class (resolved parked ht-p03.46).
- ht-p03.127: `stop` vs the observer join.
- Seams: ht-p03.45/.46. — ledger

**B6: optimistic admission and the canary.**
- ht-p03.13: admission ladder and the source of truth.
- ht-p03.23: version honesty and re-observation.
- ht-p03.49: doctor PATH check.
- ht-p03.35: npm Codex fingerprinted from its vendor binary.
- ht-p03.15/.28/.29/.30: setup, launch, hook-context and wake-submission fixes.
- ht-p03.109 and .139: tests, docs and help updated to the ladder.
- ht-p03.14.1–.14.7: the canary (bisect engine, script, tier 1, scheduled workflow with deduplicated issues, tier-0 evidence, acceptance runs, ubuntu smoke).
- Seams: ht-p03.14.8/.9 and .47/.48.
- Recipe rows Claude Code 2.1.287 and Codex 0.159.3 (70b0ba73, within ht-p03.20). — ledger; git log; run.md `nativeRerun`

**B7: release readiness. The tag push is post-merge.**
- ht-p03.16: workflows, SHA pinning, actionlint.
- ht-p03.31: truthful installer and its test matrix.
- ht-p03.125: the installer reports a failed old-daemon stop.
- ht-p03.32: release docs, CHANGELOG, README.
- ht-p03.21: configuration smoke. Intel macOS was built locally; both musl targets were NOT_EXERCISED because the Docker daemon was down. — ledger; RD/findings-closure.md §Deferrals

**B8: validator, review debt and native rerun.**
- ht-p03.18: validator negative corpus.
- ht-p03.19: review-debt leaf, which filed RD-1..3 as ht-p03.121/.122/.123, all fixed.
- ht-p03.37: findings closure ledger plus the B5 non-interference check.
- ht-p03.38: early smoke.
- ht-p03.140: driver accepts the Codex trust screen for /private/tmp scratch only; Codex pinned to 0.159.3.
- ht-p03.20: native matrix, merged 23236aaa, evidence SHA 62949fe5, 15/16 PASS. — ledger; run.md `nativeRerun`

**B9: test isolation.**
- ht-p03.1: isolated Herdr fixture.
- ht-p03.6: isolation helper.
- ht-p03.8: hook_entrypoint exit 101 root-caused as a test-only fd dup2 race.
- ht-p03.24/.25: suite migration and stronger tests.
- ht-p03.26: partial (19f76cc1), with Python suites and install_test added to CI. The serial pin stays.
- ht-p03.131: tests reap every spawned process, plus a post-suite leak check.
- ht-p03.138: default-feature builds work. — ledger; run.md

**B10: presentation and escaping.**
- ht-p03.17: contract table and goldens.
- ht-p03.33: shared escaping with display width.
- ht-p03.34: follow/IRC fixes.
- ht-p03.108: minors. — ledger

**Cross-cutting.**
- ht-p03.36: operator and design docs.
- ht-p03.50: specs reconciled.
- ht-p03.51: integration sweep.
- ht-p03.105: advertise only implemented capabilities.
- ht-p03.106: stray backups removed.
- ht-p03.126: pane_seat finds the resolved seat.
- ht-p03.135/.136: PR-roast round-1 clusters (advisory wake verification; docs match shipped behavior).
- ht-p03.141: round-2 regression-only pass.
- ht-p03.52–.70: 19 design-fix beads, applied to the specs.
- ht-p03.134: gate.
- Main merged in: eccfc030 plus fixes, 47 files, 101 hunks. — RD/combine-main-record.md
- Build speedup: 285b5f36, bd ht-2v6. — docs/dev/build-speed.md

**Ledger counts.** The ledger's own Finish metrics were UNAVAILABLE because read-ledger kept failing, so these were rebuilt by grep:
- 92 `Merge:` lines covering 91 beads.
- Rebase: 82 clean, 10 conflict.
- Seam review: 53 none, 28 cleared, 11 fixed.
- Merge check: 89 pass, 1 fail→fixed, 2 →blocker. The two blockers are both ht-p03.141, which was then merged by hand.
- 5 fix passes. — ledger

## Remaining

**Escalation and parked items.**
- **Escalation (resolved by you 2026-10-03: freeze everything while Herdr is down → follow-up ht-72q):** "pacer D5 vs root §B2 D2: wake-lane durable commits while Herdr is down" (panel 1 CONFIRM Nit / 2 REJECT). The spec bounds only the observation lane. ht-p03.9.4's test pins the wake lane at ≤ 2 commits per refused seat per backoff step. Accept that cost, or ask for its own bound. — run.md `parked`; RD/…-roast-design-1.md §Escalations
- **Parked graph changes:** 4 (`graph-pass: depth 13→13 · width 5.8→5.8 · applied 2 · parked 4`). These were drop ht-p03.41←.3, drop .19←.32, split ht-p03.9.6, and a seam contract `WakeDriveOutcome::next_due_at`. All are moot now that the epic is closed. — run.md; RD/graph-pass-judgement.md

**PR-roast punch list.** 7 items, none filed as a bead.
- out of scope (filtered): [FYI] src/harness/codex_config.rs:318: `unix_sockets` user entries are neither recorded nor warned about. Reason: pre-existing; extra hardening. — run.md `scopeFilter-round-1`
- out of scope (filtered): [Nit] src/host/native.rs:1024: a 250 ms settle holds the serial wake loop on each sent prompt. Reason: performance polish (the settle is now cancellable). — same
- out of scope (filtered): [Nit] src/store/schema.rs:542: the v10→v11 step stamps `LATEST_VERSION` instead of the literal 11. Reason: latent hygiene. — same
- out of scope (filtered): [Nit] src/cli/doctor.rs:493: CHANGELOG bullet. Reason: changelog completeness. Resolved in round 2 (637bd050). — same
- Round-2 Nit: src/notification/dispatch.rs:243: post-send verification runs inside the watchdog, so a stop or a slow read records OutcomeUnknown for a delivered prompt. — RD/…-roast-pr-2.md
- Round-2 unverified nit: tests/host/wake_submission.rs:116: the tail-window test passes for any window. — same
- Round-2 unverified nit: CHANGELOG.md:13 omits the moved `hooks.claude.settings` and `hooks.claude.adopted` keys. — same

**Findings still open or deferred.**
- **Open:** G0/B2 and Wave 15[1]–[7], held by `codex-no-initial-prompt`. On Codex 0.159.3 the TUI runs SessionStart only at the first turn (bd ht-5n6). — RD/findings-closure.md; docs/validation/report.md
- **Deferred:**
  - the serial pin and the named flakes, moved to ht-zo4
  - the Linux musl compile and the Herdr link proof; the first post-merge CI `build-targets` run proves them
  - the token-overhead benchmark
  - two remainder-capped roast candidates
  - the root-spec explicit deferrals (canary keepalive, the last idle commit, adapter PRs, canary tier 1 in CI, configurable retention, settled-warning pruning, all of B5)

  — RD/findings-closure.md §Deferrals; RD/…-design.md §Explicit deferrals

**Code buckets.**
- escalated: none.
- pendingRetry: none. The BLOCKED-AUTH episodes on ht-p03.49/.31/.12.7/.12.8/.20 all recovered.
- No `roastDesignCapped` or `roastCodeCapped`.
- Process sweeps: 0 survivors. — run.md `codeBuckets`, `escalationNote`; ledger

**Beads filed outside the tree.**
- ht-5n6 (open): Codex 0.159.3 TUI makes no startup check-in without a prompt.
- ht-4p6 (open): the native runner's `harness_ran` detector exits under `set -e`. The fix is not committed, because that would stale the matrix.
- ht-zo4 (in progress): the flakiness side quest.
- ht-5nb (open epic): the herdr-graph service-send/read amendment, which lands after this merge.
- ht-2v6 (closed): build speedup.

— bd

**Known limits from the main merge.**
- The installer cannot stop a daemon started by a named-session Herdr when `herdr` is off PATH. This is pre-existing.
- A store created at this unreleased branch's old v10 numbering is refused.
- Run the suite with stdin from `/dev/null`, as CI does.

— RD/combine-main-record.md §Unresolved

**Post-merge obligations.**
- Notify the trust-model session to start ht-xoc, and mention that migrations 0010 = B5 and 0011 = ours (schema v11).
- Start ht-5nb.
- Watch the first CI run on main, including `build-targets`.
- Optionally add the canary secrets and dispatch the canary once.
- Push `v0.1.0` and test the live release, including a clean-machine install/upgrade/uninstall with Herdr up and down.
- Run CI on two OSes unpinned once ht-zo4 lands.
- The B5 owner may mark TRUST-POLICY.md lines 5–6 implemented.

— run.md `postMergeNotify`; root spec §Follow-on

## Gotchas & surprises

- **Design roasts.** Round 1 had 14 Should-fix; the step-back said "patch" and they were fixed as ht-p03.52–.60. Round 2 had 10, fixed inline as ht-p03.61–.70: skew-tolerant stop, the ladder refusing versions older than every recipe, the pinned Claude path, observation not kicking wakes. — RD/…-roast-design-1/2.md
- **Coverage rounds reshaped the tree heavily.** Round 1: 95 findings, depth 7→12. Round 2: 66 findings, depth 13. — run.md
- **B5 amendment.** The P10/W5-1 deletions were folded into B4. — run.md `amendment-2026-10-01`
- **Code launches.**
  - Launch 1 died after the planner stalled. The network was unstable, and close-in-tree-epics failed under bd 1.3.0.
  - Launch 2 was stopped to switch to a fixed coordinator.
  - In launch 3, 4 tasks hit BLOCKED-AUTH from bulk conflict-marker stripping. They were re-rebased per hunk.
  - Launch 3's final review caught a regression that per-task "pre-existing" claims had hidden.
  - In launch 4, main moved 82 commits (B5, ht-xoc), leaving a 47-file conflict, a v10 clash and protocol 1 vs 2.

  — run.md; RD/friction.md; RD/combine-main-record.md
- **Leaked processes.** Test runs leaked 24+ processes, which led to P0 ht-p03.131 and a per-task cleanup policy. — run.md `leakFix-2026-10-02`
- **Flakiness made non-gating.** ht-p03.26 couldn't reach 10 green unpinned runs; you made flakiness non-gating and moved the rest to ht-zo4. — run.md `codeLaunch-5-stopped`
- **Detached HEAD twice.** The merge lane left the worktree on a detached HEAD twice, and the coordinator fast-forwarded the branch each time. — run.md `codeLaunch-6/7-result`
- **All-features-only gates.** Gates built only with `--all-features`, so a default-feature break reached the final review (ht-p03.138). — RD/friction.md
- **Native rerun took 3 full runs.** Run 1 was invalid: the pane shell resolved an auto-updated Claude 2.1.288. The ht-p03.20 agent was quarantined once for `pkill -f "cargo test --locked"`. — RD/native-rerun.json; run.md
- **hook_entrypoint exit 101** was a test-only fd race. macOS SUN_LEN needs a short TMPDIR. — root spec §Post-Implementation Notes; RD/integration-sweep-notes.md
- **PR roast round 1.** The step-back said "patch", with 2 clusters, fixed as ht-p03.135/.136. — RD/…-roast-pr-1-step-back.md

## Entrypoints

Read in critical-path order (ht-p03.2 → .3 → .41 → .9.3 → .9.4 → .9.6 → .27 → .23 → .32 → .19 → .37 → .51 → .20):
1. `migrations/0011_cooperative_only.sql` and `src/store/schema.rs`
2. `src/ports.rs` and `src/protocol/results.rs`
3. `src/service/pacer.rs`, `kicks.rs` and `workers.rs`
4. `src/scheduler/mod.rs`, `src/notification/dispatch.rs` and `src/host/native.rs`
5. `src/store/retention.rs`, `page_fit.rs`, `mod.rs` and `queries.rs`
6. `src/daemon/remedy.rs`, `logs.rs` and `lifecycle.rs`
7. `src/harness/admission.rs`, the recipe rows in `src/harness/{claude,codex}.rs`, `docs/compatibility/harness-versions.json` and `src/cli/doctor.rs`
8. `scripts/harness-canary.sh`, `scripts/canary/` and `.github/workflows/harness-canary.yml`
9. `.github/workflows/{ci,release}.yml`, `scripts/install.sh` and `tests/release/install_test.sh`
10. `src/view/escape.rs` and `docs/agent-usage.md`
11. `src/test_support/{isolation,isolated_herdr,spawn}.rs` and `scripts/check-no-leaked-processes`
12. Evidence: `docs/validation/report.md`, `RD/findings-closure.md` and `RD/combine-main-record.md`

— run.md `graph-pass`; `git diff main...HEAD`

## Smells

- **Sweep coverage.** The only full-suite run is at `a039f948`. Recipe rows, runner changes, the build profile and `tests/combined.rs` came after it, and were covered by clippy, the default guard, fmt and focused tests (50 + 162). This was your decision. — run.md `sweepDecision-2026-10-02`
- **Build-speedup merged after the evidence SHA.** It changed only Cargo.toml, tests, a scripts/ guard and docs, with no src/ change. This is a recorded deviation from "evidence SHA = shipped tip". — run.md `buildSpeedup-2026-10-02`
- **claude-managed PASS is an offline re-derivation** from unchanged evidence, after a runner bug (ht-4p6). The coordinator approved it, and it is clearly marked in the report and the JSON. — docs/validation/report.md; run.md `nativeRerun`
- **Codex 0.159.3 recipe row kept** despite the `codex-no-initial-prompt` FAIL. The agent judged it not a core-flow backing cell. You may reverse this. — run.md `nativeRerun`; RD/findings-closure.md
- **Parked finding ht-p03.46**, resolved by ht-p03.107. — run.md `codeBuckets.parked`
- **Fix passes merged without re-review:** ht-p03.12.9, .46, .24, .127, .140. — ledger
- **Seam review fixed at merge time:** 11 beads. ht-p03.2's merge check failed, then was fixed. — ledger
- **ht-p03.141 merged by hand** by the coordinator, with no re-roast (the regression-only pass rule). The same merge included a docs/install.md fix. — run.md `codeLaunch-7-result`
- **Final reviews of launches 3–7** were "not ready" at their tips; every must-fix has merged since. — run.md `codeBuckets.review`
- **376 deferred-minor review notes** in the ledger were never triaged into beads. — ledger
- **The main merge** was the largest single hand edit of the run: 47 files. PR roast round 1 reviewed it afterwards. — RD/combine-main-record.md
- **Wake verification** still runs inside the serial, watchdog-bounded drive loop (open round-2 Nits). — RD/…-roast-pr-2.md
