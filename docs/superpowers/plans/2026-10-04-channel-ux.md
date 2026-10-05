# Channel UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver approved concise recipient commands, tiered name lookup and conservative one-hour archival.

**Architecture:** Preserve durable instance/seat identity and perform canonical decisions in the daemon. Separate command presentation from recovery targeting. Add bounded lifecycle eligibility tracking with actual production idle evidence.

**Tech Stack:** Rust, SQLite/rusqlite, existing HostPort, Pacer/fair writer, fake clocks.

**Spec:** docs/superpowers/specs/2026-10-04-channel-ux-design.md

## Global Constraints

- CODEX_HOME remains the existing selected Codex profile (unchanged).
- Preserve native approval policy; no real config edits, shared daemon restart, branch full suite, main merge, push or release.
- One allocated additive0023 follows actual relay migration0022; never edit historical migrations.
- All daemon tests use --all-features; all spawned test children use test_support::spawn; use isolated named Herdr sessions.
- Preserve relay optional intent/rule fields and submission2/renderer2/promptv2; combined wire6 follows actual reviewed relaywire5 absorption.
- Run focused tests, cargo fmt, required all-feature clippy/default-feature check and owned-process cleanup.

### Task 1: Canonical tiered name resolution

**Files:** src/store/queries.rs, src/cli/mod.rs, tests/store/queries.rs, tests/cli runtime selector tests.
**Interfaces:** consume ResolveThreadQuery { selector, caller, caller_target }; produce existing ThreadResolved or Conflict/NotFound without protocol/schema changes. Actual cooperative caller beats inherited caller pane; foreign read scopes do not replace caller.

- [x] Add focused fixtures with two lower-tier duplicate names and one joined match. Expected assertion:
```rust
assert_eq!(resolve("team café").unwrap(), CommandResult::ThreadResolved(ThreadId::new("t")));
```
Add separate archived joined, conflicts within each layer, no caller, cross-instance, exact ID, large lower-tier population and scoped caller tests; derive IDs manually, not from resolver results.
- [x] Run `nice cargo test --locked --all-features thread_names` and demonstrate failure caused by one-pool resolution.
- [x] Implement bounded indexed tier queries (limit9 per tier). Exact IDs still win. Resolve caller against instance-scoped canonical resolved seat mapping; query joined including archived first, archived=0 second, all third. Escape bounded conflict data and name winning tier. Keep resulting ID freeze before dispatch/journal.
- [x] Run focused store and runtime selector tests, format and self-review; commit only Task1 changes. Report exact tests and observed red/green evidence.

### Task 2: Recipient-local concise command presentation

**Files:** src/cli/handoff.rs, src/cli/hook.rs, src/harness/bridge.rs/cache.rs as needed; tests/cli/handoff.rs/hook.rs/runtime_helpers.rs; integrations/skill/SKILL.md and Codex/Claude docs.
**Interfaces:** consume exact ContinuationContext plus actual recipient InstanceInputs; reuse pane_selectors/cli_prefix. Produce recipient-verified ready argv without changing HandoffPlan semantic identity or replay compatibility.

- [x] Add behavioral test proving bootstrap tells recipient to use verified hook-ready commands and retains exact fallback when recipient equivalence is not available. Add rendering tests for canonical same state/endpoint, other recipient environment, ambiguous roots, custom endpoint, replayed pinned context and continuation/remediation paths.
- [x] Run focused handoff/hook tests to demonstrate the current bootstrap or rendering fails the intended behavior.
- [x] Separate durable target identity from presentation. Before recipient-local verification retain pinned fallback; after verification render through existing equivalence helper and explain routing. Never use launcher's defaults as recipient proof. Keep fixed inbox/read task payload instructions and exact IDs. Update guidance to teach verified ordinary commands while preserving native permissions.
- [x] Run focused tests, format and self-review; commit only Task2 files. Do not overwrite relay communication/summary guidance; flag concurrent seams for controller reconciliation.

### Task 3: Durable bounded archival lifecycle

**Files:** create migrations/0023_channel_archival.sql, src/store/archival.rs and tests/store/channel_archival.rs; integrate src/store/{mod,schema}.rs, src/service/{config,workers,kicks}.rs, src/daemon/settings.rs, src/ports/* as required; update TRUST-POLICY.md and manual settings/lifecycle docs.
**Interfaces:** use existing production composer/host evidence, fair-writer admission and worker cancellation. Introduce an internal bounded archival pass API and per-instance timing; avoid public wire changes unless actual interfaces require one, then coordinate. Snapshot UI Unknown is never positive idle evidence.

- [ ] First inspect actual relay implementation and agree schema22 integration with controller. Do not fake missing22 or modify historical migrations. Add fake-clock behavior tests for grace expiry, all-left, idle joined, activity reset, legacy grace, restart/outage, human and unknown UI/composer, unresolved/unregistered binding, service-owned, requirements/invitations/receipts and transaction races.
- [ ] Verify tests fail for absent archival feature, not fixture errors. Assert archive flag/event changes while exact binding/membership/receipt snapshots remain unchanged.
- [ ] Add exactly one additive0023 with durable per-channel eligibility and indexed due work plus actual idle evidence/activity tracking. Record fresh initial grace, invalidate on canonical activity/continuity/control changes. Bounded pages must not archive until all necessary evidence and protected-work checks are complete. Recheck revisions/bindings/host evidence in deciding transaction.
- [ ] Wire cancellation-owned bounded worker or extend suitable existing lifecycle lane, validate auto_archive_after_ms (default3600000, zero off), expose health failures. Avoid per-tick full-store scans and unbounded fanout writes.
- [ ] Record automatic archive as daemon policy event without impersonating any seat or settling receipts. Amend A5/accepted limits; document conservative behavior and explicit reopen.
- [ ] Run focused lifecycle/store/worker/config/host tests, indexed volume checks, format and self-review; commit Task3 code/tests/docs.

### Task 4: Shared seams, final verification and reviewed landing coordination

**Files:** shared protocol/schema/guidance seams and spec post-implementation notes; no main edits.
**Interfaces:** actual reviewed relay SHA/landed tree, coordinator w4:p1, branch frozen SHA/check list.

- [ ] Absorb actual reviewed relay contracts/code through coordinator-approved branch integration; preserve all intent/rule fields and native Codex permissions. Reconcile skill prose, final schema23 and combined wire6 after actual reviewed relaywire5 absorption.
- [ ] Run `cargo fmt --check`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `nice scripts/check-default-features`, focused tests for changed behavior and scoped leak checker. Never full suite.
- [ ] Independent whole-branch review; fix confirmed findings and scoped re-review. Record clean reviewed SHA and exact checks, process cleanup and limitations.
- [ ] Notify coordinator readiness with frozen SHA; coordinator owns squash/main merge + combined suite/release. Read-only verify final landing before reporting DONE+MERGED; preserve worktree until then.
