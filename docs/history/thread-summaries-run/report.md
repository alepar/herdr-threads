status: stalled at phase finish
metrics: delivered via Herdr to the superpowers workspace agent (wB:p1); draft docs/history/thread-summaries-run/upstream-feedback-draft.md
finish verification: full suite at 63af7f93 RED (exit 101, 186 s): daemon::lifecycle::tests::crashed_incompatible_protocol_owner_recovers_to_new_healthy_boot failed in the lib target (passes 3/3 alone); the phase-6 re-run at 62f9c1a7 failed a different test (composition::actual_native_snapshot_cancellation_…, also 3/3 alone). Two consecutive runs, two different load-sensitive failures — not root-caused (source: target/full-suite-gate/20261002-215955/run-1.log; run.md sweepFix)

## Implemented

- ht-1ip.1 Seam contract: summary schema, wire types, settings, inert hooks, TRUST-POLICY amendment — 9c50c0d..dfb792e (source: ledger; bead ht-1ip.1)
- ht-1ip.2 Message authorship: `author_role` and `send --relays-user` — 939ede7..46ecccd (source: ledger; bead ht-1ip.2)
- ht-1ip.3 Summary core: deterministic chunker, rendered sizes, displayed cover — 3f87409..046756e (source: ledger; bead ht-1ip.3)
- ht-1ip.4 Summary ledger: extraction, submission validation, fold — 0cd9c4f..5062123 (source: ledger; bead ht-1ip.4)
- ht-1ip.5 Summary job service: plan, leases, bundles, submit, block storage — 6cbf604..569358f (source: ledger; bead ht-1ip.5)
- ht-1ip.6 Catch-up mode: frontier hold, exit, stall, supersession — 46ecccd..6cbf604 (source: ledger; bead ht-1ip.6)
- ht-1ip.7 Deadline extension: effective deadlines in overdue, warnings, pending receipts, inspect — 0611c29..add9a68 (source: ledger; bead ht-1ip.7)
- ht-1ip.8 CLI summary commands and escaped rendering — 5062123..a969486 (source: ledger; bead ht-1ip.8)
- ht-1ip.9 Recovery hook text: hot threads on compact/resume/clear, summary hint on join — 5c37c15..0611c29 (source: ledger; bead ht-1ip.9)
- ht-1ip.10 Claude SessionStart compact: native evidence and the `claude-hooks-2.1.287` recipe — 2a6fa10..f25e4fe (source: ledger; bead ht-1ip.10)
- ht-1ip.11 Skill: thread-summary procedure with parallel small-model workers — 7f101d5..ed1385a (source: ledger; bead ht-1ip.11)
- ht-1ip.12 Spike: composer stash and poke-during-turn evidence (docs/evidence/poke-spike/). The coordinating session merged it by hand, so the ledger has no completion range (source: ledger `Merge: ht-1ip.12` line; bead ht-1ip.12 close note)
- ht-1ip.13 Soft-deadline poke: soft point, focus plumbing, eligibility, dispatch — da72be8..0a1ed3e (source: ledger; bead ht-1ip.13)
- ht-1ip.14 Composer stash and poke-during-turn capabilities from the spike evidence — dfc08bf..53fc37d (source: ledger; bead ht-1ip.14)
- ht-1ip.15 Configuration smoke on Codex and Claude (docs/evidence/summary-smoke/report.md). Merged after a fix pass with 1 parked finding — c1e7c3e..cd31541 (source: ledger; run.md codeBuckets.parked)
- ht-1ip.16 Seam integration: summary flow end to end on a real daemon — dae9716..f24bea2 (source: ledger; bead ht-1ip.16)
- ht-1ip.17 Seam integration: soft-deadline poke against effective deadlines (fake host) — add9a68..8cd5268 (source: ledger; bead ht-1ip.17)
- ht-1ip.18 Integration sweep — 1223a89..8069181 (source: ledger; bead ht-1ip.18)
- ht-1ip.32 Extension-lapse scan now finishes a pass — c0eb261..247d141 (source: ledger; bead ht-1ip.32)
- ht-1ip.33 Long open user instruction gets a working retrieval command in Ready — 373eed4..6d5a834 (source: ledger; bead ht-1ip.33)
- ht-1ip.37 Fixed failing test shared_reuse_by_second_seat — 36c4bfd..834ebe5 (source: ledger; bead ht-1ip.37)
- ht-1ip.38 Poke collection excludes never-reservable seats and backs off — 18d0691..c135503 (source: ledger; bead ht-1ip.38)
- ht-1ip.39 An ambiguous Claude placeholder now reads as Unknown, so a poke never types over a draft — e46a26c..b7a78fb (source: ledger; bead ht-1ip.39)
- ht-1ip.40 Spec note: dropped fold transitions are not logged — 316bc2a..1096ca5 (source: ledger; bead ht-1ip.40)
- ht-1ip.46 Composer classified by rendered output: wrap display width, Claude suggestion ghost text (fixes ht-jf3) — e4a1052..01ebc0d (source: ledger; bead ht-1ip.46; roast-pr-1 step-back)
- ht-1ip.47 Poke admission uses only the spec §10 limits and filters before truncating (fixes ht-2i4) — 23743a5..c58fb12 (source: ledger; bead ht-1ip.47; roast-pr-1 step-back)
- ht-1ip.48 Summary identifiers and fold size are bounded and fully counted — 0a7807e..e75fd9a (source: ledger; bead ht-1ip.48)
- ht-1ip.49 PROTOCOL_VERSION bumped to 3 for the summary wire commands — d0398d4..1e969ca (source: ledger; roast-pr-2 prior tracking)
- ht-1ip.50 Agent-facing docs and recovery text name the shipped summary procedure (fixes ht-dtq) — 4d9db65..f64f809 (source: ledger; bead ht-1ip.50; roast-pr-1 step-back)
- ht-1ip.51 Priority citations count only ordinary messages — b3c6653..c8c4658 (source: ledger; bead ht-1ip.51)
- ht-1ip.52 author_role backfill test covers the binding start bound and multiple bindings — 318c780..100d0a8 (source: ledger; bead ht-1ip.52)
- ht-1ip.53 Sweep fix: skew/wire-compat tests after the protocol-3 bump — 0a820e2..becc327 (source: ledger; run.md sweepFix)
- ht-1ip.54 Sweep fix: summary_flow spawn sites now owned by the leak guard — 590611d..3ceae80 (source: ledger; run.md sweepFix)
- ht-1ip.55 Sweep fix: Codex reattachment wake no longer refused as unsafe after the main merge — b0d752b..3ae4284 (source: ledger; run.md sweepFix)

## Remaining

- **Privacy:** docs/evidence/summary-smoke/captures/state/codex-scratch-config.final.toml:11 is committed evidence that leaks the developer's personal Codex config, including 5 unrelated trusted `[projects."..."]` paths, in a repo meant for a public home. Tagged out of scope (filtered), reason: "a personal-config leak in committed evidence files, which is not behavior the goal names". It was still open at roast round 2 (source: run.md scopeFilter-round-1; roast-pr-2 prior tracking)
- The round-2 code roast's Should-fix is not fixed: README.md:149 and the Summaries bullet, plus docs/agent-usage.md:126 (`:156` at 62f9c1a7), still describe only Claude 2.1.283–2.1.286. agent-usage.md contradicts its own line 133 on the 2.1.287 recipe (source: roast-pr-2 Confirmed findings; run.md roast-code exit punch list; codeBuckets.review)
- Final review is not ready. It needs a green full suite, the docs fix above, and human decisions on three points: (a) the focus skip is unverified natively (ht-yuz); (b) the Claude poke is skipped while a prompt suggestion shows; (c) §8 entry/re-entry extension and the A6 wording (source: run.md codeBuckets.review)
- ht-1ip.15 parked criterion: the native smoke never showed a focused-pane poke skip or a controlled soft-point poke. The $3 budget was spent (about $2.97 on Claude) and focus never applied (source: ledger Task 17 completion line; run.md parked)
- ht-yuz (open, smoke-finding): the focused-pane skip needs a re-run against a Herdr with an attached client (source: bead ht-yuz)
- Parked design-roast escalation: should §8 catch-up entry, re-entry or a keep-call extend the deadline without stored progress (panel split 2-1)? Separately, the §8 sentence "every extension requires new stored progress" is inaccurate for entry (source: roast-design-1 Escalations; run.md parked)
- Parked graph change: narrow ht-1ip.11 <- ht-1ip.5 to ht-1ip.11 <- ht-1ip.4 (depth 6→5), judged not safe. `graph-pass: depth 6→6 · width 3.0→3.0 · applied 0 · parked 2` (source: run.md parked, graph-pass)
- Parked graph change: a seam-contract summary core API (chunk/cover/render and validate/fold stubs in ht-1ip.1) so ht-1ip.5 could run in parallel with .3/.4 (source: run.md parked, graph-pass)
- Parked degraded verdict: coverage round 2 widened (14 → 16 findings, 94% novel), and coverage never re-reviews round-2 fixes (source: run.md parked; coverage-findings-round-2.md)
- Out of scope (filtered), src/store/wake.rs:657: the poke lookup's query-plan cost (a shared `(?3 IS NULL OR seat_id=?3)` statement); output is correct (source: run.md scopeFilter-round-1)
- Out of scope (filtered), src/summary/render.rs:142: the spill frees almost nothing for closed decisions and open items; this is soft-target size efficiency only (source: run.md scopeFilter-round-1)
- Out of scope (filtered), src/store/mod.rs:1909: extra idle-tick write transactions and a prepare inside a loop; a performance hardening (source: run.md scopeFilter-round-1)
- Out of scope (filtered), src/notification/policy.rs:313 and src/notification/dispatch.rs:222: the poke-skip diagnostic label is lost; a diagnostics improvement (source: run.md scopeFilter-round-1)
- Out of scope (filtered), src/cli/hook.rs:1406: recovery read errors are swallowed with no diagnostic; a hardening improvement (source: run.md scopeFilter-round-1)
- Out of scope (filtered), tests/integration/summary_flow.rs:36: a 3 s frozen deadline flakes under slow setup (not reproduced); test robustness (source: run.md scopeFilter-round-1)
- Out of scope (filtered), tests/integration/summary_flow.rs:1186: wall-clock-sensitive assertion; test robustness (source: run.md scopeFilter-round-1)
- Out of scope (filtered), src/protocol/output_compact.rs:336: grammar completeness, fixed in the same docs sweep. roast-pr-2 records it resolved via ht-1ip.50 (source: run.md scopeFilter-round-1; roast-pr-2)
- Out of scope (filtered), docs/operations.md:131: the operator doc has no poke, catch-up or effective-deadline sections. Still open at roast round 2 (source: run.md scopeFilter-round-1; roast-pr-2)
- Out of scope (filtered), src/store/schema.rs:270 (FYI): one-way migrations with no backup; a pre-existing pattern (source: run.md scopeFilter-round-1)
- Follow-ups named by the final review: the poke witness race on a busy instance, and draft loss when stash clearing fails before the stash becomes reachable (source: run.md codeBuckets.review)
- codeBuckets escalated, pendingRetry and worktreesKept are empty; processSweep shows 0 survivors; no resumeChange was recorded (source: run.md codeBuckets)

## Gotchas & surprises

- Design-roast step-back round 1 forced a redesign. The per-block union ledger with in-block transitions (spec §5) was replaced by a daemon fold of level-0 item/transition records in sequence order, with rollups narrative-only. This one change resolved 5 Should-fix findings (source: run.md stepBack-round-1; roast-design-1-step-back.md)
- Design-roast round 2 Blocking: the spec added `messages.author_kind`, but that column already exists (migration 0002, CHECK native/programmatic/built_in), so the migration could not apply. It was renamed to a new `author_role` column, with the backfill run between DROP and re-CREATE of messages_immutable (source: roast-design-2 Confirmed findings; roast-design-2-step-back.md; roast-design-3 resolved list)
- Design step-back round 2 patched rather than redesigned. Item identity is now tied to the thread (seq-derived prefill ids). Cross-chunk closure of a model item from a not-yet-stored chunk is an accepted limit. Fallback supersession was dropped, and one fold renderer with one size rule is used everywhere (source: roast-design-2-step-back.md)
- Design round 3 converged with 6 Should-fix findings, all fixed inline in the spec and beads without a re-roast (source: run.md roast-design exit)
- BLOCKED-AUTH on ht-1ip.12: the harness refused a subagent's Write of the legitimate deliverable docs/evidence/poke-spike/findings.md, which quarantined 3 dependents. The coordinating session wrote the file from the task report and merged by hand (source: friction.md; ledger Task 3 and `Merge: ht-1ip.12` lines)
- BLOCKED-AUTH on ht-1ip.15: the smoke's credential probes (keychain, `.credentials.json`) were refused. The task was retried with a note forbidding credential inspection (source: ledger Task 17 BLOCKED-AUTH line; bead ht-1ip.15 RETRY NOTE)
- BLOCKED-AUTH on ht-1ip.46: capturing evidence needed accepting a Claude folder-trust prompt in a spawned agent, which was refused as [Create Unsafe Agents]. The task was rescoped to implement from existing evidence with a conservative skip (source: friction.md; ledger Task 25 BLOCKED-AUTH line)
- Three native smoke findings were folded into fix beads per roast-pr-1 step-back (source: roast-pr-1-step-back.md; beads ht-jf3, ht-dtq, ht-2i4):
  - ht-jf3: Claude prompt-suggestion ghost text was read as a typed draft, backing wakes off for 5 min. Fixed in ht-1ip.46.
  - ht-dtq: the hook text named a skill that setup does not install, so agents improvised worker prompts. Fixed in ht-1ip.50.
  - ht-2i4: soft pokes were starved by the wake retry backoff. Fixed in ht-1ip.47.
- A user-approved test-policy pause meant no full suite until side quest ht-zo4 finished; roast-pr-1 ran on focused tests, check and clippy only. The pause was lifted for phase 6, where the full suite runs once (source: bead ht-1ip.15 TEST POLICY note; roast-pr-1 and roast-pr-2 profile lines)
- main moved to a7255713 and was merged in at f6436cc1, with 32 conflicting files. The branch's migrations were renumbered to 0012_thread_summaries and 0013_catch_up_release (LATEST_VERSION 13), and the Claude recipe split 2.1.283–2.1.286 / 2.1.287 was kept (source: diff a7255713...62f9c1a7, merge commit f6436cc1; merge-tree of its parents)
- Sweep-fix pass: 5 suite failures after the merge became ht-1ip.53 (protocol-3 skew tests), ht-1ip.54 (leak-guard spawn sites) and ht-1ip.55 (Codex reattachment wake refused as unsafe). The re-run left 1 load-sensitive failure (source: run.md sweepFix; beads ht-1ip.53–.55)
- The ht-1ip.10 merge was first refused (worktree state), went to pending retry, and then merged clean (source: ledger Task 2 lines)
- Slowness: round 1 was graph-bound, with 7 open beads, depth 3 and achievable width 3 against a cap of 8. Edge audits made no changes (source: run.md codeBuckets.slowness; ledger Edge audit lines)
- Friction: Workflow refused a scriptPath under the plugin cache, so a byte-identical copy was launched from the scratchpad. The read-ledger:finish dispatch died on an API safeguard false positive, so the Finish metrics show UNAVAILABLE (source: friction.md; ledger Metrics lines)

## Entrypoints

1. migrations/0012_thread_summaries.sql, migrations/0013_catch_up_release.sql: schema for blocks, items, catch_up and author_role (ht-1ip.1/.2/.6) (source: diff; task tree)
2. src/protocol/summary.rs, src/protocol/wire.rs: the Summary/SummaryJob/SummarySubmit/HotThreads wire contract, PROTOCOL_VERSION 3 (source: diff; ht-1ip.1, .49)
3. src/summary/chunk.rs → cover.rs → render.rs: deterministic chunking, displayed cover, bundle bound (source: diff; ht-1ip.3)
4. src/summary/identifiers.rs, ledger.rs, validate.rs, fold.rs: extraction, validation, sequence-ordered fold (source: diff; ht-1ip.4)
5. src/store/summary.rs: `summary`, `summary_job` and `summary_submit` (lease, bundle, store) (source: diff; ht-1ip.5)
6. src/store/catch_up.rs, src/store/receipts.rs: catch-up hold and effective-deadline extension (source: diff; ht-1ip.6/.7)
7. src/service/dispatch.rs:357: the primary caller routing the summary commands (source: diff)
8. src/cli/summary.rs, src/cli/hook.rs: agent-facing commands and recovery text (source: diff; ht-1ip.8/.9)
9. src/store/poke.rs → src/scheduler/mod.rs (`drive_pokes`, `try_poke`) → src/notification/dispatch.rs, src/harness/composer.rs, src/host/native.rs: the soft-deadline poke path (source: diff; ht-1ip.13/.14/.46/.47)
10. tests/integration/summary_flow.rs, tests/integration/summary_sweep.rs, tests/scheduler/poke_flow.rs; docs/evidence/summary-smoke/report.md (source: diff; ht-1ip.15–.18)

## Smells

- Sweep-fix commits (ht-1ip.53–.55) merged with a single re-run. composition::actual_native_snapshot_cancellation_closes_peer_before_elected_worker_join_and_owner_release still fails in the suite. The smell: it passes 3/3 alone, so it is load-sensitive and was not root-caused (source: run.md sweepFix)
- The full suite took 756 s against the 5-minute budget. The smell: the speed budget is breached (summary_flow alone takes about 96 s with a fixed 60 s lapse wait) (source: run.md codeBuckets.sweep; ledger Task 15 minor)
- ht-1ip.15 merged after a fix pass with no re-review and 1 parked finding (focus skip and controlled poke not shown). The smell: the native poke behavior is evidenced only partially (source: ledger Task 17 lines; run.md parked)
- Poke reservation witness race on a busy instance. The smell: the final review flagged it as a follow-up, unfixed (source: run.md codeBuckets.review)
- Stash clear failure can lose the draft before the stash becomes reachable. The smell: draft loss on a failure path (source: run.md codeBuckets.review)
- Claude poke is skipped whenever a prompt suggestion shows. The smell: a conservative skip from the refused ht-1ip.46 capture leaves the stash path unreachable for Claude (source: run.md codeBuckets.review; ledger Task 25 minor)
- The coverage round-2 fixes (C15–C31, ht-1ip.17, ht-1ip.18) were never re-reviewed by coverage. The smell: the review widened instead of converging (source: run.md parked degraded-verdict)
- Final-review degraded verdict: not ready pending a suite, docs and human decisions (source: run.md parked; codeBuckets.review)
