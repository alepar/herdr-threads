super-roast verdict: clean (1 nits) [converged]
mode: PR        iteration: post-cap audit
profile (assumed): Internal developer tooling. herdr-threads is a local, same-user CLI and daemon that coordinates coding agents in Herdr terminal panes. The trust model is cooperative (TRUST-POLICY.md): no adversarial callers, and misdelivery to the wrong agent pane is the main harm. It is not network-exposed; state is durable local SQLite with journaled retries, and repo rules live in AGENTS.md.
inputs: super-auto/unified-seat-selector@6fed704d9 vs 19b999b34 (post-loop changes: ht-u3ym.12 + main absorption)
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 1 resolved · 0 regressed (0 Blocking) · 2 punch-listed (open)
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, observability, testing, dependency, hygiene-docs, deploy-safety) · raw 1 → deduped 1 → panel 0 · spot 1 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 0/0/0/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 1/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 0/0/0/0 · dependency 0/0/0/0 · hygiene-docs 0/0/0/0 · deploy-safety 0/0/0/0

## Confirmed findings
- none. The one raw finding this round was a Nit candidate and received a spot check only (one refute-seat pass), which is not panel-strength verification; it is listed under "Unverified nits" below.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this iteration. Prior rejections (iterations 1 and 2) stand unchanged; none was re-surfaced.

## Unverified nits (spot-checked)
- [Nit] src/cli/mod.rs:729 — Commit 9aadb5b2a narrowed the "a refused command leaves no allocated seat" claim in TRUST-POLICY C1, the design doc and the seat_resolution module doc, but the pass-1 comment block in `run_parsed_in_pane` (which the same commit edited) still says "so a refused command allocates nothing", the universal claim the fix retracted elsewhere. [lanes: regression] [fix-regression]
  spot check: refute seat CONFIRM at Nit (1/1 valid). Seat evidence: src/cli/mod.rs:725-730 still reads "Allocate panes wait in `deferred` for pass 2, which runs after the caller derivation / actor refusal and the capability gates, so a refused command allocates nothing"; the block was edited by 9aadb5b2a to add the agent-without-session note, so it is the one statement of the rule the narrowing missed. The seat also found the old wording at src/cli/direct_send.rs:269 ("so a refused direct send allocates nothing") and that final-review-reentry-2.md:33-34 itself flags the mod.rs comment as stale. Pass 2 (`allocate_deferred_seats`) still runs before `run_selected`'s `check_selection` / mapping refusals and `direct_send::start`'s `validate_claim` / caller-binding refusals. Impact is documentation only: a maintainer reading the code-level contract is told an invariant TRUST-POLICY now says does not hold. Not a base-branch defect (the comment was left behind by the narrowing change).
  fix-shape hint: Reword the mod.rs comment (and the direct_send.rs:269 doc comment) to match the narrowed TRUST-POLICY C1 text: pass 2 runs after the caller derivation and the stage's capability gates; later refusals (selection and mapping checks, direct-send claim and caller-binding checks) can leave an idempotent guarded seat for a real pane.

## Escalations (need human)
- none

## Prior-report tracking (iteration 2 confirmed findings)
- resolved: [Nit] TRUST-POLICY.md:99 — the normative C1 sentence and its two named mirrors (design doc, seat_resolution.rs module doc) claimed "a refused command leaves no allocated seat" while post-allocation refusals remained. Commit 9aadb5b2a narrowed all three to name the refusals that can leave an idempotent guarded seat, which is the prior fix-shape hint. The regression lane found a fourth copy of the old universal wording in the code comment at src/cli/mod.rs:729 (plus src/cli/direct_send.rs:269); that residue is a documentation-only Nit, spot-checked only, and is listed above under "Unverified nits" rather than treated as a regression of the policy finding.
- punch-listed (open): [Nit] src/cli/seat_resolution.rs:327 — direct-ACK rule (D7) implemented twice, with made-up seat IDs and a production `expect_err`. In the caller's punch list; not re-surfaced by any scout this round; not counted as resolved.
- punch-listed (open): [Nit] src/cli/seat_resolution.rs:230; src/cli/seat_resolution.rs:234 — non-HostUnavailable caller-read errors treated as no caller (iteration 1 finding, carried as punch-listed in iteration 2). In the caller's punch list; not re-surfaced; not counted as resolved.
