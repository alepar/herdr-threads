c1-01 · r1 · GAP · R13 · applied — TRUST-POLICY section ownership added to owns lines of .1 .3 .4 .12 .13 .14 .15 .20 .23 (policy edits land with each invariant commit)
c1-02 · r1 · GAP · R12-upgrade · applied — new leaf ht-54r.24 upgrade path (stale-install detection, actionable refusal, re-adoption); smoke config 8
c1-03 · r1 · GAP · R12-backfill · applied — .1 owns V29 data backfill; .22 owns lazy/catch_up row migration + v0.6.0 upgrade test
c1-04 · r1 · GAP · R8-restore · applied — .14 and .23 acceptance for Herdr restart restore holds; smoke config 7
c1-05 · r1 · GAP · R13-docs-deps · applied — .18 deps on .3 .4 .14 .15 .16 .20
c1-06 · r1 · GAP · R5-obligations · applied — .8 consumes obligations (dep .20) with acceptance
c1-07 · r1 · UNOWNED-SEAM · fingerprint/process-info port · applied — format + ProcessInfo trait in .1; implementation in .2; .4 dep .2
c1-08 · r1 · UNOWNED-SEAM · disconnect entry point · applied — .10 owns sole disconnect with side-effect hooks; .11 dep .10; .14 consumes it
c1-09 · r1 · UNOWNED-SEAM · lineage record · applied — record in .1; copy in .20; .17 dep .20
c1-10 · r1 · UNOWNED-SEAM · subagent binding creation · applied — .3 owns attach/resolve API; .11 consumes
c1-11 · r1 · UNOWNED-SEAM · foreground-binding predicate · applied — owned by .13
c1-12 · r1 · UNOWNED-SEAM · catch-up vs fast-forward · applied — .8 owns rule/threshold, .22 owns catch_up rows; .8 dep .22
c1-13 · r1 · UNOWNED-SEAM · launch placeholder fingerprint · applied — .4 consumes fingerprint (dep .2)
c1-14 · r1 · UNOWNED-SEAM · availability field · applied — .23 owns, .16 consumes (dep .23)
c1-15 · r1 · NEEDS-SPEC · ht-54r.1 · rejected — leaf, not a subepic; answered by c1-03 (backfill now in owns)
c2-01 · r2 · GAP · nested-harness precedence · applied — .2 owns innermost-harness rule + test; smoke config 3 asserts distinct Hermes binding
c2-02 · r2 · UNOWNED-SEAM · cursor seeding rule · applied — signature in .1, implementation + precedence + first-contact flag in .7; .20/.24 consume (deps)
c2-03 · r2 · GAP · R16 · applied — .10 owns the only disconnect transition (internal too); .4/.17/.23/.24 consume (deps)
c2-04 · r2 · UNOWNED-SEAM · same-pid supersede at enrollment · applied — .10 owns enroll-hook supersede with prompt-disconnect acceptance
c2-05 · r2 · GAP · same-pid vs multi-session · applied — subagents sharing pid never superseded (.2/.10 acceptance); managed daemon already refused
c2-06 · r2 · UNOWNED-SEAM · mod channel teardown · applied — .12 registers teardown hook with .10 (dep)
c2-07 · r2 · GAP · R13-docs-deps · applied — .18 deps on .24 .17 .13 .9 .23 .12
c2-08 · r2 · GAP · background agent config · applied — smoke config 9
c2-09 · r2 · UNOWNED-SEAM · subagent digest · applied — .9 owns subagent digest rule, dep .6
c2-10 · r2 · GAP · cross-pane resume · applied — .3 owns + acceptance; smoke config 1 step
c2-11 · r2 · UNOWNED-SEAM · per-binding unread query · applied — .7 owns has_unread; .12/.13 consume (deps)
c2-12 · r2 · UNOWNED-SEAM · delivery state vs digest · applied — .9 dep .22; .22 owns lazy-aware has_unread
c2-13 · r2 · UNOWNED-SEAM · availability ownership · applied — .23 owns recompute as a hook; .10 only invokes
c2-14 · r2 · UNOWNED-SEAM · Codex/Hermes hook registration · applied — owned by .11
c2-15 · r2 · UNOWNED-SEAM · migrated-binding marker · applied — .1 owns marker + session key; .22/.24 consume
c2-16 · r2 · UNOWNED-SEAM · readers sweep scope · applied — .21 owns exactly its enumerated list, hits listed with owners
c2-17 · r2 · UNOWNED-SEAM · launch placeholder replace · applied — .4 owns enroll-hook replace via .1 registry and sole disconnect
