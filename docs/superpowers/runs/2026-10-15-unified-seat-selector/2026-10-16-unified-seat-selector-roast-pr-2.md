super-roast verdict: Nit (2 confirmed) [converged]
mode: PR        iteration: 2 of 3
profile (assumed): Internal developer tooling. herdr-threads is a local, same-user CLI and daemon that coordinates coding agents in Herdr terminal panes. The trust model is cooperative (TRUST-POLICY.md): no adversarial callers, and misdelivery to the wrong agent pane is the main harm. It is not network-exposed; state is durable local SQLite with journaled retries, and repo rules live in AGENTS.md.
inputs: super-auto/unified-seat-selector@19b999b34 vs main@cee04a6d6 (merge-base)
delta vs prior: 1 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 1 resolved · 1 regressed (0 Blocking) · 1 punch-listed (open)
coverage: scouts 11/11 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, api-contract, testing, hygiene-docs, observability) · raw 3 → deduped 2 → panel 2 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 2 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=2) · reproduce 2/0/0 · refute 2/0/0 · ground 2/0/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 1/1/0/0 · premortem 0/0/0/0 · simplicity-design 1/1/1/0 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 1/1/0/0 · api-contract 0/0/0/0 · testing 0/0/0/0 · hygiene-docs 0/0/0/0 · observability 0/0/0/0

## Confirmed findings
- [Nit] TRUST-POLICY.md:99 — The fix's new normative TRUST-POLICY C1 sentence (mirrored in the design doc and the seat_resolution.rs module doc) says a seat-argument allocation runs only after the caller derivation and the command's refusals, "so a refused command leaves no allocated seat", but pass 2 (`allocate_deferred_seats`) still runs before the refusals in `run_selected` (`check_selection`, mapping errors) and `direct_send::start` (`validate_claim`, caller-binding check), so the policy claims an invariant the code does not hold. [lanes: security, regression] (regressed — fixed incompletely, see iteration 1 finding 1 at src/cli/seat_resolution.rs:416; :423) [fix-regression]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: TRUST-POLICY.md:98-100 (added in 4bfb8d9ec): allocation "runs only after every seat argument has resolved and after the caller derivation and the command's capability refusals, so a refused command leaves no allocated seat." In src/cli/mod.rs `run_caller_scoped_deferred` (~l.1227-1241) runs `derive_caller`, then `seat_resolution::allocate_deferred_seats` (pass 2, a journaled ResolveSeat), then `run_selected`. `run_selected` still refuses afterwards at src/cli/mod.rs:2368 via `caller::check_selection` (harness mismatch, src/cli/caller.rs:158) and the "local context differs from current service mapping" branches. `direct_send::start` (src/cli/direct_send.rs:307-318) refuses after allocation via `p.validate_claim(&claim)?` ("direct send caller or relay mismatch") and "direct send caller binding differs". `run_gates` (seat_resolution.rs ~611-645) mirrors only p0 receipts, direct-send `require_capabilities` and invitation confirmation. Concrete case: an agent runs `send --seat space/tab/bob --user-intent request "x"` without `--relays-user`; clap accepts it (commands.rs:2787-2799), pass 2 allocates bob's pane seat and prints `resolved:`, then `start` refuses. All three seats rated Nit: the leftover is an idempotent seat for a real pane, nothing is delivered and attribution is unchanged. The prior report's iteration-1 finding (allocation before `derive_caller`) is fixed for the `derive_caller` gates; the fix widened the normative sentence into a universal claim that the remaining post-allocation refusals falsify. TRUST-POLICY.md is normative under AGENTS.md ("A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md").
  fix-shape hint: Either narrow the sentence (and its two mirrors) to name only the refusals that actually run before pass 2 (caller derivation and the stage's capability gates), or move pass 2 after the `run_selected` / `direct_send::start` refusals so the universal claim holds.

- [Nit] src/cli/seat_resolution.rs:327 — The fix implements the direct-ACK rule (D7) a second time over pass-1 `Resolved` values in `check_direct_ack_before_allocation`, and obtains the refusal by calling `check_direct_ack` with made-up seat IDs and unwrapping the error with `expect_err` in production code; the rule now lives in two places, and the new copy depends on the other's internal behaviour. [lanes: simplicity-design]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/cli/seat_resolution.rs ~296-340 `check_direct_ack_before_allocation` compares `Resolved::Seat`/`Resolved::Pane` values itself (with a `pane_seat` daemon read for mixed pairs) and, when they differ, runs `check_direct_ack(&SeatId::new("target"), &mut vec![SeatId::new("other")]).expect_err("distinct seats are refused")` only to borrow the refusal text. The same rule still runs post-freeze at seat_resolution.rs:654 via `check_direct_ack` (src/cli/commands.rs:249-255). If `check_direct_ack` is ever relaxed in one place only, the made-up pair stops erroring and the `expect_err` panics on a user-input path (`send --seat X --require-ack Y`). The refute seat found no shared constructor, no spec text accepting the duplication, and that the code is new in this diff. Behaviour is correct today; this is maintainability only, so Nit.
  fix-shape hint: Put the refusal message in one shared constructor (e.g. `direct_ack_refusal()` in commands.rs) called from both checks, or give `check_direct_ack` a comparison predicate so one function owns the rule; either removes the made-up IDs and the production `expect_err`.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none this iteration. Prior rejections (iteration 1) stand unchanged; none was re-surfaced.

## Unverified nits (spot-checked)
- none this iteration (spot 0).

## Prior-report tracking (iteration 1 confirmed findings)
- regressed (fixed incompletely): [Nit] src/cli/seat_resolution.rs:416; src/cli/seat_resolution.rs:423 — allocation before `derive_caller`. Commit 4bfb8d9ec moved pass-2 allocation after `derive_caller` and the capability gates, which fixes the cited gates, but the fix widened the normative sentence to "a refused command leaves no allocated seat", which the remaining post-allocation refusals (`check_selection`, `validate_claim`, caller-binding check) falsify. Carried as this iteration's confirmed finding at TRUST-POLICY.md:99.
- resolved: [Nit] src/cli/seat_resolution.rs:187 — slot-by-slot allocation leaving earlier allocations behind on a later slot failure. Commit 4bfb8d9ec ("Seat stage allocates last: two passes") resolves every seat argument in pass 1 and allocates in pass 2 only after every slot resolved; no scout re-surfaced it.
- punch-listed (open): [Nit] src/cli/seat_resolution.rs:230; src/cli/seat_resolution.rs:234 — non-HostUnavailable caller-read errors treated as no caller. Deliberately left unfixed by the caller's scope filter; not re-surfaced; not counted as resolved.

## Escalations (need human)
- none
