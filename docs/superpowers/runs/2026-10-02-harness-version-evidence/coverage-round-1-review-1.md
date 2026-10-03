requirements:
R1 → ht-xoc.5, ht-xoc.7
R2 → ht-xoc.1, ht-xoc.4, ht-xoc.5, ht-xoc.7
R3 → ht-xoc.1, ht-xoc.4, ht-xoc.5, ht-xoc.7
R4 → ht-xoc.5, ht-xoc.3, ht-xoc.6
R5 → ht-xoc.8, ht-xoc.2, ht-xoc.4, ht-xoc.5
R6 → ht-xoc.1
R7 → ht-xoc.3
R8 → ht-xoc.5, ht-xoc.3
R9 → ht-xoc.6
R10 → ht-xoc.3
R11 → ht-xoc.4, ht-xoc.7
R12 → ht-xoc.5
R-new: existing version-related Health noise sources removed or folded into the new derivation → ht-xoc.5
R-new: canary known_broken reaches a client end to end with no build → ht-xoc.6, ht-xoc.3, ht-xoc.5

findings:
GAP · R4 upgrade-to-X source · no task maps contract_id to a herdr-threads release; last-working source unstated
GAP · R11 malformed classification · no task owns truncated/unparseable vs violation, no test
GAP · existing noise sources · B6 unlisted/unsupported warnings not removed/redirected
GAP · canary→manifest→client e2e · not in ht-xoc.7; release embed unverified
UNOWNED-SEAM · contract_id · ht-xoc.1 → .4 .5 .6
UNOWNED-SEAM · manifest schema 2 + branch path · ht-xoc.3 ↔ ht-xoc.6
UNOWNED-SEAM · evidence row schema · ht-xoc.4 → ht-xoc.5
UNOWNED-SEAM · unattributed reason to doctor · ht-xoc.2/.4 → .5
UNOWNED-SEAM · fetch trigger · ht-xoc.3 entry point vs callers
