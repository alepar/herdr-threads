# Final review (bounded): ht-ma6i graph clone identity

Invocation inv-9f820871-59fa-4568-9db3-d91aa9749c71. Captured source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 5f4294145590d888310c30a2013ca136fbded590 (both verified with `git rev-parse --verify`). The caller checkout was observed on super-auto/graph-clone-identity at 5f4294145 with a clean status; nothing was switched or restored. The review-source binding was recorded first and succeeded.

## Route and provenance

This is a bounded captured-source review, not a whole-range review. The sidecar `review-inputs.json` reads status `unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, no omissions, no slices, and three retained gaps, all on `docs/superpowers/runs/2026-11-04-graph-clone-identity/run.md:146` for inv-35d46ca1-baaf-4273-b92f-3cad9ad13172: incomplete claimed collection; invalid null-review bundle; incomplete split bundle (missing cluster report or slices). 34 out-of-range malformed collection records (2026-11-03 atomic-remote-handoff, 2026-10-17 handoff-profile runs) are informational. The failed assembly and its sidecar were left as found; no whole-range package was built or loaded.

The range changes 643 paths: 474 under `docs/superpowers/runs/` and 169 elsewhere. The previous bounded final review (inv-1a8ebbff, head c00972c75) inspected 22 production paths and named five landing conditions. Since c00972c75 the range adds four fix tasks (ht-ma6i.34 to .37, merges 83d7166c7, 173101785, 4fbc0c6f4, 5f4294145). This pass read the whole production delta c00972c75..5f4294145 (15 files) and then, in captured base..head hunk terms, every hunk of `src/identity/repair.rs`, `src/store/seats/cooperative.rs`, `src/store/effective.rs`, `docs/operations.md`, the non-test and two test hunks of `src/client/service.rs`, the shell-continuity hunk of `src/store/seats/continuity.rs` and the service-group hunk of `.config/nextest.toml`. Partially read and therefore not claimed in coverage: `src/store/seats/reversible.rs` (lines 324-560), `src/store/continuity_keys.rs` (lines 300-400), `src/store/seats/publication.rs` (530-625, `materialize_seat_unretired` and the replicated final retirement), `src/store/seats/operator.rs` (330-420), and outside the diff `src/host/native.rs` (`pane_shell_pid`, `check_context`).

## Requirement baseline trace

- B1 (continuity key). Unchanged since the prior trace except the alert-only clearing: `clear_alert_only_of_departed_owner` now runs in `retirement_transition`, `mark_seat_unresolved` and the reconciliation move, and `register_available` still clears alert-only rows whose judged owner changed. Operator `rebind_unresolved_seat` acts only on unresolved seats, whose rows the unresolve already cleared, and clears the target's row itself. The keyed check-in has its own digest domain and a distinct operator idempotency domain. The client registers `service_session_v3` only, with no fallback.
- B2 (pane close retires immediately; graph undo reverses it). Home `UnretireSeat` now bounds the deadline shift by the fact's `unretired_at` like the peer apply and the own-log replay (ht-ma6i.34), and the shift SQL is now the shared `LOGICAL_PENDING_*_SQL` spelling, which I checked against the Rust projection in `effective_receipt_with_key_deadline` (the `-1` to `0` cutover default now matches Rust's `COALESCE(...,0)`). On a peer, undo is still not delivered under one ordering: `materialize_seat_unretired` applies a `seat.unretired` that arrives before its `seat.retired_reversible` as a no-op (`unretire_by_fact` returns early when `frozen_episode` differs), the journal row reads `applied`, and `episode_fact_applied` then refuses every later copy, so after a fork-repair re-issue in that order the peer row stays retired and frozen. Unchanged at head; no bead tracks it.
- B3 (agent exit with pane open keeps the seat). The sweep test `agent_exit_with_pane_open_keeps_the_seat` now waits for each relaunch's own session binding (`open_binding_of`), so it no longer passes on the first session's still-open binding; the pin is stronger. It has not been run in this review.
- B4 (Herdr unchanged). Nothing in the range touches Herdr; shell continuity reads Herdr's existing `pane process-info`.

## Integration observations

- The ht-ma6i.36 shell fence is half effective against the native host: `record_shell_fingerprints` passes `expected_boot` and `expected_epoch`, but native `pane_shell_pid` calls only `check_context` (epoch, before I/O) and reads with `run_unfenced`, so the boot half is never checked and nothing is checked after the read. The write re-check (`g.host_boot`, `g.epoch`, `g.incarnation` of the active publication) closes the case where the store reconciled first. What remains is a Herdr restart during the 750 ms read, before the adapter's epoch moves and before reconciliation, which would still record the new shell under the old incarnation and later judge it continuous. That window is very narrow; it is a residual of the bead's intent, not a regression.
- `judge_shell_candidate` still reads the pane shell unfenced (ledger line 270); the decision is guarded by `ShellContinuityGuard` and `observed_matches` on the observation taken afterwards, so a stale shell read cannot commit against a moved publication.
- The un-retire bound change means a replicated obligation whose origin `created_at` is ahead of the home's `unretired_at` (clock skew) is not shifted anywhere; every node agrees, which was the point of ht-ma6i.34.
- `PersistentServiceClient::register` sends only v3, so a client built from this branch cannot talk to a v3.0.0 daemon at all, not even for v2 operations. This mirrors the earlier v1 to v2 precedent, but it is a compatibility break for programmatic clients and bears on the version number of the release that carries it.
- `ServiceBusy` now keeps the intent pending under the same key instead of completing as a refusal; consistent with the replay path.

## Ledger triage

- The four roast round 1 beads (ht-ma6i.34, .35, .36, .37) read closed and their merges are in range; the code changes match the beads' titles. Two of the prior landing conditions are discharged by these; the round 1 code roast has not been re-run against the fixes.
- Ledger line 152 (fork repair re-issues `seat.unretired` before `seat.retired_reversible`): re-verified unchanged at head and still without a bead; it is a known data-state defect on a peer that breaks B2's undo there, so it is a landing condition, not a deferred minor.
- Ledger line 261 (replicated final retirement of a local row in `publication.rs` near line 610 does not call `clear_alert_only_of_departed_owner`): confirmed by reading; the row waits for the next check-in on the pane. Follow-up.
- Ledger line 262 (TRUST-POLICY clearing-trigger list misses the new owner triggers) and line 266 (the "10 observed minutes" in docs/operations.md is not tied to a named constant): documentation follow-ups.
- Ledger lines 270-271: the unfenced decide-side read is judged above as guarded; the ports.rs scope note is fine.
- Cleanup lines 257, 260, 265, 269: task worktrees under `.worktrees/` and `/tmp/sp-44bdca52/ht-ma6i.34` to `.37` were kept because process inspection failed (exit 4); tmpfs and disk space stay held until the caller removes them.
- Metrics lines 276-279 are UNAVAILABLE (the ledger re-read was malformed), so absence of parked or fix-pass lines is not read as clean.
- Earlier triage from inv-1a8ebbff (ledger lines 63, 65, 80, 84, 92, 145, 153-154, 158-159, 175-176 and the TDD-order, fixture and file-size minors) is carried forward unchanged; nothing in the new delta touches those paths.

## Verdict
not ready (B2 peer undo defect open with no bead; bounded review only, whole-range review and full suite outstanding)

## Must fix before landing
- B2 not delivered on a peer under fork-repair ordering: an out-of-order `seat.unretired` is applied as a no-op yet journalled `applied`, so `episode_fact_applied` refuses every later copy and the peer seat stays retired and frozen (src/store/seats/publication.rs materialize_seat_unretired, src/store/seats/reversible.rs episode_fact_applied and unretire_by_fact; ledger line 152); no bead tracks it.

## Untested scope
- whole-epic review incomplete; bounded inspected hunks only; all remaining hunks and provenance gaps remain unreviewed
- Whole-range review outstanding: this pass inspected 7 changed paths in full or by named hunks; the other 162 non-run-doc paths (including TRUST-POLICY.md, src/store/seats/reversible.rs, src/store/continuity_keys.rs, src/store/seats/key_verdict.rs, src/store/seats/publication.rs, src/store/seats/observation.rs, src/cli/hook.rs, src/cli/mod.rs, src/cli/site_capability.rs, src/protocol/service.rs, src/ports.rs, integrations/claude/mod/hooks/register.js, integrations/hermes/__init__.py, both migrations and every test file) and all 474 run-record paths remain unreviewed here.
- Review-input provenance gaps retained: run.md:146 inv-35d46ca1-baaf-4273-b92f-3cad9ad13172 incomplete claimed collection; invalid null-review bundle; incomplete split bundle (missing cluster report or slices); plus 34 out-of-range malformed collection records.
- No full-suite measurement of this branch exists: the unfiltered nextest suite and leak checks are deferred to the caller after this invocation, so the branch, including the four fix merges, is not treated as tested.
- The round 1 code roast was not re-run on the ht-ma6i.34 to .37 fixes; their task reviews are the only check.
- No live Herdr, Claude, Codex or Hermes run exercised the key path, shell continuity or reversible retirement; the real-Herdr qualification design section 6 asks for lies outside this run.

## Deferred OK
- Native `pane_shell_pid` checks only the epoch before I/O and never the boot, so a Herdr restart inside the 750 ms shell read, before the adapter epoch moves and before reconciliation, can still record a new shell under the old incarnation (src/host/native.rs pane_shell_pid, src/identity/repair.rs record_shell_fingerprints); very narrow window, follow-up to add a post-read epoch check.
- Ledger line 261: replicated final retirement of a local row does not clear its alert-only key conflicts until the next check-in on the pane.
- Ledger line 270: `judge_shell_candidate` reads the shell unfenced; the decision is guarded by `ShellContinuityGuard`.
- Ledger lines 262 and 266: TRUST-POLICY clearing-trigger list and the unanchored "10 observed minutes" in docs/operations.md.
- `PersistentServiceClient` registers v3 only, so a branch client cannot reach a v3.0.0 daemon; a release-numbering consideration.
- Carried from inv-1a8ebbff: ledger lines 63, 65, 80, 84, 92, 145, 153-154, 158-159, 175, 176 and the TDD-order, fixture-substitution, file-size and duplicate-fixture minors.
