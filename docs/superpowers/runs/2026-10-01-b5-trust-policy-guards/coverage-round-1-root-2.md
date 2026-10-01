requirements:
R1 → ht-rzi.1
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
R16 → ht-rzi.1, ht-rzi.3
R-new: TRUST-POLICY.md required markers updated once shipped → (unmapped)
R-new: native evidence of resume session-id stability captured or harness excluded → ht-rzi.2
R-new: operator decisions audited operator:local-user and never on receipts → ht-rzi.1, ht-rzi.3

findings:
1 GAP .2 hint rule/acceptance
2 GAP .3 CODEX_*/Herdr-agent acceptance
3 GAP docs/README unowned
4 GAP TRUST-POLICY status
5 GAP --replace settlement acceptance
6 UNOWNED-SEAM hold-lift .1->.2
7 UNOWNED-SEAM binding provenance after continuity
8 UNOWNED-SEAM carry-forward vs session column (.5/.2)
9 UNOWNED-SEAM migration number ownership
10 UNEXERCISED-CONFIGURATION per-harness continuity/A4
