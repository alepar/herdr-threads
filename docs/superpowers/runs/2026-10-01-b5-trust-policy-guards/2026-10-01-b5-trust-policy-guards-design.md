# B5 trust policy guards — implementation design

Epic: `ht-rzi`. Normative policy: [TRUST-POLICY.md](../../../../TRUST-POLICY.md) (adopted 2026-10-01). This file
adds only the implementation-level decisions the policy leaves open; where they differ, the policy wins.

## Goal

Every guard TRUST-POLICY.md marks **required** is implemented and tested in herdr-threads, so that the
running daemon and CLI enforce the continuity invariants C2–C4 and the attribution invariants A2 and A4
(plus the accepted-limits hardening), and the user-facing docs describe the shipped behaviour.

## Scope

In scope: the required guards of C2, C3, C1 (cooperative continuity), C4, A2 (expected daemon boot), A4
(agent-to-human, second agent, wake harness), the accepted-limits hardening (`allocator.lock` without
symlink following, refuse `codex resume` launch form), the W5-2 test, the W6-R2 wording, the identifier
comments, and the operator-facing docs for new commands (`docs/operations.md`, `docs/agent-usage.md`,
README where it names behaviour).

Non-goals:
- C5 / P10 and W5-1: deletions owned by the B4 run (bead `ht-p03.2`, other branch). Not touched here.
- Adversarial verification of any kind (policy principle 1).
- New harness support beyond Claude and Codex.
- Native (real Claude/Codex) demonstration runs; deterministic tests with stand-in agents are the bar for
  this epic, except the evidence capture `ht-rzi.2` needs (see below).

## Implementation decisions

1. **Hold lift (C2).** Hold state lives in three places: `host_instances.baseline_hold_unclaimed`, per-target
   `recovery_holds` rows (read first by the effective disposition and directly by resolve, wake and inspect),
   and `recovery_baseline_releases`. One store helper, `lift_baseline_hold_if_clear(tx, instance)`, owns the
   lift. Predicate: no unresolved nonretired seat, **and** no saved seat still pending reconciliation for the
   current recovery boot/epoch (reconciliation of the current baseline has finished). When it holds, the same
   transaction clears `baseline_hold_unclaimed` and sets `released_at` on every open `recovery_holds` row of
   the instance. Governing rule: every transaction that can lower the unresolved count or finish
   reconciliation calls it: rebind, fresh seat, retire, replace, cooperative continuity, reconciliation
   retirement, structural reconfirm (Reconfirm/ReconfirmStructure), each applied reconciliation page; and once
   at daemon start for stores already stuck. The list is illustrative, the rule is normative.
   "Reconciliation finished" is a persisted fact, not worker memory: the B5 migration adds
   `host_instances.reconciled_boot` / `reconciled_epoch`, written in the transaction that applies the last
   page of a full pass for the current recovery boot/epoch with no refused (Stale) transition; the predicate
   requires them to equal `recovery_boot` / `recovery_epoch`. A pass with refusals leaves the marker behind and
   the next pass retries. At daemon start the helper lifts only when the marker already matches; otherwise
   the first completed pass calls it.
2. **Collisions (C3).** `seat retire SEAT --operator` reuses the bounded retirement cutover
   (`begin_retirement`); it is an operator action audited `operator:local-user:<uid>`. `seat rebind OLD
   --pane P --replace NEW --operator` performs NEW's retirement cutover and OLD's rebind in one deciding
   transaction. Both join the typed administrative actions; neither ACKs, accepts or sends.
3. **Cooperative continuity (C1, `ht-rzi.2`).** Reattachment is gated only on the plugin's own hook payload: a
   top-level SessionStart lifecycle check-in whose payload `source` is `resume`, whose session id equals the
   last-binding `occupant_bindings.native_session` (the existing column; no new session column) of exactly one
   unresolved nonretired seat, in a pane whose target is held or unowned. Sentinel values (`plugin_context:…`)
   and human-occupant values never match; any other source (startup/clear/new/compact) never reattaches and
   falls through to the existing hold/ordinary path. On a match the deciding transaction rebinds the seat,
   records `cooperative_continuity` in seat history (the binding itself stays `cooperative_top_level`), and
   calls `lift_baseline_hold_if_clear`.
   **The single B5 migration** (owned by `ht-rzi.1`, consumed by `ht-rzi.2`; B4 numbering risk noted at merge)
   rebuilds the STRICT `allocation_decisions` table to admit kind `cooperative_continuity` and add a nullable
   `continuity_diagnostic` column, and adds the reconciliation marker columns of decision 1.
   **Herdr's per-pane agent observation is diagnostic only** (C1: Herdr's agent field may only suggest): the
   daemon records the `agent get` agent_session comparison (match, mismatch, absent, read error) in the
   decision's `allocation_decisions.continuity_diagnostic` and shows it in `seat inspect`; it never refuses or permits reattachment.
   **Seatless check-in (client and wire).** Today the hook stops before contacting the daemon when the pane has
   no resolved seat (`src/cli/hook.rs` `find_seat` → held/unresolved Err, unowned → Quiet) and
   `CallerClaim` requires a seat and binding generation (`src/harness/bridge.rs`). `ht-rzi.2` adds a
   resume-only continuity check-in request that carries pane target, harness, session id and source but no
   seat; it is journaled under an instance-scoped intent; the daemon's response returns the chosen seat and
   binding generation, which the hook writes into the pane's client context exactly as an ordinary lifecycle
   check-in does. A refusal leaves today's diagnostics unchanged. **The deciding transaction opens the successor
   binding** (redesign after code roast round 1): one transaction rebinds the seat, opens the successor
   `cooperative_top_level` binding for the event's harness/session/target with a new generation, records
   `cooperative_continuity` and the diagnostic, and lifts the hold if clear; the reply
   `ContinuityReattached{seat, binding_generation}` is idempotent under the operation key, and the hook writes
   the pane context from it, unconditionally retiring any saved context for that seat or pane. There is no
   follow-up check-in. A lost reply is recovered by the committed binding (the next event finds the seat
   resolved and takes the ordinary path); the intent is retried only within the same hook, never by a
   per-event journal scan. Before the first reconciliation pass of the current recovery epoch (marker of
   decision 1 lagging), "no matching unresolved seat" is retryable, not final.
   **Evidence capture.** `ht-rzi.2` logs SessionStart stdin and `agent get` from inside the hook for resume,
   /new and /clear on both harnesses and counts events per resume (Claude Code #24265 reported a
   startup(new id) + resume(original id) pair). A resume-only gate makes the startup-first order safe; if the
   capture shows resume-then-startup, a later startup check-in on a seat reattached moments earlier must not
   overwrite its `native_session` (dedupe rule in `ht-rzi.2`).
4. **Agent-to-human refusal (A4).** Enforced in the daemon's lifecycle check-in decision; the CLI
   environment-marker check in `me init` is advisory hardening on top.
5. **Expected boot (A2).** Optional envelope field; absent means today's behaviour (programmatic clients
   keep working). The daemon refuses a mismatch before dispatch with a definite error.
6. **Binding carry-forward (C4).** Applied only by structural reconfirm (same terminal, Herdr boot and
   incarnation); an incarnation change still unresolves.

## Coordination

The B4 run (`.worktrees/remaining-herdr-threads-findings`) edits `src/ports.rs`, `src/store/seats.rs`,
`src/identity/reconcile.rs` and adds a migration. Whichever branch lands on `main` second resolves the
conflicts; this run does not wait on it.
