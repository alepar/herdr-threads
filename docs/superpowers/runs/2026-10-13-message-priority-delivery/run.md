# super-auto run — 2026-10-13-message-priority-delivery

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
resumeChange: 2026-10-13 · "design APPROVED (A-E) ... Move to autonomous stage now (both roasts on). Nudge dev-herdr-threads when merge-ready." (relayed by seePN0MKW, mpqluOeQD) · autonomous=f→t after the interactive root design; nested brainstorms and later gates run Mode B
phase: design
codeMechanism: Workflow

idea: Feature: message PRIORITY replaces lazy/nudge delivery modes in herdr-threads. Interactive design stage first (user reachable only via seat seePN0MKW in 1:1 thread tXPkAdb0O with --nudge; batch questions), then autonomous after design approval; both roasts ON (design + code). When integration branch is merge-ready, nudge dev-herdr-threads (taQXwBH7X). Details: priority = non-negative integer chosen by sender; 0 highest, always nudges. Recipient nudge threshold T: priority <= T nudges; negative T means everything nudges. Lazy mode removed. Default T=0, default send priority=1. Inbox listing and hook/mod injection sort by priority ascending then timestamp oldest first. Needs design: CLI flags and migration of --lazy/--nudge/--require-ack (ACK requests implied nudge today), per-seat vs per-thread threshold, existing recipient wake-on mode, wire/storage compat, docs/skill text, mod/hook surfaces.
spec: 2026-10-13-message-priority-delivery-design.md
epic: ht-yha
branch: super-auto/message-priority-delivery
base: main

approvals:
- launch-mode · interactive design (questions relayed via seat seePN0MKW, thread tXPkAdb0O, --nudge), switch to autonomous after the user approves the design (requester instruction, 2026-10-13)
- root-design · human · D1-D11 and sections A-E approved via seePN0MKW (mwRZ1YxXs, myas79LQ8, mXmhe1P2d, mpqluOeQD)
