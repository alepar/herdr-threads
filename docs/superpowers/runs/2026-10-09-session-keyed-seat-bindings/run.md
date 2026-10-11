# super-auto run — 2026-10-09-session-keyed-seat-bindings

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: fix-loop
codeMechanism: Workflow

idea: Goal: implement the new herdr-threads seat-bindings model we designed in this session, absorbing the Hermes global-enrollment work (ht-4ne, branch super-auto/hermes-global-enrollment, unmerged) it sprang from. Flags: planOneShot=false (interactive Mode A design for remaining questions), skipPlanRoast=false, skipCodeRoast=false, autonomous=true from after the design. The Codex CODEX_EXEC_SERVER_URL pane-env installer step runs separately on branch codex-daemon-env and is out of scope here.
branch: super-auto/session-keyed-seat-bindings
base: main
spec: 2026-10-10-session-keyed-seat-bindings-design.md
epic: ht-54r
roastDesignRound: 2
graph-pass: depth 7→6 · width 3.6→4.2 · applied 7 · parked 0 (human-approved; summary.rs split declined)
stepBackDesign-round-1: patch — four clusters (supersede rule, V29 row classes, human-seat transitions, claim-source completeness), no recurrence; root goal stands
roast-design: 2026-10-10-session-keyed-seat-bindings-roast-design-1.md, 2026-10-10-session-keyed-seat-bindings-roast-design-2.md

roastCodeRound: 1
roast-code: 2026-10-10-session-keyed-seat-bindings-roast-pr-1.md
stepBackCode-round-1: patch — no recurrence yet (round 1); three clusters (lazy-unread-per-binding, role-from-resolved-binding, authority-by-role-and-liveness) where the per-seat→per-binding migration was applied unevenly; sweep each rule
scopeFilter-round-1: [Should-fix] src/store/queries.rs:6514 in-scope — The inbox v2 lazy source pages the seat-level set instead of the binding's own cursor, which breaks per-binding cursors, a core goal behavior.
scopeFilter-round-1: [Should-fix] src/store/receipts.rs:315 in-scope — A top-level binding enrolled by a plain CLI call can never display-ACK, which is incorrect behavior in exact-session attribution and receipts.
scopeFilter-round-1: [Should-fix] src/store/seats.rs:4480 in-scope — A read-only subagent binding (a Hermes child) can check in for the seat, so its calls are not attributed or restricted by the role of the exact calling session.
scopeFilter-round-1: [Should-fix] src/cli/mod.rs:2797 in-scope — A CLI subagent read gets the seat-level page and never advances its own binding's cursor, contradicting the per-binding stream and cursor goal.
scopeFilter-round-1: [Should-fix] src/store/lazy_delivery.rs:287 in-scope — A binding enrolled after its seat left a thread gets old lazy mail re-delivered as unread, so per-binding unread is incorrect.
scopeFilter-round-1: [Should-fix] src/store/read_cursors.rs:197 punch-list — The per-binding unread probes are correct but scan every ordinary message, so this is a performance concern with no correctness effect.
scopeFilter-round-1: [Nit] src/harness/claude_rewrite.rs:14; src/cli/commands.rs:1760 punch-list — Only the error message is misleading, and the command is refused fail-closed as it should be, so this is a UX polish item.
scopeFilter-round-1: [Nit] src/store/lazy_delivery.rs:46 punch-list — cluster override: It is an index and perf nit with no effect on whether lazy unread is correct, unlike the other members, which are correctness defects.
scopeFilter-round-1: [Nit] src/store/bindings.rs:665 in-scope — A race can mark both bindings of a live process disconnected, a wrong result in the goal-named dead-session liveness path.
scopeFilter-round-1: [FYI] src/store/schema.rs:592; src/store/schema.rs:423; docs/operations.md:230 punch-list — Forward-only migrations are pre-existing practice and are not a goal-named behavior; the gap is a changelog and hardening matter.
scopeFilter-round-1: [Nit] TRUST-POLICY.md:312 punch-list — This is stale doc wording in the policy summary bullets, which A4 already governs.
scope-filter: 6 in-scope · 5 punch-listed
codeBuckets:
  completed: ht-54r.1 ht-54r.2 ht-54r.3 ht-54r.4 ht-54r.5 ht-54r.6 ht-54r.7 ht-54r.8 ht-54r.9 ht-54r.10 ht-54r.11 ht-54r.12 ht-54r.13 ht-54r.14 ht-54r.15 ht-54r.16 ht-54r.17 ht-54r.18 ht-54r.20 ht-54r.21 ht-54r.22 ht-54r.23 ht-54r.24 ht-54r.39
  escalated: ht-54r.19
  pendingRetry: ht-54r.25
  parked: ht-54r.5 ht-54r.12
  stalled: false
  review: not ready (F1 lazy inbox per-seat not per-binding; F2 subagent display/advance broken (ht-6v2); F3 display-ACK refused for cooperative_env_session bindings; 12 deterministic test failures; ht-54r.19 smoke + ht-54r.25 sweep not landed; ht-ozo /clear wake model undecided)
  sweep: SWEEP DEFERRED (caller-owned)
  slowness: round 1 hook.rs hot-file cap raised to 4; merge queue peaked at 3
  worktreesKept: .worktrees/super-auto-session-keyed-seat-bindings--task-ht-54r.19 — task-ht-54r.19 not merged (BLOCKED on ht-6v2)
  processSweep: stopped 0 · survived 0

approvals:
- top-split · human · ht-54r.1 LEAF, ht-54r.2 LEAF, ht-54r.3 LEAF, ht-54r.4 LEAF, ht-54r.5 LEAF, ht-54r.6 LEAF, ht-54r.7 LEAF, ht-54r.8 LEAF, ht-54r.9 LEAF, ht-54r.10 LEAF, ht-54r.11 LEAF, ht-54r.12 LEAF, ht-54r.13 LEAF, ht-54r.14 LEAF, ht-54r.15 LEAF, ht-54r.16 LEAF, ht-54r.17 LEAF, ht-54r.18 LEAF, ht-54r.19 LEAF, ht-54r.20 LEAF, ht-54r.21 LEAF, ht-54r.22 LEAF, ht-54r.23 LEAF
- design-mode · human · roast loop runs unattended (escalations/qualifiers parked); Mode A interactive design by user request at launch; autonomy from phase 3 (reserved: design questions asked, not parked)
- coverage-round-1 · c1-01..c1-14 applied auto, c1-15 rejected auto · R-list: coverage-round-1-requirements.md (R1–R16) · requirements: 16 · mapped: 14 · unmapped: 2 (R15, R16 — appended from r-new) · not degraded (2 valid reviews)
- coverage-round-2 · c2-01..c2-17 applied auto · requirements: 16 · mapped: 16 · unmapped: 0 · divergence: findings 14 → 17 · novel 16/17 (94%) · widening: yes (cap holds at 2; design roast covers round-2 fixes) · not degraded (2 valid reviews) · integration sweep bead created
baseAbsorbed: cb6052fc → 520a1780, 1 conflicted files
parked:
- 2026-10-10-session-keyed-seat-bindings-roast-design-1.md · escalation · "harness sandboxes (Linux bwrap PID namespace, Codex seatbelt) may hide the agent ancestor PID/start time so sandboxed calls are refused — spike: with the Claude sandbox on, read CLAUDE_PID start time from a sandboxed Bash call"
- 2026-10-10-session-keyed-seat-bindings-roast-design-1.md · escalation · "a sibling Bash PreToolUse hook returning updatedInput/ask may drop the --agent-id rewrite (claude-code issue 83353, last hook wins) — subagent misattributed to parent"
- 2026-10-10-session-keyed-seat-bindings-roast-design-1.md · escalation · "Codex CODEX_THREAD_ID != CODEX_SESSION_ID subagent test unverified at runtime across /new, /clear, fork, resume"
- 2026-10-10-session-keyed-seat-bindings-roast-design-1.md · escalation · "Hermes /resume: whether HERMES_SESSION_ID re-syncs to the resumed id (same binding) or rotates (needs lineage)"
- 2026-10-10-session-keyed-seat-bindings-roast-design-2.md · escalation · "Hermes compression lineage source: pre_llm_call parent_session_id documented for delegation only; spec falls back to a new binding at the seat maximum if absent"
- 2026-10-10-session-keyed-seat-bindings-roast-design-2.md · degraded-verdict · "design roast exit: converged at round 2 (Should-fix 9, 0 Blocking); punch list applied inline to spec + beads, not re-roasted"
- codeBuckets.review · escalation · "ht-ozo /clear wake model decided autonomously per spec §1.3/§1.5/§3 Wake: no rebind-grace suppression; /clear = new binding at seat-max cursors (nothing pre-existing to digest), composer wake only for mail unread by that binding; /resume keeps the binding (no binding_changed close). mod_delivery tests rewritten to this model, not restored."
