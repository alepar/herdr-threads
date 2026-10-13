# Coverage ledger — 2026-10-13-message-priority-delivery

L1 · r1 · GAP · R10 old-client/new-daemon compat · applied — owns lines: ht-yha.3 (legacy send mode->priority, old-client test), ht-yha.4 (legacy wake_on handlers, nudge-threshold handlers)
L2 · r1 · GAP · in-flight pre-v47 state · applied — ht-yha.2 states the spec's deliberate keep-as-recorded rule with a fixture migration test (no conversion; spec §Compatibility decides)
L3 · r1 · GAP · harness-side idle wake honours T · applied — ht-yha.9 owns mod Channel wake via daemon `wakes`; edge ht-yha.9<-ht-yha.4 (Native wake is daemon-side, ht-yha.4)
L4 · r1 · GAP · cross-surface order end to end · applied — new leaf ht-yha.15 Seam integration: cross-surface delivery order (deps 5.4, 9, 6, 8)
L5 · r1 · GAP · ht-yha.11 thin deps · applied — edges ht-yha.11<-3,4,6,8
L6 · r1 · UNOWNED-SEAM · delivery-row write contract (priority, decision_seq) 3<->5.2 · applied — ht-yha.3 owns priority at staging; ht-yha.5.2 owns decision_seq at publication; edge ht-yha.5.4<-ht-yha.3
L7 · r1 · UNOWNED-SEAM · watch/hook order 5<->9 · applied — ht-yha.9 order contract (preserve merge order, sort only combined batches, same tiebreaks); verified by ht-yha.15
L8 · r1 · UNOWNED-SEAM · capability advertisement · applied — ht-yha.3 advertises send.priority_v1, ht-yha.4 seat.nudge_threshold_v1, ht-yha.6 already direct.auto_join_v1
L9 · r1 · UNOWNED-SEAM · digest/check-in lazy wording 4<->8 · applied — ht-yha.4 owns the wording change with size note
L10 · r1 · GAP · TRUST-POLICY owner for ht-yha.3 · applied — ht-yha.3 owns line names it
L11 · r1 · GAP · hydrate priority population · applied — ht-yha.8 owns inbox/history population (edge ht-yha.8<-ht-yha.14); WatchMessage population already ht-yha.9
L12 · r1 · GAP · R-new-e mode-path removal · applied — ht-yha.3 owns replacing delivery_mode readers with priority/which-row
L13 · r1 · NEEDS-SPEC · ht-yha.5 routing · not re-run — answered from the subspec §Caller subsets (hook/watch/accept/presentation all page inbox v2) and covered by ht-yha.15
