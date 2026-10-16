requirements:
R1 → ht-yha.1, ht-yha.2, ht-yha.3, ht-yha.7
R2 → ht-yha.1, ht-yha.2, ht-yha.4, ht-yha.7
R3 → ht-yha.4
R4 → ht-yha.3, ht-yha.7
R5 → ht-yha.6
R6 → ht-yha.6
R7 → ht-yha.5, ht-yha.5.1, ht-yha.5.2, ht-yha.5.3, ht-yha.5.4
R8 → ht-yha.5, ht-yha.9
R9 → ht-yha.8, ht-yha.9
R10 → ht-yha.1, ht-yha.2, ht-yha.7
R11 → ht-yha.10, ht-yha.6
R12 → ht-yha.11, ht-yha.12
R-new: The new daemon handles and advertises the new commands and capabilities with their handlers → (unmapped)
R-new: New daemon serves old clients (ordinary/lazy send → p0/p1, wake-on → T, old shapes readable) → (unmapped)
R-new: Stored in-flight state migrated (non-ACK receipts, lazy invitations, pending pair invitations) → ht-yha.2
R-new: Ordering verified end to end across surfaces for one mixed pending set → (unmapped)
R-new: Lazy/ordinary mode code paths and lazy invitations removed from daemon and store → ht-yha.3, ht-yha.6, ht-yha.7

Findings (summary): GAP old-client/new-daemon; GAP in-flight state; GAP TRUST-POLICY owner for ht-yha.3; GAP cross-surface order; GAP hydrate priority population; SEAM nudge-threshold handler + capability advertisement 3/4<->7; SEAM 5<->9 watch/hook order; SEAM decision_seq writer 3<->5.2; GAP mode-path removal. NEEDS-SPEC ht-yha.5 (hook/accept/presentation routing).
