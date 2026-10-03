requirements:
R1 → ht-xoc.5, ht-xoc.7
R2 → ht-xoc.4, ht-xoc.5, ht-xoc.7
R3 → ht-xoc.1, ht-xoc.4, ht-xoc.5, ht-xoc.7
R4 → ht-xoc.5, ht-xoc.6, ht-xoc.3
R5 → ht-xoc.8, ht-xoc.2, ht-xoc.4
R6 → ht-xoc.1
R7 → ht-xoc.3
R8 → ht-xoc.5
R9 → ht-xoc.6
R10 → ht-xoc.3, ht-xoc.6
R11 → ht-xoc.1, ht-xoc.4, ht-xoc.7
R12 → ht-xoc.5
R13 → ht-xoc.5
R14 → ht-xoc.6, ht-xoc.3, ht-xoc.7
R15 → ht-xoc.6
R16 → ht-xoc.4, ht-xoc.5
R-new: recording and the fetch never add hook latency or failures → ht-xoc.3, ht-xoc.4
R-new: doctor names the source of each verdict → ht-xoc.5

findings:
GAP · hook latency/failure isolation of ensure_manifest
GAP · R7 opt-out/offline e2e
GAP · R8/R12 unsupported schema vs cache; manual row precedence
UNOWNED-SEAM · canonical version string (detected vs attributed)
UNOWNED-SEAM · canary version key vs client lookup
GAP · doctor verdict source
