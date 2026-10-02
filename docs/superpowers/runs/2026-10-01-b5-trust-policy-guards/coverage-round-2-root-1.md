requirements:
R1 → ht-rzi.1, ht-rzi.2, ht-rzi.8
R2 → ht-rzi.1
R3 → ht-rzi.1
R4 → ht-rzi.1
R5 → ht-rzi.2
R6 → ht-rzi.5
R7 → ht-rzi.5
R8 → ht-rzi.3
R9 → ht-rzi.3
R10 → ht-rzi.4
R11 → ht-rzi.4
R12 → ht-rzi.6
R13 → ht-rzi.4
R14 → ht-rzi.5
R15 → ht-rzi.6
R16 → ht-rzi.7
R17 → ht-rzi.7
R18 → ht-rzi.1, ht-rzi.2
R19 → ht-rzi.8
R20 → ht-rzi.2
R21 → ht-rzi.1, ht-rzi.3
R22 → ht-rzi.5
R-new: continuity never matches on absent/empty session id (incl. pre-migration bindings) → (unmapped)
R-new: launch/wake guards define and test Herdr-reports-no-agent behaviour → (unmapped)

findings:
1 GAP · ht-rzi.4 absent Herdr agent (Codex w/o integration, human-bound) undefined for launch and wake
2 GAP · ht-rzi.2 NULL/empty session ids and pre-migration bindings must never match
3 UNOWNED-SEAM · pane-agent host read (kind, agent_session, session_start_source) + stand-in fake shared by .2 .3 .4, no owner
4 GAP · ht-rzi.3 override audit label/never on receipts not pinned
5 UNEXERCISED-CONFIGURATION · agent→agent same-seat lifecycle re-check-in (startup, /clear, resume) not tested
