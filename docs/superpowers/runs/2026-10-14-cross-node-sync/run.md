# super-auto run — 2026-10-14-cross-node-sync

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: code
codeMechanism: Workflow

idea: Cross-node herdr-threads sync. Seed: the vision spec docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md plus research in docs/superpowers/research/2026-10-13-cross-node-sync/. Write the actual (detailed) spec, checking for any gaps, split into beads; all autonomously; both roasts on.
spec: 2026-10-14-cross-node-sync-design.md
epic: ht-5fr
roast-design: 2026-10-14-cross-node-sync-roast-design-1.md, 2026-10-14-cross-node-sync-roast-design-2.md
branch: super-auto/cross-node-sync
base: main

roastDesignRound: 2

stepBackDesign-round-1: patch — 5 findings mostly independent; clusters clone-identity and order-independent-apply swept as rules

roastDesignExit: converged at round 2 — Should-fix (6 confirmed) [converged], 0 Blocking, 5 resolved, 0 regressed, 0 escalations; punch list (4 Should-fix + 2 Nit: restore-older-than-rotation trips clone gate; own-old-incarnation suppression drops unsettled ACK obligations; restore-window same-key authoring [fix-regression]; cold-start overdue hold; memory-clone boot id; root PeerHello lacks boot) applied as spec patches without re-roast

graph-pass: depth 30→30 · width 3.2→3.2 · applied 2 · parked 8 — the parked epic narrows (L33–L38) together give depth 30→19, width 3.2→5.1; all safe no on shared-file overlap only

code-run-1: coordinator wf_516906c6-9a5 · stopReason ready-drained · completed 79 · escalated ht-5fr.4.2 · final review: not ready (fact kinds unfinished; base 89 commits behind main) · metrics: merges 79, rebase-conflicts 14, seam-reviews 31 (fixed 2), check-fails 1 (fixed 1), fix-pass entered 5 / FIXED 2 / BLOCKED 3, cancelled 5
graph-change-applied: L33–L38 epic narrows applied mid-run on user approval (relayed via herdr-threads 1:1; user ran narrow.py) — adds 4.1<-1.5, 2.1<-1.6, removes 8 epic-level edges
fix-beads-mid-run: ht-5fr.3.21 (self-log AheadOfSelf verify/dedupe, security review) merged · ht-5fr.3.32 (commit_kicks fixture vs seeded sync_logs row) merged · ht-5fr.18 (Linux process-table discovery reads other users' processes, security review) merged

main-merge: fe24ebf18 merge main into run branch (89 commits, 41 conflict files, schema baseline unchanged) + d01f73dd1 temp_path conversion
code-run-2: coordinator wf_03c7a2f1-6c4 · stopReason stalled (ht-5fr.3.37 left un-redispatched after a triage wait edge) · completed 105 total · final review: not ready — 3.37 unmerged; dial-each-other stall; store-only seat restore after wipe; register events lost actor (regression from main)
fix-beads-run-2: ht-5fr.3.37 · ht-5fr.6.30 · ht-5fr.6.31 · ht-5fr.5.27 · ht-5fr.2.25 · ht-5fr.22 · ht-5fr.23 · ht-5fr.24 (loop_inventory) — all merged except 3.37

approvals:
- top-split · auto · ht-5fr.1 PROMOTE, ht-5fr.2 PROMOTE, ht-5fr.3 PROMOTE, ht-5fr.4 PROMOTE, ht-5fr.5 PROMOTE, ht-5fr.6 PROMOTE, ht-5fr.7 LEAF, ht-5fr.8 LEAF, ht-5fr.9 LEAF, ht-5fr.10 LEAF, ht-5fr.11 LEAF, ht-5fr.12 LEAF, ht-5fr.13 LEAF
- coverage-round-1 · canonical R-list: R1–R16 (coverage-round-1-requirements.md), R-new appended as R17–R20 · requirements: 16 · mapped: 16 · unmapped: 0 · auto GAP R17 → ht-5fr.4.5 amended + 4.18 case; auto GAP R18 → ht-5fr.5.2 amended; auto GAP R19 → leaf ht-5fr.1.12; auto GAP R20 → leaf ht-5fr.1.13; auto GAP real-ssh → leaf ht-5fr.14; auto GAP ht-5fr.7/.11/.12 → edges + scope; auto UNOWNED-SEAM status reads → ht-5fr.8 owns; auto UNOWNED-SEAM marker rendering → ht-5fr.4.14/ht-5fr.13; auto reject wake-e2e (spec follow-on); auto reject leaf decomposition (promotion verdict); NEEDS-SPEC (ht-5fr.7/.8/.13, leaves under root spec) not re-run — the findings already named the gaps and were fixed
- coverage-round-2 · requirements: 20 · mapped: 20 · unmapped: 0 · divergence: findings 12 → 13, novel 92%, widening: yes · auto applied L18–L27, L29, L30 (see coverage-ledger.md); auto reject L28 same-name threads (spec §2) · no round 3 (cap); widening recorded for the design roast

parked:
- 2026-10-14-cross-node-sync-roast-design-1.md · escalation · "material dissent — §10 union re-issue / same-key facts (membership transitions, invitation.answered, receipt.settled): does the incarnations §13 same-ordinal accepted limit cover invitation.answered and receipt.settled, and must root §1/§10 'every node converges' name it"
- 2026-10-14-cross-node-sync-roast-design-1.md · escalation · "material dissent — §9 duplicate links vs cloned VMs at a third node: whether standby promotion reliably surfaces the fork, and whether a third node should raise node_collision for same node_uuid with different boot ids across links"
- 2026-10-14-cross-node-sync-roast-design-1.md · escalation · "material dissent — §4 payload bytes inside the JSON frame vs §9 1 MiB frame cap: the fit bound holds only if payload is embedded as raw JSON (encoding not stated in root); root §10 'linked forward in Have' contradicts incarnations line 170"
- graph-change · L33 · "ht-5fr.3 <- ht-5fr.1: parked — narrow to 3.2<-1.1, 3.4<-1.5, 3.6<-1.10, 3.10<-1.6 (30→23 alone); safe no: shared schema.rs, baseline SQL, app.rs, workers.rs, test_support"
- graph-change · L34 · "ht-5fr.5 <- ht-5fr.3: parked — narrow to existing 5.1<-3.2, 5.3<-3.12, 5.5/5.12/5.13<-3.9, 5.12<-3.16, 5.10<-3.8/3.13/3.15; safe no: shared sync/mod.rs, app.rs, store/mod.rs"
- graph-change · L35 · "ht-5fr.6 <- ht-5fr.5.5/5.12/5.13: parked — narrow to existing 6.2<-5.5, 6.8/6.9<-5.12, 6.8<-5.13; safe no: shared coordinator.rs, sync/mod.rs, kicks.rs"
- graph-change · L36 · "ht-5fr.4 <- ht-5fr.1: parked — narrow to 4.2<-1.10, 4.8<-1.6, NEW 4.1<-1.5; safe no: shared schema.rs, baseline SQL, test_support"
- graph-change · L37 · "ht-5fr.4 <- ht-5fr.3: parked — narrow to 4.3/4.5/4.12<-3.16, 4.4<-3.14, 4.8<-3.11, 4.15<-3.15; safe no: shared baseline SQL, facts/mod.rs"
- graph-change · L38 · "ht-5fr.2 <- ht-5fr.1: parked — narrow to 2.1<-1.5, 2.3/2.12<-1.3, 2.4/2.11<-1.10, 2.7<-1.7, 2.10/2.12<-1.11; safe no: shared ids.rs, ports.rs, schema.rs, baseline SQL"
- graph-change · L39 · "ht-5fr.4.13 <- ht-5fr.4.12: parked proposal — split-file src/store/receipts.rs (19→18 after narrows)"
- decision · ht-5fr.4.2 / ht-52dq · "spec gap: inviter-less service/operator invites on journaled threads — chose (d) now + (b) in ht-5fr.4.10: inviter_kind seat|service|operator, optional inviter, origin-node authority for non-seat kinds; fact-kinds §2/§6 amendment owed"
- security-followup · phase-4 roast · "envelope.rs payload parser differential (dup keys / non-canonical JSON vs handlers); serve.rs/coordinator.rs peer-controlled max_frame lower bound, subs_out growth, serve-job deadline; incarnations/worker.rs unsigned Have claims must not drive evidence/restore/hold"
- pipeline-defect · super-code · "evidence-only fix pass with no commit classed BLOCKED (3x: 1.2, 1.6, 2.4), cancelling 5 stacked tasks; local coordinator patch refused by permission layer (self-modification), reverted"
- decision · run-2 review finding 3 · "store-only daemon cannot restore own seats after wipe — chose: retry own seat.created (and entries parked behind it) on first host attach and at startup, instead of an operator replay step"
- decision · run-2 review finding 4 · "register events (topic/name/archive) lost actor — chose: optional actor field on thread.register payload, restoring main's behaviour"
