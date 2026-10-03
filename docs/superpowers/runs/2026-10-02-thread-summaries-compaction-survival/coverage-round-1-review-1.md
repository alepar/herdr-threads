requirements:
R1 → ht-1ip.9, ht-1ip.10, ht-1ip.11, ht-1ip.15
R2 → ht-1ip.9, ht-1ip.3
R3 → ht-1ip.5, ht-1ip.8, ht-1ip.3, ht-1ip.1
R4 → ht-1ip.11, ht-1ip.4, ht-1ip.5
R5 → ht-1ip.3, ht-1ip.5
R6 → ht-1ip.4, ht-1ip.11, ht-1ip.2
R7 → ht-1ip.2, ht-1ip.1
R8 → ht-1ip.6, ht-1ip.2
R9 → ht-1ip.7, ht-1ip.5, ht-1ip.6
R10 → ht-1ip.13, ht-1ip.12, ht-1ip.14
R11 → ht-1ip.7
R12 → ht-1ip.1
R13 → ht-1ip.15, ht-1ip.12, ht-1ip.10
R-new: Messages predating the migration get a defined author_kind and priority status → ht-1ip.1, ht-1ip.2
R-new: The daemon enforces who may call the summary commands → (unmapped)

Findings (summary; full text in session transcript):
GAP · pre-migration author_kind/priority
GAP · R12 enforcement of summary callers
GAP · R5 rollup planning ownership
GAP · end-to-end thin path before native smoke (seam integration: summary flow)
GAP · block reuse across chunking_version/prompt_version/model
UNOWNED-SEAM · is_priority predicate (ht-1ip.2 → ht-1ip.4, ht-1ip.6)
UNOWNED-SEAM · composer stash hook (ht-1ip.13 → ht-1ip.14)
UNOWNED-SEAM · skill/procedure name (ht-1ip.9 ↔ ht-1ip.11)
UNOWNED-SEAM · catch-up stored progress (ht-1ip.5 → ht-1ip.7)
UNOWNED-SEAM · soft-point rescheduling on deadline move (ht-1ip.7 → ht-1ip.13)
