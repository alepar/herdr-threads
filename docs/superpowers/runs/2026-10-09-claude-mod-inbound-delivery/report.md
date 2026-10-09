# super-auto report — 2026-10-09-claude-mod-inbound-delivery

status: completed with 0 unresolved Blocking, 4 escalations [degraded: coverage widened in round 2 (12 → 19 findings, 100% novel); round-2 fixes are not re-reviewed by coverage, final review: not ready (code-final-review-8.md; its two must-fixes were resolved after it, unreviewed by a further final review: finding 1 fixed at dc5b2f6c, live stress re-run at b562d1e4 13/14 3/3 with reload_mid_turn 2/3 ledger-only; post-cap audit clean)]
metrics: parked draft — upstream-feedback-draft.md (5 defects, 3 design questions; not sent)

Branch `super-auto/claude-mod-inbound-delivery` → `main` (merge-base 3793cf88 after absorbing main at 53b6df93). Goal and spec: [design](2026-10-09-claude-mod-inbound-delivery-design.md).

## Implemented

- ht-j16.1 seam contract: watch protocol, `AckModDelivered`, provenances, TRUST-POLICY amendment — beads, ledger
- ht-j16.2 daemon watch connection and `ModChannels` registry (Live / ReconnectGrace / RebindGrace) — beads, ledger
- ht-j16.3 `AckModDelivered` deciding transaction (`cooperative_mod_delivery`) — beads, ledger
- ht-j16.4 wake routing around live mod channels (suppress, stall handover, disconnect) — beads, ledger
- ht-j16.5 `herdr-threads watch` / `watch ack` CLI — beads, ledger
- ht-j16.6 the Claude mod: delivery state machine (context / submit / append, post-abort hold) and plugin tests — beads, ledger
- ht-j16.7 `setup claude` installs the mod via `CLAUDE_CODE_PLUGIN_DIRS`; unsetup, setup-status — beads, ledger
- ht-j16.9 live race stress driver against real Claude Code 2.1.295 TUI sessions, with evidence — beads, [evidence](../../../evidence/claude-mod-delivery/README.md)
- ht-j16.10 integration sweep: daemon, watch, mod, ack, fallback end to end — beads, ledger
- ht-j16.17–.27 PR-roast and final-review fixes: per-seat liveness, ack chunking, capability-probe resilience, mod argv = hooks argv, notices while live, install safety, truncation marker (selectors, lazy variant), relay/intent markers, no delivery after the watch run ends, indented peer text — beads, ledger
- ht-j16.28–.30 live-stress defects D1 (`/clear` rebind), D2 (reload mid-submit), D3 (Esc at permission dialog) — beads, run.md `postLoopFix`
- ht-j16.31–.33 final review 7 must-fixes: pre-submit window re-check, thread/sender names in watch lines (spec D4), stale attention submit at session start — beads, run.md `postLoopFix`
- dc5b2f6c final review 8 finding 1: predecessor batch with an attention block submitted twice after reload (fixed directly, with a delivery test) — run.md `postLoopFix`

## Verification

- Sweep: 4029 passed, 0 failed, 43 skipped @ 53b6df93 (167 s, budget 5 min); leak check clean; clippy and check-default-features clean; also passed @ 11b01cd5 before the last main merge. A loaded run at 53b6df93 failed 2 untouched hermes runtime tests, which passed alone and in the quiet re-run: flake ht-uy3 — run.md `codeBuckets.sweep`, `sweepFix`
- Mod JS tests: 83/83 (`scripts/test-claude-mod`) at dc5b2f6c — run.md `postLoopFix`
- Live stress at b562d1e4 (final code SHA), 3 iterations: 13/14 scenarios 3/3; `reload_mid_turn` 2/3 — [evidence README](../../../evidence/claude-mod-delivery/README.md), run.md `liveStress`
- Post-cap audit roast (bd8e6661 → 23fb8208 + merge 81dbb244 resolution): clean (1 nit) [converged] — [report](2026-10-09-claude-mod-inbound-delivery-roast-pr-post-cap-audit.md)

## Remaining

- Escalation (design roast 1): idle check vs submit non-atomic (user Enter between check and engine acceptance) — run.md `parked`; live `esc_interrupt_hold`/`queued_user_prompt` pass 3/3, not proven race-free
- Escalation (design roast 1): `$.session.id()` in `session.end` returns the ending id — run.md `parked`; this is live defect D1, fixed by ht-j16.28, `clear_rebind` 3/3 live
- Escalation (security review): frame markers spoofable by peer text — run.md `parked`; mitigated by ht-j16.27 indentation, recorded as a TRUST-POLICY accepted limit
- Escalation (post-cap audit): D3 heuristic may hold idle submits up to 120 s after an ordinary failed/denied tool call, engine signal unverified — run.md `parked`, audit report
- `reload_mid_turn` 2/3 live: a submit resolving just before dispose leaves its `submitting` record, the successor records a second `delivered` (one submit, one presentation, ack `already_settled`) — evidence README
- Final review 8 minors: stress model does not count predecessor submits; session-start double presentation (digest + native wake before the mod registers) not named in TRUST-POLICY; `rec.attentionVersions` dead state; drain re-pages pending bodies per attention frame; rebind-grace ack accepted before re-registration — final review, [code-final-review-8.md](code-final-review-8.md)
- out of scope (filtered): [Nit] src/service/mod_channels.rs:240 sweep stall close by seat only — scopeFilter-round-1
- out of scope (filtered): [Nit] src/service/mod_channels.rs:193; src/daemon/settings.rs:20 `set_mod_delivery` has no production caller — scopeFilter-round-1
- out of scope (filtered): [Nit] src/cli/setup.rs:1708 interactive prompt inside `execute()` — scopeFilter-round-1
- out of scope (filtered): [Nit] scripts/test-claude-mod JS mod tests unenforced (skip when `claude` absent; not in nextest/CI) — scopeFilter-round-1
- cluster dropped: mod-test-enforcement (required mode for `scripts/test-claude-mod`, wired into the sweep) — scopeFilter-round-1
- Follow-up beads: ht-182 unsupervised mod-channel worker (P2), ht-22y attention marker without launch selectors (P3), ht-oag unbounded mod delivered/emitted sets (P3) — run.md `followUps`
- Not exercised live: truncated bodies, 120 s hold and draft timers, 10-min stall/cooldown, daemon restart, in-session `/resume`/`/branch`, subagent tool calls, non-default permission modes, a real Herdr server (stand-in endpoint) — evidence README
- Post-cap audit FYI: Claude-specific branch in the shared setup dispatcher (src/cli/setup.rs:745) — audit report
- worktreesKept: none; processSweep survivors: none — run.md `codeBuckets`

## Gotchas & surprises

- Live stress found three product defects the unit/stress suites missed (D1–D3); all three trace to engine behaviour stubs did not model — evidence README
- The D2 fix chain regressed twice (ht-j16.29 → .31 → dc5b2f6c): each post-loop final review found a defect in the previous fix — final reviews 7 and 8, friction.md
- Base merge 3d10e3bb → 81dbb244 had 11 conflicts, including a behavioural port of setup D8 into main's Claude setup backend — run.md `baseAbsorbed`; covered by the post-cap audit
- Signed-in Claude profile is Keychain-bound to its path: copies are signed out, so the first ht-j16.9 run fell back to unit level — run.md `postLoopFix`
- PR roast 1 left the integration worktree detached; later roasts ran in separate detached worktrees — friction.md
- Base merge 90066db1 → 3793cf88 (53b6df93): 0 conflicts; main's warning-wake changes do not touch mod suppression; not roast-reviewed (mechanical, clean auto-merge) — run.md `baseAbsorbed`
- Slowness: merge queue peaked at 5 in fix re-entry round 1 — run.md `codeBuckets.slowness`

## Entrypoints

1. `src/protocol/watch.rs` — watch wire types and JSON-line schema
2. `src/service/mod_channels.rs` — ModChannels registry, liveness, stall
3. `src/cli/watch.rs` — the `watch` stream and `watch ack`
4. `src/service/` ack transaction and wake routing (`kicks.rs`), `TRUST-POLICY.md` A3/A4/A5/A8 rows
5. `integrations/claude/mod/hooks/register.js` — the mod (primary caller), tests in `integrations/claude/mod/tests/`
6. `src/harness/claude/setup.rs`, `src/cli/setup.rs` — install
7. `tests/integration/mod_delivery.rs`, `tests/native/claude_mod/stress.py`

## Smells

- ht-j16.21 (notices while live) merged after a fix pass with no re-review; its report predated the fix commit — ledger task 14
- ht-j16.30 (D3) merged after a fix pass that changed the stress model to keep the drop floor — ledger task 23
- Post-loop fixes ht-j16.28–.33 and dc5b2f6c had one light task review each and no roast until the post-cap audit — run.md `postLoopFix`
- D3 rests on a heuristic over an undocumented `tool.call` result shape — register.js:62-67, audit escalation
- RED-phase evidence missing across 9 tasks (not clustered by the ledger: distinct slugs) — code-final-review-8.md
- Stress floors tuned to one seed (tasks 19, 22, 23) — code-final-review-7.md
- sweepFix: none needed — run.md
