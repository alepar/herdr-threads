c1 · r1 · GAP · R2 · applied — reserve and release service dispatch owned by ht-ma6i.8 with session-level tests
c2 · r1 · UNOWNED-SEAM · service v3 advertisement · applied — registration is the only advertisement; one release; spec §4.3
c3 · r1 · GAP · R12 · applied — agent-exit regression on ht-ma6i.3
c4 · r1 · GAP · R3 · applied — first-bind acceptance on ht-ma6i.2
c5 · r1 · GAP · abandoned reversible or unretired seat · applied — Finalize on unretired; accepted limit, no timer (confirmed to the graph session)
c6 · r1 · GAP · alert cap · applied — oldest evicted, 257th alerts
c7 · r1 · GAP · conflict at full table · applied — refused, logged, accepted limit
c8 · r1 · GAP · key leak via launch or handoff · applied — scrub and accepted limit on ht-ma6i.6
c9 · r1 · GAP · hookless callers · applied — helper feeds plain CLI; no reattach accepted limit
c10 · r1 · UNOWNED-SEAM · conflict hold vs evidence paths · applied — hold binds ordinary resolution only; rebind clears; edge ht-ma6i.1 onto ht-ma6i.8
c11 · r1 · UNOWNED-SEAM · refusal mapping to client · applied — ht-ma6i.6 ServiceBusy and conflict lines
c12 · r1 · UNOWNED-SEAM · full-table log · applied — ht-ma6i.2
c13 · r1 · GAP · R1 cases · applied — managed_launch-only and pre-agent shape in the test
c14 · r1 · GAP · goal every pane seat · applied — goal narrowed to unnamed non-person; no-row accepted limit
c15 · r1 · GAP · B2 keyless · rejected — source design D2 scopes un-retire to seats with a recorded key; stated in spec §4.3
c16 · r1 · GAP · R5 R6 R9 R10 on a peer · applied — ht-ma6i.7 lines
c17 · r1 · GAP · R2 all final paths · applied — one choke point on ht-ma6i.3
c18 · r1 · UNOWNED-SEAM · evidence to the final row · applied — ht-ma6i.3
c19 · r1 · GAP · consumer contract documentation · applied — docs/programmatic-service-client.md update on ht-ma6i.9 (first disposed rejected on a wrong premise that no such doc exists)
c20 · r2 · GAP · R17 · applied — unretire and finalize over a real session on ht-ma6i.3; sweep
c21 · r2 · GAP · conflict hold on an owned pane · applied — hold only without a resolved owner
c22 · r2 · GAP · fingerprint row lifecycle · applied — deleted at final retire and un-retire; rival only non-retired
c23 · r2 · GAP · allocation kick · applied — ht-ma6i.1
c24 · r2 · UNOWNED-SEAM · claimless ResolveSeat · applied — ContinuityKeyPending row, per entry point
c25 · r2 · GAP · idle agent during lag · applied — kept keyed claim decided after the pass; no hook retry needed
c26 · r2 · GAP · sweep thin · applied — cold restart, reserve-with-seat, finalize, relaunch-before-unretire, B3
c27 · r2 · GAP · own-log restore · applied — ht-ma6i.7, accepted limit
c28 · r2 · UNOWNED-SEAM · kept key vs G5 · applied — ht-ma6i.4, edge onto ht-ma6i.2
c29 · r2 · UNOWNED-SEAM · peer inspect fields · applied — moved to ht-ma6i.10, edge onto ht-ma6i.7
c30 · r2 · GAP · rev/unret/final, peer row · applied — ht-ma6i.7; peer deletes on unretired
c31 · r2 · UNOWNED-SEAM · retained intent · applied — ht-ma6i.6
c32 · r2 · UNOWNED-SEAM · reversible read in verdict · applied — read as state retired
c33 · r2 · GAP · unretired in recovery surfaces · applied — ht-ma6i.3
c34 · r2 · GAP · outgoing obligations of a frozen seat · applied — ht-ma6i.10
c35 · r2 · GAP · send after un-retire · applied — ht-ma6i.3
c36 · r2 · GAP · upgrade window on an unresolved seat · applied — reserve-with-seat admits unresolved
c37 · r2 · GAP · real harness env · parked — outside the run; source design §6 qualification owns live harness checks
g1 · graph · GRAPH-EDGE · ht-ma6i.7 <- ht-ma6i.3 · kept — carries the reversible transitions
g2 · graph · GRAPH-EDGE · ht-ma6i.3 <- ht-ma6i.8 · kept — carries the ledger lookup and release
g3 · graph · GRAPH-EDGE · ht-ma6i.10 <- ht-ma6i.7 · parked — safe no: only one criterion uses .7; moving it to the sweep would give depth 5
