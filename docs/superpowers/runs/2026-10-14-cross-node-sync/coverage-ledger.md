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
