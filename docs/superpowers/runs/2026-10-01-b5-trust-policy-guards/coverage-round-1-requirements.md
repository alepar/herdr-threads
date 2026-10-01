R1 The instance-wide restore hold lifts once no unresolved nonretired seat remains (C2)
R2 `seat retire SEAT --operator` exists and retires a seat through the bounded retirement cutover (C3)
R3 `seat rebind OLD --pane P --replace NEW --operator` retires NEW and rebinds OLD in one decision (C3)
R4 A rebind refused because the target is owned carries both resolutions as argv (C3)
R5 A top-level lifecycle check-in whose harness session id uniquely matches an unresolved seat's last binding reattaches that seat with `cooperative_continuity` provenance; ambiguity, a fresh session, or a contradicting Herdr agent_session hint falls back to the operator path (C1)
R6 A joined seat stays available across a daemon restart when its mapping is structurally reconfirmed (C4)
R7 A request addressed to a different daemon boot is refused before dispatch (A2)
R8 The daemon refuses an agent-to-human lifecycle check-in over an open cooperative_top_level binding unless --operator (A4)
R9 `me init` refuses when agent environment markers are present or Herdr reports an agent in the pane (A4)
R10 `launch` refuses to start an agent for a seat whose bound agent is live in another pane (A4)
R11 A wake prompt goes only to an agent of the seat's bound harness (A4)
R12 `allocator.lock` is opened without following symlinks (Accepted limits)
R13 The `codex resume` launch form is refused until captured (Accepted limits)
R14 A test pins stage_recipient staging unavailable when the effective observation generation differs (W5-2)
R15 The self-marker hint says "when run in this pane"; ids.rs comment says 112 bits; prep-id reuse bound is commented
R16 Operator and agent docs (docs/operations.md, docs/agent-usage.md, README) describe the new commands and behaviours
R17 TRUST-POLICY.md marks shipped guards implemented (no "not yet implemented"/"today" wording for them)
R18 The hold-lift predicate runs in every last-unresolved-seat transaction of decision 1 (rebind, fresh seat, retire, replace, cooperative continuity, reconciliation retirement)
R19 An end-to-end F6 restore scenario is tested (incarnation change → holds → reattach → collision resolution → hold lift → ordinary allocation)
R20 Native evidence that Claude and Codex resume keep the session id is captured, or the harness is excluded from C1
R21 Operator decisions (retire, replace, override) are audited operator:local-user:<uid> and never appear on receipts
R22 The CLI client fills expected_boot on every request
