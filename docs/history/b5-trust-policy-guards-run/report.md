status: completed with 0 unresolved Blocking, 2 escalations [degraded: coverage widening (round 2 fixes not re-reviewed), seam integration folded into sweep, final review: not ready, sweep: FAIL c2118486 (1 load-sensitive test; re-runs clean)]
metrics: https://github.com/alepar/superpowers/issues/10

# B5 trust policy guards — run report (2026-10-01)

Run state: `run.md` (this directory). Epic `ht-rzi`, branch `trust-model-invariants`, base `main@55512edb`.
Normative design: `TRUST-POLICY.md`; implementation design: `2026-10-01-b5-trust-policy-guards-design.md`.
Ledger: `ledger/progress.md` (copied from the integration worktree at teardown).

## Implemented

Source: beads closed under `ht-rzi`; `run.md` `codeBuckets` and `fixLoop-round-1`; ledger completion lines.

| Bead | What landed | Ledger commits |
|---|---|---|
| ht-rzi.9 | Herdr pane-agent observation port (kind, agent_session, read-error distinct) + stand-in fake | aab8104..4bb1a23 |
| ht-rzi.1 | C2/C3: `lift_baseline_hold_if_clear` (flag + `recovery_holds`, reconciliation marker), `seat retire --operator`, `seat rebind --replace --operator`, refusal argv, migration 0010 | e065d99..874b4c5 |
| ht-rzi.2 | C1 cooperative continuity: resume-only, `native_session` match, Herdr session as diagnostic only, evidence note in docs/compatibility | de6bcaa..f6ba883 |
| ht-rzi.3 | A4: daemon refuses agent→human lifecycle check-in without `--operator`; `me init` marker/Herdr checks | f6ba883..ac0bc55 |
| ht-rzi.4 | A4: launch refuses a second agent; wake only to the bound harness; `codex resume` launch form refused | 0754671..58821d3 |
| ht-rzi.5 | A2 expected daemon boot (pre-dispatch refusal); C4 binding carry-forward | 6cd3340..77961f8 |
| ht-rzi.6 | `allocator.lock` O_NOFOLLOW, self-marker wording, id comments | 8b9223e..67f0158 |
| ht-rzi.7 | Docs: README, operations, agent-usage, TRUST-POLICY status flips (C5 left to B4) | 9db6474..296aab9 |
| ht-rzi.8 | Integration sweep: F6 walking-skeleton and cross-bead tests (`tests/integration/trust_policy.rs`) | 8425be2..b2e380a |
| ht-rzi.18 | Fix r1 (redesign): continuity decided in one transaction that opens the successor binding; per-event intent scan removed; retryable before reconciliation | 26a87cf..560292d |
| ht-rzi.19 | Fix r1: carry-forward complete (provenance set, availability anchor, receipt timers, pending carry); marker-wiring test | 51a705d..26a87cf |
| ht-rzi.20 | Fix r1: diagnostic read off the fenced path, no NativeCli epoch bump | 775ccdd..df3b96a |
| ht-rzi.21 | Fix r1: one open-binding query for launch and wake; no-binding wake refused | 46d2817..905c017 |
| ht-rzi.22 | Fix r1: single client agent-evidence rule (3-variable allowlist), honored `--operator` | c3703fe..4d4a5d2 (fix pass) |
| ht-rzi.23 | Fix r1: PROTOCOL_VERSION 2 with client-side check; skew docs | 59ae47c..d2bb964 |
| ht-rzi.24 | Sweep fix: continuity retry test no longer depends on wall-clock budget | b6f0831..cdff672 |

## Remaining

- **Sweep:** FAIL c2118486 — 1 of 1611 tests run: service resolution::identity_final_currentness_check_… (capture-overlap assertion) under heavy load from 249 leaked test daemons; 10/10 clean re-runs of the service target on a quiet machine (main 4/4); classified load-sensitive, not fixed (sweep-fix pass already spent). First sweep at fce6e130 failed the continuity retry test (wall-clock budget) → fixed by ht-rzi.24. `hook_entrypoint` `three_thread_startup_…` / `twenty_thousand_…` fail on `main` but passed in the final sweep.
- **Leaked test processes:** 244 orphaned test daemons from this run's task worktrees and 4 private test Herdr servers were killed during phase 6; bead `ht-6y1` tracks the fixture leak.

Source: `run.md` `parked:`, `scopeFilter-round-1`, `fixLoop-exit`; parked beads (label `parked:ht-rzi`).

- **Escalation (design roast 1): Herdr hint role for ht-rzi.2.** You were asked to choose diagnostic-only / re-read / mandatory; the session Stop hook forbade pausing, so option 1 (diagnostic-only, recommended) was applied as an assumption. Overturn if you disagree.
- **Escalation (design roast 1): Claude Code #24265** (resume may emit startup(new id)+resume(orig id)); unverified on current versions. Resume-only gating covers the startup-first order; native capture for interactive Claude 2.1.287, Codex 0.159.3 resume and `herdr agent get` during resume is still not captured.
- Punch list from code roast round 2 (converged), filed as parked beads:
  - `ht-kqz` [Should-fix] reused continuity intent replays an old reattachment as success (`src/cli/hook.rs:1029`).
  - `ht-6ry` [Nit, fix-regression] `scripts/install.sh` stops the old daemon only after swapping in the protocol-2 binary.
  - `ht-4rt` small follow-ups (hook protocol check, retry doc, ids.rs "10 bits" comment, committed-but-not-installed diagnostic, intent-scan hardening).
- From the super-code final review (fix loop round 1), parked as `ht-p63`: same-pane-id resume while the daemon keeps running may strand the seat; untested.
- `[FYI] src/cli/journal.rs:769` — out of scope (filtered): pre-existing unguarded intent scan; goal names only `allocator.lock` (in `ht-4rt`).
- Coverage round 2 fixes (C15–C28) were never re-reviewed (widening: yes); the design roast and integration sweep absorbed them.
- P10 / W5-1 deletions belong to the B4 run (`ht-p03.2`), relayed to that session.
- **Merge coordination:** migration `0010` / schema v10 will collide with B4 (`super-auto/remaining-herdr-threads-findings`, at 0009); B4 also edits `seats.rs`, `ports.rs`, `reconcile.rs`. Whichever lands second renumbers.
- Graph pass: 4 candidates, all kept (`graph-pass:` line in run.md); none parked.

## Gotchas & surprises

- Run material trimmed before merge: engine/coordinator script snapshots, roast args JSON and tree dumps were removed from the branch (reproducible from the skill SHAs in run.md); reports, specs, ledgers and run.md remain.

Source: roast reports and step-back files in this directory; `run.md` stepBack lines; ledger `Slowness:`; `friction.md`.

- Design roast 1 (Blocking): the C2 lift had to release per-target `recovery_holds` rows, not only the instance flag; redesign → one lift helper behind a persisted reconciliation marker (`…-roast-design-1-step-back.md`).
- Design roast 1: Herdr 0.9.1 exposes no `session_start_source` on reads; `occupant_bindings.native_session` already existed (no new column).
- Code roast 1 step-back redesign: the implemented two-request continuity (seatless decide + follow-up check-in + every-event intent replay) stranded seats when the pane differed from the saved context; replaced by a single deciding transaction (`…-roast-pr-1-step-back.md`).
- ht-rzi.3 was quarantined because the planner noted, but did not apply, a missing edge on ht-rzi.1's migration; fixed by hand (blockers `ht-8rx`, `ht-ka1`).
- The 4.3 coordinator's planner hung (no first response, 6×15 min); run switched skill source 4.3 → 4.5 → 4.6 at round boundaries (`run.md` `skillSource:`).
- A full-suite preview run concurrently with a 154-agent roast produced 13 false failures (harness `--version` timeouts); the phase-6 sweep ran alone.

## Entrypoints

Source: dependency order of the bead tree.

1. `TRUST-POLICY.md` — the invariants (C1–C5, A1–A5) every change below implements.
2. `migrations/0010_b5_trust_guards.sql`, then `src/store/seats.rs` (`lift_baseline_hold_if_clear`, retire/replace, `decide_continuity`, carry-forward) and `src/store/effective.rs`.
3. `src/ports.rs` / `src/host/native.rs` — pane-agent observation port.
4. `src/identity/repair.rs` and `src/service/dispatch.rs` — the daemon-side continuity and A4 decisions.
5. `src/cli/hook.rs` (seatless resume path), `src/cli/me.rs` / `src/cli/mod.rs` (A4 client rule), `src/cli/launch.rs`, `src/notification/dispatch.rs` (wake).
6. `tests/integration/trust_policy.rs` — the end-to-end F6 scenario.

## Smells

Source: ledger (fix passes, deferred minors), parked records in `run.md`, final reviews.

- ht-rzi.22 needed a fix pass and merged it without re-review (ledger `Metrics: fix-pass — entered 1 · FIXED 1`).
- Final reviews (both coordinator runs) flagged that most "review clean" verdicts rest on implementers' reports: tests were written after code in 7 tasks and reviewers did not execute tests in 6. The phase-6 sweep is the first independent execution.
- Parked degraded-verdict: coverage widening; seam integration for ht-rzi.9 folded into ht-rzi.8.
- Parked assumption: the Herdr hint decision (escalation above) was made for you.
- `hook_entrypoint` `three_thread_startup_…` and `twenty_thousand_…` fail on the branch and on `main@55512edb` alike (pre-existing, load-sensitive); `ten_thousand_…` failed once under load in the preview.
