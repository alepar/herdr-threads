# Report: message priority replaces lazy/nudge delivery (2026-10-13-message-priority-delivery)

status: clean [degraded: sweep: 9de1002a — 6418 passed, 2 failed, 0 errors, 44 skipped; failing: cli::topology_handoff::tests::live::downstream_new_thread_create_links_parent_child_in_same_real_transaction, package manifest::failpoints_are_compiled_out_of_release_builds (timeout); command: nice cargo nextest run --locked --all-targets --all-features --no-fail-fast @ 9de1002a]
qualifier: sweep: 9de1002a — 6418 passed, 2 failed, 0 errors, 44 skipped; failing: cli::topology_handoff::tests::live::downstream_new_thread_create_links_parent_child_in_same_real_transaction, package manifest::failpoints_are_compiled_out_of_release_builds (timeout); command: nice cargo nextest run --locked --all-targets --all-features --no-fail-fast @ 9de1002a
metrics: upstream feedback delivered to the superpowers workspace agent (herdr pane wB:pB), draft docs/superpowers/runs/2026-10-13-message-priority-delivery/upstream-feedback-draft.md

**Second update (2026-10-14), supersedes the open decision above.** The user chose connected-only views: ht-yha.57 drops a session's pending view on any disconnect and rebuilds it from the cursor on reconnect, so a died session costs only its cursor and self-pruning marks (TRUST-POLICY.md A8). ht-yha.58 fixed a real missed-p0-wake bug behind flake ht-3eic: the mod channel stamped its Attention push with a clock read taken before the store read, so a row committed in between never armed the stall handover (regression test red before, green after; 30/30 under load). ht-yha.59 synced the spec and dropped dead migration lines. Main 6d8bc095 merged at 9de1002a. Final review wf_cee76482-6b1: no Critical/Important defect. Sweep @ 9de1002a under host load average ~62: 6418 passed, 2 failed (topology_handoff live test passes 5/5 isolated, filed ht-p1eu; the release-build manifest test hit its 300 s timeout, ht-wur); no leaks. (git log; bd; sweep-9de1002a.log; super-code-final-review-wf_cee76482-6b1.md)

**Update after the hand-back (2026-10-14) — supersedes the Remaining items 1-5 and the escalations below.** The user approved a fix run, then decided: ServiceSend follows D9; each session keeps its own pending view; break backwards compatibility with protocol 8 and crate 1.0.0; the stale-test rewrites (ht-yha.41/.42) are fine; the two leftovers were fixed; the merge into main goes to the main tab. (run.md resumeChange and approvals)
- Fixed and merged: ht-yha.44 + ht-yha.54 per-session pending views (binding_pending; no delivery or wake decision reads the seat-wide state; views only for bindings that can still be offered rows) · ht-yha.45 lazy composer wake not owed to a binding that already displayed the row · ht-yha.46 mod stall rule counts admitted p0 delivery rows · ht-yha.47 ServiceSend receipts only for explicit recipients · ht-yha.49 PROTOCOL_VERSION 8, crate 1.0.0, breaking CHANGELOG entry · ht-yha.50 small --max-bytes reads of p0 messages · ht-yha.51 markers_ok fails without a header · ht-yha.52 deadline-class walks see only pending published items (config5 passes) · ht-yha.53 dead old-version fallbacks removed · ht-yha.55 spec synced with Post-Implementation Notes. (bd; ledger)
- Main dc2996c3 merged at a2f1ae2d (3 small conflicts); q4 retirement fixture fixed 0873fa12; skew test pairs protocol 7/8 db18eefc; unreleased v47 repair paths dropped e6de8153 so the schema audit rejects altered objects. (git log)
- Sweep @ e6de8153: 6412 passed, 4 failed (doctor stub-binary race ht-92dd, mod stall timing ht-3eic, two Hermes capture timeouts ht-uy3); all four pass 3/3 in isolation, 245 s, no leaks. (sweep-e6de8153.log; bd)
- **Open decision:** a binding whose process exited (a crash or a killed pane) is never ended, so its own pending view grows by one row per publish to its seat; it costs no other session. Accept as recorded in TRUST-POLICY.md, or bound it (drop the view when a sibling takes over or an end event follows the exit; re-seed on resume). (super-code-final-review-wf_352db1df-8a1.md)
- Not run: the pre-release wake e2e against this 1.0.0 candidate (a person runs it before a release).

## Implemented

All 31 tasks in `codeBuckets.completed` closed. Epic ht-yha and sub-epic ht-yha.5 are closed.
- ht-yha.13: split src/store/lazy_delivery.rs into modules, commits 086e185..3d85b5e, review clean (bd; ledger L6)
- ht-yha.2: schema v47 with priority, nudge threshold and ordered pending indexes, commits 88fe85c..f0c3691, clean (bd; ledger L10)
- ht-yha.1: priority protocol types and wire fields, commits f0c3691..864e67d, clean (bd; ledger L15)
- ht-yha.14: split the inbox_v2 module out of src/store/queries.rs, commits 0a24b6c..8f5efc8, clean (bd; ledger L22)
- ht-yha.5.1: inbox v2 cursor format v2, commits a1f2892..2caa7dd, clean (bd; ledger L26)
- ht-yha.5.3: DeadlineWindow, commits 2caa7dd..5cd52d0, clean (bd; ledger L62)
- ht-yha.3: send publication by priority (delivery rows for everyone, receipts only for explicit ACK), commits 5cd52d0..d9f19be, clean (bd; ledger L68)
- ht-yha.4: wake by per-seat nudge threshold, commits d9f19be..933f979, clean (bd; ledger L76)
- ht-yha.7: CLI `--priority/-p`, `--nudge` alias, `nudge-threshold`, flag removals, old-daemon fallback, commits 933f979..f153f88, **fix pass** (bd; ledger L83)
- ht-yha.5.2: DeliveryRowSeek, the exact ordered pending seek, commits f153f88..1b9b3f3, clean (bd; ledger L91)
- ht-yha.8: framing and markers by priority (`[pN]`), commits 1b9b3f3..af744f0, clean (bd; ledger L98)
- ht-yha.11: wake e2e matrix for priorities and thresholds, commits af744f0..21d886e, **fix pass** (bd; ledger L104)
- ht-yha.9: watch stream and Claude mod in priority order, commits 21d886e..b593e14, clean (bd; ledger L112)
- ht-yha.6: invitations always p0 and confirming; 1:1 pairs auto-join, commits 3f5e9db..b040d42, clean (bd; ledger L118)
- ht-yha.5.4: inbox v2 min-head merge, commits b040d42..1702dfd, **fix pass** (bd; ledger L129)
- ht-yha.10: docs and skill text, commits 1702dfd..7bb64cb, clean (bd; ledger L138)
- ht-yha.12: configuration smoke covering threshold x priority x ACK/invitation/1:1, commits 7bb64cb..7618ca8, clean (bd; ledger L143)
- ht-yha.15: cross-surface delivery-order seam test, commits 7618ca8..0ebddfb, **fix pass** (bd; ledger L149)
- ht-yha.16: integration sweep, commits 0ebddfb..9930830, clean (bd; ledger L154)
- Fixes from code-roast round 1: ht-yha.35 v46→v47 replay test, 90bc82d..2956dfc · ht-yha.32 displayed-state predicate, 2956dfc..1146acb · ht-yha.31 drain_pages keeps a sink's Halt, 1146acb..63fbb79 · ht-yha.36 a failed probe never downgrades priority, 63fbb79..2e5e6da · ht-yha.37 one batch order key, bc4a4da..c7eb7df · ht-yha.34 merge readiness, c7eb7df..e1bc9b9; all clean (bd; ledger L183-L204)
- Fixes from the regression pass: ht-yha.33 old-CLI cursor (option c), a5fc04d..19aec70 · ht-yha.39 pair-invitation text, 19aec70..75d54ac · ht-yha.38 seat_unread_sql! connected-only again, 75d54ac..79a20c7; all clean (bd; ledger L224-L232)
- Fixes from the sweep: ht-yha.40 delivery rows stream as Lazy, 4a2793f..e9c0407 · ht-yha.41 `--max-bytes` and `[pN]`, e9c0407..970ed3f · ht-yha.42 no `[pN]` against an old daemon, 0b1a3e7..d5e4c65; all clean (bd; ledger L256-L281)
- ht-yha.43, the config5 sweep-regression bead, was filed and then closed without a fix as "not a sweep-fix regression" (bd; friction.md 11:17:14Z)

## Remaining

Delivery-correctness items first:
1. **Item A / audit mod.rs:131.** `seat_owes_display_sql!` counts process-exited bindings as owing a display, and every Claude session ends by process exit. Seats therefore build up delivery rows pinned at `state='pending'`, and the first inbox or hook page comes back empty after about 100 of them (a reproduce probe showed 251 pending rows and an empty first page). The tip sweep's config5 failure shows the same symptom. The TRUST-POLICY.md:800 "accepted limit" was written by fix pass ht-yha.38, not by the user. This is branch-introduced relative to main. (final review; punch list [final review] A, [sweep] config5, [post-cap audit] FYI mod.rs:131; roast-pr-post-cap-audit.md Escalations)
2. **Merge-seam regression at src/ports.rs:2188 and tests/scheduler/dispatch.rs:5335.** The orchestrator's seam decision added `lazy` to `for_seat_attention`. A one-process sibling (for example a Codex `/side` binding) can now get a nudge after it was already served in-turn, which breaks Delivery contract rule 4. The guard test was weakened to an empty reason set. (roast-pr-post-cap-audit.md Should-fix; punch list [post-cap audit]; run.md baseAbsorbed)
3. **Final review B.** `mod_stall_oldest` (src/store/seats.rs:5214) counts only receipts. A wedged but still-registered mod holding a p0 delivery row (a bare `--nudge`, or the first p0 pair message) is never stall-closed, so the seat never wakes. (super-code-final-review-wf_33b8fe4a-079.md; punch list [final review] B)
4. **`body --max-bytes` 256 to about 342 is refused for every p0 message.** `fit_annotated_read` (src/cli/mod.rs:1752) gives up below 256. ht-yha.41 raised the test bound from 256 to 384 instead of fixing the code. (final review; punch list [final review], [post-cap audit] token_diet.rs:274)
5. **`markers_ok` passes vacuously in the release-gate e2e** when no header is on screen (scripts/wake-e2e.py:192/208). (final review; punch list [final review])
- Final review C: a new client shows `[lazy]` against an old daemon and `[p0]` against a new one, during the upgrade window only (punch list [final review])
- Final review D: docs/agent-usage.md:79 contradicts D9 for a human `--require-ack` recipient, and three `wake_on: all` doc comments are stale (punch list [final review])
- Final review F4: `nudge-threshold` treats a failed capability probe as an old daemon (punch list [final review])
- Final review: `complete_addressed` is unused and bypasses `MARK_DISPLAYED_SQL` (punch list [final review])
- Final review: still to do are a sweep at the tip with `check-default-features` and `check-no-leaked-processes --run-id`, the wake e2e (never run), and closing ht-cbb, whose fix landed in 011c24d2 (super-code-final-review-wf_33b8fe4a-079.md "Must fix" item 6, "Untested scope")
- Post-cap audit FYI: compact `--machine` output for p0 pages now starts with `[p0] <id>` rows, against docs/agent-usage.md:5 (roast-pr-post-cap-audit.md Unverified nits)
- Punch list from roast round 1 (out of scope, filtered): wake.rs:80/50 lose the old bound on wake probes · handoff/automatic.rs:320 human-bound handoff edge case · read_cursors.rs:497 prune cost is linear · control.rs:2242 test-only legacy marker · register.js:357 two-pump arrival order · run.md:21 · schema.rs:835 forward-only migration (roast-pr-1-punch-list.txt; run.md scopeFilter-round-1)
- Punch list from roast round 2: inbox.rs:335 unbounded `binding_lazy_window` walk on every hook digest · mod.rs:112 macro scope · binding_lazy.rs:207 superseded/end-event test · send_priority.rs:91 probe pattern. The audit notes that 62994aa1 appears to have fixed mod.rs:112 and binding_lazy.rs:207, but they stay open until someone reconciles them. (roast-pr-2-punch-list.txt; roast-pr-post-cap-audit.md Step 3)
- **Escalation: ServiceSend vs D9.** ServiceSend keeps receipts for its whole audience (`AckAudience::Everyone`). The recommendation is to keep it and record it as an accepted limit in TRUST-POLICY.md and the spec. Needs the user's decision. (run.md parked; final review)
- **Escalation: ht-yha.33 option (c).** An old CLI cannot follow a new daemon's format-2 `next:` cursor. The orchestrator accepted this as a limit; it needs the user's ratification or another scheme. ht-yha.42 option (C) also needs the user's acknowledgement. (run.md parked; final review; roast-pr-post-cap-audit.md Escalations)
- Parked graph changes from the graph pass (`graph-pass: depth 8→7 · width 2.4→2.7 · applied 2 · parked 5`): drop edges ht-yha.8←.4, ht-yha.6←.5.2 and ht-yha.4←.3 (each same-file, not safe); proposed seams: a staging.rs seam contract for ht-yha.3/4/5.2, and a frame golden-fixture seam for ht-yha.8/9 (run.md parked)
- Conflict-resolved hunks: merging main 998d7d34 (via 6657c2be) conflicted in AGENTS.md, TRUST-POLICY.md and scripts/wake-e2e.py. Only the post-cap audit reviewed them, and it found item 2 above. (run.md baseAbsorbed; roast-pr-post-cap-audit.md inputs)
- Flake beads outside the epic: ht-3wm (q5 search refit, ReadBudgetExhausted) and ht-kk8 (doctor_reports hermes line), both open and neither run at base (bd)
- worktreesKept: none · processSweep survivors: none · roastCodeExit: converged (run.md)

## Gotchas & surprises

- Two network outages (ENOTFOUND) forced relaunches. The first killed 17 dispatches, including the ht-yha.3, .7 and .5.3 implementers. (friction.md 23:30:41Z; run.md slowness)
- False re-merge blocker: after the relaunch, ht-yha.1 (already merged at 0a24b6c3) was re-merged, which raised a false blocker ht-xml. That blocker cancelled ht-yha.7 (its partial worktree was discarded) and held its dependents. The orchestrator closed both by hand. Flagged for upstream as a pipeline defect. (friction.md 23:33:50Z, 03:49:43Z, 03:52:31Z; ledger L44)
- Decisions the orchestrator took during the autonomous run, which the user has not reviewed:
  - ht-yha.33 option (c), which closed blocker ht-xot (friction.md 08:44:48Z)
  - ht-yha.41/.42 option C: rewrite stale test expectations, no change to spec or code (friction.md 10:21:27Z)
  - the `for_seat_attention` merge seam (run.md baseAbsorbed)
- Sweeps ran over the 300 s budget: 329 s at dae11184 (the release-build manifest test alone took about 289 s) and 308 s at e8ecb681 (sweep-dae11184.log, sweep-e8ecb681.log)
- The config5 failure at e8ecb681 is deterministic, with no src change since dae11184 where it passed. doctor_reports and q5 pass 3/3 in isolation. (friction.md 11:17:14Z)
- The flakiness tab was not running, so the flakes went into the tracker as ht-3wm and ht-kk8 instead (friction.md 11:18:29Z)
- Both design-roast rounds patched the spec, and decisions D1-D11 held. Coverage round 2 amended §Compatibility (old-reader variants, WakeOnQuery reverse, old-daemon pair fallback). (run.md approvals stepBackDesign-round-1, coverage-round-2; roast-design-1-step-back.md)
- Code step-back round 1 chose a patch over four clusters: drain-halt, merge-readiness, seat-pending-predicate and batch-order-key (roast-pr-1-step-back.md)
- The run was bound by the shape of the task graph: depth 6 and achievable width 3 against a cap of 8, with every critical chain through ht-yha.3→ht-yha.4. The merge queue peaked at 3. (run.md slowness; ledger L20/L40/L58)
- The ledger is incomplete: "landed≠completed 31 vs 1", so finding no problem lines in it does not mean clean (ledger L288; final review)
- Profile wf_24ee41b2-d4e: the bottleneck was ht-yha.3, 5h58m on the critical path (48%), over 3 attempts. Largest what-if (a model estimate): unlimited merge lanes, saving about 1h03m (8%). (profile-wf_24ee41b2-d4e.md)
- Profile wf_e43661f8-02c: the bottleneck was ht-yha.36, 23m (50%). Largest what-if (a model estimate): a faster ht-yha.34, saving about 2m (4%). (profile-wf_e43661f8-02c.md)
- Profile wf_ecf6a4ce-925: the bottleneck was ht-yha.38, 11m (55%). Largest what-if (a model estimate): a faster ht-yha.38, saving about 2m (8%). (profile-wf_ecf6a4ce-925.md)

## Entrypoints

Note: `git diff 3a1203e0...HEAD` (269 files) includes main's v0.11.0 changes absorbed through 6657c2be, such as src/cli/installer.rs and src/harness/codex/setup.rs. `git diff 998d7d34 HEAD` shows only the branch's changes.
1. src/protocol/priority.rs: priority and threshold types and their wire fields (diff; ht-yha.1)
2. migrations/0047_message_priority.sql and src/store/schema.rs: the v47 schema, backfills and indexes (diff; ht-yha.2)
3. src/store/lazy_delivery/staging.rs: send publication, delivery rows vs receipts (diff; ht-yha.3)
4. src/store/lazy_delivery/mod.rs (`seat_owes_display_sql!`, item A), completion.rs and wake.rs: pending, displayed and wake predicates (diff; ht-yha.4/.32/.38)
5. src/store/queries/inbox_v2/{mod,order,sources,deadline_window}.rs and src/protocol/pagination.rs: the ordered merge and cursor format 2 (diff; ht-yha.5.x/.33)
6. src/delivery/assemble.rs, page.rs and frame.rs: batch order key, drain and `[pN]` framing (diff; ht-yha.8/.31/.37)
7. src/cli/send_priority.rs, nudge_threshold.rs, output.rs and the `fit_annotated_read` refit in src/cli/mod.rs: the CLI surface (diff; ht-yha.7/.36/.41)
8. src/ports.rs `for_seat_attention` (the merge seam, item 2) and src/store/seats.rs `mod_stall_oldest` (item B) (diff)
9. integrations/claude/mod/hooks/register.js: the mod's mirror of the ordering (diff; ht-yha.9)

## Smells

- ht-yha.7, ht-yha.11, ht-yha.5.4 and ht-yha.15 each merged after a fix pass, with no second review. The smell: these fixes were never reviewed again. (ledger L83/L104/L129/L149; ledger Metrics "after fix pass 4")
- The regressionPass-round-2 fixes ht-yha.38, ht-yha.39 and the amended ht-yha.33 merged without a re-roast. The smell: the post-cap audit later found ht-yha.38 fixed its target incompletely (mod.rs:131, regressed), and ht-yha.33 closed by acceptance rather than code. (run.md regressionPass-round-2; roast-pr-post-cap-audit.md)
- The sweep fixes ht-yha.40, .41 and .42 also merged without a re-roast. The re-run at e8ecb681 still had 3 failures, now different tests (config5, doctor_reports, q5). Regression bead ht-yha.43 was filed and closed. The smell: ht-yha.41 raised a test bound instead of fixing the code, and .41/.42 were rewritten to match the code once their beads forbade test edits. (run.md sweepFix; friction.md 10:21:27Z, 11:17:14Z; roast-pr-post-cap-audit.md)
- The merge-seam decision at baseAbsorbed was taken by the orchestrator and comes with a weakened guard test. The smell: an unreviewed decision changes the wake rules. (run.md baseAbsorbed; roast-pr-post-cap-audit.md)
- Missing red-first or base-run evidence for TDD and flake claims in tasks 4, 5, 7, 8, 12, 13, 20 and 21. The smell: those claims are unverified, not clean. (super-code-final-review-wf_33b8fe4a-079.md Step 2)
- Recurring `[doc-drift]` (×3) and `[missing-edge-case-test]` (×3) minors. The smell: no task owned a final sweep of the shared docs. (final review Step 2; ledger L265)
- Parked findings in the ledger: none. Parked `degraded-verdict` records: none. DONE_WITH_CONCERNS reports: none. (ledger Metrics "parked 0"; run.md)
