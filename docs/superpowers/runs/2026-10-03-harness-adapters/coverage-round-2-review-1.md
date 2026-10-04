requirements:
R1 → ht-3bi.1, ht-3bi.2.1, ht-3bi.3.3, ht-3bi.5.1, ht-3bi.9, ht-3bi.7
R2 → ht-3bi.2.1, ht-3bi.2.3, ht-3bi.2.4.3, ht-3bi.7
R3 → ht-3bi.1, ht-3bi.4, ht-3bi.9
R4 → ht-3bi.4, ht-3bi.2.4.1, ht-3bi.7
R5 → ht-3bi.1, ht-3bi.2.1, ht-3bi.6.3, ht-3bi.6.4, ht-3bi.7
R6 → ht-3bi.2.2, ht-3bi.6.3, ht-3bi.6.4, ht-3bi.7, ht-3bi.8
R7 → ht-3bi.2.3, ht-3bi.2.4.1, ht-3bi.2.4.2, ht-3bi.3.4, ht-3bi.3.6, ht-3bi.6.4, ht-3bi.6.6
R8 → ht-3bi.2.4.1, ht-3bi.2.4.2, ht-3bi.2.4.3, ht-3bi.7
R9 → ht-3bi.3.1, ht-3bi.3.4, ht-3bi.7
R10 → ht-3bi.3.2, ht-3bi.5.1, ht-3bi.7
R11 → ht-3bi.5.1, ht-3bi.5.2, ht-3bi.6.1, ht-3bi.6.2, ht-3bi.7
R12 → ht-3bi.5.3, ht-3bi.5.4, ht-3bi.6.5, ht-3bi.7
R13 → ht-3bi.6.1, ht-3bi.6.3, ht-3bi.6.4, ht-3bi.7, ht-3bi.8
R14 → ht-3bi.3.3, ht-3bi.3.4, ht-3bi.3.5, ht-3bi.3.6, ht-3bi.6.6
R15 → ht-3bi.7, ht-3bi.6.6, ht-3bi.8
R16 → ht-3bi.8, ht-3bi
R17 → ht-3bi.8, ht-3bi.3.6
R18 → ht-3bi.7, ht-3bi, coordinator ownership by explicit user override

R-new: none
findings: []
previous-round-fixes:
- finding: C1 / R18
  status: closed
  evidence: Amendment assigns ht-3bi.7 focused conservation regressions for default0, explicit batching/retry overrides and legacy HealthSettings fallback. Root requires concrete post-main-absorption checks before freeze. Coordinator ownership of final sweep/main merge/cleanup/release explicitly user-overridden.
  new-gap: none
  new-unowned-seam: none
NEEDS-SPEC: none
