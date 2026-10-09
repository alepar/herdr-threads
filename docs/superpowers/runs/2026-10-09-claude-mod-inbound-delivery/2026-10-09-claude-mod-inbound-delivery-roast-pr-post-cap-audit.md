super-roast verdict: clean (1 nits) [converged]
mode: pr        iteration: post-cap audit
profile (assumed): herdr-threads is a local, single-user Rust daemon plus CLI that relays messages between coding agents in Herdr panes, with a bundled Claude Code delivery mod (JS). The trust model is cooperative and same-user (AGENTS.md, TRUST-POLICY.md); the store is local SQLite behind a Unix socket, with no network exposure, no external users and no money or regulated data. Rollback is a binary swap plus `setup`/`unsetup`. Blast radius is low: a mod delivery outage degrades to the native wake path and is recovered by a plugin reload or daemon restart.
inputs: super-auto/claude-mod-inbound-delivery@23fb8208 vs bd8e6661 (scoped post-cap audit; + merge 81dbb244 resolution)
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 2 resolved · 0 regressed (0 Blocking) · 4 punch-listed (open)
coverage: scouts 14/14 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, regression, data-migrations, deploy-safety, api-contract, observability, testing, dependency, hygiene-docs) · raw 2 → deduped 2 → panel 1 · spot 1 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 1 · rr 0.00 · rg 0.00 · fg 1.00 · unanimous 0.00 · ground-loo n/a (n=0) · reproduce 0/0/1 · refute 0/1/0 · ground 0/1/0
lane-yield (found/confirmed/unique/refuted): correctness 0/0/0/0 · security 0/0/0/0 · premortem 0/0/0/0 · simplicity-design 1/0/0/1 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · regression 0/0/0/0 · data-migrations 0/0/0/0 · deploy-safety 0/0/0/0 · api-contract 0/0/0/0 · observability 0/0/0/0 · testing 1/0/0/0 · dependency 0/0/0/0 · hygiene-docs 0/0/0/0

prior-report tracking (iteration 2 confirmed findings):
- resolved: [Should-fix] src/protocol/watch.rs:86 (truncation marker omits the launch selectors) — fixed by ht-j16.24 (18e587d5): `truncation_marker_for(prefix, id, lazy)` renders the `--state-dir`/`--host-endpoint` prefix the watch child was launched with, and the module doc at 23fb8208 states the rule. Not re-surfaced by any current packet.
- resolved: [Nit] src/protocol/watch.rs:86 (truncated lazy rows told to `ack`) — fixed by the same commit (18e587d5): a lazy row's marker names only `body`; the module doc at 23fb8208 says "its marker names only `body`". Not re-surfaced.
- punch-listed (open): [Nit] src/service/mod_channels.rs:240 (sweep stall close by seat only).
- punch-listed (open): [Nit] src/service/mod_channels.rs:193; src/daemon/settings.rs:20 (`set_mod_delivery` has no production caller).
- punch-listed (open): [Nit] src/cli/setup.rs:1708 (interactive `[Y/n]` inside `execute()`).
- punch-listed (open): [Nit] scripts/test-claude-mod:13; scripts/test-claude-mod:1 (JS mod tests unenforced).
- prior rejection (src/harness/claude_mod.rs:301, `ManagedPolicy::Unverifiable`) and prior unverified nit (integrations/claude/mod/hooks/register.js:592, no JS-side LAUNCH test) were not re-surfaced; both stand as reported in iteration 2.

## Confirmed findings
- none

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none

## Unverified nits (spot-checked)
- [FYI] src/cli/setup.rs:745 — The main merge (81dbb244) adds a hard-coded `registration.metadata().id == Harness::Claude.as_str()` branch to the shared, registry-driven setup dispatcher that calls `complete_setup_status`, which re-resolves the legacy environment and request the adapter's own `status()` already resolved; `StatusRequest` has no field to express a setup-status-only run, so the Claude-specific logic lives outside the adapter. [lanes: simplicity-design] (spot: REJECT FYI — the refute seat confirms the facts (src/cli/setup.rs:745 branch; src/harness/claude/setup.rs:778-809 re-resolution; `StatusRequest` carries only scope, environment and native_binary) but finds no spec or TRUST-POLICY violation: spec D8 requires that only `setup-status` runs the version gate and daemon probe, the branch meets it with a safe silent fallback, the finding itself concedes "today's output is correct", and the proposed purpose/depth field on `StatusRequest` would change main's shared adapter trait for hypothetical future adapters. Design preference, not a defect. Severity set to FYI per the seat; one refute-seat pass is not panel-strength verification.)

## Escalations (need human)
- integrations/claude/mod/hooks/register.js:67 — UNVERIFIED external (reproduce seat). Claim: the D3 fix (ht-j16.30) treats any main tool result with `isError: true` or a `deny` as a user rejection, so a turn ending in an ordinary failed tool call or a rule/hook deny, followed by a text answer, holds idle peer submits for up to HOLD_IDLE_MS (120 s) or until the next turn completes, and no test or live-stress scenario covers that false-positive. The internal mechanism is confirmed by all three seats (register.js:67 heuristic; `S.turnRejected` set on every main tool call and reset only in onTurnStart; `onTurnComplete` sets `abortHoldSince`; delivery.test.ts:210 and :334-338 both key only on `isError: true`). What stays open: whether Claude Code 2.1.295 reports a failing Bash call or a permissions.deny rule as `isError`/`deny` on the mod `tool.call` result is undocumented; the reproduce seat could only find `is_error: true` in the CLI transcript JSON (docs/evidence/claude-285-hook-capture/payloads/transcript-evidence.json:84), a different surface. Context for the human: the refute and ground seats both REJECT at FYI because the design spec (docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md:381, commit 2b2d930c) explicitly states "This is a heuristic: a denied or failed tool the model recovered from with no further tool call also holds, which delays a submit until the next user turn or 120 s idle", and register.js:62-67 and the README repeat it; so the over-match is a documented, accepted tradeoff and the residual point is a test-hygiene remark. The route is escalation because the external premise is unverified; the seat evidence suggests it is an accepted limit, not a regression. Cheapest check per the finding's spike: a live-stress scenario where the model runs `false` via an allowed Bash rule, then answers, with a peer message sent mid-turn; if the submit lands within seconds with no `held post_abort` ledger entry, the finding drops.
