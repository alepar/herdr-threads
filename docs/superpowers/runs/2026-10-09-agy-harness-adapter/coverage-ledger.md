# Coverage ledger — 2026-10-09-agy-harness-adapter

c1 · r1 · GAP · R13 · applied — new leaf ht-poh.9 outside-adapter accounting tool (run by orchestrator at final SHA)
c2 · r1 · GAP · agy-after-sweep ordering · applied — leaf edges 4.4←2.3, 4.6←2.2, 4.7←2.4, 4.7←2.3, 4.1←2.6
c3 · r1 · GAP · R14 final-SHA evidence · applied — gate ht-poh.10 "Gate: fix loop exited" + leaf ht-poh.6.9 (fallback declared)
c4 · r1 · GAP · R16 golden baselines · applied — leaf ht-poh.2.7, gating 2.2–2.6 and 3.3
c5 · r1 · GAP · R8 CI wiring · rejected — ht-poh.1 description already states CI + per-change wiring (title amended to say so)
c6 · r1 · GAP · R3 resume/C1 · applied — ht-poh.4.3 title names resume via C1 (description already had the C1 acceptance)
c7 · r1 · GAP · R5 agy IdleWake · applied — ht-poh.5 description names IdleWake::Native + composer path; no new edge (wake uses existing dispatcher)
c8 · r1 · GAP · R7 bootstrap V2 wiring · applied — .4 edges rebuilt (4.8←ht-poh.8 was lost to a comma-parse defect); 6.4←ht-poh.8, 6.4←4.8, handoff flow added to 6.4
c9 · r1 · GAP · R15 context · applied — ht-poh.4.3 title names startup check-in context
c10 · r1 · GAP · TRUST provenance · rejected — ht-poh.3.4 lands cooperative_hook_delivery (DP9 L4) and ht-poh.4.3 the witness text (D9); descriptions carry it
c11 · r1 · ORPHAN · ht-poh.7 · applied — ht-poh.1 narrowed to the script/allowlist seam; ht-poh.7 owns AGENTS.md text + audit
c12 · r1 · UNOWNED-SEAM · DeliveryPolicy/wire, local_evidence/hook_step, scenario format, agyenv API, relay, wake_on, src/delivery API, allowlist format, composer · rejected — inputs defect: owns lines were inline bold; normalized onto own lines in all 31 beads (owners: 3.1, 4.1, 6.1, 6.2, 6.3, 3.5, 3.2, ht-poh.1, 4.5)
c13 · r1 · UNOWNED-SEAM · offered ledger 3.4↔5 · applied — consumes line on ht-poh.5 (owner 3.4)
c14 · r1 · GAP · runner deps 6.8 · applied — 6.8←3.4, 6.8←3.6
c15 · r1 · NEEDS-SPEC · ht-poh.2/3/4/6 goals · rejected — inputs defect (spec headers matched sibling ids); round 2 inputs use header-normalized specs
