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
R-new: carry-forward only on structural reconfirm; incarnation change still unresolves → ht-rzi.5
R-new: DaemonBootChanged is a definite retryable rejection → ht-rzi.5
R-new: one Herdr pane-agent read in host adapter and stand-in Herdr → (unmapped)
R-new: --operator override audited operator:local-user, off receipts → (unmapped)

findings:
1 GAP · ht-rzi.3 override audit label + override argv undecided (--operator vs --new-seat --operator)
2 UNOWNED-SEAM · pane-agent host read + stand-in fake (.2 .3 .4); owner ht-rzi.2 proposed
3 UNOWNED-SEAM · docs ownership split between .1/.3 and .7
4 GAP · ht-rzi.7 TRUST-POLICY acceptance must exclude C5 (owned by B4)
5 NARRATIVE-EDGE · ht-rzi.7<-ht-rzi.6 unnameable artifact
6 UNEXERCISED-CONFIGURATION · wake for human-bound seat
7 UNOWNED-SEAM · lifecycle check-in branch order between .2 and .3; .3 consumes provenance from .2
