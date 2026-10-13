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
L14 · r2 · GAP · accept display redefinition · applied — ht-yha.6 owns line names accept_delivery.rs filter + first_ordinary_walk_stop + AGENTS bullet
L15 · r2 · GAP · legacy invite mapping · applied — ht-yha.6 owns old-client lazy Invite -> confirming + test; spec §Compatibility
L16 · r2 · GAP · new client/old daemon pair · applied — ht-yha.7 owns legacy creation-policy fallback; spec §Compatibility
L17 · r2 · UNOWNED-SEAM · direct.auto_join_v1 advertisement · applied — ht-yha.6 owns line
L18 · r2 · GAP · items readable by old clients/mod; WakeOnQuery reverse · applied — spec §Compatibility decision (variants kept, priority additive); ht-yha.8 owns old-reader compat + fixture test; ht-yha.4 owns WakeOnQuery reverse
L19 · r2 · GAP · JS mod order vs mixed set · applied — ht-yha.15 shared JSON fixture through the mod's sort/frame
L20 · r2 · GAP · overdue warnings source · applied — spec §Compatibility; ht-yha.3 tests it
L21 · r2 · GAP · delivery_mode readers outside store · applied — split by file: ht-yha.4 wake.rs, ht-yha.5.4 sources.rs, ht-yha.8 framing; sweep greps the remainder
L22 · r2 · GAP · ht-yha.11 edge to 9 · applied — edge ht-yha.11<-ht-yha.9
L23 · r2 · UNOWNED-SEAM · staging.rs 3/5.2 · applied — edge ht-yha.5.2<-ht-yha.3
L24 · r2 · GAP · person-authored priority · applied — spec §Compatibility: separate unchanged axis; ht-yha.10 rewords the AGENTS note
L25 · r2 · UNOWNED-SEAM · watch order preservation 9/5.4 · applied — ht-yha.9 tests combine-sort only; ht-yha.15 verifies preservation (no edge)
L26 · r2 · GAP · agy/Hermes/Codex delivery paths · applied — ht-yha.8 owns confirming they consume frame()/page only; DeliveryPolicy has no class branch (map)
L27 · r2 · GAP · legacy in-flight invitation behaviour · applied — ht-yha.6 owns behaviour test on ht-yha.2 fixture; ht-yha.5.3 acceptance already orders deadline-less legacy invitations last
L28 · r2 · UNOWNED-SEAM · hydrate.rs 4/8 · applied — edge ht-yha.8<-ht-yha.4
L29 · r2 · UNOWNED-SEAM · AGENTS wake-e2e text 10/11 · applied — edges ht-yha.10<-ht-yha.11, ht-yha.9
L30 · r2 · NEEDS-SPEC × 4 · not re-run — answered by the spec §Compatibility amendment in this round
