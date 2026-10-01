# Coverage ledger — ht-rzi

C1 · r1 · UNOWNED-SEAM · ht-rzi.2<-ht-rzi.1 · applied — edge added (consumes hold-lift predicate), call made unconditional, .2 acceptance tests lift on last reattachment
C2 · r1 · GAP · ht-rzi.2 hint rule · applied — .2 description states decision-3 hint as mandatory; acceptance covers match/mismatch/absent/fresh start
C3 · r1 · UNOWNED-SEAM · binding provenance after continuity · applied — owned by .2: binding cooperative_top_level, continuity in seat history only; cross-checks in sweep ht-rzi.8
C4 · r1 · UNOWNED-SEAM · carry-forward vs session column · applied — .5 updates binding in place preserving all columns; cross-check in sweep ht-rzi.8
C5 · r1 · UNOWNED-SEAM · B5 migration number · applied — .2 owns the single B5 migration; .1 and .5 state no migration
C6 · r1 · GAP · R16 docs/README · applied — new leaf ht-rzi.7 (docs) depending on .1-.6
C7 · r1 · GAP · R17 TRUST-POLICY status · applied — folded into ht-rzi.7
C8 · r1 · GAP · R19 F6 end to end · applied — integration sweep ht-rzi.8 created now with the F6 scenario and cross-bead checks
C9 · r1 · GAP · ht-rzi.1 lift paths · applied — .1 lists all call sites; acceptance per path
C10 · r1 · GAP · ht-rzi.1 replace settlement · applied — .1 acceptance: NEW recipient-retired, nothing moves to OLD
C11 · r1 · GAP · ht-rzi.3 triggers · applied — .3 acceptance: CODEX_*, Herdr-reported agent
C12 · r1 · UNEXERCISED-CONFIGURATION · harnesses · applied — extended .2 (Claude w/ hint, Codex w/o hint via hook entry) and .4 (both directions) acceptance instead of a separate smoke leaf
C13 · r1 · GAP · ht-rzi.5 human bindings · applied — carry-forward covers operator_human; acceptance added
C14 · r1 · GAP · R22 CLI expected_boot fill · applied — .5 description + acceptance
