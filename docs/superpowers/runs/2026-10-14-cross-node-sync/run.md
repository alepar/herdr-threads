# super-auto run — 2026-10-14-cross-node-sync

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: roast-design
codeMechanism: Workflow

idea: Cross-node herdr-threads sync. Seed: the vision spec docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md plus research in docs/superpowers/research/2026-10-13-cross-node-sync/. Write the actual (detailed) spec, checking for any gaps, split into beads; all autonomously; both roasts on.
spec: 2026-10-14-cross-node-sync-design.md
epic: ht-5fr
roast-design: 2026-10-14-cross-node-sync-roast-design-1.md
branch: super-auto/cross-node-sync
base: main

roastDesignRound: 2

stepBackDesign-round-1: patch — 5 findings mostly independent; clusters clone-identity and order-independent-apply swept as rules

approvals:
- top-split · auto · ht-5fr.1 PROMOTE, ht-5fr.2 PROMOTE, ht-5fr.3 PROMOTE, ht-5fr.4 PROMOTE, ht-5fr.5 PROMOTE, ht-5fr.6 PROMOTE, ht-5fr.7 LEAF, ht-5fr.8 LEAF, ht-5fr.9 LEAF, ht-5fr.10 LEAF, ht-5fr.11 LEAF, ht-5fr.12 LEAF, ht-5fr.13 LEAF
- coverage-round-1 · canonical R-list: R1–R16 (coverage-round-1-requirements.md), R-new appended as R17–R20 · requirements: 16 · mapped: 16 · unmapped: 0 · auto GAP R17 → ht-5fr.4.5 amended + 4.18 case; auto GAP R18 → ht-5fr.5.2 amended; auto GAP R19 → leaf ht-5fr.1.12; auto GAP R20 → leaf ht-5fr.1.13; auto GAP real-ssh → leaf ht-5fr.14; auto GAP ht-5fr.7/.11/.12 → edges + scope; auto UNOWNED-SEAM status reads → ht-5fr.8 owns; auto UNOWNED-SEAM marker rendering → ht-5fr.4.14/ht-5fr.13; auto reject wake-e2e (spec follow-on); auto reject leaf decomposition (promotion verdict); NEEDS-SPEC (ht-5fr.7/.8/.13, leaves under root spec) not re-run — the findings already named the gaps and were fixed
- coverage-round-2 · requirements: 20 · mapped: 20 · unmapped: 0 · divergence: findings 12 → 13, novel 92%, widening: yes · auto applied L18–L27, L29, L30 (see coverage-ledger.md); auto reject L28 same-name threads (spec §2) · no round 3 (cap); widening recorded for the design roast

parked:
- 2026-10-14-cross-node-sync-roast-design-1.md · escalation · "material dissent — §10 union re-issue / same-key facts (membership transitions, invitation.answered, receipt.settled): does the incarnations §13 same-ordinal accepted limit cover invitation.answered and receipt.settled, and must root §1/§10 'every node converges' name it"
- 2026-10-14-cross-node-sync-roast-design-1.md · escalation · "material dissent — §9 duplicate links vs cloned VMs at a third node: whether standby promotion reliably surfaces the fork, and whether a third node should raise node_collision for same node_uuid with different boot ids across links"
- 2026-10-14-cross-node-sync-roast-design-1.md · escalation · "material dissent — §4 payload bytes inside the JSON frame vs §9 1 MiB frame cap: the fit bound holds only if payload is embedded as raw JSON (encoding not stated in root); root §10 'linked forward in Have' contradicts incarnations line 170"
