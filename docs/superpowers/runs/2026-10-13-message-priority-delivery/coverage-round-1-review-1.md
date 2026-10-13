requirements:
R1 → ht-yha.1, ht-yha.3, ht-yha.7
R2 → ht-yha.1, ht-yha.2, ht-yha.4, ht-yha.7
R3 → ht-yha.4
R4 → ht-yha.3, ht-yha.7
R5 → ht-yha.6
R6 → ht-yha.6
R7 → ht-yha.5, ht-yha.5.1, ht-yha.5.2, ht-yha.5.3, ht-yha.5.4
R8 → ht-yha.5, ht-yha.9
R9 → ht-yha.8
R10 → ht-yha.1, ht-yha.2, ht-yha.7
R11 → ht-yha.10, ht-yha.3, ht-yha.6
R12 → ht-yha.11
R-new: A new daemon serves old clients (legacy mode→priority, legacy wake_on→threshold, items readable by old clients) → (unmapped)
R-new: In-flight pre-v47 state (pending lazy invitations, non-ACK receipts/deadlines) is handled deliberately → ht-yha.2
R-new: Harness-side wake paths (mod Channel idle submit, DeliveryPolicy) wake only for priority <= T → (unmapped)
R-new: The same pending set comes out in the same order on every surface, checked end to end → (unmapped)

Findings (summary): GAP R10 old-client/new-daemon; GAP in-flight migration; GAP harness-side idle wake; GAP cross-surface order e2e; GAP ht-yha.11 deps thin; SEAM 3<->5.2 delivery-row write contract; SEAM 5<->9 watch/hook order; SEAM capability advertisement 3/4/6<->7; SEAM 4<->8 digest/check-in text. NEEDS-SPEC ht-yha.5 (routing of hook pages/watch drains).
