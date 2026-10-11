requirements:
R1 → ht-54r.1, ht-54r.3, ht-54r.21, ht-54r.17
R2 → ht-54r.2, ht-54r.3, ht-54r.5, ht-54r.6, ht-54r.4
R3 → ht-54r.7, ht-54r.22, ht-54r.9
R4 → ht-54r.20
R5 → ht-54r.7, ht-54r.8, ht-54r.22
R6 → ht-54r.9, ht-54r.12, ht-54r.13
R7 → ht-54r.10, ht-54r.11, ht-54r.23, ht-54r.16
R8 → ht-54r.14
R9 → ht-54r.15
R10 → ht-54r.17, ht-54r.11, ht-54r.20
R11 → ht-54r.4
R12 → ht-54r.1, ht-54r.6
R13 → ht-54r.18
R14 → ht-54r.19
R-new: An existing seat's state is converted into V29 bindings and cursors so an upgraded install delivers no message twice and skips none → (unmapped)
R-new: An agent started before the upgrade re-enrolls under a session binding on its next call without losing its cursor → (unmapped)
R-new: Every path that ends a binding goes through one disconnect transition so seat_active, presence, wake and archival agree → ht-54r.10
findings: (see super-design ledger r1 entries; GAPs R12 migration, R12 upgrade, R13 TRUST, R13 docs deps, R8 restore holds, R5 obligations; SEAMs process-info, disconnect path, lineage, foreground, catch-up vs fast-forward)
NEEDS-SPEC: ht-54r.1 — whether §4 Migration V29 includes migrating per-seat data and re-enrolling pre-upgrade agents
