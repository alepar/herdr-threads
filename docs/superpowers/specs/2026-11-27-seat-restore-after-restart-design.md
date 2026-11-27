# Seat restoration after a Herdr restart (ht-iygh)

Status: draft for the coordinator (snSGTpBk7xGd). D1-D3 await the user's decision (root cause and trade-offs sent as
message mnSGrsoflUST in thread dAV70ibpnBK); D4 and D5 change no invariant. Source: the user's request relayed by the
Mac aisw tab (sQ0VWtXSQNJM, messages mQ0VgzEsnRBF and mQ0VCUlwoIxh); evidence in mQ0VEmAThRnA and mQ0V9Zq7aYUP
(files under `/Users/alepar/AleCode/aisw/ht-iygh-diag/` on the Mac).

## Requested outcomes

(a) A restart reconciles against the coherent host snapshot automatically: same pane plus the same live agent
session means the same seat, with no operator step. (b) A seat whose pane is really gone stays dormant and holds
nothing. (c) Holds apply per target, never globally. (d) Operator repair only for real ambiguity, and a candidate list
of plausible seats only. (e) A seat that still exists receives its mail: an unresolved or held mapping never refuses a
send or loses mail.

## Root cause (Mac, 3.6.0, Herdr 0.9.5)

- 11-21 09:54Z the Mac rebooted. 18:22:24Z Herdr started cold (pid 4392). The incarnation is the server's pid and
  start time (`ServerIncarnation`, `src/host/native.rs`), so it is new, and C2 unresolves every saved seat
  (`plan_page_for`: `!same_incarnation` -> `MarkUnresolved`). The bindings ended `seat_unresolved` at 18:22:33-36Z.
- Herdr 0.9.5's session restore kept every pane id, minted new terminal ids, and re-ran each Codex as
  `codex --no-daemon --approve-for-me resume <same session id>`. Each pane's Herdr `agent_session.value` equals that
  id and the seat's last top-level binding session.
- None of the seven resumed Codex sessions has had a turn since (last rollout events 18:22:36-39Z). The Codex TUI
  runs no hook on resume, so no `same_session` claim was ever made. This is the accepted limit *An idle hooks-only
  session with no recovery claim waits for its next turn*. Nothing breaks the loop: an unresolved seat is never
  woken, and Herdr's agent field "never moves or wakes the seat".
- `baseline_hold_unclaimed` is one flag per server: every unowned baseline target stays held until no unresolved
  seat of the server remains. That gives 15 held targets for 8 unresolved seats.
- w1A:p2 ran a fresh `claude` (no `--resume`) in sQ0VWtXSQNJM's own saved pane. A new session never takes an old
  seat, so it was refused `target has unresolved seat mapping`. The refusal's repair list
  (`resolution_recovery_refusal`) names the first three unresolved seats of the whole server by ordinal, not the
  seat saved at that pane. That seat's earlier repairs (10-27, 10-29) each followed a cold Herdr restart with a
  non-resumed agent in the pane.
- w3Z:p1 is the real pane-gone case: its workspace was closed. Restored-pane retirement (ht-wpq4) already retires
  that seat reversibly after 10 minutes of observed absence.
- Ordinary thread sends and invitations already queue to unresolved members, and the mail survives rebind and C1
  (rows are keyed by seat id). Direct `send --seat`, pair ensure, `handoff --existing` and the cross-node handoff
  check refuse through `eligible_delivery_recipient`. No TRUST-POLICY rule requires that refusal.

## Decisions

| # | Outcome | Decision | Policy effect |
|---|---|---|---|
| D1 | (a) | **Host session continuity.** After each completed reconciliation pass of a new or unknown incarnation, and after kept claims, process continuity and shell continuity, the observation lane judges each unresolved local unnamed seat (reason `other`) on a Herdr host. It decides only when all of the following hold. (1) The current coherent publication shows the seat's saved target id. (2) Herdr's `agent_session` for that pane, read once per seat by the lane's existing pane read, has a value equal to the native session of a top-level, non-human, non-`managed_launch` binding of the seat that its latest unresolve ended (`seat_unresolved`, placed under an earlier incarnation). (3) The pane's `agent` names that binding's harness. (4) The pane has no resolved owner. (5) No kept C1 claim names the target or one of the seat's sessions. (6) No binding of another non-retired seat names that session. (7) No other unresolved seat is saved at that target. The decision is the C1 transaction `rebind_unresolved_seat` pinned to the judged seat and generation, under a daemon operation key per (seat, generation, session): hold released, the binding reopened **disconnected** (no fingerprint; a dormant wake target as after a Codex resume today), and `allocation_decisions` kind `host_session_continuity`. The session's next hook connects it as an ordinary check-in. A definitive mismatch is not read again for 10 minutes per (seat, generation), as for ht-hsyg. | New C1 row and provenance value `host_session_continuity` (seat history only). Weakens "Herdr's agent field never moves or wakes the seat" for this one shape: **user decision**. |
| D1b | (a), alternative | The same evidence (1)-(7) only makes the seat a **wake target** while an item with p <= T is pending. The native lane types the wake into the saved pane under the ordinary composer gates. The woken turn's hook makes the `same_session` claim, and C1 decides. Nothing moves on Herdr's word, but the seat stays unresolved, and its pane held, until mail arrives or the agent takes a turn. | Weakens only "never wakes". Rule 2 of the delivery contract holds. |
| D2 | (c) | **Per-target holds.** A new incarnation still unresolves seats as C2 says, but the restore hold covers only the saved targets of unresolved seats of that server (`recovery_holds` rows written at unresolve), not every baseline member. `baseline_hold_unclaimed` stops gating resolution. A target that no unresolved seat names resolves as created-after-baseline. A seat's hold is released by every path that releases one today, and also when the seat retires (restored-pane retirement included). | New accepted limit *Restore hold covers saved targets only*: if Herdr re-issues a seat's pane under another id and a **fresh** agent starts there, that agent gets a new seat (role split). The old seat waits for C1, D1, operator repair or restored-pane retirement. A resumed session there still reattaches by C1 before enrollment. **User decision.** |
| D3 | (d), w1A:p2 | **Pane continuity for a fresh start.** A top-level lifecycle claim with a session that matches no seat, at pane P that is the saved target of exactly one unresolved seat X (reason `other`, after the pass, no kept claim on P or of X), binds onto X when no live session of X appears: no Herdr `agent_session` in the publication names one of X's sessions and no binding of X is connected. It is the C1 transaction with provenance `pane_continuity`. | Weakens "a new native session never takes an old seat": **user decision**. The alternative is today's behaviour (operator rebind, or a ht-0f0r service restore). |
| D4 | (d) | **Plausible candidates only.** `resolution_recovery_refusal` lists only the unresolved seats saved at the refused target (none: no rebind line), and says "N unresolved seats on this server" instead of the baseline-lift sentence. Health and `seat inspect` are unchanged. | None (diagnostic text). |
| D5 | (e) | **Mail to an existing seat.** Direct send (CLI `direct_send` inspection, store pair ensure and pair send), `handoff --existing` (CLI and store delivery effect) and the cross-node `AddrTarget::Check` accept a local, non-retired seat in any mapping state: `resolved`, `unresolved` or held. A retired, missing or foreign seat is still refused. Mail queues as for a thread send: the recipient counts as unavailable, an ACK deadline starts only once it is available (`eligible_at_snapshot`), and the sender sees the existing `recipient_unavailable` warning. A handoff that launches a new pane keeps its live-pane checks. Pane-form addressing of a held pane keeps its refusal (D4 now names the saved seat). | None. C2 holds block resolution, not mail. `docs/agent-usage.md` ("unresolved, held ... targets refuse") is updated. |

## Trust argument

- D4 and D5 move no seat and attribute nothing. D5 delivers to the seat id the sender named. The recipient's
  obligations start only when it is available, as for any unavailable member today.
- D1 rests on host-reported evidence that three independent records agree on: Herdr's restore launched the session
  id it reports, that id is the session the seat's own binding held when the restart ended it, and the pane id is
  the seat's saved one. The residual risk is Herdr's cached session going stale before any turn (a `/resume` into
  another session in the TUI before the first prompt). The seat then sits on that pane until the session's next
  check-in names a different session, which enrolls under the ordinary resolved-pane rules. That is a role
  misplacement for one pane, never a merge. Receipts still carry only the session's own claims.
- D2 trades a role split for availability. Herdr 0.9.5 keeps pane ids across a cold restart (observed on the Mac;
  to be confirmed on an isolated Linux Herdr before merge). An id that changed proves nothing, so the hold on it
  protected only a role that a resumed session could also recover through C1.
- D3 is the largest weakening: the pane locator alone attributes the old seat's threads and obligations to a new
  session.

## Tests (test-first, per decision)

- D4: a refusal at a held pane names exactly the seat saved there, even when lower-ordinal unresolved seats exist.
- D5: direct send, pair ensure, `handoff --existing` and the remote check succeed for an unresolved seat and for one
  at a held pane. The mail is delivered after an operator rebind. An ACK request's deadline starts at availability.
  Retired, missing and foreign seats are still refused.
- D1/D1b/D2/D3 (once decided): store tests per guard; an isolated-Herdr cold restart (`scripts/lib/isolated-herdr.sh`)
  with a stand-in hookless session, checking that pane ids survive and the seat restores without an operator.

## Not changed

The incarnation definition, the structural, process, shell and key paths, kept claims, restored-pane retirement and
target-absence retirement, ht-0f0r service restore, and the operator verbs.
