# super-auto run — 2026-10-13-message-priority-delivery

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
resumeChange: 2026-10-13 · "design APPROVED (A-E) ... Move to autonomous stage now (both roasts on). Nudge dev-herdr-threads when merge-ready." (relayed by seePN0MKW, mpqluOeQD) · autonomous=f→t after the interactive root design; nested brainstorms and later gates run Mode B
phase: report
codeMechanism: Workflow

idea: Feature: message PRIORITY replaces lazy/nudge delivery modes in herdr-threads. Interactive design stage first (user reachable only via seat seePN0MKW in 1:1 thread tXPkAdb0O with --nudge; batch questions), then autonomous after design approval; both roasts ON (design + code). When integration branch is merge-ready, nudge dev-herdr-threads (taQXwBH7X). Details: priority = non-negative integer chosen by sender; 0 highest, always nudges. Recipient nudge threshold T: priority <= T nudges; negative T means everything nudges. Lazy mode removed. Default T=0, default send priority=1. Inbox listing and hook/mod injection sort by priority ascending then timestamp oldest first. Needs design: CLI flags and migration of --lazy/--nudge/--require-ack (ACK requests implied nudge today), per-seat vs per-thread threshold, existing recipient wake-on mode, wire/storage compat, docs/skill text, mod/hook surfaces.
roast-design: 2026-10-13-message-priority-delivery-roast-design-1.md, 2026-10-13-message-priority-delivery-roast-design-2.md
roast-code: 2026-10-14-message-priority-delivery-roast-pr-1.md, 2026-10-14-message-priority-delivery-roast-pr-2.md, 2026-10-14-message-priority-delivery-roast-pr-post-cap-audit.md
spec: 2026-10-13-message-priority-delivery-design.md
epic: ht-yha
branch: super-auto/message-priority-delivery
base: main

roastDesignRound: 2
roastCodeRound: 2
roastCodeExit: converged

graph-pass: depth 8→7 · width 2.4→2.7 · applied 2 · parked 5

parked:
- super-code-final-review-wf_ecf6a4ce-925.md · escalation · "ht-yha.33 shipped option (c) (old CLI cannot follow a new daemon's next: inbox cursor; recorded in spec §Compatibility and docs/agent-usage.md) — orchestrator decision in the autonomous run, needs the user's ratification or another scheme"
- final-review (super-code, wf_24ee41b2-d4e) · escalation · "ServiceSend keeps receipts for its whole audience (AckAudience::Everyone), outside D9's 'receipts only for explicit --require-ack'; D9 was decided for agent/human sends — recommended default: keep ServiceSend unchanged as service-owned required delivery and record it as an accepted limit in TRUST-POLICY.md and the spec; user decision needed at hand-back"
- graph-pass · graph-change · "ht-yha.8 <- ht-yha.4 drop (same-file hydrate.rs, safe no)"
- graph-pass · graph-change · "ht-yha.6 <- ht-yha.5.2 drop (same-file read_cursors.rs, safe no)"
- graph-pass · graph-change · "ht-yha.4 <- ht-yha.3 drop (same-file staging.rs + real-send test, safe no; depth 7→6 with a staging.rs seam contract)"
- graph-pass · graph-change · "proposal: staging.rs seam contract for ht-yha.3/4/5.2"
- graph-pass · graph-change · "proposal: frame golden fixture seam for ht-yha.8/9"

codeBuckets:
  completed: ht-yha.1, ht-yha.2, ht-yha.3, ht-yha.4, ht-yha.5.1, ht-yha.5.2, ht-yha.5.3, ht-yha.5.4, ht-yha.6, ht-yha.7, ht-yha.8, ht-yha.9, ht-yha.10, ht-yha.11, ht-yha.12, ht-yha.13, ht-yha.14, ht-yha.15, ht-yha.16, ht-yha.31, ht-yha.32, ht-yha.33, ht-yha.34, ht-yha.35, ht-yha.36, ht-yha.37, ht-yha.38, ht-yha.39, ht-yha.40, ht-yha.41, ht-yha.42
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: not ready (A stale pending delivery rows via process-exited bindings and cursor-only body reads push new rows off first pages; B mod stall rule counts only receipts so a wedged mod holding a p0 delivery row never recovers; body --max-bytes 256-342 refused for p0; markers_ok vacuous; ServiceSend and ht-yha.33/.42 need the user)
  sweep: e8ecb681 — 6392 passed, 3 failed, 0 errors, 44 skipped; failing: session_bindings_smoke::config5_resume_within_a_page_and_past_it, golden_surfaces::doctor_reports, lazy_markers::q5_continuation_public::q5_search_public_refits_preserve_2048_byte_filter_route_actor_and_cursor; command: nice cargo nextest run --locked --all-targets --all-features --no-fail-fast @ e8ecb681
  slowness: round 1: graph-bound — 17 open beads, depth 6, achievable width 3 vs cap 8; edge audit armed now; round 1: merge queue peaked at 3 (first invocation); network outage relaunches ×2 (ENOTFOUND)
  worktreesKept:
  processSweep: stopped 0 · survived 0

friction: 14 events
sweepFix: 3 failing → ht-yha.40, ht-yha.41, ht-yha.42 · re-run e8ecb681 — 6392 passed, 3 failed, 0 errors, 44 skipped; failing: session_bindings_smoke::config5_resume_within_a_page_and_past_it, golden_surfaces::doctor_reports, lazy_markers::q5_continuation_public::q5_search_public_refits_preserve_2048_byte_filter_route_actor_and_cursor; command: nice cargo nextest run --locked --all-targets --all-features --no-fail-fast @ e8ecb681

approvals:
- launch-mode · interactive design (questions relayed via seat seePN0MKW, thread tXPkAdb0O, --nudge), switch to autonomous after the user approves the design (requester instruction, 2026-10-13)
- top-split · auto · ht-yha.1 LEAF, ht-yha.2 LEAF, ht-yha.3 LEAF, ht-yha.4 LEAF, ht-yha.5 PROMOTE, ht-yha.6 LEAF, ht-yha.7 LEAF, ht-yha.8 LEAF, ht-yha.9 LEAF, ht-yha.10 LEAF, ht-yha.11 LEAF, ht-yha.12 LEAF, ht-yha.13 LEAF
- coverage-round-1 · canonical R-list: coverage-round-1-requirements.md (R1-R12) · requirements: 12 · mapped: 12 · unmapped: 0 · R-new appended for round 2: R13 old clients on a new daemon + new handlers/capabilities, R14 in-flight pre-v47 state handled per spec, R15 one ordering verified across every surface, R16 delivery_mode-dependent store paths replaced · auto L1-L12 applied (owns lines on ht-yha.2/3/4/8/9/5.2, edges 9<-4, 8<-14, 11<-3/4/6/8, 5.4<-3, new leaf ht-yha.15) · L13 NEEDS-SPEC not re-run (answered from subspec)
- coverage-round-2 · R-list: coverage-round-2-requirements.md (R1-R16) · auto L14-L29 applied (spec §Compatibility amended: old-reader variants kept, WakeOnQuery reverse, old-daemon pair fallback, overdue-warning source, person-authored priority separate; owns lines on ht-yha.3/4/5.4/6/7/8/9/10/15; edges 11<-9, 5.2<-3, 8<-4, 10<-11, 10<-9) · L30 NEEDS-SPEC answered by the amendment · divergence: findings 12 → 16, novel 15/16 (94%), widening: yes (cap holds at two rounds; the design roast reviews round 2's fixes) · root integration sweep ht-yha.16 created
- stepBackDesign-round-1: patch — clusters C1 legacy readers, C2 pending predicate, C3 wire validators, C4 receipt side effects, C5 root/subspec drift; decisions D1-D11 hold
- design-roast-round-1 fixes · spec-only, applied inline: root spec §Legacy readers/wire validators/receipt side effects added, §Ordering per-binding displayed rule, Inbox v2 merge Must-not, NudgeThreshold Hides; subspec delivery-row filter; bead amendments ht-yha.1/2/3/4/5.2/5.4/6/7/8/9/16, edge 5.2<-4 · re-roast: yes (design decisions and data handling changed)
- design-roast exit · converged at round 2 (Should-fix (9 confirmed) [converged], 0 Blocking, 0 escalations) · punch list applied inline as spec/bead text, no re-roast: per-binding marks written on AdvanceCursors and copied on cursor inheritance, seat-level pending stays binding-based (V34), offline-peer join floor, note_published + decision_seq copy owned by ht-yha.4 (edges 4<-3, 6<-5.2; 5.2<-4 reason updated), messages_immutable drop/recreate order, priority-keyed lazy_manifest_source, human explicit-ACK wording, D9 wording per mXmhe1P2d · nothing declined
- stepBackCode-round-1: patch — four clusters (drain-halt, merge-readiness, seat-pending-predicate, batch-order-key); findings local, no approved decision changes
- scopeFilter-round-1: [Should-fix] src/delivery/page.rs:103; src/delivery/page.rs:101; src/delivery/page.rs:98 in-scope — The change made hook delivery pulls read the whole backlog before emitting, because a sink's Halt no longer stops paging. That is incorrect behavior in the goal-named hook page path.
- scopeFilter-round-1: [Should-fix] src/store/lazy_delivery/completion.rs:68 in-scope — The new ordered pending seek never lists a row for a reconnecting sibling binding that still has it unread. This is incorrect delivery of pending items in the goal-named ordered inbox and hook path.
- scopeFilter-round-1: [Should-fix] src/protocol/pagination.rs:538 in-scope — A new daemon issues format-2 cursors that an old client rejects. This breaks the old-client/new-daemon compatibility that D10 names.
- scopeFilter-round-1: [Should-fix] docs/superpowers/runs/2026-10-13-message-priority-delivery/run.md:34 in-scope — Model-visible AUTOMATIC_PENDING_GUIDANCE still describes pair invitations, contradicting D7. Upgrade tests and pair_accept_delivers_mixed_opening_mail_once are recorded as failing, so goal-named behavior is unverified.
- scopeFilter-round-1: [Should-fix] src/store/lazy_delivery/wake.rs:80; src/store/lazy_delivery/wake.rs:50 punch-list — cluster override: The other member of seat-pending-predicate is a correctness defect in the displayed-state predicate. This one only loses the old bound on wake probes and does not change which items wake.
- scopeFilter-round-1: [Should-fix] tests/store/priority_send.rs:480 in-scope — The design requires a v46-prepared, v47-retried send test. This is a missing test for the D10 stored-row migration.
- scopeFilter-round-1: [Nit] src/store/handoff/automatic.rs:320 punch-list — The failure needs a handoff to a human-bound seat, and no real path to that was established. This is an edge-case hardening issue, not a goal-named behavior.
- scopeFilter-round-1: [Nit] src/store/read_cursors.rs:497; src/store/read_cursors.rs:496 punch-list — The prune DELETE costs time linear in thread length. This is a performance improvement and does not change correctness.
- scopeFilter-round-1: [Nit] src/cli/send_priority.rs:66 in-scope — A transient capability probe failure silently downgrades the sender's chosen priority and blames the daemon version. That is a misleading result in the goal-named priority send and D10 capability-gate path.
- scopeFilter-round-1: [Nit] src/delivery/assemble.rs:412; src/delivery/assemble.rs:391 in-scope — Rust batch ordering breaks ties by SourceRank, contradicting its own doc comment and the mod's arrival-order tie-break. This is a goal-named ordering inconsistency under D11.
- scopeFilter-round-1: [Nit] integrations/claude/mod/tests/delivery.test.ts:2493 in-scope — The mod and Rust tests pin a priority-less message, even an ACK one, as p1. D10 maps ordinary to p0 and ACK requests are p0, so this is wrong behavior against an old daemon in the goal-named ordering path.
- scopeFilter-round-1: [Nit] src/store/control.rs:2242 punch-list — The test-only legacy marker is a documented seam, and D6/D7 have dedicated tests. This is a coverage-hygiene improvement and does not show a missing test for goal-named behavior.
- scopeFilter-round-1: [FYI] integrations/claude/mod/hooks/register.js:357 punch-list — The two-pump arrival ordering exists on base. The spec only promises ordering within a batch, so this is optional hardening.
- scopeFilter-round-1: [FYI] docs/superpowers/runs/2026-10-13-message-priority-delivery/run.md:21 punch-list — cluster override: The other merge-readiness member shows concrete D7 defects (a stale guidance string and failing tests). This one is an unchanged pre-existing behavior awaiting a user decision.
- scopeFilter-round-1: [FYI] src/store/schema.rs:835; src/store/schema.rs:836 punch-list — A forward-only migration is the project norm, it exists on base, and the goal names no rollback path. This is an optional operational note.
- scope-filter: 8 in-scope · 7 punch-listed
- fix-round-1 beads · ht-yha.31 drain-halt · ht-yha.32 seat-pending-predicate (completion.rs) · ht-yha.33 old-client cursor format · ht-yha.34 merge-readiness (failing tests, guidance, doc drift) · ht-yha.35 v46→v47 replay test · ht-yha.36 probe-failure downgrade · ht-yha.37 batch-order-key · ServiceSend Must-fix not filed (parked for the user)
- regressionPass-round-2: 3 filed · [Should-fix] src/store/lazy_delivery/mod.rs:112; src/store/lazy_delivery/wake.rs:13; src/store/lazy_delivery/mod.rs:109, [Should-fix] src/store/lazy_delivery/mod.rs:110; src/store/lazy_delivery/completion.rs:68, [Should-fix] integrations/skill/SKILL.md:17 · no re-roast (beads ht-yha.38, ht-yha.39; the amended ht-yha.33 rides the same re-entry)
- baseAbsorbed: 998d7d34 → 6657c2be, 3 conflicted files (AGENTS.md, TRUST-POLICY.md, scripts/wake-e2e.py); seam decision (orchestrator): a threshold-admitted delivery-row wake (reason lazy) counts as seat attention in for_seat_attention like pending_receipt; test one_process_composer_wake_serves_a_threshold_admitted_delivery_row
- root-design · human · D1-D11 and sections A-E approved via seePN0MKW (mwRZ1YxXs, myas79LQ8, mXmhe1P2d, mpqluOeQD)
