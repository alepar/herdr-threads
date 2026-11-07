# Report — 2026-11-04-graph-clone-identity

status: implemented · review fixes ht-ma6i.44-.49 landed · full suite 9439/9440 @ 0a10478c0 (1 pre-existing flake, reproduced on main) · merge is the coordinator's

branch: super-auto/graph-clone-identity @ 81cd1a7ae (base main @ cf13ed76c)

## Delivered (epic ht-ma6i, 25 beads)
- G2 ht-ma6i.1 (+.36, .38): seat-level shell fingerprint; live handoff reattaches bare, exited-agent and managed_launch-only pane seats (structural_shell_continuity); fenced on the responding server's boot.
- D1 ht-ma6i.2/.5/.6/.8/.9 (+.20, .21, .35): HERDR_THREADS_CONTINUITY_KEY; ReserveContinuityKey (optional seat), ReleaseContinuityKey, ContinuityAlerts over service_session_v3; KeyVerdict; conflict hold and alerts; SeatInspect continuity_key.
- D2 ht-ma6i.3/.7/.10 (+.22, .34, .39): reversible retirement, UnretireSeat, FinalizeRetirement; frozen obligations and deadline shift; seat.retired_reversible and seat.unretired facts converge in any order; SeatInspect retirement_state.
- G5 ht-ma6i.4 (+.41, .42): an unresolved seat whose target stays absent retires after 10 observed minutes, reversibly when key-bearing; only a live claimant counts.
- Hygiene and policy: .11 sweep, .23, .37, .40, .43; TRUST-POLICY updated with each.

## Evidence
- nextest --all-targets --all-features: 9421 passed, 0 failed, 61 skipped, 270.6 s; leak check pass; clippy, check-default-features, check-harness-neutral, check-wire-compat pass (sweep/ logs).
- Code roast: r1 9 → r2 2 → r3 clean; post-cap audit 1 (fixed, ht-ma6i.42, task-reviewed only).

## Remaining
- Every code roast round ran bounded ([review readiness incomplete]): invocation inv-35d46ca1's final review was cut short by a usage limit and the tooling has no recovery; the run directory was unreviewed. Collection gaps retained.
- Accepted limits recorded in TRUST-POLICY (Hermes gateway key, first-turn hooks, no threads timer, keyless close final, ACK-warning origin while frozen).
- Task worktrees removed at the user's request (clean, merged, no live process); the run worktree stays for the coordinator's review.
- Real-Herdr and live-harness qualification (source design §6) is outside this run.
