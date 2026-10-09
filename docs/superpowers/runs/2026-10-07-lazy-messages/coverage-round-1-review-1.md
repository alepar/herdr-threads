requirements:
R1 → ht-big.2.2, ht-big.3, ht-big.5.1
R2 → ht-big.2.2, ht-big.3, ht-big.6
R3 → ht-big.1, ht-big.2.2, ht-big.4.1
R4 → ht-big.2.1, ht-big.2.2
R5 → ht-big.1, ht-big.3, ht-big.5.1
R6 → ht-big.3, ht-big.7
R7 → ht-big.3, ht-big.5.1, ht-big.5.2
R8 → ht-big.5.2
R9 → ht-big.4.1, ht-big.5.1, ht-big.5.2
R10 → ht-big.2.1, ht-big.2.2, ht-big.3, ht-big.6
R11 → ht-big.4.2, ht-big.8
R12 → ht-big.1, ht-big.4.1, ht-big.4.2, ht-big.7, ht-big.8
R13 → ht-big.7, ht-big.9

GAP — R3 daemon-side lazy-send rejection
Evidence: R3 requires rejection at CLI and daemon; ht-big.4.1 owns CLI validation; compact ht-big.2.2 does not explicitly own incompatible-option rejection.
Fix: ht-big.2.2 owns daemon rejection lazy ACK-seat/pane/deadline including direct protocol tests.
GAP — R10 lifecycle semantics
Evidence: compact ht-big.6 first sentence allocates publication/restart but no explicit postjoin/leaving/retired/archival owner.
Fix: ht-big.2.1/2.2 retention/exclusion, ht-big.3 completion, ht-big.6 four lifecycle tests.
ORPHAN: none. UNOWNED-SEAM: none.
