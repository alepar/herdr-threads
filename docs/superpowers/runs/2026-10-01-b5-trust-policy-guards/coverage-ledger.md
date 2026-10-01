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
C15 · r2 · UNOWNED-SEAM · pane-agent observation port · applied — new Seam contract ht-rzi.9; .2 .3 .4 depend on it (boundary contract lines); seam integration folded into sweep ht-rzi.8 instead of a separate Seam integration bead
C16 · r2 · GAP · ht-rzi.4 absent agent · applied — rules stated: launch proceeds when no agent detected; wake requires detected kind == bound harness; human-bound never woken
C17 · r2 · UNEXERCISED-CONFIGURATION · human-bound wake · applied — .4 acceptance
C18 · r2 · GAP · ht-rzi.2 null session · applied — never matches on NULL/empty; pre-migration seats → operator path; acceptance
C19 · r2 · GAP · ht-rzi.3 override audit · applied — audit label, operator_human binding, off receipts; guidance 'me init --operator'
C20 · r2 · UNEXERCISED-CONFIGURATION · same-seat agent re-check-in · applied — .3 acceptance (startup, /clear, resume)
C21 · r2 · UNOWNED-SEAM · docs ownership · applied — docs files removed from .1/.3; .7 sole owner
C22 · r2 · GAP · ht-rzi.7 C5 · applied — C5 marker kept, named in Status line, excluded from grep
C23 · r2 · NARRATIVE-EDGE · ht-rzi.7<-ht-rzi.6 · applied — line restated: consumes allocator.lock no-follow guard (marker flip)
C24 · r2 · UNOWNED-SEAM · lifecycle check order · applied — owned by .2 (A4 refusal → C1 reattach → hold refusal); .3 consumes
C25 · r2 · GAP · ht-rzi.1 restore/daemon-start lift · applied — predicate also after reconciliation page and at daemon start; acceptance
C26 · r2 · UNOWNED-SEAM · codex resume capture vs launch refusal · applied — .2 owns: manual 'codex resume' path, launch refusal stays; .7 documents
C27 · r2 · UNSATISFIABLE-ACCEPTANCE · ht-rzi.5 session column · applied — criterion restated; session column checked in sweep
C28 · r2 · GAP · R19 sweep scenarios · applied — .8 extended
