super-roast verdict: Should-fix (1 confirmed) [low coverage] [review readiness incomplete]
mode: pr        iteration: post-cap audit
profile (assumed): Production internal developer tool. A durable, SQLite-backed coordination store run by a local daemon; seat state, bindings, receipts and continuity evidence are durable data with integrity floors. Users are same-user coding agents and the operator on one machine; no external users, no money. A keyless seat's retirement is final, so a wrong retirement is not rolled back by a restart.
inputs: super-auto/graph-clone-identity@a345c61719f2f64dec83d7be57404e22b6af65c9 vs @88950e620e81a20bb673e281e6850332b24af223 (post-cap audit: final-review fix round ht-ma6i.40-.41)
delta vs prior: 1 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 0 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs, api-contract) · raw 2 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage bounded, provenance unavailable — basis: "post-cap audit of 88950e620e81a20bb673e281e6850332b24af223..a345c61719f2f64dec83d7be57404e22b6af65c9; inspected: every changed path outside docs/superpowers/runs/; unreviewed: run directory; whole-range provenance unavailable since invocation inv-35d46ca1"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0
lane-yield (found/confirmed/unique/refuted): correctness 1/1/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 1/1/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · test-causality-lifecycle 0/0/0/0 · test-isolation-environment 0/0/0/0 · test-suite-cost 0/0/0/0 · hygiene-docs 0/0/0/0 · api-contract 0/0/0/0

Readiness note: the verdict carries `[review readiness incomplete]` and `[low coverage]` because the engine reports coverage bounded and provenance unavailable (the run directory under docs/superpowers/runs/ was not inspected; whole-range provenance has been unavailable since invocation inv-35d46ca1). Stage coverage itself was complete: 14/14 scouts returned, no dead triage, dedupe or judge seat, no panel cap, no remainder cap, judge completion 100%. `[converged]` is not set: the engine reports `convergenceEligible: false` and this is a post-cap audit, not a fix-loop round.

Prior-report tracking: the prior report (iteration 3 of 3) listed no confirmed findings; its two resolved findings (shell fingerprint read fence, src/host/native.rs:1405 and tests/service/observation_lane.rs:197) are not re-surfaced by any lane this round and stay resolved. The one finding below is new: it concerns the G5 claimed-pane rule that the fix round ht-ma6i.41 (commit e712747bc) introduced, and no prior section lists it. Severity: two seats rated Should-fix, the refute seat Nit. Should-fix stands: the profile is a durable coordination store with integrity floors and TRUST-POLICY's own rule forbids ending a seat on heuristic evidence; the refute seat's mitigations (narrow window, keyed retirement reversible) do not cover a keyless seat, whose retirement is final. No floor applies: the seats describe a narrow, untested regression in the unsafe direction, not data loss on real data at scale, and the ground seat explicitly rated it below Blocking.

## Confirmed findings
- [Should-fix] src/store/seats/observation.rs:1443; src/store/seats/observation.rs:1506 — The G5 claimed-pane rule's `terminal_claimed_elsewhere` predicate has no `ended_at IS NULL` filter on `occupant_bindings` and no non-retired seat-state filter on `seats`, so a departed seat's ended binding or a retired seat's retained structural proof makes an unresolved seat's shown target count as absent and retires it (finally, for a keyless seat) after the grace, where before the diff the same state only withheld a watch and kept waiting. [fix-regression] [lanes: correctness, regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/store/seats/observation.rs:1443-1457 at a345c61: `SELECT EXISTS(SELECT 1 FROM seats WHERE id!=?1 AND host_id=?2 AND structural_terminal_id=?3 AND structural_incarnation=?4) OR EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id!=?1 AND terminal_id=?3 AND incarnation=?4)`; no `ended_at IS NULL`, no `state!='retired'`, while other binding reads in the same file filter on `ended_at IS NULL` (lines 269, 1192, 1208, 1987). Lines 1506-1512 now use `own_target_shown = !terminal_claimed_elsewhere(..)`; a true result skips the `TargetShown`/`None` return and enters the absence count, which after RESTORED_PANE_GRACE_MS yields `TargetGone` and a `restored_target_absent` retirement. Ground seat: `retire_row` (src/store/seats/publication.rs ~640) updates only state, generation, retired_at and retired_seq, leaving `structural_terminal_id`; `end_local_seat` sets `ended_at` and deletes nothing (bindings.rs:1349, operator.rs:387), so a departed claimant's rows keep matching. Before the diff the same SQL lived only in the `claimed_elsewhere` closure that withheld a watch (fail-safe: a wrong answer cost a delay). The new TRUST-POLICY G5 *Absent* rationale ("that pane is the other seat's") and the amended accepted limit cover only a session whose refused claim enrolled a fresh seat in the pane, not a claimant that has since left; the parallel shell-continuity rule (TRUST-POLICY:490) excludes retired seats' rows. The new test `a_target_showing_another_seats_pane_counts_as_absent` inserts only `resolved` seats with open `connected` bindings, so the ended-binding and retired-seat cases are untested. Reproduce seat's sequence: s2 runs in terminal T at unresolved s1's saved target and exits; s1's Codex TUI session is resumed in T (no hook until its first turn, no kept C1 claim); every coherent pass counts T as claimed and s1 retires, finally if keyless, while its own session sits idle in a pane owned by nobody.
  fix-shape hint: in the absence-path use only, filter the seats half on `state!='retired'` and the bindings half on `ended_at IS NULL` (keep the watch-withholding use as it was), and add one test each for an ended binding and a retired seat's retained proof; align the TRUST-POLICY G5 *Absent* text and accepted limit with the narrowed predicate in the same commit.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this iteration. The prior iterations' rejections stand unchanged; no scout re-surfaced any of them and no packet carries `previouslyRejected`.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none