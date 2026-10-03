# Coverage ledger — ht-xoc
C1 · r1 · GAP · R4 upgrade-to-X source · applied — ht-xoc.6 writes supported_since from the passing release contract; ht-xoc.5 action selection rule (upgrade / pin to newest local verified else manifest last_working / issue)
C2 · r1 · GAP · R11 malformed classification · applied — classifier existed in ht-xoc.1 (truncated in inputs); made explicit (Ok|Violation|Malformed) + truncated test; ht-xoc.7 scenario
C3 · r1 · GAP · existing noise sources · applied — ht-xoc.5 inventories and removes/routes every version-related Health/doctor emitter; acceptance with B6 ladder in place
C4 · r1 · GAP · canary→manifest→client e2e · applied — ht-xoc.7 fixture branch file scenario; release embed check in ht-xoc.6
C5 · r1 · GAP · manual manifest edits · applied — ht-xoc.6 manifest.py validate|set, source=manual rows never overwritten
C6 · r1 · UNOWNED-SEAM · contract_id · applied — owns line on ht-xoc.1 (derivation, stability, CLI json); consumers .4 .5 .6 already have edges
C7 · r1 · UNOWNED-SEAM · manifest schema 2 + branch path · applied — owns line on ht-xoc.3; ht-xoc.6 consumes (edge exists)
C8 · r1 · UNOWNED-SEAM · evidence schema · applied — owns line on ht-xoc.4 incl. verified definition; ht-xoc.5 consumes (edge exists)
C9 · r1 · UNOWNED-SEAM · unattributed reason → doctor · applied — ht-xoc.4 persists last_unattributed; ht-xoc.5 renders
C10 · r1 · UNOWNED-SEAM · fetch trigger · applied — ht-xoc.3 ensure_manifest entry point called from ht-xoc.4's recording path; new edge ht-xoc.4←ht-xoc.3; state function reads only
C11 · r1 · UNOWNED-SEAM · attribution result type · applied — ht-xoc.2 owns Attributed|Unattributable{reason}
