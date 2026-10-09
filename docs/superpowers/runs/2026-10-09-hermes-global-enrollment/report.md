# super-auto report — 2026-10-09-hermes-global-enrollment

status: clean [degraded: final review: not ready (decision needed on nested/headless different-harness binding replacement — no TRUST-POLICY accepted-limit sentence; startup_enrollment_hermes_.* outside the serializing nextest override; branch unmeasured at review time, since measured green)]
metrics: parked draft docs/superpowers/runs/2026-10-09-hermes-global-enrollment/upstream-feedback-draft.md (unfiled; 2 defects, 2 design questions)

Branch `super-auto/hermes-global-enrollment` @ tip, base `main` (absorbed at 3264587b). Full suite green at e89d816e: 4465 passed, 0 failed, 180 s wall, leak check clean (run.md `codeBuckets.sweep`, `sweepFix`).

## Decision for the human (before merge)

- **Nested / headless different-harness binding replacement.** Headless Hermes one-shots (`hermes -z`, `chat -q`) report `platform="cli"`; checked in the local Hermes source at `hermes_cli/oneshot.py:583`. So they carry the enrollment witness. Combined with ht-4ne.4's broadened context retire (`lifecycle_check_in`, `saved.harness != event.harness`, spec D10), this has three effects:
  - a Claude or Codex agent that runs `hermes -z …` in its own pane has its binding replaced by that run;
  - a plain shell pane gets a lasting seat;
  - Claude and Codex now replace rather than conflict with each other in one pane.

  Code roast panels rejected this: r1 3/3 on the nested case, r2 2/3 on the one-shot case. Their reasons were the cooperative model and that `claude -p` already behaves this way. The final review still asks for, at minimum, one accepted-limit sentence in TRUST-POLICY.md's Hermes/A4 text. The other option is narrowing the retire rule. Not changed by the run. (source: final review, run.md `codeBuckets.review`; roast-pr-1.md and roast-pr-2.md `## Rejected`)

## Implemented

- ht-4ne.1 seam contract: the `enrollment_witness` envelope field, `EnrollmentPolicy`/`EnrollmentKind`, and the bridge refresh with a stale-bridge hint in setup-status. Commits 5e994cd..8adc2f1. (source: ledger)
- ht-4ne.4 behavior: a witnessed cli `on_session_start` enrolls via `enroll_pane` with no check-in; the Unowned-turn fallback; cross-harness context retire; TRUST-POLICY in the same commit. Commits 5db6bd5..c916930. (source: ledger)
- ht-4ne.2 user-facing docs. Commits 32cd5ac..26f9874. (source: ledger)
- ht-4ne.3 configuration smoke across 11 configurations in an isolated Herdr. Commits 26f9874..64b6503. (source: ledger)
- ht-4ne.5 integration sweep tests. Commits 64b6503..3ca3da6. (source: ledger)
- ht-4ne.10 fix: stale launch-only text (`docs/compatibility/README.md`, `adapter.rs` comment). Commits 71146f7..6e5626b. (source: ledger; roast-pr-1 findings)
- ht-4ne.11 fix: tests for operator retire → re-enroll, stale-hint absence, and a visible smoke skip. Commits fef9388..2094bf9. (source: ledger; final-review untested scope)
- ht-4ne.12 sweep fix: smoke-test spawn sites owned via `test_support::spawn`. Commits d692e6d..e89d816. (source: ledger; run.md `sweepFix`)

## Remaining

- [Should-fix] `.config/nextest.toml:101`: the `startup_enrollment_hermes_*` hook tests run against the real 1.2 s cap but are outside the serializing override. Out of scope (filtered): test-scheduling hardening. (source: run.md `scopeFilter-round-1`)
- [Nit] `tests/cli/hook.rs:8260`: the concurrent start/first-turn test can't detect a double allocation (constant fixture seat, no upper bound). Out of scope (filtered): test strength. (source: run.md `scopeFilter-round-1`)
- Cluster dropped, `hermes-hook-cap-isolation`: widen the nextest override, narrow the concurrent-test retry, detect double allocation. This is the shared cause behind the two items above, plus the final review's `UnknownOutcome` retry concern. (source: run.md `cluster dropped:`)
- final review: the nested/headless different-harness binding replacement decision (above). (source: run.md `codeBuckets.review`)
- final review: the Python bridge tests (`integrations/hermes/test_bridge.py`, 38 tests) are not run by cargo/CI. This predates the branch. (source: final review)
- final review: no start-vs-first-turn race test against a real daemon (only start-vs-start). (source: final review)
- Out of scope by design (spec D9 / Follow-on): a live real-Hermes e2e (ht-9pj); wake via `agent_session` (ht-77b); resume continuity (ht-3u6); `/background` side agents (ht-e73); Linux / `setproctitle` wake recognition. Hermes claims were checked against the local install, not the pinned 0.21.5 release. (source: spec Follow-on; final review)
- Conflict-resolved hunks no roast reviewed: none. The base absorption merged with 0 conflicts. (source: run.md `baseAbsorbed`)

## Gotchas & surprises

- Bare Hermes is not process-detected by Herdr on macOS: it runs as `python3`, without `setproctitle`. Herdr's plugin sets only `agent_session`, which herdr-threads doesn't read. So enrollment had to come from the Hermes hooks themselves. (source: `bare-hermes-detection-evidence.md`)
- Design roast round 1 changed the daemon-start decision: the fallback keeps the turn path's existing `ensure_running`. (source: run.md `stepBackDesign-round-1`)
- Design roast round 2 added the cross-harness context-retire fix, which is the source of the decision above. (source: run.md `roast-design exit`)
- Code step-back round 1: patch, two clusters, no redesign. (source: run.md `stepBackCode-round-1`)
- Slowness: the graph was the binding constraint (depth 4, achievable width 2 vs cap 8); the edge audit found no removable edge. (source: run.md `codeBuckets.slowness`; ledger `Edge audit:`)
- No run profile was written by super-code's Finish, so per-bead timing is not preserved. (source: friction.md)

## Entrypoints

1. `src/harness/adapter.rs`: `EnrollmentPolicy`, `EnrollmentKind`, `DecodedEvent::enrollment`, `EventIntent::Enroll`.
2. `src/harness/hermes.rs`: strict parse/decode of `enrollment_witness`, and `enrollment_policy()`.
3. `integrations/hermes/__init__.py`: where the bridge emits the witness on cli `on_session_start`. Fixture: `tests/fixtures/hermes/enrollment-envelopes.json`.
4. `src/cli/hook.rs`: `enroll_pane`, `check_in` with `Enroll::IfUnowned`, and `lifecycle_check_in` cross-harness retire (~2476).
5. `TRUST-POLICY.md`: the Hermes enrollment bullet, the accepted limits and A4.

(source: task tree order and `git diff main...branch`)

## Smells

- Sweep-fix ht-4ne.12 merged with no re-roast. The re-run at e89d816e is green: 4465/4465. (source: run.md `sweepFix`)
- `src/cli/hook.rs`: `enroll_pane` duplicates `check_in`'s resolver block, including the "mapping changed; retry" string, and that guard has no test (roast-pr-1 spot-confirmed Nits). (source: roast-pr-1.md `## Unverified nits`)
- The concurrent start/first-turn test retries on empty output after ≥1 s, masking one observed check-in `UnknownOutcome` whose cause is unconfirmed. The roast spot-rejected this as a deliberate tolerance; the final review graded it must-fix. (source: roast-pr-1.md; final review)
- No parked findings, no `DONE_WITH_CONCERNS` reports, no task needed a fix pass. All 8 reviews were clean on first pass. (source: ledger `Metrics:`)
