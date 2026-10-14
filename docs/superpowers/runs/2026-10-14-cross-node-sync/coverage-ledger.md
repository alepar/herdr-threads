# Coverage ledger — ht-5fr

L1 · r1 · flag-sweep · ht-5fr.1.7 · applied — promotion overruled at review; seam decided in node-identity §8, remaining scope split into ht-5fr.1.11; kept as leaf.
L2 · r1 · flag-sweep · ht-5fr.4.11 · applied — promotion overruled at review; split into ht-5fr.4.11/4.16/4.17, decisions already in fact-kinds §§2,3,8,12,16; kept as leaf.
L3 · r1 · flag-sweep · ht-5fr.6.9 · applied — promotion overruled at review; open questions settled in incarnations §12, split into 6.9/6.13/6.15; kept as leaf.
L4 · r1 · flag-sweep · ht-5fr.6.10 · applied — promotion overruled at review; failpoint decided in incarnations §11, split into 6.10/6.14; kept as leaf.
L5 · r1 · citation · ht-5fr.2.6 · applied — malformed "(needs: ht-5fr.9, or added here)" citation rewritten to a resolvable (needs: ht-5fr.9).
L6 · r1 · GAP · R17 · applied — seats publish at enrollment (fact-kinds §4, 063d4b54); ht-5fr.4.5 amended; ht-5fr.4.18 case "A invites a never-referenced seat on B".
L7 · r1 · GAP · R18 · applied — ht-5fr.5.2 refuses pubkey/node_uuid mismatch and known-genesis mismatch; edge → ht-5fr.1.10; rule added to ht-5fr.9 scope.
L8 · r1 · GAP · R19 · applied — new leaf ht-5fr.1.12 (ID consumers) under ht-5fr.1.
L9 · r1 · GAP · R20 · applied — new leaf ht-5fr.1.13 (forward node-schema migrations); node-identity §5 amended (94e8e29d).
L10 · r1 · GAP · R8 real ssh · applied — new leaf ht-5fr.14 (real-ssh two-node check in the Linux Docker runner; self-test without Docker; live run pre-release).
L11 · r1 · GAP · ht-5fr.7 · applied — edges → ht-5fr.5.7, ht-5fr.5.10; composition and two-daemon discovery test in scope.
L12 · r1 · GAP · ht-5fr.11 · applied — edges → ht-5fr.8, ht-5fr.13; marker, sync verify and resync assertions; conditional discovery case.
L13 · r1 · GAP · ht-5fr.12 · applied — edges → ht-5fr.13, ht-5fr.4.14, ht-5fr.7; marker format, remote-seat display, discovery in scope.
L14 · r1 · GAP · wake e2e · rejected — root spec places cross-node wake e2e cases in follow-on outside this tree.
L15 · r1 · GAP · leaf decomposition · rejected — promotion review judged ht-5fr.7/.8/.9/.10/.12/.13 LEAF; no new evidence.
L16 · r1 · UNOWNED-SEAM · sync status/unapplied read API · applied — ht-5fr.8 owns the store query (src/store/sync_status.rs) and snapshot read.
L17 · r1 · UNOWNED-SEAM · remote seat rendering + marker · applied — ht-5fr.4.14 owns remote-seat rendering (6 bytes/row); ht-5fr.13 owns marker render (≈40 bytes/node), edge → ht-5fr.4.14; reach rule stays with ht-5fr.5.4.
L18 · r2 · GAP · R6 daemon-level wake · applied — ht-5fr.11 case (9): nudge/lazy/require-ack/settled history between two daemons with fake Herdr.
L19 · r2 · UNOWNED-SEAM · ingest→Apply in composed daemon · applied — ht-5fr.5.10 → ht-5fr.3.13; 5.10 asserts applied rows; 3.13 owns the kick rule.
L20 · r2 · GAP · ht-5fr.11 discovery · applied — edge → ht-5fr.7; case unconditional.
L21 · r2 · GAP · R17 across daemons · applied — ht-5fr.11 case (10): CLI invite + direct pair of a never-referenced remote seat.
L22 · r2 · GAP · node decommission · applied — accepted limit in root spec §8/§12 (0571bdec) and ht-5fr.9 scope.
L23 · r2 · UNOWNED-SEAM · baseline/delta cutover · applied — ht-5fr.1.13 owns the rule; every baseline-editing leaf cites it.
L24 · r2 · UNOWNED-SEAM · sync.discover · applied — ht-5fr.7 owns it.
L25 · r2 · UNOWNED-SEAM · ht-5fr.1.8 ↔ 1.12 · applied — edge 1.8 → 1.12.
L26 · r2 · GAP · claim host producers · applied — ht-5fr.2.3 audits all producers, host precedence and foreign_session change; ht-5fr.2.12 second-server hook case; multi-host spec §1 amended.
L27 · r2 · GAP · remote seat occupancy · applied — ht-5fr.2.6 refuses non-local seats on every claim/bind/check-in path; ht-5fr.9 rule; ht-5fr.4.18 negative case.
L28 · r2 · GAP · same-name threads · rejected — root spec §2: names are labels and may repeat; IDs are identity.
L29 · r2 · UNOWNED-SEAM · reach rule · applied — ht-5fr.5.4 owns sync_node_contact and the reach rule; ht-5fr.13 consumes it.
L30 · r2 · GAP · docs scope · applied — ht-5fr.12 covers legacy purge, store-only/multi-host doctor and health, state-dir addressing; edges → 1.9, 1.11, 2.10.
L31 · graph · GRAPH-EDGE · ht-5fr.3 <- ht-5fr.9 · applied — narrowed to existing ht-5fr.3.16 <- ht-5fr.9 (replicated_from provenance); ht-5fr.3.9 needs no edge per its own description
L32 · graph · GRAPH-EDGE · ht-5fr.2 <- ht-5fr.9 · applied — narrowed to existing ht-5fr.2.6 <- ht-5fr.9 (claim attribution by host)
L33 · graph · GRAPH-EDGE · ht-5fr.3 <- ht-5fr.1 · parked — narrow to 3.2<-1.1, 3.4<-1.5, 3.6<-1.10, 3.10<-1.6 (30→23 alone); safe no: shared schema.rs, baseline SQL, app.rs, workers.rs, test_support
L34 · graph · GRAPH-EDGE · ht-5fr.5 <- ht-5fr.3 · parked — narrow to existing 5.1<-3.2, 5.3<-3.12, 5.5/5.12/5.13<-3.9, 5.12<-3.16, 5.10<-3.8/3.13/3.15; safe no: shared sync/mod.rs, app.rs, store/mod.rs
L35 · graph · GRAPH-EDGE · ht-5fr.6 <- ht-5fr.5.5/5.12/5.13 · parked — narrow to existing 6.2<-5.5, 6.8/6.9<-5.12, 6.8<-5.13; safe no: shared coordinator.rs, sync/mod.rs, kicks.rs
L36 · graph · GRAPH-EDGE · ht-5fr.4 <- ht-5fr.1 · parked — narrow to 4.2<-1.10, 4.8<-1.6, NEW 4.1<-1.5; safe no: shared schema.rs, baseline SQL, test_support
L37 · graph · GRAPH-EDGE · ht-5fr.4 <- ht-5fr.3 · parked — narrow to 4.3/4.5/4.12<-3.16, 4.4<-3.14, 4.8<-3.11, 4.15<-3.15; safe no: shared baseline SQL, facts/mod.rs
L38 · graph · GRAPH-EDGE · ht-5fr.2 <- ht-5fr.1 · parked — narrow to 2.1<-1.5, 2.3/2.12<-1.3, 2.4/2.11<-1.10, 2.7<-1.7, 2.10/2.12<-1.11; safe no: shared ids.rs, ports.rs, schema.rs, baseline SQL
L39 · graph · GRAPH-EDGE · ht-5fr.4.13 <- ht-5fr.4.12 · parked proposal — split-file src/store/receipts.rs (19→18 after narrows)
L40 · graph · GRAPH-EDGE · 38 critical leaf edges · kept — each carries a named artifact (see graph-pass judge output)
