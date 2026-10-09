requirements:
R1 → ht-big.2.2, ht-big.3, ht-big.5.1
R2 → ht-big.2.2, ht-big.3, ht-big.6
R3 → ht-big.1, ht-big.2.2, ht-big.4.1
R4 → ht-big.2.1, ht-big.2.2, ht-big.3
R5 → ht-big.1, ht-big.3, ht-big.5.1
R6 → ht-big.3, ht-big.6, ht-big.7
R7 → ht-big.3, ht-big.5.1, ht-big.5.2
R8 → ht-big.5.2
R9 → ht-big.4.1, ht-big.5.1, ht-big.5.2
R10 → ht-big.2.2, ht-big.6
R11 → ht-big.4.2, ht-big.8
R12 → ht-big.1, ht-big.4.1, ht-big.4.2, ht-big.7, ht-big.8
R13 → ht-big.6, ht-big.7, ht-big.9

UNOWNED-SEAM — R3 daemon-side lazy combination rejection.
Input evidence: R3 requires ACK-seat/pane/deadline rejection at CLI and daemon; ht-big.4.1 owns CLI validation, ht-big.2.2 preparation/publication/replay but no explicit compact owner of daemon rejection.
Fix: assign daemon validation/direct protocol rejection tests to ht-big.2.2 before preparation/publication.
GAP — R10 lifecycle delivery preservation.
Input evidence: R10 covers postjoin/leaving/retired/archival; ht-big.2.2 frozen audience and ht-big.6 publication/restart scope do not explicitly allocate all four.
Fix: ht-big.6 focused lifecycle regressions and narrow fixes: postjoin exclusion, addressed retention across leaving/retirement/archive, absence new backlog/attention.
No orphan tasks.
