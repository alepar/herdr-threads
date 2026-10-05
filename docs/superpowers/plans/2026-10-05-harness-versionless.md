# Harness Version Independence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Codex and Claude core operations work without installed-version probes, while strict input validation and honest evidence remain intact.

**Architecture:** Registration-bound operational contract handles replace installed-version witnesses on core paths. Optional runtime metadata is separate; unknown runtime cannot grant rich capabilities or verify an exact build. Existing evidence transport carries scoped diagnostics without inventing runtime identity.

**Tech Stack:** Rust, serde JSON, rusqlite, existing owned-config and native Herdr ports.

**Spec:** docs/superpowers/specs/2026-10-05-harness-versionless-design.md

## Global Constraints

- Implement against main f04676cb in harness-versionless-design; do not modify or consume harness-adapters integration.
- Preserve strict JSON/field/value/role/event decoding and canonical daemon identity/receipt authority.
- Do not bypass the managed wrapper, invent version/build identity or grant unsupported composer/poke/native capabilities.
- Ordinary setup, launch, hooks, doctor and daemon observation execute no version/help/schema probes.
- Codex rollout creator metadata does not identify the managed wrapper target; known-resume suppression stays.
- Stop installing obsolete sandbox allowances; global networking allowance remains withheld. Preserve owned unsetup/conflict behavior.
- Historical evidence/migrations/contracts remain historical; append migration 24 if required and freeze for adapter absorption.
- Tests use isolated HOME/CLAUDE_CONFIG_DIR/CODEX_HOME, owned test children and private named Herdr sessions. Never stop shared Herdr.
- Coordinate serial cargo build slot with main; no full suite in this lane. Focused tests plus fmt/clippy/default-features are required.
- Each worker owns only its task files and does not spawn children. Controller dispatches independent reviews.

### Task 1: Operational contracts and versionless hook parsing

**Files:** Create src/harness/operational.rs; modify src/harness/mod.rs, codex.rs, claude.rs, src/cli/hook.rs; tests/harness and tests/cli/hook.rs, tests/hook_entrypoint.rs.

**Interfaces:** Produce private-field CodexContract and ClaudeContract handles with registered constructors. Produce parse_event_for_contract(bytes: &[u8], event_id: &str, contract: &CodexContract) and corresponding Claude entry returning Result<LifecycleEvent, ContextError>. Hook InstalledHarness selects a contract independent of PATH. Existing versioned APIs remain diagnostic/fixture compatibility only. New Capability::ContractValidatedInput never implies Supported/native receipt.

- [ ] Write regressions: observe_harness_in(Codex/Claude, absent PATH, budget, None) returns a declared contract; valid callbacks decode. Reject invalid paired child fields, missing session/tool/source and unknown schema. A wrapper that would log/abort if invoked must have an empty invocation log after hook observation.

```rust
let installed = observe_harness_in(Harness::Codex, None, Duration::from_millis(100), None).unwrap();
assert!(parse_event(&installed, valid_codex_payload).is_ok());
assert!(parse_event(&installed, invalid_role_payload).is_err());
```

- [ ] Run `nice cargo test --locked --all-features versionless` under the granted build slot; record behavioral RED, not only compiler errors.
- [ ] Add registered handles and strict parser entrypoints. Share shape validation with old versioned parsers; do not duplicate permissive fallback. Hook observation returns handles directly, and parse-failure reporting is no longer optimistic-only. Registered-event mismatch must refuse mutation.

```rust
pub fn parse_event_for_contract(bytes: &[u8], event_id: &str, _: &CodexContract) -> Result<LifecycleEvent, ContextError> {
    let value = input(bytes, event_id)?;
    check_hooks_v1(&value)?;
    let mut event = parse_shape(&value, event_id)?;
    event.capability = Capability::ContractValidatedInput;
    Ok(event)
}
```

- [ ] Run relevant hook/parser tests; update obsolete operational version-refusal expectations while retaining real diagnostic version tests. Commit task and report exact APIs/RED/GREEN.

### Task 2: Setup and launch use declared contracts

**Files:** src/cli/setup.rs, src/cli/launch.rs, src/harness/setup.rs; tests/setup_cli.rs, tests/cli/launch.rs, tests/harness/setup.rs.

**Interfaces:** Consume registered handles. Observed installation stores binary, version: Option<String>, recipe/contract identifier and contract_declared admission. Plan Codex via plan_codex_for_contract; old version planner remains fixture/diagnostic compatibility if still used. setup::observe resolves executable only; its returned optional Codex handle is not InstalledVersion. Caller-facing metadata is null and supported/native claims are not manufactured.

- [ ] Add a wrapper that logs every invocation and fails on --version/--help; setup/status and guarded launch preparation must not invoke it for admission. Assert owned hook registration installed, foreign config preserved, no sandbox network allowance installed, and invalid launch form refused before host start.

```rust
let (observed, contract) = observe(&request, &env).unwrap();
assert_eq!(observed.version, None);
assert!(contract.is_some());
assert!(!wrapper_log.exists());
```

- [ ] Run focused tests and record RED.
- [ ] Replace observation/planning witness dependency, remove cold schema-cache warming and version refusal messaging from launch. Keep selected wrapper/PATH, captured launch restrictions, configured hook checks, duplicate/layer inspection and owned transaction checks. Stop installing obsolete socket/root/network changes. Owned manifests retain safe status/unsetup and conflicts; cleanup removes only owned unchanged settings.

```rust
pub fn plan_codex_for_contract(existing: &[EventGroups], hook_argv: &[String], _: &CodexContract) -> Result<CodexSetupPlan, SetupError> {
    plan_codex(existing, hook_argv)
}
```

- [ ] Run focused setup/launch tests, adapting historical allowance-install tests to preserved legacy unsetup ownership tests. Commit and report.

### Task 3: Honest daemon/doctor state and unavailable-runtime diagnostics

**Files:** TRUST-POLICY.md (A4 and related accepted limits), src/app.rs and its harness evidence/state consumers, src/cli/doctor.rs, src/cli/hook_evidence.rs, src/harness/state.rs, src/harness/attribution.rs, src/store/harness_evidence.rs, src/store/schema.rs, src/store/mod.rs, src/ports.rs, src/daemon/harness_evidence.rs, src/daemon/harness_states.rs, migrations/0024_harness_contract_diagnostics.sql; relevant tests/service, tests/store, tests/daemon, tests/cli, tests/integration.

**Interfaces:** Existing HarnessEvidence { version: None, unattributed_reason, contract_id, event, outcome, session_id } remains the wire transport. Add bounded table keyed (harness, session_id, contract_id) for unavailable-runtime violations: event, field, first/last seen timestamps. Existing Health limitations and HarnessStateReport.unattributed.reason can project actionable failure text without new wire fields or fake version rows. Missing session uses parse-failure diagnostics. Any optional installed metadata is absent by default.

- [ ] Add RED regressions for daemon/doctor with failing wrapper, versionless violation persistence without exact-version evidence, later success preserving same-session failure, malformed input not verifying anything, retained historical versions not vetoing core admission, and resume creator attribution suppression.

```rust
assert!(harness_evidence::all(db, "codex").unwrap().is_empty());
assert!(diagnostic_text.contains("PreToolUse"));
assert!(diagnostic_text.contains("session_id"));
```

- [ ] Run focused tests and record behavioral RED.
- [ ] Daemon observes executable/config availability without probes. Status remains cooperative with explicit unverified evidence; rich poke capabilities are NONE without separate captured qualification. Doctor reports metadata unknown as informational, does not repair/install/pin based on version ladder, and emits actual config/payload failures.
- [ ] Preserve exact attributed evidence as diagnostics; remove ladder/manifest refusal from core health derivation. Never attribute Codex current runtime from a creator header. Persist unknown-runtime violations with size/retention caps and sticky same-session failures; project actionable diagnostics through existing fields. Append migration 24, with clean/new/store upgrade and incomplete-schema verification. Historical migrations stay immutable.

```sql
CREATE TABLE harness_contract_diagnostics (
  harness TEXT NOT NULL, session_id TEXT NOT NULL, contract_id TEXT NOT NULL,
  event TEXT NOT NULL, field TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
  PRIMARY KEY (harness, session_id, contract_id)
);
```

- [ ] Amend A4 and related accepted-limit wording in the same source commit; preserve canonical provenance/continuity. Run focused daemon/store/evidence/doctor tests and commit. Preserve compatibility with old daemons through existing capability-gated notes and local parse diagnostics.

### Task 4: Integrated review, normative documentation and validation

**Files:** TRUST-POLICY.md, setup/launch/help and compatibility docs affected by removed operational probes, approved spec post-implementation notes, implementation evidence report; any fixes found by the scoped/final reviews.

**Interfaces:** Freeze exact final source APIs, migration24, affected files and commits for harness-adapters absorption before Task30. Preserve Herdr 0.9.3 lane ownership of host gates.

- [ ] Review each task against its brief and full diff; resolve Important/Critical findings and record rulings/deferred minors in the ledger.
- [ ] Review the Task3 A4 and accepted-limit amendments for separately qualified rich capabilities and metadata absence; do not weaken canonical provenance/continuity.
- [ ] Verify ordinary paths call no version/help/schema diagnostics. Retain canary metadata utilities and historical native captures without turning them into operational gates.
- [ ] Run `cargo fmt`, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `nice scripts/check-default-features`, and focused tests relevant to all changed paths. Serial build slot remains required.
- [ ] Dispatch broad independent code review. Fix its complete findings in one pass and scoped re-review. Commit reviewed artifacts.
- [ ] Deliver exact reviewed commits/footprint/API and test evidence to main and adapter owner. Main owns integrated sweep and merge; verify DONE+MERGED independently before cleanup-safe final completion. Stop every owned helper/test process; no shared server or foreign pane cleanup.

## Approved implementation file-map clarification (2026-10-05)

This records the actual bounded reviewed seams; it does not redesign behavior or replace original approval `4ce8f9fc`. Controller clarification baseline `e6b5a1a` preceded Task3.

Task3's actual health and retained diagnostic consumers include `src/daemon/health.rs`, `src/harness/codex.rs` (retired crate-private observer only), `tests/daemon/health_harness_state.rs`, `tests/daemon/health_optimistic.rs`, `tests/cli/doctor_json.rs`, `tests/cli/doctor_labels.rs`, `tests/harness/state.rs`, `tests/integration/doctor_admission_seam.rs`, `tests/integration/harness_version_evidence.rs`, `tests/service/admission_reobserve.rs` and doctor-only expectations in `tests/setup_cli.rs`. StorePort/SqliteStore changes are evidence-only wiring. Task3 original footprint is 29 files, cumulative R1 footprint 34 unique files.

The required doctor/canary I1 correction owns exactly six seams: `scripts/canary/admission.py`, `scripts/harness-canary.sh`, `scripts/canary/probe_schema.json` (description only), `scripts/canary/test_admission.py`, `scripts/canary/test_tier0_checks.py`, and `tests/integration/doctor_admission_seam.rs` (overlaps the original footprint). Live/saved consumers use core evaluation; explicit historical evaluation/generator/capture qualification remains separate. Cumulative scoped rereview cleared through `2d09b8de`.

Task4 worker ownership is documentation only: README, compatibility guide/transcript scope, spec post-implementation notes/INDEX, this narrow file-map clarification and implementation audit. It does not edit TRUST-POLICY, source, tests, CLI glue, migrations, installer or adapters, run production validation commands or integrate branches. Parent owns final source/API/hash freeze, lint/default/focused gates, broad review, handoff/archive and independently verified main merge. Existing task checkboxes remain historical plan checkpoints, not proof that these final gates have completed.
