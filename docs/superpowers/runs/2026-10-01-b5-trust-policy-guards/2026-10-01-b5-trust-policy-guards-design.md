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

1. **Hold lift (C2).** The instance-wide `baseline_hold_unclaimed` flag is cleared in the same transaction
   that leaves zero unresolved nonretired seats (rebind, fresh seat, retire, replace, cooperative
   continuity, reconciliation retirement). Per-target `recovery_baseline_releases` stay as they are.
2. **Collisions (C3).** `seat retire SEAT --operator` reuses the bounded retirement cutover
   (`begin_retirement`); it is an operator action audited `operator:local-user:<uid>`. `seat rebind OLD
   --pane P --replace NEW --operator` performs NEW's retirement cutover and OLD's rebind in one deciding
   transaction. Both join the typed administrative actions; neither ACKs, accepts or sends.
3. **Cooperative continuity (C1, `ht-rzi.2`).** The harness session id from the hook payload is persisted
   on `occupant_bindings`. A top-level *lifecycle* check-in in a pane whose target is held or unowned, and
   whose session id equals the last binding session of exactly one unresolved nonretired seat, rebinds that
   seat to the target in the deciding transaction (releasing the hold, recording provenance
   `cooperative_continuity` in seat history). Herdr's own `agent_session` report (`pane.report_agent_session`
   from Herdr's integration hooks, visible via `agent get`/`agent list`) is a **hint**: when present it must
   equal the claimed session id or the reattachment is refused to the operator path; when absent the claim
   alone suffices (Herdr may lack the integration, e.g. Codex panes observed without it). A Herdr
   `session_start_source` of `startup`/`new`/`clear` (a fresh session) also refuses reattachment. Never
   proof, never on receipts. Schema change via the next migration number; coordinate numbering with B4.
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
