# super-auto run — 2026-10-02-harness-version-evidence

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
resumeChange: 2026-10-02 · "[coordinator update] flakiness side quest landed on main (a7255713) ... 'no full suite' pause is lifted. Speed budgets in AGENTS.md still apply" · base main merged in at a7255713; phase-6 sweep command = nice scripts/full-suite-gate 1
phase: roast-code

idea: Epic ht-xoc (harness version evidence), base main (26eef585, B6 landed). Design already exists: spec docs/superpowers/specs/2026-10-02-harness-version-evidence-design.md (two design roasts already run: docs/superpowers/reviews/2026-10-02-harness-version-evidence-roast-design-{1,2}.md, round 2 converged) and the filed bead tree ht-xoc.1-.8 — start from the design's coverage checks. Fully autonomous (no questions), both roasts on (design roast and code roast). Merge back into main at the end (user pre-authorized). Test policy: ht-zo4 (flakiness side quest) is closed, so the full suite may run; respect AGENTS.md speed budgets on main.
branch: super-auto/harness-version-evidence
base: main
skillSource: ~/.claude/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar4.11/skills @ 6.4.2-alepar4.11 (6.4.2-alepar4.11)
migrated: session-loaded skill text was 6.4.2-alepar4.6; the run follows the installed and published 4.11 files read from disk instead of restarting the session
spec: ../../specs/2026-10-02-harness-version-evidence-design.md
epic: ht-xoc

approvals:
- top-split · auto · ht-xoc.1 LEAF, ht-xoc.2 LEAF, ht-xoc.3 LEAF, ht-xoc.4 LEAF, ht-xoc.5 LEAF, ht-xoc.6 LEAF, ht-xoc.7 LEAF, ht-xoc.8 LEAF (pre-filed decomposition adopted per invocation; promotion review skipped)
- coverage-round-1 · canonical R-list: R1-R12 (coverage-round-1-requirements.md; R13-R16 appended from r-new) · requirements: 12 · mapped: 12 · unmapped: 0 · auto 11 applied (C1-C11 in coverage-ledger.md): amended ht-xoc.1-.7, new edge ht-xoc.4←ht-xoc.3
- coverage-round-2 · canonical R-list: R1-R16 (coverage-round-2-requirements.md) · requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 11 → 8, novel 100%, widening: no · auto 8 applied (C12-C19), C20 NEEDS-SPEC not honored (findings actionable) · new edge ht-xoc.2←ht-xoc.1 · integration sweep ht-xoc.7 adopted

roastDesignRound: 2
roast-design: 2026-10-02-harness-version-evidence-roast-design-1.md, 2026-10-02-harness-version-evidence-roast-design-2.md

parked:
- graph-pass · graph-change · "proposal: seam-contract attribution result type + reader signature (ht-xoc.2/.4) to take the ht-xoc.8 spike off the main chain (depth 5→4)"
- graph-pass · graph-change · "proposal: seam-contract evidence row type + read API (ht-xoc.4/.5) so .5's table tests run beside .4 (with the first, depth 5→3)"
- 2026-10-02-harness-version-evidence-roast-design-2.md · escalation · "UNVERIFIED external: whether resuming a Codex rollout appends a later session_meta and where; assigned to spike ht-xoc.8 with kill criteria"
- 2026-10-02-harness-version-evidence-roast-design-1.md · escalation · "Material dissent: attribution step 3 vs the Codex shared app-server daemon (process start older than exe mtime on the same inode) — dissolved by the round-1 redesign (transcript attribution; no mtime guard)"
stepBack-round-1: redesign — applied: process-tree/executable version attribution → version read from the harness's own session transcript at transcript_path (dissolves 2 + the parked escalation); clusters contract-scoping, evidence-transport, refused-split, opt-out-semantics patched; roll-up rule, registered-event classification and token-scope nit fixed inline
- roast-design loop exit · converged at round 2 (Should-fix 8 confirmed [converged], 0 Blocking) · punch list applied inline to spec and beads ht-xoc.1/.2/.4/.5/.8 (transcript read rule, buffered SessionStart, transport/gate/heartbeat, registered-event --event flag, contract selection, refused-version evidence)
graph-pass: depth 5→5 · width 1.6→1.6 · applied 0 · parked 2
assumption: ht-xoc.8 Codex half was BLOCKED-AUTH (codex exec --dangerously-bypass-hook-trust and app-server hooks/list refused); not routed around — answered from read-only analysis of existing Codex rollouts plus the Claude half the task captured; Codex resumed sessions declared unattributable (docs/compatibility/harness-transcript-version.md)
assumption: final-review F3 decided autonomously — canary takes verified_max/exclusions under main's contract and re-probes known_broken rows from other contracts (bead ht-xoc.14)
phase3-launch-1: wf_b7d446aa-7f9 → ready-drained; completed ht-xoc.1, .3, .6; escalated ht-xoc.8 (BLOCKED-AUTH); final review NOT READY (F1 settings.json double schema Critical, F2 publish gate, F3 canary re-probe) → fix beads ht-xoc.12, .13, .14; relaunching

codeBuckets:
  completed: ht-xoc.1, ht-xoc.2, ht-xoc.3, ht-xoc.4, ht-xoc.5, ht-xoc.6, ht-xoc.7, ht-xoc.12, ht-xoc.13, ht-xoc.14, ht-xoc.18, ht-xoc.19, ht-xoc.20, ht-xoc.21, ht-xoc.22
  escalated:
  pendingRetry:
  parked:
  stalled: false
  review: NOT READY (fix-loop-1 final review: evidence step on the hook critical path before observe_harness_in (cuts --version probe budget); doctor 'working' for a verified recipe known_broken version (latent); Codex resume relies on uncaptured source=resume; minor gate slot / manifest freshness / downgrade notes; full-suite sweep outstanding)
  sweep: SWEEP DEFERRED (caller-owned)
  slowness: launch 1 drained on BLOCKED-AUTH spike (ht-xoc.8) — answered by session, relaunched
roastCodeRound: 2
roast-code: 2026-10-02-harness-version-evidence-roast-pr-1.md
stepBack-round-1: patch — 12 r1 findings are independent local defects plus one spec clause (Codex resume attribution) narrowed to the spike's verdict; 4 clusters swept (codex-resume-attribution, user-visible-docs-sync, hook-path-bounded, store-error-paths); red loop_inventory test fixed alongside
scopeFilter-round-1: [Should-fix] src/harness/attribution.rs:87 in-scope — Codex resumed session attributed to old version; breaks broken-only-when-observed
scopeFilter-round-1: [Should-fix] src/daemon/harness_evidence.rs:162 in-scope — failed store write loses held SessionStart outcome
scopeFilter-round-1: [Should-fix] docs/install.md:184; docs/agent-usage.md:144 in-scope — docs mislead about hook path and Health counting
scopeFilter-round-1: [Nit] src/cli/hook_evidence.rs:236 punch-list — delays verification only; no wrong state
scopeFilter-round-1: [Nit] src/harness/state.rs:423 punch-list — contract rollback/downgrade not goal-named
scopeFilter-round-1: [Nit] scripts/canary/manifest.py:306 in-scope — last_working fallback can name the broken or newer version as pin target
scopeFilter-round-1: [Nit] scripts/canary/release_contract.sh:58; .github/workflows/harness-canary.yml:157 punch-list — annotation level / infra hardening
scopeFilter-round-1: [Nit] src/harness/setup.rs:396; src/harness/setup.rs:404 punch-list — cluster override: old-build setup downgrade note separable from stale foreign-hook/Health docs
scopeFilter-round-1: [Nit] src/harness/attribution.rs:109; src/cli/hook.rs:2042 punch-list — hook hardening; harness timeout caps damage
scopeFilter-round-1: [Nit] src/app.rs:1199; src/app.rs:1201 punch-list — cluster override: advisory Health visibility gap separable from recorder lost-write bug
scopeFilter-round-1: [Nit] src/harness/setup.rs:756; src/cli/doctor.rs:850 punch-list — cluster override: Codex re-trust wording separable from stale foreign-hook/Health docs
scopeFilter-round-1: [Nit] src/harness/attribution.rs:193; src/harness/attribution.rs:209 punch-list — cluster override: parse-cost optimization separable from wrong-version attribution
scope-filter: 4 in-scope · 8 punch-listed
fixLoop-round-1: epic reopened; filed ht-xoc.18 (codex-resume-attribution), .19 (store-error-paths), .20 (user-visible-docs-sync), .21 (manifest.py:306), .22 (loop_inventory test); re-entering super-code
fixLoop-launch-1: wf_a0cdda7f-a67 (super-code re-entry, deferSweep, beads ht-xoc.18-.22)
fixLoop-launch-1 result: root-closed; 15 merges, 0 failed; final review NOT READY (see fixloop-1-final-review.json)
