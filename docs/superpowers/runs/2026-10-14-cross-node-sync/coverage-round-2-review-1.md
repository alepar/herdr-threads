requirements:
R1 → ht-5fr.1.2, ht-5fr.2.7, ht-5fr.2.8, ht-5fr.2.12, ht-5fr.5.11
R2 → ht-5fr.1.1, ht-5fr.1.6, ht-5fr.3.10, ht-5fr.1.12
R3 → ht-5fr.3.2, ht-5fr.3.6, ht-5fr.3.8, ht-5fr.4.3, ht-5fr.4.5
R4 → ht-5fr.3.3, ht-5fr.3.7, ht-5fr.3.8, ht-5fr.3.13, ht-5fr.5.13
R5 → ht-5fr.3.11, ht-5fr.4.8, ht-5fr.4.9, ht-5fr.4.10, ht-5fr.4.11, ht-5fr.4.12, ht-5fr.4.18
R6 → ht-5fr.4.6, ht-5fr.4.11, ht-5fr.4.16, ht-5fr.3.16, ht-5fr.6.5
R7 → ht-5fr.4.13, ht-5fr.13, ht-5fr.5.6
R8 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.5.7, ht-5fr.5.10, ht-5fr.14
R9 → ht-5fr.5.12, ht-5fr.5.13, ht-5fr.4.18, ht-5fr.6.11, ht-5fr.11
R10 → ht-5fr.6.1, ht-5fr.6.15, ht-5fr.8
R11 → ht-5fr.7, ht-5fr.8, ht-5fr.1.9, ht-5fr.6.12
R12 → ht-5fr.4.7, ht-5fr.4.14, ht-5fr.13
R13 → ht-5fr.1.4, ht-5fr.1.8, ht-5fr.1.9
R14 → ht-5fr.9, ht-5fr.4.15, ht-5fr.12
R15 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.3.8, ht-5fr.6.2, ht-5fr.10, ht-5fr.11
R16 → ht-5fr.10, ht-5fr.11
R17 → ht-5fr.4.5, ht-5fr.4.7, ht-5fr.4.14, ht-5fr.4.18
R18 → ht-5fr.5.2, ht-5fr.9
R19 → ht-5fr.1.12
R20 → ht-5fr.1.13
R-new: Every harness integration that builds a caller claim sends the Herdr server locator. → ht-5fr.2.3
R-new: A local caller can never claim, bind or act as a remote seat. → (unmapped)
R-new: Concurrent creation of same-named threads on two partitioned nodes resolves deterministically after heal. → (unmapped)
findings: see coverage-findings-round-2.md (R6 daemon wake, claim host producers, remote seat occupancy, same-name threads, discovery case edge, reach rule owner, sync.discover owner, docs scope)
