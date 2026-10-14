requirements:
R1 → ht-5fr.1.2, ht-5fr.2.7, ht-5fr.2.8, ht-5fr.2.12, ht-5fr.5.9, ht-5fr.5.11, ht-5fr.11
R2 → ht-5fr.1.1, ht-5fr.1.6, ht-5fr.1.12, ht-5fr.3.10, ht-5fr.4.10
R3 → ht-5fr.3.2, ht-5fr.3.5, ht-5fr.3.6, ht-5fr.3.8, ht-5fr.4.3, ht-5fr.4.5, ht-5fr.4.4
R4 → ht-5fr.3.3, ht-5fr.3.7, ht-5fr.3.8, ht-5fr.3.13, ht-5fr.5.13, ht-5fr.10
R5 → ht-5fr.3.11, ht-5fr.4.8, ht-5fr.4.9, ht-5fr.4.10, ht-5fr.4.11, ht-5fr.4.12, ht-5fr.4.16, ht-5fr.4.18
R6 → ht-5fr.3.16, ht-5fr.4.6, ht-5fr.4.16, ht-5fr.6.5, ht-5fr.6.11
R7 → ht-5fr.4.13, ht-5fr.13, ht-5fr.5.6
R8 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.5.7, ht-5fr.5.10, ht-5fr.14
R9 → ht-5fr.5.12, ht-5fr.5.13, ht-5fr.4.10, ht-5fr.4.18, ht-5fr.11
R10 → ht-5fr.6.1, ht-5fr.6.15, ht-5fr.5.12, ht-5fr.8
R11 → ht-5fr.7, ht-5fr.8, ht-5fr.1.9
R12 → ht-5fr.4.7, ht-5fr.4.14, ht-5fr.13, ht-5fr.2.5
R13 → ht-5fr.1.4, ht-5fr.1.8, ht-5fr.1.9
R14 → ht-5fr.9, ht-5fr.4.15, ht-5fr.12
R15 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.6.2, ht-5fr.3.8, ht-5fr.10, ht-5fr.11
R16 → ht-5fr.10, ht-5fr.11
R17 → ht-5fr.4.5, ht-5fr.4.7, ht-5fr.4.14, ht-5fr.4.18
R18 → ht-5fr.5.2, ht-5fr.9
R19 → ht-5fr.1.12
R20 → ht-5fr.1.13
R-new: A remote fact ingested by a running daemon reaches an idle local recipient's delivery and wake lanes end to end, proven at daemon level with a fake Herdr. → ht-5fr.3.13, ht-5fr.4.16
R-new: A permanently gone node does not pin its seats as live remote members forever. → (unmapped)
findings: see coverage-findings-round-2.md (daemon wake, ingest→Apply in composed daemon, discovery case edge, R17 across daemons, decommission, baseline cutover, sync.discover owner, 1.8↔1.12 ordering)
