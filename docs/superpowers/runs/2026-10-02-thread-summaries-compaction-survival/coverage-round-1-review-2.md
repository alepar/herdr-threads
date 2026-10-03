requirements:
R1 → ht-1ip.9, ht-1ip.10, ht-1ip.15
R2 → ht-1ip.9
R3 → ht-1ip.5, ht-1ip.8, ht-1ip.3
R4 → ht-1ip.11, ht-1ip.4, ht-1ip.5
R5 → ht-1ip.3, ht-1ip.5
R6 → ht-1ip.4, ht-1ip.2
R7 → ht-1ip.2, ht-1ip.1
R8 → ht-1ip.6
R9 → ht-1ip.7, ht-1ip.5
R10 → ht-1ip.13, ht-1ip.12, ht-1ip.14
R11 → ht-1ip.7
R12 → ht-1ip.1
R13 → ht-1ip.15
R-new: The daemon enforces who may call the summary commands → (unmapped)
R-new: Stored blocks are not mixed across chunking_version changes → (unmapped)
R-new: Catch-up is entered at a defined trigger with one frozen frontier shared by Ready and the hold → ht-1ip.6

Findings (summary; full text in session transcript):
GAP · R12 enforcement of summary callers
GAP · block invalidation on version change
GAP · R1 Claude compact fallback when evidence is not admitted
GAP · R8 catch-up entry trigger and frozen frontier
UNOWNED-SEAM · stash hook (ht-1ip.13 → ht-1ip.14)
UNOWNED-SEAM · Ready/stall/progress signal (ht-1ip.5 → ht-1ip.6, ht-1ip.7)
UNOWNED-SEAM · priority predicate (ht-1ip.2, ht-1ip.4, ht-1ip.6)
UNOWNED-SEAM · procedure name (ht-1ip.9 ↔ ht-1ip.11)
UNOWNED-SEAM · bundle/submission format (ht-1ip.5, ht-1ip.4 → ht-1ip.11)
UNOWNED-SEAM · effective deadline consumed by soft point (ht-1ip.7 → ht-1ip.13)
UNOWNED-SEAM · Claude compact admission (ht-1ip.10 → ht-1ip.9)
NEEDS-SPEC: ht-1ip.6 (a leaf, not a subepic; not re-run — spec §7 defines entry, frontier and stall, and the bead amendments now state them)
