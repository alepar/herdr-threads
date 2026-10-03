requirements:
R1 → ht-xoc.5, ht-xoc.3, ht-xoc.7
R2 → ht-xoc.1, ht-xoc.4, ht-xoc.5, ht-xoc.7
R3 → ht-xoc.1, ht-xoc.4, ht-xoc.5, ht-xoc.7
R4 → ht-xoc.5, ht-xoc.6, ht-xoc.3
R5 → ht-xoc.8, ht-xoc.2, ht-xoc.4
R6 → ht-xoc.1
R7 → ht-xoc.3, ht-xoc.4
R8 → ht-xoc.5
R9 → ht-xoc.6
R10 → ht-xoc.3, ht-xoc.6
R11 → ht-xoc.1, ht-xoc.4, ht-xoc.7
R12 → ht-xoc.5
R13 → ht-xoc.5
R14 → ht-xoc.6, ht-xoc.3, ht-xoc.5, ht-xoc.7
R15 → ht-xoc.6, ht-xoc.3
R16 → ht-xoc.2, ht-xoc.4, ht-xoc.5
R-new: manifest fetched for a running version whose payloads are Malformed or whose hooks never fire → ht-xoc.3, ht-xoc.4

findings:
GAP · R14 fetch trigger for malformed/never-seen versions (C10 shortfall)
GAP · R14 writer→reader conformance (C4 shortfall)
GAP · R13 ladder Refused only below floor (C3 shortfall)
NEEDS-SPEC: ht-xoc.3 (not honored: findings actionable)
