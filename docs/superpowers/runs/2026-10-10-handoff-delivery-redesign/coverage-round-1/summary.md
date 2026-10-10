# Coverage round 1 assembly summary

requirements: 35 · mapped: 35 · unmapped: 0

Valid reviews: ROOT 3; receipts 3; invitations 3; handoff 3; hooks 3. Coverage is not degraded. Initial3insufficient reviews per pass repaired in same round using literal complete input; no reviewer tools.

## Unioned finding C1

UNSATISFIABLE-ACCEPTANCE (unwired): ht-0lo.3.2 requires observing a durable ACK deadline warning without registration but ht-0lo.1.2 owns unavailable-recipient due discovery and sender-addressed warning lifecycle and is outside its blocker closure. Raised in ROOT+handoff by reviewer1; reviewers2/3 clean. Union retains it for parent verification.

Evidence: ht-0lo.3.2: "Test missing/stale profile hooks through guarded handoff and observe durable ACK deadline warning without registration." Acceptance requires every named scenario. ht-0lo.1.2 owns "bounded indexed due discovery plus sender-addressed open/clear warning routing". Closure of .3.2: .1.1, .2.1, .2.2, .3.1; no path to/from .1.2.

Proposed fix: .3.2 blocked-by .1.2 with consumes unavailable-recipient receipt due discovery and originating-seat warning lifecycle; add (needs: ht-0lo.1.2) to warning observation.

No spec/tree/tracker/run-state mutation performed by assembler. Parent owns evidence verification/disposition.

## Requirements mapping union

R1 → ht-0lo.1.1
R2 → ht-0lo.1.1, ht-0lo.1.3
R3 → ht-0lo.1.1, ht-0lo.1.3
R4 → ht-0lo.1.1, ht-0lo.1.2, ht-0lo.1.3
R5 → ht-0lo.1.3
R6 → ht-0lo.1.2, ht-0lo.1.3
R7 → ht-0lo.1.1
R8 → ht-0lo.1.2
R9 → ht-0lo.2.2, ht-0lo.2.3
R10 → ht-0lo.2.3
R11 → ht-0lo.2.1, ht-0lo.2.2
R12 → ht-0lo.2.2
R13 → ht-0lo.2.2
R14 → ht-0lo.2.1, ht-0lo.2.2, ht-0lo.2.3
R15 → ht-0lo.2.1, ht-0lo.2.2, ht-0lo.2.3
R16 → ht-0lo.2.2, ht-0lo.2.3
R17 → ht-0lo.2.2, ht-0lo.2.3
R18 → ht-0lo.2.1
R19 → ht-0lo.3.1
R20 → ht-0lo.3.1, ht-0lo.3.2
R21 → ht-0lo.3.1, ht-0lo.3.2
R22 → ht-0lo.3.2
R23 → ht-0lo.3.1, ht-0lo.3.2
R24 → ht-0lo.3.2
R25 → ht-0lo.3.1, ht-0lo.3.2
R26 → ht-0lo.3.2
R27 → ht-0lo.1.2, ht-0lo.2.2, ht-0lo.3.2
R28 → ht-0lo.4.1
R29 → ht-0lo.4.1, ht-0lo.4.2
R30 → ht-0lo.4.2
R31 → ht-0lo.4.2
R32 → ht-0lo.4.2
R33 → ht-0lo.4.1, ht-0lo.4.2
R34 → ht-0lo.1.1, ht-0lo.2.2, ht-0lo.2.3, ht-0lo.3.2, ht-0lo.4.1, ht-0lo.4.2
R35 → ht-0lo.1, ht-0lo.1.1, ht-0lo.1.2, ht-0lo.1.3, ht-0lo.2, ht-0lo.2.1, ht-0lo.2.2, ht-0lo.2.3, ht-0lo.3, ht-0lo.3.1, ht-0lo.3.2, ht-0lo.4, ht-0lo.4.1, ht-0lo.4.2
