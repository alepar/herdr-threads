# super-auto run — 2026-10-14-cross-node-sync

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: finish
roastCodeExit: converged
roastCodeRound: 2
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

code-run-3: coordinator wf_d24bf449-543 · stopReason closure-unconfirmed (ht-5fr.15 landed, review clean, bead left open → closed by session) · all 111 beads landed
fix-beads-run-3: ht-5fr.5.29 (dial-each-other stall) · ht-5fr.4.37 (store-only seat restore) · ht-5fr.4.38 (register actor) · ht-5fr.4.40 (actor bound to signing node, security) · ht-5fr.3.37 — all merged
main-merge-2: 8ed588a10 merge main (v1.0.0, v1.0.1, ht-yha priority model; 0047 folded into node baseline) + 88eebd5f1 / 8d33a7afd / 224168876 / bca0c246f — reconcile items 1–10 done; 57 tests ignored pending items 11/17/19/22
reconcile-beads: ht-5fr.31 (items 11,12,13,15) · ht-5fr.32 (14,16) · ht-5fr.33 (17) · ht-5fr.34 (18) · ht-5fr.35 (19) · ht-5fr.36 (20,21) · ht-5fr.37 (22,23); item 24 done in 56e8b1e42
fix-beads-pre-roast: ht-5fr.38 (two branch test failures) · ht-5fr.39 (stale docs, dead_code) · ht-5fr.40 (envelope parser differential) · ht-5fr.41 (serve resource bounds) · ht-5fr.42 (unsigned Have claims) · ht-5fr.43 (message.sent apply checks) · ht-5fr.44 (load-bearing remote-seat filter tests) · ht-5fr.45 (coordinator minors) · ht-5fr.46 (apply/startup minors)

code-run-4: coordinator wf_3465ea49-f0a · stopReason closure-unconfirmed (stale: ht-5fr.15 already closed) · 129 merges, all reconcile and pre-roast beads merged (+ ht-5fr.49 two-node auto-ACK, ht-5fr.51 accept-grace) · final review: not ready — overdue hold lifts before ingested settlements apply (config_wipe flake); hold ignores unlinked dial targets; loop_inventory fails; main 7 commits behind (v1.1.0)
fix-beads-run-4: ht-5fr.53 (hold waits for apply) · ht-5fr.54 (hold counts unlinked dial targets) · ht-5fr.55 (loop_inventory allowlist) · ht-5fr.56 (silent local-only fallbacks → errors) · ht-5fr.57 (SKILL.md wording, pre-accept body flush coverage)
main-merge-3: REFUSED by the permission layer (session merge of main v1.1.0, 7 commits) — left for the human; must precede the merge to main

code-run-5: coordinator wf_b06245f4-b3c · stopReason closure-unconfirmed (stale ht-5fr.15) · ht-5fr.53–.58 merged, all ht-5fr beads closed; stale ht-33gb closed · final review: no new product defect; must-fix: main v1.1.0 merge (human), full suite on merged tree, landing order vs ht-794l (human); follow-ups: journaled_route test-build fallback, author_answer local-only route

fix-round-1: coordinator wf_014f01df-1a7 · ht-5fr.60, ht-5fr.61 merged · final review: not ready — main v1.1.0 merge, license allowlist (release blocker; punch-listed r1), tag_k walk and JoinError (punch-listed r1), landing order vs ht-794l, no full suite

codeBuckets:
  completed: all ht-5fr beads (148 landed across 8 coordinator invocations; last wf_933c0649-482)
  escalated:
  pendingRetry:
  parked: ht-5fr.31, ht-5fr.62
  stalled: false
  review: not ready (main v1.1.0 not merged; landing order vs ht-794l undecided; release tooling breaks on main — about.toml lacks BSD-3-Clause, 8-char ID regexes in wake-e2e/validate-package/real-ssh-e2e; punch-list robustness items open)
  sweep: 7e558f4 — 7438 passed, 0 failed, 0 errors, 47 skipped; failing: none; command: nice cargo nextest run --locked --all-targets --all-features (wall 188 s; leaks none; check-default-features pass) @ 7e558f43a
  slowness: run 1 merge queue peaks; run 4 round 1 merge queue peak 4
  worktreesKept:
  processSweep: stopped 0 · survived 0
sweepFix: 8 failing → ht-5fr.62, ht-5fr.63, ht-5fr.64, ht-5fr.65 · re-run 7e558f4 — 7438 passed, 0 failed, 0 errors, 47 skipped; failing: none; command: nice cargo nextest run --locked --all-targets --all-features @ 7e558f43a

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
- decision · main v1 reconcile (2026-10-15-cross-node-sync--main-v1-reconcile-design.md, 23894daad) · "payloads adopt priority 0..9 + require_ack in place at v1; invitation.created drops lazy; cross-node pair auto-join authored by each seat's home node (direct_auto_join provenance); protocol 9"
- approval · user · 2026-10-15 · clean start for v1.0.x users CONFIRMED (CHANGELOG marker removed)
- approval · user · 2026-10-15 · three autonomous decisions (inviter_kind; store-only seat restore on host attach; register actor bound to the signing node) LGTM
- approval · user · 2026-10-15 · main-v1 reconcile spec amendments (23894daad) LGTM
- graph-change · L33–L38 · applied mid-run on user approval (relayed via herdr-threads 1:1; user ran narrow.py)
- decision · main-merge-2 · "until ht-5fr.31 lands, a new pair still creates a pair invitation, now under main's always-confirming policy (wakes the peer); no lazy invitation kept"
feedback: evidence-only-fix pipeline defect sent via herdr agent prompt to superpowers main tab wB:p8 (.superpowers/sdd/ht-5fr-plan/upstream-feedback-evidence-only-fix.md)
- decision · run-4 final review item 2 · "overdue hold: root spec §7 ('every linked peer and every enabled dial target') wins over the peer-link sub-spec's active-links narrowing — code fixed in ht-5fr.54, sub-spec gets a dated note"
- coordination · ht-794l (peer seat snbemykTY, branch super-auto/inbox-injection-auto-ack) · "user ruling relayed: display auto-ACKs across nodes; agreed wire how=policy, no SettleHow variant, capability hook.context_receipts_v1; adapter OnHold::Fail; their combined proof aad7798bc green 51/51; this branch's two-node coverage ht-5fr.49"
- stepBackCode-round-1: patch — Round 1: the findings are mostly independent defects; three of them are clusters where one rule was applied unevenly (node-scoped context resolution, sweeping consumers of the old instances/ layout, JoinError handling in the coordinator loop). Sweep each cluster with its rule. No redesign is needed.
- scopeFilter-round-1: 4 in-scope ([Blocking] src/main.rs:117, [Should-fix] src/cli/mod.rs:616, [Should-fix] src/sync/coordinator/ingest.rs:203, [Should-fix] scripts/install.sh:383) · 11 punch-listed (see 2026-10-16-cross-node-sync-roast-pr-1-punch-list.txt; validate-native-demo.py and paths.rs by clusterOverride) · cluster dropped: join-error · fix beads ht-5fr.60 (node-context cluster), ht-5fr.61 (credit drop)
- roast-code round 2 · Should-fix (2 confirmed) [converged] · 0 Blocking, 4 resolved, 11 punch-listed (open) · new: [Should-fix] scripts/wake-e2e.py:874 (11-char tagged IDs vs 8-char regexes in wake-e2e, validate-package.sh, real-ssh-e2e.py), [Nit] src/sync/coordinator/ingest.rs:294 (shared WorkerStatus slot) → punch list
- decision · ht-5fr.62 / ht-sjmt (autonomous) · "summary_flow useful-page tests seeded legacy receipts rows that cannot exist under the clean start: rewrite with real sends (history too); a p1 mail shown 'empty' behind real history is a reader bug to fix, not an assertion to relax"
- post-loop changes: 15112f108..7e558f43a touched tests only (peer_stdio, topology_handoff, agy_delivery, summary_flow, token_diet) — no behaviour change, so no post-cap audit roast; listed in report Remaining as not roast-reviewed
- baseAbsorbed: not absorbed — main 3ff902c77 (v1.1.0, 7 commits) merge refused by the permission layer; left to the human; the sweep did not judge the combined tree
friction: 31 events
- upstream-feedback: draft parked at upstream-feedback-draft.md (6 defects, 2 design questions, 3 doc gaps), awaiting the user's file/scrub decision at the phase-7 menu
