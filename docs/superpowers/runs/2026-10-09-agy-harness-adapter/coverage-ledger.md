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
c16 · r2 · GAP · R16 goldens · applied — 2.7←7; 2.1, 3.2, 3.6, 8 ←2.7 (3.7 via 3.2/3.3); 2.7 also snapshots bootstrap argv
c17 · r2 · GAP · R13 run · applied — 6.9←9 and 6.9 runs scripts/outside-adapter-report, commits output
c18 · r2 · UNOWNED-SEAM · Outside-adapter trailer · applied — ht-poh.1 owns trailer format; 9←1 consumes; diff is authoritative
c19 · r2 · GAP · strict check · applied — new leaf ht-poh.2.8 (deps 2.2–2.6, 3.3, 3.4); 6.9←2.8
c20 · r2 · GAP · ht-poh.5 edges · applied — 5←3.2, 3.4, 4.4, 4.5, 4.7; CI end-to-end HookPull test in 5's acceptance
c21 · r2 · UNOWNED-SEAM · downgrade record/executable identity · applied — owned by ht-poh.5 (5 extends agy doctor.rs); 6.2 consumes identity
c22 · r2 · UNOWNED-SEAM · SKILL_MD · applied — 3.6 owns wake-on section; 4.4←3.6 consumes
c23 · r2 · UNOWNED-SEAM · delivery::page · applied — 3.2 owns addendum
c24 · r2 · UNOWNED-SEAM · wake_suppressed · applied — 3.6←3.3, 3.7←3.3; 3.6 consumes
c25 · r2 · UNOWNED-SEAM · setup-status JSON / prepare_startup_input · applied — owns addenda on 4.4 / 4.6
c26 · r2 · GAP · R7 handoff impl · applied — 4.6 retitled launch+handoff policy, 4.6←ht-poh.8; 4.8 docs+check only
c27 · r2 · GAP · ht-poh.8 ordering · applied — 8←1, 8←2.7, 8←2.2
c28 · r2 · GAP · 3.2←1, 3.3←3.2 · applied
c29 · r2 · GAP · R12 truncated · rejected — requirement-list extraction artefact; R12 text in the root spec names all three tiers (6.1 fixtures/offline, 6.2+6.4–6.7 gate, 6.3+6.8 live)
c30 · r2 · GAP · R2 refinement record · applied — root R2 and canonical R2 reworded; Refinements section already records D4
c31 · r2 · GAP · provenance of witness-derived role · rejected — cooperative_top_level is the existing binding provenance recorded by every hook check-in (TRUST A3); no new value is introduced
c32 · r2 · UNOWNED-SEAM · allow-member-only engine · rejected — ht-poh.2.1 owns SettingsKindSpec/OwnedAllowSpec + allow-member-only engine semantics (owns line); 4.4 reaches it via 2.3
c33 · r2 · UNOWNED-SEAM · lifecycle digest rule · applied — 3.4 owns omit_digest addendum; agy keeps today's lifecycle digest by default
c34 · r2 · UNOWNED-SEAM · composer capture server · applied — 4.5 uses the frozen probe copy, recipe recorded
c35 · r2 · GAP · Herdr host kind agy · applied — 4.5 acceptance asserts herdr reports kind agy
c36 · r2 · GAP · DP10 owner · rejected — DP10 is the delivery-policy spec's guard every .3 leaf's acceptance cites; no separate deliverable
c37 · r2 · GAP · agy fixtures owner · applied — 4.2 owns payload fixtures; 4.6 owns help fixture
