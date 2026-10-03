# super-auto run — 2026-10-02-thread-summaries-compaction-survival

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: done

idea: Thread summaries for compaction survival + soft-deadline ACK poke — design agreed in this conversation (Mode A brainstorm complete); full brief at <scratch>/scratchpad/thread-summaries-design-brief.md (two specs, one epic; don't re-ask recorded decisions)
epic: ht-1ip
spec: 2026-10-02-thread-summaries-compaction-survival-design.md
branch: super-auto/thread-summaries-compaction-survival
base: main
feedback: delivered via Herdr to the superpowers workspace agent (wB:p1) 2026-10-02, draft upstream-feedback-draft.md (not filed on GitHub, per user)
resumeChange: 2026-10-02 · "lets just merge to main branch ignoring test failures … do remove personal codex config" · phase-7 menu answered: squash-merge into main despite the load-sensitive suite failures; personal Codex config redacted; main@7d0016ed merged in first (migrations 0013/0014); follow-ups ht-hqg (§8 option A) and ht-6jt (prompt-suggestion setup) filed; focused-pane smoke to run after merge

roast-design: 2026-10-02-thread-summaries-compaction-survival-roast-design-1.md, 2026-10-02-thread-summaries-compaction-survival-roast-design-2.md, 2026-10-02-thread-summaries-compaction-survival-roast-design-3.md
roastDesignRound: 3
graph-pass: depth 6→6 · width 3.0→3.0 · applied 0 · parked 2
roast-code: 2026-10-02-thread-summaries-compaction-survival-roast-pr-1.md, 2026-10-02-thread-summaries-compaction-survival-roast-pr-2.md
roastCodeRound: 2
stepBack-round-1: redesign — applied: per-block union ledger with in-block transitions (spec §5) → daemon fold of level-0 item/transition records in sequence order, rollups narrative-only (dissolves 5)
stepBack-round-2: patch — r1 fold held; three clusters (item identity tied to the thread, drop fallback supersession, one fold renderer/size rule) plus independent patches incl. Blocking author_kind collision → new author_role column (round counted as progress: 12 resolved, Blocking 0→1 is a new finding, not thrash)
- roast-design exit · round 3 · Should-fix (6 confirmed) [converged] · punch list of 6 fixed inline in spec and beads without re-roast (open-item seq, release as push, soft_poked_at on receipt_state, no invocation_role + accepted limit, bundle_bytes soft target, bundle fold window); no escalations this round

approvals:
- top-split · auto · ht-1ip.1 LEAF, ht-1ip.2 LEAF, ht-1ip.3 LEAF, ht-1ip.4 LEAF, ht-1ip.5 LEAF, ht-1ip.6 LEAF, ht-1ip.7 LEAF, ht-1ip.8 LEAF, ht-1ip.9 LEAF, ht-1ip.10 LEAF, ht-1ip.11 LEAF, ht-1ip.12 LEAF, ht-1ip.13 LEAF, ht-1ip.14 LEAF, ht-1ip.15 LEAF
- coverage-round-1 · canonical R-list: R1 "recovery hook text names hot threads and the summary procedure", R2 "join summary hint", R3 "summary returns Ready or Work", R4 "parallel workers; validated immutable shared blocks", R5 "index-aligned rollups fit ~10k tokens", R6 "user instructions never dropped; identifiers extracted; open items carried", R7 "author_kind and --relays-user", R8 "catch-up hold with bypasses and exits", R9 "progress-based effective deadline extension", R10 "safe soft-deadline poke", R11 "hard-deadline warning on effective deadline", R12 "TRUST-POLICY amendments", R13 "native evidence on both harnesses"; R-new appended for round 2: R14 "pre-migration messages get author_kind", R15 "daemon enforces summary callers", R16 "blocks never mixed across chunking_version", R17 "catch-up entry trigger and one frozen frontier" · requirements: 13 · mapped: 13 · unmapped: 0 · auto applied C1–C14 (14 findings; 7 GAP, 7 UNOWNED-SEAM; new leaf ht-1ip.16; edge ht-1ip.11<-ht-1ip.5)
- roast-code exit · round 2 · Should-fix (1 confirmed) [converged] · no [fix-regression] findings, no regression pass · punch list: [Should-fix] README.md:149; docs/agent-usage.md:126 plus the 11 r1 punch-listed findings
- stepBack-round-1: patch — 19 r1 code findings mostly independent; 7 clusters (poke-admission, composer-classification, agent-facing-docs, summary-size-bounds, dropped-diagnostics, poke-scan-cost, wallclock-tests); final-review smoke beads ht-2i4/ht-jf3/ht-dtq fold into poke-admission/composer-classification/agent-facing-docs
- coverage-round-2 · auto applied C15–C31 (17 findings; 10 GAP, 7 UNOWNED-SEAM; new leaf ht-1ip.17; edge ht-1ip.16<-ht-1ip.2) · divergence: findings 14 → 16 · novel 15/16 (94%) · widening: yes · integration sweep ht-1ip.18 created · loop ended (fixed two rounds)

sweepFix: 5 failing → ht-1ip.53, ht-1ip.54, ht-1ip.55 · re-run 62f9c1a7 — 2411 passed, 1 failed, 0 errors, 37 skipped; failing: composition::actual_native_snapshot_cancellation_closes_peer_before_elected_worker_join_and_owner_release (passes 3/3 alone; load-sensitive); command: TMPDIR=/tmp/htm/tmp nice scripts/full-suite-gate 1 @ 62f9c1a7

parked:
- coverage-findings-round-2.md · degraded-verdict · "coverage round 2 widening: yes (14 → 16 findings, 94% novel); round-2 fixes are never re-reviewed by coverage — proceeded to the design roast"
- graph-pass · graph-change · "narrow ht-1ip.11 <- ht-1ip.5 to ht-1ip.11 <- ht-1ip.4 (skill prose may need only contract types + validator rules; depth 6→5) — safe no, parked"
- graph-pass · graph-change · "proposal: seam-contract summary core API (chunk/cover/render and validate/fold stubs in ht-1ip.1) so ht-1ip.5 runs parallel to ht-1ip.3/.4 — parked"
- 2026-10-02-thread-summaries-compaction-survival-roast-design-1.md · escalation · "§8 entry extension without stored progress: whether catch-up re-entry or a keep-call may extend at all (panel 2-1 dissent); the §8 'every extension requires new stored progress' wording is inaccurate for entry"
- super-code final review · degraded-verdict · "ht-1ip.15 parked: native smoke could not show the focused-pane skip or a controlled soft-point poke (budget; findings ht-yuz, ht-2i4) — accepted as parked, surfaced at hand-back"

codeBuckets:
  completed: ht-1ip.1, ht-1ip.2, ht-1ip.3, ht-1ip.4, ht-1ip.5, ht-1ip.6, ht-1ip.7, ht-1ip.8, ht-1ip.9, ht-1ip.10, ht-1ip.11, ht-1ip.12, ht-1ip.13, ht-1ip.14, ht-1ip.15, ht-1ip.16, ht-1ip.17, ht-1ip.18, ht-1ip.32, ht-1ip.33, ht-1ip.37, ht-1ip.38, ht-1ip.39, ht-1ip.40, ht-1ip.46, ht-1ip.47, ht-1ip.48, ht-1ip.49, ht-1ip.50, ht-1ip.51, ht-1ip.52, ht-1ip.53, ht-1ip.54, ht-1ip.55
  escalated:
  pendingRetry:
  parked: ht-1ip.15
  stalled: false
  review: not ready (no code change required; needs green full suite, docs fix README.md:149/agent-usage.md:156, human decisions: ht-yuz focus-skip unverified natively, Claude poke skipped while prompt suggestion shows, §8 entry/re-entry extension + A6 wording; follow-ups: poke witness race on busy instance, stash clear-failure draft loss before stash becomes reachable)
  sweep: 62f9c1a7 — 2411 passed, 1 failed, 0 errors, 37 skipped; failing: composition::actual_native_snapshot_cancellation_closes_peer_before_elected_worker_join_and_owner_release; command: TMPDIR=/tmp/htm/tmp nice scripts/full-suite-gate 1 (756 s, over the 5-minute budget) @ 62f9c1a7
  slowness: round 1: graph-bound — 7 open beads, depth 3, achievable width 3 vs cap 8; edge audit armed now
  worktreesKept:
  processSweep: stopped 0 · survived 0
scopeFilter-round-1: [Should-fix] src/harness/composer.rs:122 in-scope — Incorrect behavior in the goal-named safe pane poke: a soft-wrapped CJK or emoji draft is classed safe and the retype corrupts the person's draft.
scopeFilter-round-1: [Should-fix] src/summary/identifiers.rs:105 in-scope — Incorrect extraction in the goal-named summary path: a fenced block becomes one multi-KB Error identifier stored and carried into every fold and bundle.
scopeFilter-round-1: [Should-fix] src/store/wake.rs:657 punch-list — Query-plan cost of the poke lookup; output is correct, so it is a quality improvement to goal-named code rather than a correctness defect.
scopeFilter-round-1: [Should-fix] src/protocol/wire.rs:15 in-scope — The new summary wire commands ship without a PROTOCOL_VERSION bump, so skew with an old daemon goes undetected and requests are dropped silently in a goal-named path.
scopeFilter-round-1: [Should-fix] docs/agent-usage.md:73 in-scope — The authoritative agent doc still says the plugin does not start a summary, which is misleading in the goal-named summary and recovery path.
scopeFilter-round-1: [Nit] src/store/summary.rs:1548 in-scope — Incorrect behavior in the summary path: a human seat's info or warn event line counts as a priority message and can be cited to supersede a user instruction.
scopeFilter-round-1: [Nit] src/summary/render.rs:142 punch-list — cluster override: Size-efficiency of a soft target only, with no wrong result, unlike the identifier cap which stores wrong data.
scopeFilter-round-1: [Nit] src/summary/fold.rs:202 in-scope — over_budget and sizes.fold_bytes understate the fold the caller receives, a misleading result in the goal-named summary path.
scopeFilter-round-1: [Nit] src/scheduler/mod.rs:259 in-scope — Truncating to 16 seats before the eligibility filters can leave later due seats never poked, so the goal-named poke does not fire for them.
scopeFilter-round-1: [Nit] src/store/mod.rs:1909 punch-list — Extra idle-tick transactions and a prepare inside a loop; a performance hardening that does not affect correctness.
scopeFilter-round-1: [Nit] src/notification/policy.rs:313; src/notification/dispatch.rs:222 punch-list — Diagnostic label is lost for poke skips; the outcome itself is correct, so this is a diagnostics improvement.
scopeFilter-round-1: [Nit] src/cli/hook.rs:1406 punch-list — Recovery read errors are swallowed with no diagnostic; behavior is otherwise correct, so this is a hardening and diagnostics improvement.
scopeFilter-round-1: [Nit] docs/evidence/summary-smoke/captures/state/codex-scratch-config.final.toml:11 punch-list — A personal-config leak in committed evidence files, which is not behavior the goal names.
scopeFilter-round-1: [Nit] tests/store/schema.rs:3278 in-scope — Missing test coverage for the author_role backfill bound, which feeds the goal-named human-priority summary behavior.
scopeFilter-round-1: [Nit] tests/integration/summary_flow.rs:36 punch-list — Test flakiness under slow setup that was not reproduced; a test-robustness improvement, not a failing test.
scopeFilter-round-1: [Nit] tests/integration/summary_flow.rs:1186 punch-list — Wall-clock sensitivity in a test that fails loudly when slow; a test-robustness improvement.
scopeFilter-round-1: [Nit] src/protocol/output_compact.rs:336 punch-list — cluster override: Grammar completeness only; it is fixed in the same docs sweep as the in-scope stale-summary text.
scopeFilter-round-1: [Nit] docs/operations.md:131 punch-list — cluster override: Operator-doc completeness for the poke and deadline sections; the in-scope part of the cluster is the stale agent-usage summary text.
scopeFilter-round-1: [FYI] src/store/schema.rs:270 punch-list — One-way migrations with no backup are a pre-existing pattern the goal does not name.
scope-filter: 8 in-scope · 11 punch-listed
