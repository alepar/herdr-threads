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
R-new: TRUST-POLICY.md status markers updated once shipped → (unmapped)
R-new: end-to-end F6 flow (restart→holds→reattach→lift) → (unmapped)
R-new: CLI client fills expected_boot → ht-rzi.5

findings:
1 UNOWNED-SEAM hold-lift .1->.2
2 GAP .2 hint rule/acceptance
3 UNOWNED-SEAM binding provenance after continuity (.3/.4/.5)
4 GAP docs/README + TRUST-POLICY status
5 GAP walking skeleton F6 end to end
6 UNEXERCISED-CONFIGURATION per-harness continuity, wake both directions
7 GAP .3 CODEX_*/Herdr-agent acceptance
8 GAP --replace settlement acceptance
9 GAP .5 carry-forward limited to cooperative bindings
10 GAP .5 CLI fills expected_boot untested
