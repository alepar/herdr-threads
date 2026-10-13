# super-auto run — 2026-10-13-message-priority-delivery

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
resumeChange: 2026-10-13 · "design APPROVED (A-E) ... Move to autonomous stage now (both roasts on). Nudge dev-herdr-threads when merge-ready." (relayed by seePN0MKW, mpqluOeQD) · autonomous=f→t after the interactive root design; nested brainstorms and later gates run Mode B
phase: code
codeMechanism: Workflow

idea: Feature: message PRIORITY replaces lazy/nudge delivery modes in herdr-threads. Interactive design stage first (user reachable only via seat seePN0MKW in 1:1 thread tXPkAdb0O with --nudge; batch questions), then autonomous after design approval; both roasts ON (design + code). When integration branch is merge-ready, nudge dev-herdr-threads (taQXwBH7X). Details: priority = non-negative integer chosen by sender; 0 highest, always nudges. Recipient nudge threshold T: priority <= T nudges; negative T means everything nudges. Lazy mode removed. Default T=0, default send priority=1. Inbox listing and hook/mod injection sort by priority ascending then timestamp oldest first. Needs design: CLI flags and migration of --lazy/--nudge/--require-ack (ACK requests implied nudge today), per-seat vs per-thread threshold, existing recipient wake-on mode, wire/storage compat, docs/skill text, mod/hook surfaces.
roast-design: 2026-10-13-message-priority-delivery-roast-design-1.md, 2026-10-13-message-priority-delivery-roast-design-2.md
spec: 2026-10-13-message-priority-delivery-design.md
epic: ht-yha
branch: super-auto/message-priority-delivery
base: main

roastDesignRound: 2

graph-pass: depth 8→7 · width 2.4→2.7 · applied 2 · parked 5

parked:
- graph-pass · graph-change · "ht-yha.8 <- ht-yha.4 drop (same-file hydrate.rs, safe no)"
- graph-pass · graph-change · "ht-yha.6 <- ht-yha.5.2 drop (same-file read_cursors.rs, safe no)"
- graph-pass · graph-change · "ht-yha.4 <- ht-yha.3 drop (same-file staging.rs + real-send test, safe no; depth 7→6 with a staging.rs seam contract)"
- graph-pass · graph-change · "proposal: staging.rs seam contract for ht-yha.3/4/5.2"
- graph-pass · graph-change · "proposal: frame golden fixture seam for ht-yha.8/9"

approvals:
- launch-mode · interactive design (questions relayed via seat seePN0MKW, thread tXPkAdb0O, --nudge), switch to autonomous after the user approves the design (requester instruction, 2026-10-13)
- top-split · auto · ht-yha.1 LEAF, ht-yha.2 LEAF, ht-yha.3 LEAF, ht-yha.4 LEAF, ht-yha.5 PROMOTE, ht-yha.6 LEAF, ht-yha.7 LEAF, ht-yha.8 LEAF, ht-yha.9 LEAF, ht-yha.10 LEAF, ht-yha.11 LEAF, ht-yha.12 LEAF, ht-yha.13 LEAF
- coverage-round-1 · canonical R-list: coverage-round-1-requirements.md (R1-R12) · requirements: 12 · mapped: 12 · unmapped: 0 · R-new appended for round 2: R13 old clients on a new daemon + new handlers/capabilities, R14 in-flight pre-v47 state handled per spec, R15 one ordering verified across every surface, R16 delivery_mode-dependent store paths replaced · auto L1-L12 applied (owns lines on ht-yha.2/3/4/8/9/5.2, edges 9<-4, 8<-14, 11<-3/4/6/8, 5.4<-3, new leaf ht-yha.15) · L13 NEEDS-SPEC not re-run (answered from subspec)
- coverage-round-2 · R-list: coverage-round-2-requirements.md (R1-R16) · auto L14-L29 applied (spec §Compatibility amended: old-reader variants kept, WakeOnQuery reverse, old-daemon pair fallback, overdue-warning source, person-authored priority separate; owns lines on ht-yha.3/4/5.4/6/7/8/9/10/15; edges 11<-9, 5.2<-3, 8<-4, 10<-11, 10<-9) · L30 NEEDS-SPEC answered by the amendment · divergence: findings 12 → 16, novel 15/16 (94%), widening: yes (cap holds at two rounds; the design roast reviews round 2's fixes) · root integration sweep ht-yha.16 created
- stepBackDesign-round-1: patch — clusters C1 legacy readers, C2 pending predicate, C3 wire validators, C4 receipt side effects, C5 root/subspec drift; decisions D1-D11 hold
- design-roast-round-1 fixes · spec-only, applied inline: root spec §Legacy readers/wire validators/receipt side effects added, §Ordering per-binding displayed rule, Inbox v2 merge Must-not, NudgeThreshold Hides; subspec delivery-row filter; bead amendments ht-yha.1/2/3/4/5.2/5.4/6/7/8/9/16, edge 5.2<-4 · re-roast: yes (design decisions and data handling changed)
- design-roast exit · converged at round 2 (Should-fix (9 confirmed) [converged], 0 Blocking, 0 escalations) · punch list applied inline as spec/bead text, no re-roast: per-binding marks written on AdvanceCursors and copied on cursor inheritance, seat-level pending stays binding-based (V34), offline-peer join floor, note_published + decision_seq copy owned by ht-yha.4 (edges 4<-3, 6<-5.2; 5.2<-4 reason updated), messages_immutable drop/recreate order, priority-keyed lazy_manifest_source, human explicit-ACK wording, D9 wording per mXmhe1P2d · nothing declined
- root-design · human · D1-D11 and sections A-E approved via seePN0MKW (mwRZ1YxXs, myas79LQ8, mXmhe1P2d, mpqluOeQD)
