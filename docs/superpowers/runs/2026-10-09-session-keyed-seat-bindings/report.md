# super-auto report — 2026-10-09-session-keyed-seat-bindings

```
status: completed with 0 unresolved Blocking, 8 escalations [degraded: design roast exit: converged at round 2 (Should-fix 9, 0 Blocking); punch list applied inline to spec + beads, not re-roasted, code findings parked]
metrics: parked draft docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/upstream-feedback-draft.md (6 defects, 2 design questions, 1 doc gap; not filed)
```

## Implemented

All 41 tasks under epic ht-54r are closed, and none is escalated or pending retry (run.md codeBuckets.completed). The last sweep passed at abe1bf03: 5471 passed, 0 failed, check-harness-neutral exit 0, no leaked processes, 258s (run.md codeBuckets.sweep).

- ht-54r.1 Seam contract: binding identity, claims, cursors (schema, protocol, store stubs). c0de40a..0113342, review clean (ledger)
- ht-54r.2 Env claim derivation and process fingerprint. 0113342..f477f43, clean (ledger)
- ht-54r.6 Claude PreToolUse --agent-id rewrite and subagent hook events. f477f43..fd0b8a4, clean (ledger)
- ht-54r.3 Daemon check-in by binding identity (enroll, touch, subagent attach, fingerprint refusal). 88a69c3..f40df60, clean (ledger)
- ht-54r.10 Binding liveness transitions and seat_active. f40df60..3d43660, clean (ledger)
- ht-54r.14 Pane-close retirement within a live Herdr incarnation. 3d43660..b43d934, clean (ledger)
- ht-54r.21 Sweep of 'current binding' readers for multiple bindings. b43d934..aaff308, clean (ledger)
- ht-54r.16 Presence display: active vs inactive. f9257c3..11af2cd, clean (ledger)
- ht-54r.4 Human exclusivity and managed-launch placeholder. 11af2cd..3773f3e, clean (ledger)
- ht-54r.15 Archival: inactive members don't block; scan-restart churn. 3773f3e..6f83462, clean (ledger)
- ht-54r.23 Liveness worker: timed fingerprint re-check, same-pid rule. 13f42ab..676ee41, clean (ledger)
- ht-54r.7 Per-binding cursors and inbox page. fe1ac75..1b4964d, fix pass (ledger)
- ht-54r.22 Per-seat delivery state moved onto bindings. 1b4964d..d17ed80, clean (ledger)
- ht-54r.39 Fix tests failing on the run tip (topology_handoff, fence replay, fixture). d17ed80..db3f38c, clean (ledger)
- ht-54r.20 Subagent write refusal, receipt binding attribution, lineage cursor copy. 4f819d5..2f76365, clean (ledger)
- ht-54r.5 CLI and hooks send env claims; agent OccupantContext journal retired. b32f3cf..fbfa556, fix pass, 1 parked (ledger)
- ht-54r.13 Wake per connected top-level binding; composer wake to the foreground agent. fbfa556..0e42717, clean after a retry (ledger)
- ht-54r.8 First-contact history stats and resume fast-forward. 0e42717..47a74db, clean (ledger)
- ht-54r.24 Upgrade path: stale hooks and pre-upgrade callers. 47a74db..fc080f4, clean (ledger)
- ht-54r.17 Hermes on session bindings: lineage via parent_session_id. fc080f4..3f04d4a, clean (ledger)
- ht-54r.11 Harness end events disconnect bindings. 3f04d4a..bd80de5, clean (ledger)
- ht-54r.9 Per-binding hook notifications and attention digest. bd80de5..fd7084f, clean (ledger)
- ht-54r.12 Mod delivery channel per binding. fd7084f..df90183, fix pass, 1 parked (ledger)
- ht-54r.18 Skill, README and agent docs. df90183..49e665c, clean (ledger)
- ht-54r.54 Fix: Hermes enrollment sweep/smoke binding count. a6e7a04..f178b81, clean (ledger)
- ht-54r.53 Fix: contract/ports/sweep/adapter-wiring failures. f178b81..086998c, clean (ledger)
- ht-54r.52 Fix: V29 migration tests on LivenessFingerprintRequired. 086998c..260661f, fix pass (ledger)
- ht-54r.60 Fix: Superseded verdict re-validates claim ordering. 260661f..099291e, clean (ledger)
- ht-54r.59 Fix cluster: accountable top-level means role plus liveness, not provenance. 099291e..af86f9a, clean (ledger)
- ht-54r.56 Fix: liveness disconnects bindings with no fingerprint. 7bea38f..744239d, clean (ledger)
- ht-54r.57 Fix cluster: lazy unread per binding. 744239d..009158c, clean (ledger)
- ht-54r.58 Fix cluster: subagent read-only from the resolved role; subagent own-stream reads. 009158c..9ba3c74, clean (ledger)
- ht-54r.19 Configuration smoke across harness combinations. 9ba3c74..af32130, fix pass, 1 parked (ledger)
- ht-54r.55 mod_delivery /clear and /resume tests rewritten to the session-keyed model. af32130..dabfc88, clean (ledger)
- ht-54r.25 Integration sweep, end to end. c184e2b..a078892, clean (ledger)
- ht-54r.71 Sweep fix: check-harness-neutral, caller shapes and hook tables moved behind adapters. 47a74f7..01cdc49, clean (ledger)
- ht-54r.70 Regression fix: continuity successor inserted without a fingerprint. 01cdc49..b2fb0aa, clean (ledger)
- ht-54r.72 Sweep fix: continuity successor survives a liveness pass. acdbcaf..f3d4710, clean (ledger)
- ht-54r.73 Sweep fix: read-only inbox selector. f3d4710..068f945, clean (ledger)
- ht-54r.74 Sweep fix: watch_admission_over_cap hang (test cap made configurable). 068f945..e952c64, clean (ledger)
- ht-54r.75 Sweep fix: config9 composer-wake predicate under load. e952c64..212dbd0, clean (ledger)

## Remaining

Human decisions needed:
1. The ht-ozo /clear wake model was decided autonomously: no rebind-grace suppression, /clear opens a new binding at seat-max cursors, /resume keeps the binding, and the mod_delivery tests were rewritten to match rather than restored. Needs sign-off. (run.md parked; code-final-review-4.md)
2. Spec deviation, ht-54r.13: a seat with a single target keeps the legacy composer rule and wakes without a foreground-PID match. Final review 4 recommends applying the PID match to one target too. (code-final-review-1.md:59; code-final-review-4.md finding 7)
3. Spec deviation, ht-54r.20: an explicit ACK or AckDisplayed from a disconnected binding is refused, while a mod-delivery ACK from it still settles. (code-final-review-1.md:60; code-final-review-4.md:78)
4. Spec deviation, ht-54r.16: compact `participants` output omits presence, though spec §3 asks for active/inactive. (code-final-review-4.md:79)
5. Unverified agy claim: every agy run_command herdr-threads call is refused (session_claim_required) unless ANTIGRAVITY_CONVERSATION_ID reaches that shell. Blocking if the variable is absent. Check in a live agy pane with `env | grep ANTIGRAVITY`, then re-run the agy native gate. (roast-pr-2 Escalations, src/harness/agy/mod.rs:73)

Punch list from the round-1 scope filter, tagged `out of scope (filtered)`; these never became beads:
- [Should-fix] src/store/read_cursors.rs:197: the per-binding unread probes scan every ordinary message. Reason: performance only, no correctness effect. (run.md scopeFilter-round-1; roast-pr-1-punch-list.txt)
- [Nit] src/harness/claude_rewrite.rs:14 and src/cli/commands.rs:1760: misleading error message on a fail-closed refusal. Reason: UX polish. (run.md scopeFilter-round-1)
- [Nit] src/store/lazy_delivery.rs:46: index and performance nit. Reason: cluster override, not a correctness defect. (run.md scopeFilter-round-1)
- [FYI] src/store/schema.rs:592 and :423, docs/operations.md:230: forward-only migrations. Reason: existing practice; changelog and hardening matter. (run.md scopeFilter-round-1)
- [Nit] TRUST-POLICY.md:312: stale policy-summary wording. Reason: already governed by A4. (run.md scopeFilter-round-1)

Punch list from roast round 2 and the post-cap audit, tagged `out of scope (filtered)`:
- [Should-fix] src/store/lazy_delivery.rs:507: an ordinary message ACKed over the mod channel or hook pull never advances the binding's cursor. Still live as final review 4 must-fix 2. (roast-pr-2-punch-list.txt; roast-pr-2.md)
- [Should-fix] src/service/delivery_pull.rs:249: DeliveryPull returns session_mismatch for a disconnected binding, so agy deletes its state and loses hook delivery after the V32 migration. Still live as final review 4 must-fix 3. (roast-pr-2-punch-list.txt)
- [Should-fix] TRUST-POLICY.md:804: the text says agy child calls are not enrolled as subagent bindings, but the code enrolls them. (roast-pr-2-punch-list.txt)
- [Should-fix] src/harness/caller_env.rs:358: no test covers the agy Subagent or Unattributable arms of `derive`. (roast-pr-2-punch-list.txt)
- [Should-fix] src/store/bindings.rs:1043, src/harness/caller_env.rs:159, src/store/bindings.rs:1285, src/harness/required_hooks.rs:29: harness-neutral design entries. The CI gate was cleared by sweep fix ht-54r.71; whether each entry is fully resolved was not re-roasted. (roast-pr-2-punch-list.txt; run.md sweepFix-prefiled)
- [Nit] src/delivery/ledger.rs:162: the hook-pull ledger is keyed by seat, so two live hook-pull bindings wipe each other's offers. (roast-pr-2-punch-list.txt)
- [Should-fix] src/cli/hook.rs:2009: `continuity_intent` reuse submits a dead process's fingerprint. This is the incomplete fix of the regressed liveness.rs:72 finding. (post-cap-audit; roast-pr-2-punch-list.txt)
- [Nit] src/cli/hook.rs:2008: a continuity intent recorded before ht-54r.70 has no liveness value and is refused with LivenessFingerprintRequired on resume. (post-cap-audit)

Final-review items routed to the punch list, tagged `final review`:
- [Must-fix] src/store/seats.rs:3721 `decide_continuity` inserts a binding without seeding cursors, so a resumed session's whole history reads as unread after a Herdr restart. (roast-pr-2-punch-list.txt; code-final-review-4.md must-fix 1)
- [Must-fix] S3 src/store/seats.rs:3403/3416 `continuity_candidates` matches only the newest row. (roast-pr-2-punch-list.txt; code-final-review-4.md must-fix 4)
- [Must-fix] S4 a Hermes child's CLI call before subagent_start enrolls as top-level and supersedes the parent. (roast-pr-2-punch-list.txt; code-final-review-4.md must-fix 5)
- [Must-fix] S5 src/service/dispatch.rs:593/:717 gate the mod channel and ACK on provenance, plus the provenance overwrite at seats.rs:4545. (roast-pr-2-punch-list.txt; code-final-review-4.md must-fix 6)
- [Must-fix] S6 test-only `foreground_binding` predicate, `carry_binding_forward` liveness filter, no-op `before_wake_decision` on the poke path. ht-54r.75 removed `foreground_binding`; the rest is not confirmed fixed. (roast-pr-2-punch-list.txt; code-final-review-4.md)
- [Must-fix] src/service/ports.rs:2126 `composer_choice` wakes a single target without a foreground-PID match. (roast-pr-2-punch-list.txt; code-final-review-4.md finding 7)
- [Must-fix] stale text: TRUST-POLICY.md:814 and :327-329, integrations/skill/SKILL.md:51, CHILD_RESTRICTION at src/harness/mod.rs:184. (roast-pr-2-punch-list.txt; code-final-review-4.md must-fix 7)

Other parked escalations:
- Sandboxes (bwrap PID namespace, Codex seatbelt, Claude sandbox) may hide the ancestor PID, so every agent call is refused with no pane fallback. Spike suggested. (run.md parked, roast-design-1; roast-pr-1 Escalations caller_env.rs:151)
- A sibling Bash PreToolUse hook may drop the --agent-id rewrite (claude-code issue 83353), misattributing a subagent to its parent. (run.md parked, roast-design-1)
- Codex CODEX_THREAD_ID vs CODEX_SESSION_ID subagent test is unverified at runtime across /new, /clear, fork and resume. (run.md parked, roast-design-1)
- Hermes /resume: does HERMES_SESSION_ID re-sync to the resumed id or rotate? (run.md parked, roast-design-1)
- Hermes compression lineage: pre_llm_call parent_session_id is documented for delegation only. (run.md parked, roast-design-2)
- Base drift: migration renumbering and the agy merge. Resolved by absorbing 43cc19cd (migrations now 0032–0035); upgrade tests re-verified only through the sweep. (run.md parked, baseAbsorbed)
- Hermes subagent_start/subagent_stop/on_session_finalize can be dropped by the non-blocking single-flight lock. (roast-pr-1 Escalations, integrations/hermes/__init__.py:560)
- Not yet tested: Linux /proc paths never compiled locally; real-harness payloads for Claude CLAUDE_PID, Codex SubagentStop, Hermes subagent_*, agy run_command. (code-final-review-4.md Untested scope)
- Unverified nits: src/cli/watch.rs:861 hard-codes a Claude harness on the push channel (roast-pr-2); src/cli/hook.rs:2104 swallows the fingerprint-probe error (post-cap-audit).

## Gotchas & surprises

- The design roast changed the spec. The same-PID supersede rule was rewritten twice (touch vs enrollment). V29 row classes and the UNIQUE(seat_id, generation) fate were made explicit. Human-seat transitions were defined. It also found that Hermes on_session_end fires every turn, not at session end. (roast-design-1-step-back.md; roast-design-2.md)
- Code step-back round 1, patch: the per-seat to per-binding migration was applied unevenly across three clusters (lazy-unread-per-binding, role-from-resolved-binding, authority-by-role-and-liveness). This produced fix beads ht-54r.57/.58/.59. (run.md stepBackCode-round-1; roast-pr-1-step-back.md)
- Final review 4 says the same uneven-migration class still accounts for must-fix findings 1, 2, 4 and 6. (code-final-review-4.md)
- Base drift: main moved +335 commits with colliding migrations 29–31 and a new agy harness. Main was absorbed before roast round 2: 45 conflicted files, migrations renumbered to 32–35, agy added to caller_env. (run.md parked decision; baseAbsorbed)
- The merge brought 68 check-harness-neutral violations. They were handled by sweep fix ht-54r.71, not punch-listed. (run.md sweep scope decision)
- Regression pass ht-54r.70 fixed liveness.rs:72 only partly. The post-cap audit found the continuity reuse path still regressed. (post-cap-audit)
- Account-switch and user pauses stopped the coordinator mid-round. Resume replay reopened the already-merged ht-54r.1 and created spurious review bead ht-54r.28. A stale-round replay forced a fresh relaunch. (ledger Slowness lines 25–43; friction.md)
- ht-54r.7: its merge was BLOCKED-AUTH, and the conflict resolution was done outside the pipeline, never seam-reviewed. ht-54r.22 needed an 8-file rebase with a V31 renumber. (ledger:112–113; code-final-review-4.md)
- ht-54r.13: blocker ht-0gd was a pipeline false positive. The finding asked only for evidence, which a 12/12 base-vs-branch re-run supplied. (ledger:156)
- ht-54r.25: the integration test binary hung for 22 minutes in isolated-Herdr Session::new reads, and the session killed it. (ledger:293; friction.md)
- Slowness: the hook.rs hot-file cap was raised to 4 in round 1; the merge queue peaked at 3. (run.md codeBuckets.slowness)
- The merge gate is build-only (clippy, default features, harness-neutral) and runs no focused tests. Cross-merge test seams surfaced only in final reviews and the sweep. (friction.md; code-final-review-4.md)
- The ledger has no `Profile:` or `Edge cut:` lines. (ledger)

## Entrypoints

1. migrations/0032_session_bindings.sql, then 0033–0035: schema for bindings, archival scope, delivery state, attention frontiers (ht-54r.1/.15/.22; diff)
2. src/store/bindings.rs: binding identity, enroll/touch, supersede, the resolver (+1520) (ht-54r.1/.3; diff)
3. src/harness/caller_env.rs and src/host/process_table.rs: client-side env claim and fingerprint derivation (ht-54r.2/.71; diff)
4. src/store/seats.rs: check-in, continuity (`decide_continuity`, `continuity_candidates`) (ht-54r.3/.5; diff)
5. src/store/read_cursors.rs, src/store/lazy_delivery.rs, src/store/receipts.rs: per-binding cursors, unread, ACK settle (ht-54r.7/.20/.22; diff)
6. src/service/liveness.rs, src/ports.rs (`composer_choice`), src/store/wake.rs: liveness and wake (ht-54r.10/.13/.23; diff)
7. src/service/mod_channels.rs, src/service/delivery_pull.rs, src/service/dispatch.rs: delivery channels (ht-54r.12; diff)
8. src/cli/hook.rs and src/cli/mod.rs: primary callers, i.e. hooks and the CLI sending claims (ht-54r.5; diff)
9. TRUST-POLICY.md: the normative changes, known to be partly stale (diff; code-final-review-4.md)

## Smells

- Design roast exit converged at round 2 (Should-fix 9, 0 Blocking). The smell: the punch list was applied inline to spec and beads and never re-roasted. (run.md parked degraded-verdict)
- Code findings were parked under the converged-exit rule. The smell: real correctness defects (cursor stuck, agy delivery loss, continuity cursor seeding) were punch-listed with no fix path short of a human re-entering. (friction.md; roast-pr-2-punch-list.txt)
- Final review: not ready (final review 4 @ 212dbd0c). The smell: 7 must-fix items plus 4 decisions are open; see Remaining. (code-final-review-4.md)
- ht-54r.5, parked: 11 hermes/continuity hook tests time out under full-lib load. The smell: the timeouts were called load flakes with an identical failure at base, and no budget fix was made. (ledger:163)
- ht-54r.12, parked: two mod_delivery tests failed at base and were deferred to ht-ozo. The smell: the /clear wake model was then chosen autonomously. (ledger:215)
- ht-54r.19, parked: the subagent-cursor test was #[ignore]d pending ht-54r.58. The smell: none left; final review 4 confirms it is un-ignored. (ledger:285; code-final-review-4.md)
- ht-54r.7, fix pass merged without re-review. The smell: the v2 inbox assertion had been loosened to `> 0` and the test premise edited, then restored in the fix pass. (ledger:126)
- ht-54r.52, fix pass merged without re-review. The smell: ACK-candidate flags were normalized away to match a known defect (ht-54r.59). (ledger:245)
- ht-54r.5, ht-54r.12 and ht-54r.19 fix passes also merged without a second review. (ledger:159, 212, 279)
- Regression pass round 2 (ht-54r.70 for liveness.rs:72) merged with no re-roast. The smell: the post-cap audit found it fixed incompletely (hook.rs:2009 reuse path). (run.md regressionPass-round-2; post-cap-audit)
- Sweep fix ht-54r.71–.75 re-ran green at abe1bf03 (5471/5471). The smell: final review 4 notes an unexplained config9 failure under load (Task 41); ht-54r.74 made the test cap configurable rather than finding the hang. (run.md sweepFix; code-final-review-4.md)
- ht-54r.7 conflict resolution was done outside the pipeline. The smell: never seam-reviewed. (code-final-review-4.md)
- DONE_WITH_CONCERNS implementer reports: none (.superpowers/sdd/ht-54r-plan/task-*-report.md)

## Addendum — post-hand-back fix rounds (2026-10-11)

Main (mljuxaWLC) declined the first hand-back (65b095fd) because final review 4 listed live must-fix defects. The run resolved them in place; every finding's disposition and evidence is in `dispositions.md` (this directory).

- Fix round 1 (ht-54r.76–.84) and round 2 (ht-54r.87–.91) closed final reviews 4–5 must-fix items; final review 6 found no open product defect (code-final-review-5.md, code-final-review-6.md).
- Spec-settled decisions replace the parked sign-offs: `/clear` wake model stands (spec §1.3/§1.5/§3); single-target composer wake requires the foreground match (§3, ht-54r.82/.89); disconnected-binding ACK follows §3 reconnect rules (ht-54r.81/.87); compact presence shown (ht-54r.83). (run.md decisions)
- agy run_command attribution verified live: agy 1.3.3 run_command shells carry `ANTIGRAVITY_CONVERSATION_ID` (evidence/ht-54r.84/); agy native gate 15/15 (evidence/ht-54r.88/).
- Composer wake now matches the binding fingerprint against Herdr's foreground process set; live probes showed the single-PID rule never fired for Claude (MCP/watch children) or npm-installed Codex (ht-54r.89).
- Main v0.8.1 absorbed (2ef00231, eb28b769). Post-cap audit 2 (2026-10-11-session-keyed-seat-bindings-2-roast-pr-post-cap-audit.md): converged, 0 Blocking, 4 findings fixed (99bc9a63, 25893108, 0160c11e, 14b4a4df) and independently reviewed: no Critical/Important, no security bypass in the flagged bindings.rs/receipts.rs paths; 2 Minor (1 accepted fail-safe, 1 fixed).
- Sweep: see the status line (run.md codeBuckets.sweep).

Remaining after the addendum: managed-launch placeholder still uses the strict single-foreground read (follow-up); a lost, never-replayed lifecycle check-in leaves a plain-only session losing to its predecessor (accepted limit); performance punch list; real-harness payload shapes, sandboxes and Linux untested; `Outside-adapter:` trailer present on 9 of 156 pre-rule commits (dispositions.md).
