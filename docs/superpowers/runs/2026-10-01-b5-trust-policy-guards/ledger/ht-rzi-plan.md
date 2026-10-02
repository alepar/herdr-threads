# ht-rzi plan: B5 cooperative trust policy guards (TRUST-POLICY.md)

Epic `ht-rzi`. Normative policy: `TRUST-POLICY.md` (repo root). Implementation decisions:
`docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-design.md`
(where they differ, the policy wins). Integration branch / worktree: `trust-model-invariants`.

## Mapping

| n | bead | task | filesTouched |
|---|---|---|---|
| 1 | ht-rzi.9 | Seam contract: Herdr pane-agent observation port + stand-in fake | src/ports.rs, src/host/native.rs, src/host/observation.rs, src/cli/mod.rs, tests/integration/sweep.rs |
| 2 | ht-rzi.1 | F6 repair path: hold lift, seat retire, rebind --replace (+ the single B5 migration) | migrations/0010_b5_trust_guards.sql, src/store/schema.rs, src/store/seats.rs, src/store/effective.rs, src/store/mod.rs, src/store/operator.rs, src/store/queries.rs, src/ports.rs, src/protocol/commands.rs, src/protocol/results.rs, src/identity/repair.rs, src/identity/reconcile.rs, src/service/dispatch.rs, src/service/workers.rs, src/cli/commands.rs, src/cli/mod.rs, src/cli/journal.rs, src/cli/retry.rs, src/cli/human.rs, tests/store/control.rs, tests/store/schema.rs, tests/store/operator.rs, tests/service/operator.rs, tests/service/worker_health.rs, tests/cli/commands.rs |
| 3 | ht-rzi.5 | Daemon-restart continuity: expected boot in requests, binding carry-forward (P35, O1, W5-2) | src/protocol/wire.rs, src/protocol/results.rs, src/client/local.rs, src/daemon/transport.rs, src/cli/exit.rs, src/cli/follow.rs, src/ports.rs, src/identity/reconcile.rs, src/store/seats.rs, src/store/schema.rs, tests/daemon/transport.rs, tests/contracts.rs, tests/store/receipts.rs, tests/identity/reconcile.rs, tests/service/composition.rs |
| 4 | ht-rzi.6 | Trust-edge hygiene: O_NOFOLLOW allocator.lock, wording, id comments | src/cli/journal.rs, tests/cli/journal.rs, src/harness/mod.rs, tests/cli/hook.rs, src/protocol/ids.rs, src/store/public_ids.rs |
| 5 | ht-rzi.3 | Agent-to-human binding guards for me init (Wave 18) | src/protocol/commands.rs, src/ports.rs, src/service/dispatch.rs, src/store/seats.rs, src/store/mod.rs, src/cli/me.rs, src/cli/mod.rs, src/cli/commands.rs, src/cli/journal.rs, src/cli/retry.rs, src/harness/bridge.rs, tests/store/cooperative_checkin.rs, tests/store/facade.rs, tests/contracts.rs, tests/cli/cooperative.rs, tests/integration/operator_ux.rs, tests/integration/sweep.rs, tests/hook_entrypoint.rs |
| 6 | ht-rzi.4 | Launch and wake harness guards (Wave 28, W9-2, W6-C3) | src/harness/launch.rs, src/cli/launch.rs, src/ports.rs, src/host/native.rs, src/store/wake.rs, src/notification/dispatch.rs, tests/harness/launch.rs, tests/cli/launch.rs, tests/store/wake.rs, tests/scheduler/dispatch.rs |
| 7 | ht-rzi.2 | Cooperative continuity: session-id reattachment of unresolved seats | src/store/seats.rs, src/store/schema.rs, src/store/mod.rs, src/store/queries.rs, src/ports.rs, src/protocol/authority.rs, src/protocol/commands.rs, src/protocol/results.rs, src/service/dispatch.rs, src/identity/repair.rs, src/cli/hook.rs, src/cli/journal.rs, src/cli/retry.rs, src/cli/human.rs, src/harness/bridge.rs, src/harness/claude.rs, src/harness/codex.rs, src/harness/context.rs, docs/compatibility/cooperative-continuity-resume.md, tests/store/cooperative_checkin.rs, tests/cli/hook.rs, tests/hook_entrypoint.rs |
| 8 | ht-rzi.7 | Docs: describe shipped B5 guards (README, operations, agent-usage, TRUST-POLICY status) | README.md, docs/operations.md, docs/agent-usage.md, TRUST-POLICY.md, integrations/skill/SKILL.md |
| 9 | ht-rzi.8 | Integration sweep: B5 trust policy guards end to end | tests/service.rs, tests/service/trust_policy.rs, tests/integration.rs, tests/integration/trust_policy.rs |

## Planner notes (read before dispatching)

- **Hot files.** `src/ports.rs` is written by tasks 1, 2, 3, 5, 6, 7 and `src/store/seats.rs` by 2, 3, 5, 7.
  Every edit there is additive (new trait methods with defaults, new enum variants, new functions); no task
  renames or moves existing items, so rebases stay mechanical. With `hotFileCap: 3` the coordinator will
  defer the fourth concurrent writer; that is expected.
- **Missing dependency edge (task 5 -> task 2).** Task 5 (ht-rzi.3) writes its `--operator` audit row into
  `allocation_decisions` with kind `operator_human_override`. Only the single B5 migration (task 2, ht-rzi.1)
  admits that kind, and ht-rzi.3 is not blocked by ht-rzi.1 in the bead graph. Task 5 orders that step last
  and states the precondition; if task 2 has not merged when task 5 reaches it, the implementer rebases onto
  the integration branch, or reports BLOCKED naming ht-rzi.1. Prefer dispatching task 5 after task 2 merges.
- **One migration.** Only task 2 adds a migration (`migrations/0010_b5_trust_guards.sql`, schema v10). It
  admits every new `allocation_decisions` kind B5 needs: `operator_retire` (task 2),
  `operator_human_override` (task 5) and `cooperative_continuity` (task 7), plus the nullable
  `continuity_diagnostic` column (task 7) and the reconciliation marker (task 2). No other task touches
  `migrations/` or the schema version. Note the B4 numbering risk (`ht-p03.2` also adds a migration) in task
  2's commit message.
- **New wire types are additive.** To avoid breaking dozens of struct literals, new commands are new
  `Command` variants (`OperatorRetire`, `OperatorReplace`, `OperatorCheckIn`, `ContinuityCheckIn`) rather
  than new fields on `OperatorRebind`/`CheckIn`. `WireRequest.expected_boot` (task 3) is the one new field
  on an existing struct; task 3 updates its literals in `tests/daemon/transport.rs` and `tests/contracts.rs`.
- **Agent environment in tests.** Tasks 5, 8 and 9 run `me init` through the built binary. When `cargo test`
  is itself run by an agent, `CLAUDECODE` / `CODEX_*` are set in the test process environment; every test
  harness that spawns `herdr-threads me init` must `env_remove` them (task 5 does this for the existing
  harnesses).

## Global constraints (every task)

Carried verbatim from the epic `ht-rzi` and `AGENTS.md`:

- Epic: "Implement the required guards decided in TRUST-POLICY.md (adopted 2026-10-01, branch
  trust-model-invariants). Resolves F6 and the B5 bucket of docs/history/remaining-findings-2026-10-01.md.
  P10 and W5-1 are not here: they are deletions folded into B4 (ht-p03.2). Each child names the policy
  invariant it implements; a change that weakens an invariant must update TRUST-POLICY.md in the same
  commit."
- AGENTS.md: "TRUST-POLICY.md is normative for seat continuity, caller attribution, receipt provenance and
  operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
  `src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
  `src/cli/hook.rs`, launch or wake."
  - "Do not add adversarial caller verification; the model is cooperative and same-user."
  - "Record claims honestly: a new way to attribute an action needs a provenance value defined in the
    policy."
  - "Never merge seats, never move a seat or end a binding on heuristic evidence."
  - "Decide in the daemon against the canonical view (A2); client-local files are hints, never authority."
  - "A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same
    commit."
- Design non-goals: C5 / P10 and W5-1 are owned by B4 (`ht-p03.2`), not touched here; no adversarial
  verification; no new harness beyond Claude and Codex; deterministic stand-in tests are the bar (native
  runs only for ht-rzi.2's evidence capture).
- Gates for every task: `cargo fmt --all -- --check` and
  `cargo check --locked --all-targets --all-features` must pass; run the task-relevant tests named in the
  section with `cargo test --locked --all-features <filter> -- --test-threads=1`. Several suites are
  macOS-only (`#[cfg(target_os = "macos")]`); run on the host platform.
- Test layout: unit tests live in `tests/<area>/<file>.rs` files pulled into `src/` modules with
  `#[cfg(test)] #[path = "../../tests/..."] mod tests;` (e.g. `tests/store/control.rs` from
  `src/store/control.rs`, `tests/cli/hook.rs` from `src/cli/hook.rs`). `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs` and `tests/hook_entrypoint.rs` (needs
  `--features test-support`) are integration crates.

---

## Task 1

**Bead:** ht-rzi.9 — Seam contract: Herdr pane-agent observation port + stand-in fake.

**filesTouched:** src/ports.rs, src/host/native.rs, src/host/observation.rs, src/cli/mod.rs,
tests/integration/sweep.rs

**Bead description (verbatim).** Compilable boundary for reading Herdr's per-pane agent state, consumed by
ht-rzi.2 (diagnostics only), ht-rzi.3 and ht-rzi.4. Deliver: one host-port method (src/ports.rs host trait)
returning, for a target, Result<Option<PaneAgentObservation { kind: Option<String> (Herdr's
detection-based agent kind, e.g. claude/codex), agent_session: Option<String> (Herdr integration report;
diagnostic value only) }>> — a read error is distinct from absent; the production implementation over Herdr
'agent get'/pane records in src/host/native.rs (src/host/observation.rs:127-138 already parses
agent_session); the deterministic stand-in Herdr support so tests can script kind/agent_session per pane
(present, absent, mismatched, read error); and a CLI-side single-read helper usable by me init. Herdr 0.9.1
exposes no session_start_source on any read route, so the port has no such field. Inert by default: no
caller changes behaviour in this bead.

**Acceptance criteria (verbatim).** Compiles; suite green; unit tests show the native impl maps agent get
output (with and without agent_session, and a read error) and the fake scripts
present/absent/mismatched/error values.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Contract consumers rely on (do not deviate):**
- `crate::ports::PaneAgentObservation { pub kind: Option<String>, pub agent_session: Option<String> }`,
  `#[derive(Debug, Clone, PartialEq, Eq)]`.
- `HostPort::observe_pane_agent(&self, target: &HostTargetId, context: &HostCallContext)
  -> Result<Option<PaneAgentObservation>, ApiError>`, default body `Ok(None)` (adapters without the route,
  including every existing test fake, report "no agent").
  - `Ok(None)`: Herdr answered and reports no agent in the pane (`agent_not_found`).
  - `Ok(Some(obs))`: Herdr's agent record for exactly that pane.
  - `Err(_)`: the read failed (transport, timeout, any other Herdr error, malformed record).
- CLI helper `pub(crate) fn pane_agent(context: &RuntimeContext, clock: &Arc<dyn Clock>, pane: &HostTargetId)
  -> Result<Option<PaneAgentObservation>, ApiError>` in `src/cli/mod.rs`: one `observe_pane_agent` call on
  a fresh `NativeCli` over `context.host_endpoint` with a 2 s budget and no expected boot/epoch.
- Stand-in Herdr (`tests/integration/sweep.rs` `FakeHost`) answers `agent.get` per pane from the pane JSON
  it was started with (see step 5). No existing pane JSON changes meaning.

**Steps (TDD):**

1. In `src/host/observation.rs`, write failing unit tests first (add a `#[cfg(test)] mod pane_agent_tests`
   at the end of the file) for a new parser `normalize_pane_agent(raw: &str, target: &str)
   -> Result<Option<PaneAgentObservation>, ApiError>`:
   - `agent_info_with_session_maps_kind_and_session`: raw
     `{"id":"x","result":{"type":"agent_info","agent":{"agent":"claude","agent_status":"idle","pane_id":"w4:p1","terminal_id":"term_1","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"sess-1"}}}}`
     -> `Some(PaneAgentObservation { kind: Some("claude"), agent_session: Some("sess-1") })`.
   - `agent_info_without_session_has_none_session`: same without `agent_session` (and with
     `"agent_session":null`) -> `agent_session: None`.
   - `agent_info_without_kind_has_none_kind`: no `agent` string -> `kind: None`.
   - `agent_info_for_another_pane_is_an_error`: `pane_id` `w4:p2` for target `w4:p1` -> `Err`.
   - `wrong_result_type_is_an_error`: `"type":"pane_info"` -> `Err`.
2. Implement in `src/host/observation.rs` (reuse `envelope`, `field`, `invalid`):
   ```rust
   /// Herdr `agent.get` for one pane. Detection-based kind and the
   /// integration's agent session are diagnostic/best-effort evidence only.
   pub fn normalize_pane_agent(
       raw: &str,
       target: &str,
   ) -> Result<Option<crate::ports::PaneAgentObservation>, ApiError> {
       let result = envelope(raw, "agent_info")?;
       let agent = result.get("agent").ok_or_else(|| invalid("missing agent record"))?;
       if agent.get("pane_id").and_then(Value::as_str) != Some(target) {
           return Err(invalid("agent record names another pane"));
       }
       let kind = agent
           .get("agent")
           .and_then(Value::as_str)
           .filter(|kind| !kind.is_empty())
           .map(str::to_owned);
       let agent_session = match agent.get("agent_session") {
           None | Some(Value::Null) => None,
           Some(session) => Some(field(session, "value")?.to_owned()),
       };
       Ok(Some(crate::ports::PaneAgentObservation { kind, agent_session }))
   }
   ```
   (`envelope` already turns a structured Herdr error into `Err`; the `agent_not_found` -> `Ok(None)`
   mapping happens in the adapter, step 4, because the transport surfaces structured errors as `Err` before
   the body reaches the parser.)
3. In `src/ports.rs` add the struct next to `SafeWakeTarget` (doc comment: TRUST-POLICY C1/A4 — Herdr's agent
   field may only suggest; never moves a seat, allocates one or ends a binding) and the `HostPort` method
   with the default `Ok(None)` body after `launch_native`. Export nothing else.
4. In `src/host/native.rs` override it in `impl HostPort for NativeCli`:
   ```rust
   fn observe_pane_agent(
       &self,
       target: &HostTargetId,
       context: &HostCallContext,
   ) -> Result<Option<ports::PaneAgentObservation>, ApiError> {
       self.check_context(context)?;
       let started = Instant::now();
       let limit = Duration::from_millis(750);
       match self.run(&["agent", "get", target.as_str()], &context.budget, limit) {
           Err(failure) if failure.code == ErrorCode::NotFound => Ok(None),
           Err(failure) => Err(failure),
           Ok(raw) => {
               let parsed = crate::host::observation::normalize_pane_agent(&raw, target.as_str())?;
               self.check_after_parse(&context.budget, started, limit)?;
               Ok(parsed)
           }
       }
   }
   ```
   (`check_context` at ~1012 only checks the epoch when `expected_boot` is Some, so the CLI's
   `expected_boot: None` context passes.) Add native unit tests in the existing `mod tests` of
   `src/host/native.rs` using the `fixture(...)` helper (one ping + one operation exchange):
   `pane_agent_maps_agent_get_with_session`, `pane_agent_maps_agent_get_without_session`,
   `pane_agent_not_found_is_absent` (answer
   `{"id":..,"error":{"code":"agent_not_found","message":"no agent"}}` -> `Ok(None)`),
   `pane_agent_other_error_is_read_error` (error code `permission_denied` -> `Err`, code `Unauthorized`).
   Each asserts the request was `agent.get` with params `{"target":"w4:p1"}`.
5. Stand-in Herdr: extend `FakeHost::start` in `tests/integration/sweep.rs` so `agent.get` is answered from
   the pane list instead of always `agent_not_found`:
   - find the pane whose `pane_id` equals `params.target`; none -> `agent_not_found`;
   - pane has `"agent_get_error": "<code>"` -> `{"id":..,"error":{"code":"<code>","message":"scripted read error"}}`;
   - pane has an `"agent"` string -> `{"id":..,"result":{"type":"agent_info","agent":{"agent":<agent>,
     "agent_status":<pane agent_status>,"pane_id":..,"terminal_id":..,"agent_session":<pane agent_session or absent>}}}`;
   - otherwise `agent_not_found`.
   Keep every other method's answer unchanged (plain shell panes still have no agent, so wake/launch tests
   behave as before). Add `pub(crate) fn agent_pane(id, terminal, kind, session: Option<&str>) -> Value`
   next to `pane(...)` building `{"agent":kind, "agent_session":{"agent":kind,"kind":"id","source":"herdr:<kind>","value":session}}`
   on top of `pane(id, terminal)`.
6. Add `#[test] fn stand_in_herdr_scripts_pane_agent_present_absent_mismatched_and_error()` in
   `tests/integration/sweep.rs`: start `FakeHost` with four panes (claude with session `s-1`, plain shell,
   codex with session `other`, one with `"agent_get_error":"permission_denied"`), call
   `herdr_threads::host::native::NativeCli::new(socket, clock).observe_pane_agent(...)` for each, and assert
   present (`kind claude`, `session s-1`), absent (`Ok(None)`), mismatched (session `other` != `s-1`), error
   (`Err`, code `Unauthorized`). Use a scratch dir under `/private/tmp` like the other sweep tests (socket
   path length).
7. CLI helper: add `pane_agent` (contract above) to `src/cli/mod.rs` near `run_in_pane`, with a doc comment
   "single best-effort read for `me init`; never authority". It is unused in this bead: mark
   `#[allow(dead_code)] // consumed by ht-rzi.3` only if the build warns.
8. Run: `cargo test --locked --all-features --lib host:: -- --test-threads=1`,
   `cargo test --locked --all-features --test integration stand_in_herdr -- --test-threads=1`, then the
   gates. Commit: "Add Herdr pane-agent observation port and stand-in (ht-rzi.9)".

**Deliverable / tests:** `PaneAgentObservation` + `HostPort::observe_pane_agent` (default inert), NativeCli
impl over `agent.get`, `FakeHost` agent scripting, `cli::pane_agent` helper. Tests:
`host::observation::pane_agent_tests::*`, `host::native::tests::pane_agent_*`,
`integration::sweep::stand_in_herdr_scripts_pane_agent_present_absent_mismatched_and_error`.

---

## Task 2

**Bead:** ht-rzi.1 — F6 repair path: hold lift, seat retire, rebind --replace. Implements TRUST-POLICY C2/C3.
Owns the single B5 migration.

**filesTouched:** migrations/0010_b5_trust_guards.sql, src/store/schema.rs, src/store/seats.rs,
src/store/effective.rs, src/store/mod.rs, src/store/operator.rs, src/store/queries.rs, src/ports.rs,
src/protocol/commands.rs, src/protocol/results.rs, src/identity/repair.rs, src/identity/reconcile.rs,
src/service/dispatch.rs, src/service/workers.rs, src/cli/commands.rs, src/cli/mod.rs, src/cli/journal.rs,
src/cli/retry.rs, src/cli/human.rs, tests/store/control.rs, tests/store/schema.rs, tests/store/operator.rs,
tests/service/operator.rs, tests/service/worker_health.rs, tests/cli/commands.rs

**Bead description (verbatim).** TRUST-POLICY C2/C3. Spec: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-design.md decisions 1-2. Roast: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-design-1.md (+ -step-back.md).
(1) Hold lift (decision 1, cluster hold-lift-single-helper): one store helper lift_baseline_hold_if_clear(tx, instance). Predicate: no unresolved nonretired seat AND reconciliation of the current recovery boot/epoch baseline has finished (no saved seat still pending reconciliation). When it holds, in the same transaction: clear host_instances.baseline_hold_unclaimed AND set released_at on every open recovery_holds row of the instance (recovery_holds is read first by effective.rs:144-149 and directly by seats.rs:2789, :3455, wake.rs:50, queries.rs:689; today only seats.rs:3305 releases one target). Call it from every transaction that can lower the unresolved count or finish reconciliation: rebind, fresh seat, retire, replace, reconciliation retirement, structural reconfirm (Reconfirm/ReconfirmStructure), each applied reconciliation page, and once at daemon start for stores already stuck. ht-rzi.2 calls it from cooperative continuity.
(2) New operator command 'seat retire SEAT --operator' over the existing bounded retirement cutover (begin_retirement; today retirement only comes from reconciliation of an absent target).
(3) 'seat rebind OLD --pane P --replace NEW --operator': retire NEW and rebind OLD onto P in one deciding transaction; nothing moves NEW->OLD; NEW's pending obligations settle as recipient-retired.
(4) The rebind refusal for an owned target carries both resolutions as argv.
owns: lift_baseline_hold_if_clear helper and its call sites (except cooperative continuity), seat retire --operator, seat rebind --replace --operator, collision refusal argv. consumes: existing begin_retirement cutover.
files: src/store/seats.rs, src/store/effective.rs, src/store/schema.rs, src/store/mod.rs or daemon startup (daemon-start call), src/protocol/commands.rs, src/protocol/results.rs, src/cli/commands.rs. Owns the single B5 migration (see below). User-facing docs are owned by ht-rzi.7.
Reconciliation marker (roast-design-2 punch list): 'reconciliation finished' is persisted as host_instances.reconciled_boot/reconciled_epoch, written in the transaction applying the last page of a full pass for the current recovery boot/epoch with no refused (Stale) transition (src/service/workers.rs ~1283-1300 page loop, src/identity/reconcile.rs); the lift predicate requires it to equal recovery_boot/recovery_epoch; daemon start lifts only when it already matches.
The single B5 migration (owned here; B4 numbering risk noted in the commit message): add host_instances.reconciled_boot/reconciled_epoch; rebuild STRICT allocation_decisions to admit kind 'cooperative_continuity' and add nullable continuity_diagnostic TEXT (consumed by ht-rzi.2). files: + migrations/.

**Acceptance criteria (verbatim).** Tests: a held pane with an ExplicitHold recovery_holds row resolves via
ordinary seat resolve after the last unresolved seat is RETIRED (not rebound); the lift does not fire while
seats of the current baseline are still resolved-pending-reconciliation; lift fires in the same transaction
for each of rebind, --new-seat, retire, --replace, reconciliation retirement, structural reconfirm of the
last unresolved seat (and not before); a restore where every saved seat is structurally reconfirmed leaves
no hold; a store with the flag set / open recovery_holds and zero unresolved seats is cleared at daemon
start. retire --operator retires and settles obligations; rebind --replace is atomic (no window where P is
unowned, concurrent resolve loses); after --replace NEW's pending obligations are recipient-retired and OLD
gains none of NEW's threads, invitations or obligations; refusal error lists both argv; retire/--replace
audited operator:local-user:<uid>, never on receipts; non-operator calls get operator_required. Marker
written only by a refusal-free full pass for the current recovery boot/epoch; lift never fires while it
lags; migration applies cleanly to a populated pre-B5 store and allocation_decisions accepts kind
cooperative_continuity.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Design decisions made here (planner):**
- The migration admits kinds `operator_retire`, `operator_human_override` (ht-rzi.3's `me init --operator`
  audit) and `cooperative_continuity`, so B5 needs exactly one migration.
- `seat retire` and `seat rebind --replace` are new typed administrative actions: new `Command` /
  `OperatorCommand` / `OperatorRequest` variants, not new fields on `OperatorRebind` (keeps every existing
  `OperatorRebind { .. }` literal and the persisted operator digest tuples unchanged).
- Retire is not a target claim, so it takes no host observation (like orphan invite). Replace claims `P`,
  so it carries an `OperatorTargetGuard` exactly like rebind.
- Audit rows: retire writes one `allocation_decisions` row (kind `operator_retire`, `operator_label =
  actor.audit_label()`); replace writes two in its one transaction: NEW `operator_retire`, OLD
  `operator_rebind`, both labelled. Nothing is written on receipts.
- "operator_required" = the CLI's existing refusal for a missing `--operator` flag
  (`invalid("operator required")`, exit 2), as `seat rebind` does today; the daemon additionally refuses a
  non-owner peer with `Unauthorized` (existing operator arm).
- The reconciliation marker is written by a dedicated small transaction issued right after the last page of
  a published pass whose pages had zero refused transitions; it re-verifies the publication is still active
  and still the current recovery boot/epoch, writes the marker and calls the lift helper in that same
  transaction. (Transitions are separate guarded transactions, so "the transaction applying the last page"
  is realised as this end-of-pass transaction.)

**Steps (TDD):**

A. Migration (schema v10).
1. Failing tests in `tests/store/schema.rs` (look at the existing v8->v9 tests and copy their fixture style):
   `v9_store_with_rows_migrates_to_v10_preserving_allocation_history` (build a v9 store via
   `initialize`, insert two `allocation_decisions` rows and a `host_instances` row, set `user_version` back
   if needed per existing pattern, reopen -> rows keep ordinals/values, `reconciled_boot`/`reconciled_epoch`
   are NULL, `continuity_diagnostic` NULL) and `v10_allocation_decisions_accepts_b5_kinds` (insert kinds
   `operator_retire`, `operator_human_override`, `cooperative_continuity` succeed; kind `bogus` fails;
   `continuity_diagnostic` accepts `match|mismatch|absent|read_error` and rejects `maybe`).
2. Create `migrations/0010_b5_trust_guards.sql`:
   ```sql
   -- B5 trust policy guards (TRUST-POLICY C1-C3). One migration for the epic.
   -- Reconciliation marker: the recovery boot/epoch whose saved-seat pass
   -- finished with no refused transition (decision 1 of the B5 design).
   ALTER TABLE host_instances ADD COLUMN reconciled_boot TEXT;
   ALTER TABLE host_instances ADD COLUMN reconciled_epoch INTEGER CHECK(reconciled_epoch IS NULL OR reconciled_epoch >= 0);
   -- SQLite cannot alter a CHECK: rebuild allocation_decisions with identical
   -- rows, ordinals and indexes; the kind set grows and a nullable diagnostic
   -- column records the cooperative-continuity Herdr comparison. No trigger,
   -- view or foreign key refers to allocation_decisions.
   CREATE TABLE allocation_decisions_v10 (
       ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
       instance_id TEXT NOT NULL REFERENCES host_instances(id),
       target_id TEXT NOT NULL,
       seat_id TEXT NOT NULL,
       kind TEXT NOT NULL CHECK(kind IN ('ordinary', 'operator_fresh', 'operator_rebind', 'operator_retire', 'operator_human_override', 'cooperative_continuity')),
       decided_at INTEGER NOT NULL,
       host_boot TEXT NOT NULL,
       epoch INTEGER NOT NULL CHECK(epoch >= 0),
       generation INTEGER NOT NULL CHECK(generation >= 0),
       operator_label TEXT,
       continuity_diagnostic TEXT CHECK(continuity_diagnostic IS NULL OR continuity_diagnostic IN ('match', 'mismatch', 'absent', 'read_error'))
   ) STRICT;
   INSERT INTO allocation_decisions_v10 (ordinal, instance_id, target_id, seat_id, kind, decided_at, host_boot, epoch, generation, operator_label)
   SELECT ordinal, instance_id, target_id, seat_id, kind, decided_at, host_boot, epoch, generation, operator_label FROM allocation_decisions;
   DROP TABLE allocation_decisions;
   ALTER TABLE allocation_decisions_v10 RENAME TO allocation_decisions;
   CREATE INDEX allocation_decisions_target ON allocation_decisions(instance_id, target_id, ordinal);
   CREATE INDEX allocation_decisions_seat_history ON allocation_decisions(seat_id, ordinal);
   ```
3. `src/store/schema.rs`: `const V10: &str = include_str!("../../migrations/0010_b5_trust_guards.sql");`
   with a doc line; fresh-store arm (`0 =>`) runs V10 and sets `user_version` 10; every `1..=8` arm appends
   `migrate_v9_to_v10(conn)?`; arm `9 =>` becomes `verify_existing(conn)?` of the v9 shape (rename the
   current `verify_existing` body into `verify_existing_v9_shape` mirroring `verify_existing_v8_shape`) then
   `migrate_v9_to_v10` then `verify_existing`; add arm `10 => verify_existing(conn)`; add
   `migrate_v9_to_v10` (copy of `migrate_v8_to_v9` with V10 / 10) and `verify_existing_v10` (normalized
   `sqlite_master.sql` of `allocation_decisions` contains `'cooperative_continuity'` and
   `continuity_diagnostic`, and `pragma_table_info('host_instances')` has `reconciled_boot` and
   `reconciled_epoch`), called last from `verify_existing`. Fix any check in `verify_existing_v1` that
   pins the old allocation_decisions SQL (grep `allocation_decisions` in schema.rs). Also grep
   `verify_query_connection_version` / `user_version` expectations elsewhere (e.g. `src/store/connection.rs`
   reader version checks) and bump 9 -> 10 where it pins the current version.
4. Commit message for the migration commit must contain: "B4 numbering risk: ht-p03.2 (branch
   remaining-herdr-threads-findings) also adds a migration; whichever lands second renumbers."

B. The lift helper.
5. Failing store tests in `tests/store/control.rs` (reuse `fixture(100)`, `snapshot_for_test`,
   `staged_test_snapshot`, the operator fresh-seat pattern of
   `first_published_baseline_hold_survives_later_snapshots_and_operator_release`):
   - `lift_requires_reconciliation_marker_for_current_recovery_boot_epoch`: publish a baseline with one
     unresolved seat, then mark it retired directly; call `seats::lift_baseline_hold_if_clear` inside a
     transaction -> returns `false` and flag/holds remain while `reconciled_*` is NULL or names another
     epoch; set `reconciled_boot/epoch` = `recovery_boot/epoch` -> returns `true`, `baseline_hold_unclaimed=0`,
     every `recovery_holds` row of the instance has `released_at` set, other instances untouched.
   - `lift_does_not_fire_while_an_unresolved_seat_remains`.
6. Implement in `src/store/seats.rs` (near `mark_seat_unresolved`):
   ```rust
   /// TRUST-POLICY C2: instance-wide restore-hold release when nothing is left
   /// to protect. Predicate: reconciliation of the current recovery boot/epoch
   /// finished (persisted marker) and no unresolved nonretired seat remains.
   /// Clears `baseline_hold_unclaimed` and releases every open `recovery_holds`
   /// row of the instance in the caller's transaction. Returns whether it lifted.
   pub(crate) fn lift_baseline_hold_if_clear(
       tx: &Transaction<'_>,
       instance: &str,
       at: UtcMillis,
   ) -> Result<bool, ApiError> {
       type Marker = Option<(Option<String>, Option<i64>, Option<String>, Option<i64>, i64)>;
       let row: Marker = tx.query_row(
           "SELECT recovery_boot,recovery_epoch,reconciled_boot,reconciled_epoch,baseline_hold_unclaimed FROM host_instances WHERE id=?1",
           [instance], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
       ).optional().map_err(store_error)?;
       let Some((recovery_boot, recovery_epoch, reconciled_boot, reconciled_epoch, flag)) = row else {
           return Ok(false);
       };
       if recovery_boot.is_none() || recovery_boot != reconciled_boot || recovery_epoch != reconciled_epoch {
           return Ok(false);
       }
       let unresolved: bool = tx.query_row(
           "SELECT EXISTS(SELECT 1 FROM seats WHERE instance_id=?1 AND state='unresolved')",
           [instance], |r| r.get(0),
       ).map_err(store_error)?;
       let open_holds: bool = tx.query_row(
           "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND released_at IS NULL)",
           [instance], |r| r.get(0),
       ).map_err(store_error)?;
       if unresolved || (flag == 0 && !open_holds) {
           return Ok(false);
       }
       tx.execute("UPDATE host_instances SET baseline_hold_unclaimed=0 WHERE id=?1", [instance]).map_err(store_error)?;
       tx.execute("UPDATE recovery_holds SET released_at=?1 WHERE instance_id=?2 AND released_at IS NULL",
           params![at.0, instance]).map_err(store_error)?;
       schema::bump_lifecycle_revision(tx, instance)?;
       schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
       Ok(true)
   }
   ```
   (Check `apply_eligibility_transition`'s contract before calling it; if a lift is not an eligibility
   change drop that line. `effective.rs` needs no change: with the flag cleared a baseline member is
   `UnambiguousUnclaimed`, and released holds are no longer `ExplicitHold`.)
7. Call sites (each in the deciding transaction, after the state change, using the decision's `at.utc`):
   - `mutate_operator` (rebind and fresh seat) after the existing per-target `recovery_holds` release.
   - the new retire and replace decisions (steps D/E).
   - `apply_reconciliation_transition`: the `Reconfirm | ReconfirmStructure` arm and the
     `BeginRetirement` arm (after `begin_retirement_fence`); also `MarkUnresolved`-noop does not call.
   - the new end-of-pass marker transaction (step C).
   - daemon start: in `SqliteStore::new` (`src/store/mod.rs`) after `open_writer`, run one IMMEDIATE
     transaction that calls `lift_baseline_hold_if_clear(&tx, &instance, clock.utc_now())` and commits
     (lifts only when the marker already matches; no-op on a fresh store without the instance row).
   - ht-rzi.2 adds the cooperative-continuity call; do not add it here.
8. Tests (in `tests/store/control.rs`): `operator_rebind_of_last_unresolved_seat_lifts_holds_in_same_transaction`,
   `operator_fresh_seat_lifts_when_no_unresolved_remains`, `reconfirm_structure_of_last_unresolved_seat_lifts_and_not_before`
   (two unresolved seats; first reconfirm leaves the flag, second clears it — both after the marker matches),
   `reconciliation_retirement_of_last_unresolved_seat_lifts`,
   `daemon_start_clears_stuck_flag_and_open_holds_when_marker_matches` (seed a store: flag=1, open holds,
   zero unresolved, marker == recovery; construct `SqliteStore::new` -> cleared; same seed with lagging
   marker -> untouched), and the AC scenario
   `explicit_hold_pane_resolves_after_last_unresolved_seat_is_retired` (ExplicitHold row on `pane-h`;
   retire the last unresolved seat via the new store retire; `effective_recovery_disposition(.., "pane-h")`
   is no longer `ExplicitHold`/`BaselineHeld` and ordinary `seats::resolve_seat` allocates there).
   Also `restore_with_every_seat_structurally_reconfirmed_leaves_no_hold` in `tests/identity/reconcile.rs`
   is owned by ht-rzi.8; here assert the store-level equivalent: after reconfirming every unresolved seat
   and recording the pass, flag 0 and no open holds.

C. Reconciliation marker.
9. Failing tests: in `tests/store/control.rs`
   `record_reconciliation_pass_writes_marker_only_for_current_recovery_publication` (stale/other
   publication -> no write; current -> `reconciled_boot/epoch == recovery_boot/epoch` and lift runs), and in
   `tests/service/worker_health.rs` `pass_with_refused_transition_leaves_marker_behind` /
   `refusal_free_pass_records_marker_once` driving `spawn_observation_loop` (or the extracted helper, see 11)
   with the file's existing fake `ObservationStore`, counting `record_reconciliation_pass` calls.
10. Store: `pub fn record_reconciliation_pass(context, conn, published: &PublishedSnapshot, budget) ->
    Result<bool, ApiError>` in `src/store/seats.rs`: one budgeted decision; `active_publication(tx,
    &published.id)` must equal `*published` and `host_instances.recovery_boot/recovery_epoch` must equal
    `published.boot/epoch`, else `Ok(false)`; then `UPDATE host_instances SET reconciled_boot=?1,
    reconciled_epoch=?2 WHERE id=?3`, then `lift_baseline_hold_if_clear`, `Ok(true)`. Port it:
    `StorePort::record_reconciliation_pass(&self, published: &PublishedSnapshot, budget) -> Result<bool,
    ApiError>` with default `Ok(false)` (`src/ports.rs`); `SqliteStore` impl delegates (`src/store/mod.rs`);
    `observation_store::ObservationStore` (in `src/identity/reconcile.rs`) gains the same method with a
    default `Ok(false)`, the blanket `impl<T: StorePort + ?Sized>` delegates, and `ScheduledStore`
    (`src/service/workers.rs` ~1121) delegates under `writer.enter_background`.
11. Worker: in `spawn_observation_loop` (`src/service/workers.rs` ~1283) carry a `refused: bool` in the
    continuation tuple (`(outcome, after, high, refused)`), OR-ing `page.transitions_refused > 0`; when a
    `Published` pass ends (`next_after_ordinal == None`) and `!refused`, call
    `port.record_reconciliation_pass(published, &budget)` before `host_evidence.record_reconciled`; ignore
    an `Err` (log-free; the next pass retries). Extract the end-of-page decision into a small pure fn
    (`fn pass_complete(refused_so_far: bool, page: &ReconcilePageProgress) -> (bool /*done*/, bool /*refused*/)`)
    so worker_health tests can pin it without timing.

D. `seat retire SEAT --operator`.
12. Failing tests: `tests/cli/commands.rs` `seat_retire_requires_operator_flag` (parse `seat retire s1`
    -> error text "operator required"; `seat retire s1 --operator` -> `MutationSpec::Retire(seat)`);
    `tests/store/control.rs` `operator_retire_retires_settles_and_audits` (seat with a pending invitation and
    pending receipt; store `mutate_operator(OperatorRequest::Retire(..))` -> seat `retired`, binding ended,
    a `retirements` job exists; advance it to completion with `control::advance_retirement` -> invitation
    and receipt `recipient_retired`; one `allocation_decisions` row kind `operator_retire` with
    `operator_label = 'operator:local-user:501'`; no receipts/messages carry the label; replay with the same
    operation key returns the same result; retiring an already-retired seat is `NotFound`/`Conflict`
    without a new row); `tests/service/operator.rs`
    `elected_operator_retire_over_ipc_is_audited_and_peer_checked` (mismatched peer -> `Unauthorized`).
13. Protocol: `OperatorRetire { seat: SeatId, operation: OperationId }` (`deny_unknown_fields`) in
    `src/protocol/commands.rs`; `Command::OperatorRetire`, `OperatorCommand::Retire` (+ `TryFrom` arm),
    add to `Command::validate`/any exhaustive match the compiler flags. Results:
    `CommandResult::OperatorRetired(SeatId)`, `IntentKind::OperatorRetire` (`src/protocol/results.rs`).
14. Plumbing: `OperatorRequest::Retire(OperatorRetire)` (no guard) and `OperatorTargetGuard::try_new` returns
    `Err("retire has no target guard")` for it (`src/ports.rs`); `consume_for_decision` -> `Ok(())`;
    `src/identity/repair.rs::operator` handles `OperatorCommand::Retire` like `OrphanInvite` (direct store
    call, no observation); `src/service/dispatch.rs` adds `Command::OperatorRetire(_)` to the operator arm;
    `src/store/operator.rs`: `operation`, `digest` (`("operator_retire", instance, &c.seat)`), the wire
    mapping and `validate_result` (`(Retire(c), OperatorRetired(seat)) => seat == &c.seat`);
    `src/store/queries.rs` ~334 add `OperatorRetired(v)` to the seat-yielding results.
15. Store decision in `mutate_operator` (`src/store/seats.rs`): early branch for `OperatorRequest::Retire`
    running `schema::execute_idempotent_transaction` under `actor.operation_scope(instance)`; precheck: seat
    exists in `elected_instance` and is not retired (`state!='retired'`), else `NotFound` "seat is not a
    live seat of this instance"; decision: read `host_instances.host_boot/host_epoch` and the seat's
    `target_id` (else latest `occupant_bindings.target_id`, else `''`) and `target_generation`; call
    `control::begin_retirement_fence(tx, at.utc, seat, instance, boot.unwrap_or(""), epoch, target, gen)`;
    insert the `operator_retire` audit row (target as above, boot/epoch/generation as above,
    `operator_label = actor.audit_label()`); `lift_baseline_hold_if_clear`; result
    `CommandResult::OperatorRetired(seat)`. The existing retirement worker advances the job (bounded
    quanta) and settles obligations as recipient-retired.
16. CLI: `SeatSub::Retire { seat: String, #[arg(long)] operator: bool }` (refuse with
    `invalid("operator required")` when false, like `Rebind`); `MutationSpec::Retire(SeatId)` ->
    `WireCommand::OperatorRetire`; `operator_semantic` maps it to `SemanticMutation::OperatorRetire { seat }`
    (`src/cli/journal.rs`: variant, `is_operator`, `IntentKind::OperatorRetire`, `to_command`);
    `src/cli/retry.rs` ~327 pairs it with `CommandResult::OperatorRetired(_)`; `src/cli/human.rs` renders
    "Seat {seat} retired; its pending obligations settle as recipient-retired."; help text names the
    abandonment semantics.

E. `seat rebind OLD --pane P --replace NEW --operator`.
17. Failing tests: `tests/cli/commands.rs` `seat_rebind_replace_parses_to_operator_replace`;
    `tests/store/control.rs` `operator_replace_retires_new_and_rebinds_old_atomically`: OLD unresolved, NEW
    resolved on P with a joined thread, a pending invitation and a pending receipt, OLD has its own thread.
    After `mutate_operator(OperatorRequest::Replace(..))`: NEW `retired`, OLD `resolved` on P, P never
    observed unowned (assert within the same transaction outcome: a concurrent `seats::resolve_seat` on P
    issued after the call returns OLD / `TargetAlreadyOwned`, never a fresh seat); NEW's invitation/receipt
    `recipient_retired` after advancing the retirement job; OLD's memberships, invitations and receipts are
    unchanged (none of NEW's); two audit rows (`operator_retire` NEW, `operator_rebind` OLD) both labelled
    `operator:local-user:501`; replay returns `OperatorRebound(OLD)`; NEW not owning P -> refused
    `TargetAlreadyOwned`, nothing written. `tests/service/operator.rs`
    `elected_operator_replace_over_ipc` (end to end through `DomainService`, fake host fresh observation).
18. Protocol: `OperatorReplace { seat: SeatId, target: HostTargetId, replace: SeatId, operation: OperationId }`;
    `Command::OperatorReplace`, `OperatorCommand::Replace`; result reuses `CommandResult::OperatorRebound(OLD)`;
    `IntentKind::OperatorReplace`.
19. Plumbing as for rebind: `OperatorTargetGuard::try_new` takes `command.target` for `Replace`;
    `OperatorRequest::Replace(OperatorReplace, OperatorTargetGuard)` + `consume_for_decision`;
    `repair.rs::operator` observes `request.target` like rebind; dispatch operator arm; store/operator.rs
    `operation`, `digest` (`("operator_replace", instance, &c.target, &c.seat, &c.replace)`), wire mapping,
    `validate_result` (`(Replace(c), OperatorRebound(seat)) => seat == &c.seat`).
20. Store: extend `mutate_operator`'s match to `OperatorRequest::Replace` with kind `operator_rebind` for
    OLD; precheck: OLD unresolved (as rebind), NEW `resolved` with `target_id = P` in this instance and NEW
    != OLD (else `TargetAlreadyOwned` "replace target is not owned by NEW"); skip the generic
    `target_free` precheck for Replace (P is owned by NEW by design); in the decision closure, after the
    guard is consumed, call `control::begin_retirement_fence(tx, at.utc, NEW, instance, boot, epoch, P,
    expected_generation_sql)` and insert NEW's `operator_retire` audit row, then assert `target_free` (now
    true) and run the existing rebind body for OLD (same transaction, so P is never unowned and a
    concurrent resolve serializes behind the writer), then the lift helper.
21. CLI: `SeatSub::Rebind` gains `#[arg(long)] replace: Option<String>`; with `--replace NEW` it parses to
    `MutationSpec::Replace { seat, pane, replace }` -> `SemanticMutation::OperatorReplace { seat, target,
    replace }` -> `WireCommand::OperatorReplace`; retry/human as above ("Seat {old} rebound to {pane};
    seat {new} retired.").

F. Collision refusal argv (C3).
22. Failing test `tests/store/control.rs` `rebind_onto_owned_target_lists_both_resolutions`: OLD unresolved,
    NEW resolved on P; `mutate_operator(Rebind OLD -> P)` -> `TargetAlreadyOwned` whose `detail` contains
    `herdr-threads seat retire OLD --operator` and
    `herdr-threads seat rebind OLD --pane P --replace NEW --operator` (literal ids substituted).
23. Implement: in `mutate_operator`'s precheck for `Rebind`, when `!target_free`, look up the resolved owner
    of P (`SELECT id FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1`) and
    build the detail "target {P} is owned by live seat {NEW}; seats are never merged. Abandon the old
    seat: `herdr-threads seat retire {OLD} --operator`, or abandon the new role: `herdr-threads seat rebind
    {OLD} --pane {P} --replace {NEW} --operator`". Keep `restart_argv: None` (two choices). If the owner is
    not a live seat (observation mismatch), keep today's message. Update any existing test that pinned the
    old text.

G. Finish.
24. Run: `cargo test --locked --all-features --lib store:: -- --test-threads=1`,
    `cargo test --locked --all-features --lib service:: -- --test-threads=1`,
    `cargo test --locked --all-features --test service operator -- --test-threads=1`,
    the `tests/cli/commands.rs` crate filter for `seat_`, then the gates. Commit per section (A..F) or as
    one commit; the migration commit carries the B4 numbering note.

**Deliverable / tests:** schema v10; `lift_baseline_hold_if_clear` with every non-continuity call site and
the daemon-start call; persisted reconciliation marker; `seat retire --operator`; `seat rebind --replace
--operator`; collision refusal listing both argv. Tests named in steps 1, 5, 8, 9, 12, 17, 22.

---

## Task 3

**Bead:** ht-rzi.5 — Daemon-restart continuity: expected boot in requests, binding carry-forward (P35, O1,
W5-2). Implements TRUST-POLICY A2/C4.

**filesTouched:** src/protocol/wire.rs, src/protocol/results.rs, src/client/local.rs,
src/daemon/transport.rs, src/cli/exit.rs, src/cli/follow.rs, src/ports.rs, src/identity/reconcile.rs,
src/store/seats.rs, src/store/schema.rs, tests/daemon/transport.rs, tests/contracts.rs,
tests/store/receipts.rs, tests/identity/reconcile.rs, tests/service/composition.rs

**Bead description (verbatim).** TRUST-POLICY A2/C4. (1) P35: add optional expected_boot to the WireRequest
envelope (src/client/local.rs ~155, src/protocol/wire.rs); LocalSocketClient already holds
descriptor.boot_id; the daemon refuses a mismatch before dispatch with a definitive DaemonBootChanged error
(client maps it to a retryable definite rejection, not UnknownOutcome). (2) O1: when a structural reconfirm
(same terminal, Herdr boot and incarnation) is applied after a daemon restart, carry the open binding
(cooperative_top_level or operator_human) forward to the new host epoch by updating it in place, preserving
every other binding column (harness, session, execution, generation) (Reconfirm/ReconfirmStructure apply in
src/store/seats.rs ~1811-1840, ~2087-2096; effective_registered_availability src/store/schema.rs:1356), so
the first send no longer warns recipient_unavailable. (3) W5-2: store test for current binding + bumped
snapshot generation -> available_at NULL and exactly one prepared_unavailable_warnings row
(src/store/messages.rs ~421).
owns: WireRequest expected_boot field and pre-dispatch refusal error code, binding carry-forward on
structural reconfirm, W5-2 store test. files: src/protocol/wire.rs, src/protocol/results.rs (error code),
src/client/local.rs, src/daemon/ (dispatch), src/store/seats.rs (reconfirm apply), src/store/schema.rs,
tests/store/receipts.rs or tests/service/cooperative.rs.
LocalSocketClient fills expected_boot from descriptor.boot_id on every request. No schema migration
(wire-only field; carry-forward is an in-place update).

**Acceptance criteria (verbatim).** Tests: request with stale expected_boot refused pre-dispatch, nothing
applied; daemon stop/ensure then send to a joined, structurally reconfirmed seat starts its timer with no
warning; a Herdr-incarnation change still makes it unresolved; W5-2 test present. A CLI request through
LocalSocketClient carries expected_boot == descriptor.boot_id; a daemon restart between descriptor read and
send surfaces DaemonBootChanged as a definite rejection, not UnknownOutcome. A human-bound (operator_human)
joined seat also stays available after daemon restart + structural reconfirm. Carry-forward leaves every
existing binding column unchanged (the session column added by ht-rzi.2 is checked in the integration sweep
ht-rzi.8).

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Key finding for (2) (read before coding).** After a daemon restart with the same Herdr incarnation the
seat usually stays `resolved` and `plan_page` (`src/identity/reconcile.rs` ~323) emits **no transition** for
it (see `tests/service/composition.rs::actual_native_daemon_restart_keeps_same_terminal_seat_resolvable`);
the new daemon boot resumes a higher host epoch (`resume_after_epoch`), so
`effective_registered_availability` (binding `host_epoch` must equal `host_instances.host_epoch`) goes false
and the first send warns. The carry-forward must therefore cover both paths that structurally reconfirm a
seat in the new epoch: (a) the `Reconfirm | ReconfirmStructure` apply for a host-invalidated seat, and (b) a
resolved seat whose open registered binding lags the publication epoch on the same terminal, boot and
incarnation — a new guarded transition `ReconciliationAction::CarryForward { target, terminal }`.

**Steps (TDD):**

A. W5-2 (pure test, do first).
1. Add `#[test] fn current_binding_with_bumped_snapshot_generation_is_unavailable_and_warns_once()` to
   `tests/store/receipts.rs` next to `send_does_not_start_timer_from_stale_registered_binding`: seat `b`
   joined with a *current* registered binding (same boot/epoch as `host_instances`, `target_generation`
   equal to the seat's), then bump the effective target's structural generation (update the published
   `snapshot_targets`/`observed_targets` row for the seat's target, as the file's `setup()` seeds it — read
   it first) so `effective_observation(..).structural_generation != binding.target_generation`; send ->
   `effective_receipt(..).available_at == None` and `SELECT count(*) FROM prepared_unavailable_warnings
   WHERE ...affected seat b...` (or the published warnings, per the `unavailable_joined_recipient_gets_one_warning_for_its_episode`
   pattern) == 1. It should pass against current code (it pins W5-2); if it fails, that is a real bug —
   report it rather than changing `stage_recipient`.

B. Expected boot (P35, A2).
2. Failing tests in `tests/daemon/transport.rs`:
   - `stale_expected_boot_is_refused_before_dispatch_and_applies_nothing`: real socket owner (copy the
     setup of `wrong_instance_and_boot_stop_do_not_cancel_real_socket_owner`), send a mutating command with
     `expected_boot: Some(other_uuid)` -> response `Err(DaemonBootChanged)` carrying the daemon's real boot,
     handler never invoked (count handler calls / assert no store row).
   - `matching_or_absent_expected_boot_dispatches` (Some(real) and None both reach the handler).
   - `client_maps_daemon_boot_changed_to_definite_rejection`: fake listener answers with
     `daemon_boot = actual` and `result: Err(DaemonBootChanged)` to a client constructed with
     `Some(expected)`; `call_definitive` -> `Ok(Err(e))` with `e.code == DaemonBootChanged` (contrast
     `client_rejects_wrong_boot_after_submission_without_retry`, which stays `UnknownOutcome` for any other
     result).
   - `local_client_sends_expected_boot_from_descriptor`: fake listener decodes the request and asserts
     `request.expected_boot == Some(boot.to_string())`; with `None` boot the field is absent on the wire.
3. `src/protocol/results.rs`: add `ErrorCode::DaemonBootChanged` (doc: "The request named a daemon boot
   that is no longer running; nothing was dispatched. Re-read the descriptor and retry.").
   `src/cli/exit.rs` maps it to `EXIT_UNAVAILABLE`; `src/cli/follow.rs::classify` -> `Transient`. Leave
   `cli/retry.rs::is_deterministic_rejection` unchanged (not listed => intent kept => retryable).
4. `src/protocol/wire.rs`: `WireRequest` gains
   `#[serde(default, skip_serializing_if = "Option::is_none")] pub expected_boot: Option<String>`; the
   custom `Raw` gets `#[serde(default)] expected_boot: Option<String>`; validate it with `valid_uuid` when
   present (invalid -> deserialize error). Update every `WireRequest { .. }` literal
   (`tests/daemon/transport.rs`, `tests/contracts.rs`, `src/client/local.rs`) with `expected_boot: None`
   unless the test needs one. Add/adjust the wire contract test in `tests/contracts.rs` that pins the
   envelope fields.
5. `src/daemon/transport.rs` (~302, right after the `expected_instance` check and before any dispatch):
   if `request.expected_boot.as_deref().is_some_and(|boot| boot != daemon_boot)` write a `WireResponse`
   with `result: Err(api_error(ErrorCode::DaemonBootChanged, "request expected daemon boot X; this daemon
   is boot Y; nothing was applied"))` and return, mirroring the instance-mismatch block.
6. `src/client/local.rs::exchange_selected`: set `expected_boot: self.expected_boot.map(|b| b.to_string())`;
   after decoding the response, before the `correlates_to` check: if `response.result` is
   `Err(e)` with `e.code == DaemonBootChanged` and the response's `version`, `request_id` and `instance`
   correlate (call `response.correlates_to(&request, None)`), return `Ok(Err(e))` (definite rejection).
   Everything else unchanged.
7. Integration-style test (macOS, `tests/service/composition.rs`, reuse `ActualNativeFixture`):
   `request_with_previous_boot_after_restart_is_daemon_boot_changed` — build a `LocalSocketClient` from the
   pre-restart descriptor (old boot), `fixture.restart()`, call through it -> `DaemonBootChanged`, not
   `UnknownOutcome`, and nothing recorded.

C. Binding carry-forward (O1, C4).
8. Failing tests in `tests/identity/reconcile.rs` (store-backed; copy an existing published-page test that
   applies `ReconfirmStructure`):
   - `reconfirm_structure_after_restart_carries_open_binding_forward`: host-invalidated seat with an open
     registered `cooperative_top_level` binding (epoch 3); publication at epoch 4, same boot/incarnation/
     terminal -> after apply: seat resolved; the same binding row (same ordinal) has `host_epoch=4` and
     `target_generation` = the publication's structural generation; `harness`, `native_session`,
     `execution_id`, `generation`, `observation_provenance`, `registered_at`, `terminal_id`, `incarnation`
     unchanged; `effective_registered_availability(seat)` is `Some("cooperative_top_level")`.
   - `resolved_seat_with_lagging_binding_epoch_gets_carry_forward_transition`: resolved seat, open
     registered binding at epoch 3, publication epoch 4 same terminal/boot/incarnation -> `plan_page` emits
     `CarryForward`; applying it updates the binding in place as above.
   - `operator_human_binding_is_carried_forward_too`.
   - `incarnation_change_still_marks_unresolved_and_carries_nothing`.
   - `carry_forward_is_stale_when_binding_changed` (expected binding generation moved -> `Stale`).
9. `src/ports.rs`: add `ReconciliationAction::CarryForward { target: HostTargetId, terminal: TerminalId }`
   (doc: "C4: a resolved seat structurally reconfirmed in a newer host epoch of the same boot and
   incarnation keeps its open binding; the binding's host epoch and target generation move forward in place;
   nothing else changes"); `SnapshotSavedSeat` gains `pub bound_epoch: Option<u64>` (open binding's
   `host_epoch`), filled in `saved_seat_scalar` / saved-seat page queries in `src/store/seats.rs`; fix every
   `SnapshotSavedSeat { .. }` literal the compiler flags (tests in `tests/identity/reconcile.rs`).
10. `plan_page` (`src/identity/reconcile.rs`): in the final `else { None }` branch (resolved, same target,
    observed match, no positive absence), emit `CarryForward { target, terminal }` when
    `saved.bound_epoch.is_some_and(|e| e < page.publication.epoch)` and the seat has an open registered
    binding (`saved.active_binding_execution.is_some()`) on the same terminal; otherwise `None`.
11. `apply_reconciliation_transition` (`src/store/seats.rs`):
    - Validation phase: `CarryForward` validates like `ReconfirmStructure` (snapshot match of target and
      terminal in this publication, structural proof, same boot and incarnation as the binding, not owned by
      another seat) but requires `state='resolved'`.
    - Decision phase, new helper used by both arms:
      ```rust
      /// C4: carry the open binding forward to the publication's host epoch in
      /// place; harness, session, execution, generation and provenance stay.
      fn carry_binding_forward(tx, seat: &SeatId, generation: i64, boot: &str, epoch: i64, target: &str, target_generation: i64) -> Result<(), ApiError> {
          tx.execute("UPDATE occupant_bindings SET host_epoch=?1,target_generation=?2 WHERE seat_id=?3 AND generation=?4 AND ended_at IS NULL AND registered_at IS NOT NULL AND host_boot=?5 AND target_id=?6 AND observation_provenance IN ('cooperative_top_level','operator_human')",
              params![epoch, target_generation, seat.as_str(), generation, boot, target]).map_err(store_error)?;
          Ok(())
      }
      ```
      Call it in the `Reconfirm | ReconfirmStructure` arm after the seat update (generation is the
      unchanged seat generation; boot/epoch from `transition.publication`), and in the new `CarryForward`
      arm after `UPDATE seats SET target_generation=?1 WHERE id=?2 AND generation=?3 AND state='resolved'`
      (Stale if 0 rows), followed by `update_structural_proof`, `bump_lifecycle_revision`,
      `apply_eligibility_transition`. Close the open unavailability marker the same way
      `register_cooperative` does (`UPDATE seats SET unavailability_open=0`) only when the binding was
      carried (rows changed == 1).
    - Keep ht-rzi.1's lift-helper calls in the reconfirm arm if already merged (rebase cleanly).
12. Service test (macOS, `tests/service/composition.rs`):
    `daemon_restart_keeps_joined_cooperative_seat_available_without_warning` — extend the
    `actual_native_daemon_restart_keeps_same_terminal_seat_resolvable` flow: register a cooperative
    lifecycle check-in for the seat and join a thread with a second seat; restart; wait for the restarted
    boot's publication and reconciliation; send to the seat -> its receipt has `available_at` set and no
    `prepared_unavailable_warnings` row. Same for an `operator_human` binding
    (`daemon_restart_keeps_joined_human_seat_available`).
13. Run: `cargo test --locked --all-features --lib daemon::transport -- --test-threads=1`,
    `--lib identity::reconcile`, `--lib store::receipts`, `--test contracts`,
    `--test service composition -- --test-threads=1`, then the gates. Commit:
    "Expected daemon boot and binding carry-forward (ht-rzi.5)".

**Deliverable / tests:** `WireRequest.expected_boot`, `ErrorCode::DaemonBootChanged` refused pre-dispatch and
surfaced as a definite rejection, `ReconciliationAction::CarryForward` + in-place carry-forward on
reconfirm, W5-2 test. Tests named in steps 1, 2, 7, 8, 12.

---

## Task 4

**Bead:** ht-rzi.6 — Trust-edge hygiene: O_NOFOLLOW allocator.lock, wording, id comments. TRUST-POLICY
Accepted limits.

**filesTouched:** src/cli/journal.rs, tests/cli/journal.rs, src/harness/mod.rs, tests/cli/hook.rs,
src/protocol/ids.rs, src/store/public_ids.rs

**Bead description (verbatim).** TRUST-POLICY Accepted limits. (1) src/cli/journal.rs:467 private_open
(:1066-1072) uses secure_options (O_NOFOLLOW) like the context journal (src/harness/context.rs ~893). (2)
W6-R2: src/harness/mod.rs:243 hint -> '...mark it as self when run in this pane'. (3)
src/protocol/ids.rs:133 comment says 104 random bits; it is 112. (4) Comment the send-preparation id reuse
bound (62^8 ~ 2^47.6; src/store/public_ids.rs fresh()).

**Acceptance criteria (verbatim).** allocator.lock leaf symlink is refused (test); hint text updated with its
test; comments corrected.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Steps (TDD):**
1. Failing test in `tests/cli/journal.rs`:
   ```rust
   #[test]
   fn allocator_lock_leaf_symlink_is_refused() {
       let dir = temp();
       let journal = Journal::open(&dir).unwrap();
       let elsewhere = temp();
       std::fs::create_dir_all(&elsewhere).unwrap();
       let victim = elsewhere.join("victim");
       std::fs::write(&victim, b"").unwrap();
       let _ = std::fs::remove_file(dir.join("allocator.lock"));
       std::os::unix::fs::symlink(&victim, dir.join("allocator.lock")).unwrap();
       assert!(journal.record(scope(), send(), 1).is_err(), "a symlinked allocator.lock must not be followed");
       assert!(journal.lock().is_err());
       std::fs::remove_dir_all(dir).unwrap();
       std::fs::remove_dir_all(elsewhere).unwrap();
   }
   ```
   (`Journal::lock` is private but the test module is a child of `src/cli/journal.rs`; it is already used
   by `reserved_crash_gap_is_never_reused`.) Run it: it fails today (the symlink is followed).
2. Fix `private_open` in `src/cli/journal.rs`:
   ```rust
   /// Sandbox-writable (Codex) instance directory: never follow a leaf
   /// symlink (TRUST-POLICY Accepted limits), like the context journal.
   fn private_open(path: &Path) -> io::Result<File> {
       let mut options = OpenOptions::new();
       options.read(true).write(true).create(true);
       #[cfg(unix)]
       options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
       options.open(path)
   }
   ```
   (`OpenOptionsExt` is already imported; `libc` is a dependency.) Re-run the test: green.
3. Hint: change `src/harness/mod.rs:243` to
   `"Your seat in this pane: {seat} (thread participants and thread show mark it as self when run in this pane).\n"`.
   In `tests/cli/hook.rs` (~654) strengthen the existing assertion to the full sentence:
   `format!("Your seat in this pane: {} (thread participants and thread show mark it as self when run in this pane).", digest.seat.as_str())`
   (write the assertion first, see it fail, then change the text). Grep `tests/` and `docs/` for the old
   sentence and update any other copy.
4. `src/protocol/ids.rs` (~133): "the 104 fully random bits" -> "the 112 bits of a v4 UUID outside its
   version and variant bytes (bytes 6 and 8 are skipped whole, so 4 random bits there are discarded)". Read
   the loop (it skips bytes 6 and 8: 14 bytes x 8 = 112 bits) and keep the bias sentence consistent
   (`2^112 mod 62^8` bias is below 2^-56 still holds; recheck the arithmetic and state it).
5. `src/store/public_ids.rs` `fresh()`: add a doc comment: "Suffixes are drawn from 62^8 ≈ 2^47.6 values.
   A retired id is only reused if a fresh draw hits it and nothing references it any more: the probability
   per draw is about (retired ids) x 2^-47.6, and such a reuse is harmless (TRUST-POLICY Accepted limits,
   Probabilistic identifiers). Live ids are checked for collisions in every slot."
6. Run `cargo test --locked --all-features --lib cli::journal -- --test-threads=1` and
   `--lib cli::hook`, the gates. Commit: "Trust-edge hygiene (ht-rzi.6)".

**Deliverable / tests:** `allocator.lock` opened with `O_NOFOLLOW`; W6-R2 hint wording; corrected id
comments. Tests: `cli::journal::tests::allocator_lock_leaf_symlink_is_refused`, the strengthened hint
assertion in `tests/cli/hook.rs`.

---

## Task 5

**Bead:** ht-rzi.3 — Agent-to-human binding guards for me init (Wave 18). Implements TRUST-POLICY A3/A4.
Consumes the Task 1 port (ht-rzi.9).

**filesTouched:** src/protocol/commands.rs, src/ports.rs, src/service/dispatch.rs, src/store/seats.rs,
src/store/mod.rs, src/cli/me.rs, src/cli/mod.rs, src/cli/commands.rs, src/cli/journal.rs, src/cli/retry.rs,
src/harness/bridge.rs, tests/store/cooperative_checkin.rs, tests/store/facade.rs, tests/contracts.rs,
tests/cli/cooperative.rs, tests/integration/operator_ux.rs, tests/integration/sweep.rs,
tests/hook_entrypoint.rs

**Bead description (verbatim).** TRUST-POLICY A3/A4. (1) Daemon-side: refuse a lifecycle check-in with
harness=human while the seat's open binding has provenance cooperative_top_level, unless the request is
--operator (store lifecycle path src/store/seats.rs ~3671-3695 ends any open binding today). Daemon-side
because a forged contexts/ file can bypass the CLI (Wave 30). (2) Client-side best effort in src/cli/me.rs
(and derive_selection in src/cli/mod.rs ~1064 when the local context is Human): refuse when CLAUDECODE or
CODEX_* env markers are present, or Herdr reports an agent in the pane (one pane read). Refusal guidance
names the override argv 'herdr-threads me init --operator'.
owns: daemon refusal of agent-to-human lifecycle check-in without --operator; me init / derive_selection
agent-marker refusal. files: src/store/seats.rs (lifecycle check-in), src/cli/me.rs, src/cli/mod.rs.
blocked-by ht-rzi.9: consumes boundary contract. boundary contract: ht-rzi.9.
consumes: cooperative_top_level binding provenance (existing value; ht-rzi.2 keeps it for reattached
bindings) and the check order owned by ht-rzi.2. User-facing docs are owned by ht-rzi.7.

**Acceptance criteria (verbatim).** Tests: me init over a live cooperative_top_level binding is refused with
operator guidance; --operator path allowed and audited; human->agent check-in still replaces the human
binding; me init with CLAUDECODE set refuses; flagless command from a Human context with agent markers
refuses. me init with a CODEX_* variable set refuses; me init in a pane where the stand-in Herdr reports a
claude or codex agent refuses. The --operator override records audit operator:local-user:<uid>, the
resulting binding is operator_human, and no receipt carries the operator label. A cooperative_top_level seat
on its own resolved target receiving a startup, /clear and resume lifecycle check-in through
src/cli/hook.rs replaces the binding each time, is never refused, and records no cooperative_continuity.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Design decisions made here (planner):**
- The override is a new wire command `Command::OperatorCheckIn(CheckIn)` (same payload as `CheckIn`, so no
  `CheckIn { .. }` literal changes). The daemon accepts it only for `claim.harness == Human` and a
  `Lifecycle` mode (else `InvalidRequest`), from the owner peer (cooperative path already requires peer ==
  owner; build `OperatorActor::from_peer(peer, owner_uid)` for the audit label).
- `RegisterAvailableRequest` gains `pub operator: Option<OperatorActor>` (update its literals in
  `src/service/dispatch.rs`, `src/ports.rs` tests, `tests/contracts.rs`, `tests/store/facade.rs`,
  `tests/store/cooperative_checkin.rs`).
- Refusal code: `ErrorCode::Unauthorized` (deterministic, so `me init`'s intent is discarded; not
  `Conflict`, which `me init` rewords as "check-in in progress"), detail: "seat {seat} is bound to a
  {harness} agent (cooperative_top_level); a person's check-in never replaces an agent's binding. Run it in
  your own shell pane, or override as the local account: `herdr-threads me init --operator`".
- Audit: one `allocation_decisions` row, kind `operator_human_override`, `operator_label =
  actor.audit_label()`, target = the seat's target, boot/epoch/target generation from the mapping. **This
  kind is admitted only by Task 2's migration (`migrations/0010_b5_trust_guards.sql`).** Do this step last;
  if that file is not on your base, rebase onto the integration branch; if Task 2 has still not merged,
  report BLOCKED naming ht-rzi.1.
- Env markers: `CLAUDECODE` (any value) or any variable whose name starts with `CODEX_`. Pure function so it
  is testable without mutating the process environment.

**Steps (TDD):**

A. Daemon refusal (A4).
1. Failing tests in `tests/store/cooperative_checkin.rs` (reuse the file's lifecycle check-in helpers):
   - `human_lifecycle_check_in_over_cooperative_top_level_binding_is_refused`: seat with an open
     `cooperative_top_level` binding; a `harness: Human` lifecycle `CheckIn` through
     `register_available` with `operator: None` -> `Err(Unauthorized)` whose detail contains
     `herdr-threads me init --operator`; the agent binding is still open, no new binding, generation
     unchanged.
   - `human_to_agent_lifecycle_check_in_still_replaces_human_binding`.
   - `agent_lifecycle_check_ins_on_own_target_always_replace` (startup/clear/resume are all `Lifecycle`
     mode at the store: three successive agent lifecycle check-ins each end the previous binding and open a
     new `cooperative_top_level` one; none refused; no `allocation_decisions` row of kind
     `cooperative_continuity`).
   - `operator_human_override_replaces_agent_binding_and_is_audited` (after step 6):
     `operator: Some(actor 501)` -> new binding `harness='human'`, `observation_provenance='operator_human'`;
     one `allocation_decisions` row kind `operator_human_override`, label `operator:local-user:501`; no
     `receipts`/`messages` column anywhere contains `operator:local-user` (grep the receipt observation
     columns used in `tests/integration/operator_ux.rs::provenance`).
2. Implement in `register_cooperative` (`src/store/seats.rs` ~3628), inside the decision closure before the
   lifecycle branch ends bindings:
   ```rust
   if lifecycle && claim.harness == crate::protocol::authority::Harness::Human {
       let open: Option<(String, String)> = tx.query_row(
           "SELECT observation_provenance,harness FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
           [seat.as_str()], |r| Ok((r.get(0)?, r.get(1)?)),
       ).optional().map_err(store_error)?;
       if let Some((provenance, harness)) = open
           && provenance == crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE
           && operator.is_none()
       {
           return Err(api_error(ErrorCode::Unauthorized, format!(/* detail above */)));
       }
   }
   ```
   Thread `operator: Option<&OperatorActor>` from `register_available` (new parameter) and
   `RegisterAvailableRequest.operator` through `src/store/mod.rs`. Use a distinct idempotency digest when
   `operator` is Some (e.g. `schema::canonical_digest(&("operator_check_in", &command.mode, &command.claim))`)
   so the same key cannot be replayed across the two forms.
3. Wire + dispatch: `Command::OperatorCheckIn(CheckIn)` in `src/protocol/commands.rs` (validate like
   `CheckIn`; it is not a `PermitMutation` variant). In `src/service/dispatch.rs`, handle it by running
   the same cooperative path as `PermitMutation::CheckIn` with `RegisterAvailableRequest { operator:
   Some(actor), .. }` after checking `harness == Human` and `mode` is `Lifecycle`; refactor
   `cooperative_mutation` minimally (e.g. an `operator: Option<OperatorActor>` parameter used only by the
   CheckIn arm). Any non-owner peer is `Unauthorized` (existing check).
4. Daemon/service test in `tests/store/facade.rs` or `tests/service/cooperative.rs` (pick the file whose
   helpers already drive `DomainService` with `PermitMutation::CheckIn`):
   `operator_check_in_requires_human_lifecycle` (agent harness or Current mode -> `InvalidRequest`).

B. Client: `me init --operator` and env/Herdr markers.
5. Failing unit tests in `tests/cli/cooperative.rs` (module of `src/cli/mod.rs`):
   - `agent_markers_detects_claudecode_and_codex_prefix`:
     `agent_marker([("CLAUDECODE","1")])` -> `Some("CLAUDECODE")`; `[("CODEX_HOME","/x")]` ->
     `Some("CODEX_HOME")`; `[("PATH","/bin"),("CODEXX","1")]` -> `None`.
   - `derive_selection_refuses_human_context_with_agent_marker` and
     `derive_selection_refuses_human_context_when_herdr_reports_agent` (inject the env iterator and a
     pane-agent reader closure; see step 7).
6. `src/cli/mod.rs`: add
   ```rust
   /// Agent environment markers (TRUST-POLICY A4, best effort): `CLAUDECODE`
   /// or any `CODEX_*` variable. Returns the first marker name found.
   pub(crate) fn agent_marker<K: AsRef<str>, V>(vars: impl IntoIterator<Item = (K, V)>) -> Option<String> {
       vars.into_iter()
           .map(|(key, _)| key.as_ref().to_owned())
           .find(|key| key == "CLAUDECODE" || key.starts_with("CODEX_"))
   }
   ```
   and a shared refusal builder `agent_evidence_refusal(pane, evidence: &str) -> RunError` producing
   `invalid_request` with "... `me init` and person-pane commands never act as an agent ({evidence}).
   Run them in your own shell pane, or override as the local account: `herdr-threads me init --operator`".
7. `derive_selection` (`src/cli/mod.rs` ~1064): when the selected context's `harness == Harness::Human`,
   refuse if `agent_marker(std::env::vars())` is Some, or if `pane_agent(context, clock, &pane)` (Task 1
   helper) returns `Ok(Some(obs))` with `obs.kind` in `claude|codex`. A read error does not refuse (best
   effort; the daemon refusal is the guard). `derive_selection` needs the `RuntimeContext`; thread it from
   `derive_caller` (callers already hold `context`). For testability split the body into
   `derive_selection_with(env, read_agent: impl Fn(&HostTargetId) -> Result<Option<PaneAgentObservation>, ApiError>, ..)`.
8. CLI flag: `MeSub::Init { #[arg(long)] operator: bool }` -> `CliAction::MeInit { operator: bool }`
   (`src/cli/commands.rs`); update the `CliAction::MeInit` arms in `src/cli/mod.rs`. In
   `run_me_init(parsed, operator, ..)` (`src/cli/me.rs`):
   - without `--operator`: refuse on `agent_marker(std::env::vars())`; replace the existing
     `host.pane(..).agent` read with Task 1's `pane_agent` helper and refuse when it reports a
     `claude`/`codex` (any non-None) kind; keep the existing agent-context refusal
     (`agent_seat_refusal`) but append the `--operator` override argv to its text;
   - with `--operator`: skip those client-side refusals, force a fresh lifecycle
     (`MutationSpec::CheckInLifecycle` with a new `me-init:` event id) and mark it operator so the journal
     submits `Command::OperatorCheckIn`. Plumbing: `MutationSpec::CheckInLifecycle` gains
     `operator: bool` (one construction site in `src/cli/commands.rs`, two in `src/cli/me.rs`, patterns in
     `src/cli/mod.rs`); `SemanticMutation::CooperativeCheckIn` gains
     `#[serde(default, skip_serializing_if = "std::ops::Not::not")] operator: bool` and its `to_command`
     emits `Command::OperatorCheckIn(CheckIn { .. })` when true (`src/cli/journal.rs`;
     `harness/bridge.rs::pending_request` and `prepare_event` carry the flag through unchanged; default
     false everywhere else); `src/cli/retry.rs` accepts `CheckedIn` for it.
   - Update `ME_INIT_HELP` with one sentence on refusals and `--operator`.
9. Integration (`tests/integration/operator_ux.rs`, built binary + stand-in Herdr):
   - In `Plugin::run` add `.env_remove("CLAUDECODE")` and remove every `CODEX_*` variable present in the
     test process (`for (k, _) in std::env::vars() { if k.starts_with("CODEX_") { command.env_remove(k); } }`);
     do the same in `tests/integration/sweep.rs`'s command builder. Add an optional `extra_env` parameter
     (or a `run_with_env`) for the marker tests.
   - `me_init_refuses_with_claudecode_or_codex_marker` (`CLAUDECODE=1`, then `CODEX_HOME=/tmp/x`; exit 2,
     stderr names `herdr-threads me init --operator`).
   - `me_init_refuses_where_stand_in_herdr_reports_claude_or_codex` (panes built with Task 1's
     `agent_pane(.., "claude", ..)` and `"codex"`).
   - `me_init_over_live_agent_binding_is_refused_then_operator_override_records_audit` (agent checks in on
     the person's pane via cooperative flags, `me init` refused with operator guidance; `me init
     --operator` succeeds; DB: open binding `human`/`operator_human`, one `operator_human_override` row
     labelled `operator:local-user:<uid of the test process>`; receipts provenance never contains
     `operator:local-user`).
   - `flagless_command_from_human_context_with_agent_marker_refuses` (after a successful `me init`, run
     `inbox` with `CLAUDECODE=1` in the same pane -> refused).
   - Keep the existing assertions of `person_pane_identity_sends_and_acks_without_flags_with_operator_provenance`
     green (the "never takes over" texts remain substrings).
10. Hook regression (AC last sentence) in `tests/hook_entrypoint.rs`:
    `cooperative_seat_on_own_target_startup_clear_resume_always_replace_binding` — installed Claude hook
    in a pane with a resolved seat: SessionStart `startup`, then `clear`, then `resume` payloads (same pane,
    new session ids for startup/clear, resumed id for resume); after each, exactly one open binding, new
    generation, provenance `cooperative_top_level`, hook stdout carries the context (never refused); no
    `allocation_decisions` row with kind `cooperative_continuity` (query `count(*)` by kind — the kind exists
    only after Task 2's migration; with the v9 schema the count is trivially 0, so the assertion holds on
    either base).
11. Last step — audit row (requires Task 2's migration): in the decision closure, when `operator` is Some,
    insert `INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label)
    VALUES (?1,?2,?3,'operator_human_override',?4,?5,?6,?7,?8)` with the mapping's boot/epoch/revision and
    `actor.audit_label()`. Make the step-1 audit test pass.
12. Run: `--lib store::` filter `cooperative_checkin`, `--lib cli::` filter `cooperative`,
    `--test integration operator_ux`, `--features test-support --test hook_entrypoint cooperative_seat_on_own_target`,
    then the gates. Commit: "Agent-to-human binding guards for me init (ht-rzi.3)".

**Deliverable / tests:** daemon refusal of a human lifecycle check-in over a `cooperative_top_level` binding
without `--operator`; `me init --operator` (`Command::OperatorCheckIn`) audited `operator:local-user:<uid>`;
client env-marker and Herdr-agent refusals in `me init` and Human-context `derive_selection`. Tests named in
steps 1, 4, 5, 9, 10.

---

## Task 6

**Bead:** ht-rzi.4 — Launch and wake harness guards (Wave 28, W9-2, W6-C3). Implements TRUST-POLICY A4 /
Accepted limits. Consumes the Task 1 port (ht-rzi.9).

**filesTouched:** src/harness/launch.rs, src/cli/launch.rs, src/ports.rs, src/host/native.rs,
src/store/wake.rs, src/notification/dispatch.rs, tests/harness/launch.rs, tests/cli/launch.rs,
tests/store/wake.rs, tests/scheduler/dispatch.rs

**Bead description (verbatim).** TRUST-POLICY A4 / Accepted limits. (1) Wave 28: launch_managed
(src/harness/launch.rs ~440-560) refuses when the seat's open binding is agent provenance and Herdr reports a
claude/codex agent in that binding's pane (seat inspect + agent get); the name suffix retry
(src/host/native.rs:210-216) then cannot start a second agent for the seat. (2) W9-2: carry the binding
harness into SafeWakeTarget (src/ports.rs ~1533) and compare with agent.agent in cooperative_wake_ready
(src/host/native.rs ~964-994). (3) W6-C3: refuse the top-level 'codex resume' launch form
(src/harness/launch.rs:283) like fork/review until a live capture exists. Coordinate with B6 ht-p03.28
(launch correctness) to avoid overlapping edits.
owns: launch live-bound-agent refusal, SafeWakeTarget harness field + cooperative_wake_ready comparison,
codex resume launch-form refusal. files: src/harness/launch.rs, src/cli/launch.rs, src/ports.rs
(SafeWakeTarget), src/host/native.rs, src/store/wake.rs.
blocked-by ht-rzi.9: consumes boundary contract. boundary contract: ht-rzi.9.
Absent-agent rules (Herdr agent kind is detection-based, independent of the integration): launch guard — no
agent detected in the bound pane means no live bound agent, launch proceeds; wake — the pane's detected kind
must equal the bound harness, an absent or unknown kind refuses the wake (as today's kind check does); a
human-bound seat never receives a wake prompt.

**Acceptance criteria (verbatim).** Tests: launch onto a seat whose bound agent is live elsewhere is refused
with a clear message; wake to a pane whose agent kind differs from the bound harness is refused (no prompt
sent); 'codex resume' launch refused with evidence message. Wake refusal covered in both directions
(claude-bound seat with codex pane agent, codex-bound with claude); launch refusal covered for a live claude
and a live codex bound agent. Wake to a human-bound seat whose pane shows a claude or codex agent sends no
prompt; wake to a codex-bound seat whose pane has no detected agent sends no prompt; launch for a seat whose
bound pane shows no agent proceeds.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Design decisions made here (planner):**
- Open binding for launch: new `LaunchSeatResolver::open_binding(&self, seat: &SeatId, budget) ->
  Result<Option<OpenBinding>, ApiError>` with default `Ok(None)`; `OpenBinding { target: HostTargetId,
  provenance: String }`. `DaemonSeatResolver` implements it by paging `SeatInspect` history (pages of 50,
  following the cursor, at most 20 pages) and returning the last `SeatHistoryItem::Binding` with
  `ended_at == None`.
- Launch guard: provenance `cooperative_top_level` and `host.observe_pane_agent(binding.target)` returns
  `Ok(Some(obs))` with kind `claude` or `codex` -> `TargetUnsafe`. `Ok(None)` or a kind outside
  claude/codex -> proceed. A read `Err` is returned as the launch error (nothing started).
- Wake: `ReservedWakeAuthority::Cooperative` gains `harness: Option<String>` (the open binding's harness, set
  in `src/store/wake.rs::cooperative_authority` from the same binding row it already reads);
  `SafeWakeTarget` gains `pub bound_harness: Option<String>`, set by the notification dispatcher from the
  reservation after `host.safe_wake_target(..)` returns; `cooperative_wake_ready` requires
  `kind == bound_harness` when `bound_harness` is Some. A seat with no open binding keeps today's rule
  (recognized `claude`/`codex` kind) — the policy's "bound harness" exists only when a binding exists.
  Human-bound seats are already excluded in `current_authority` (`harness='human'` -> no authority); keep
  it and pin it with a test.

**Steps (TDD):**

A. `codex resume` launch form (W6-C3).
1. In `tests/harness/launch.rs` move the three accepted top-level `resume` cases ("interactive resume" `-m m
   resume ID`, `--remote ADDR resume`, `--remote-auth-token-env VAR resume`, and any other top-level resume
   row) from the accepted table to a refusal test `codex_top_level_resume_form_is_refused_until_captured`
   asserting `InvalidRequest` and that the detail names the missing live capture; keep `exec resume`
   accepted. See it fail.
2. In `codex_form` (`src/harness/launch.rs` ~283) replace the `"resume"` arm with
   `"resume" => Err(error(ErrorCode::InvalidRequest, "managed launch refuses the Codex `resume` form: no live capture shows it loading the owned hooks (TRUST-POLICY Accepted limits); run `codex resume` by hand in the pane, or use `exec resume`"))`.
   Remove `CodexLaunchForm::Resume` and its doc bullet (now unused; keep the enum otherwise), and update the
   `CODEX_UNSUPPORTED_SUBCOMMANDS` doc ("`exec` is a handled form; `resume` is refused until captured").

B. Launch live-bound-agent refusal (Wave 28).
3. Failing tests in `tests/harness/launch.rs` (its `FakeHost`/`FakeSeats` implement `HostPort` /
   `LaunchSeatResolver`; add scripted `observe_pane_agent` and `open_binding` to them):
   `launch_refused_when_bound_claude_agent_is_live_elsewhere`,
   `launch_refused_when_bound_codex_agent_is_live_elsewhere` (open binding on `w4:p9`,
   provenance `cooperative_top_level`, pane agent kind `claude`/`codex` -> `TargetUnsafe`, `launch_native`
   never called, message names the seat, the pane and "second agent"),
   `launch_proceeds_when_bound_pane_shows_no_agent` (`Ok(None)` -> launch_native called),
   `launch_proceeds_for_human_bound_seat` (provenance `operator_human`, no refusal from this guard).
4. Implement in `launch_managed` right after `seats.resolve_for_launch(..)`:
   ```rust
   if let Some(bound) = seats.open_binding(&seat, &budget)?
       && bound.provenance == crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE
   {
       let observed = host.observe_pane_agent(&bound.target, &HostCallContext {
           budget: budget.clone(), expected_boot: None, expected_epoch: None,
       })?;
       if let Some(kind) = observed.and_then(|agent| agent.kind).filter(|kind| kind == "claude" || kind == "codex") {
           return Err(error(ErrorCode::TargetUnsafe, format!(
               "seat {} is bound to a live {kind} agent in pane {}; launch refuses to start a second agent for the seat (TRUST-POLICY A4). Use that pane, or retire/rebind the seat as the operator",
               seat.as_str(), bound.target.as_str())));
       }
   }
   ```
   Add `OpenBinding` + the trait method (default `Ok(None)`) to `src/harness/launch.rs`; implement it for
   `DaemonSeatResolver` and forward it in `RecordingResolver` (`src/cli/launch.rs`); add a
   `tests/cli/launch.rs` test `daemon_resolver_open_binding_returns_last_open_binding` against its
   existing fake client if one exists there (else cover via the harness tests only).

C. Wake harness comparison (W9-2).
5. Failing native tests in `src/host/native.rs` `mod tests` (macOS; reuse `cooperative_wake_with` + its
   `tamper` hook to set `target.bound_harness`): `cooperative_wake_refuses_claude_bound_seat_with_codex_agent`,
   `cooperative_wake_refuses_codex_bound_seat_with_claude_agent` (methods called `["pane.get","agent.get"]`,
   no `agent.prompt`), `cooperative_wake_refuses_codex_bound_seat_without_detected_agent`,
   `cooperative_wake_prompts_matching_bound_harness`.
6. `src/ports.rs`: `SafeWakeTarget.bound_harness: Option<String>` (doc: TRUST-POLICY A4 wake rule);
   `ReservedWakeAuthority::Cooperative.harness: Option<String>`. Fix struct literals (native.rs
   `safe_wake_target` sets `bound_harness: None`; the ports test adapter; `tests/scheduler/dispatch.rs`
   ~3418; `tests/store/wake.rs` ~746).
7. `cooperative_wake_ready` (`src/host/native.rs` ~964): after the recognized-kind check add
   `if let Some(bound) = target.bound_harness.as_deref() && kind != Some(bound) { return Err(format!("agent kind {} differs from the bound harness {bound}", kind.unwrap_or("none"))); }`.
8. `src/store/wake.rs::cooperative_authority`: extend its binding query with `harness` and return it in
   `Cooperative { harness, .. }` (None when no open binding). `src/notification/dispatch.rs` (~131): after
   `safe_wake_target` returns `Some(mut target)`, set `target.bound_harness` from
   `ReservedWakeAuthority::Cooperative { harness, .. }`. Tests: `tests/store/wake.rs`
   `cooperative_reservation_carries_bound_harness` and
   `human_bound_seat_with_agent_in_pane_gets_no_wake_authority` (open `human` binding + effective
   observation of a claude/codex pane -> no reservation); `tests/scheduler/dispatch.rs`
   `dispatcher_passes_bound_harness_to_prompt_target` (its `FakeNativeHost` records the `SafeWakeTarget`
   given to `submit_prompt`).
9. Run `--lib harness::launch`, `--lib cli::launch`, `--lib host::native`, `--lib store::wake`,
   `--lib scheduler::`, then the gates. Commit: "Launch and wake harness guards (ht-rzi.4)".

**Deliverable / tests:** launch refusal for a live bound agent, wake harness comparison end to end, `codex
resume` launch-form refusal. Tests named in steps 1, 3, 5, 8.

---

## Task 7

**Bead:** ht-rzi.2 — Cooperative continuity: session-id reattachment of unresolved seats. Implements
TRUST-POLICY C1. Consumes Task 2 (lift helper + migration columns) and Task 1 (pane-agent port).

**filesTouched:** src/store/seats.rs, src/store/schema.rs, src/store/mod.rs, src/store/queries.rs,
src/ports.rs, src/protocol/authority.rs, src/protocol/commands.rs, src/protocol/results.rs,
src/service/dispatch.rs, src/identity/repair.rs, src/cli/hook.rs, src/cli/journal.rs, src/cli/retry.rs,
src/cli/human.rs, src/harness/bridge.rs, src/harness/claude.rs, src/harness/codex.rs,
src/harness/context.rs, docs/compatibility/cooperative-continuity-resume.md,
tests/store/cooperative_checkin.rs, tests/cli/hook.rs, tests/hook_entrypoint.rs

**Bead description (verbatim).** TRUST-POLICY C1. Spec: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-design.md decision 3 (normative; redesigned after design roast round 1 — Herdr observation is diagnostic only). Roast: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-design-1.md (+ -step-back.md).
(1) Gate (only the plugin's own hook payload): a top-level SessionStart lifecycle check-in with payload source == resume, in a pane whose target is held or unowned, whose session id equals the last-binding occupant_bindings.native_session (EXISTING column, filled by src/harness/bridge.rs:247-255) of exactly one unresolved nonretired seat. Sentinel values (plugin_context:...) and human-occupant values never match; NULL/empty never matches; any other source never reattaches and falls through to the existing path.
(2) On a match, in the deciding transaction: rebind the seat to the target; the open binding stays cooperative_top_level; write cooperative_continuity to seat history only (new constant in src/protocol/authority.rs; never on receipts; written as an allocation_decisions row of kind cooperative_continuity (admitted by ht-rzi.1's migration)); call lift_baseline_hold_if_clear (needs: ht-rzi.1) unconditionally.
(3) Diagnostics: read the pane's Herdr agent_session through the ht-rzi.9 port; record the comparison (match, mismatch, absent, read error) in that row's continuity_diagnostic column (ht-rzi.1's migration) and show it in seat inspect. It never refuses or permits reattachment.
(4) Seatless check-in (client + wire): today the hook stops before contacting the daemon when the pane has no resolved seat (src/cli/hook.rs find_seat, :891) and CallerClaim needs seat + binding generation (src/harness/bridge.rs:255-268, prepare_event :357-361). Add a resume-only continuity check-in request carrying pane target, harness, session id and source but no seat, journaled under an instance-scoped intent; the daemon's response returns the chosen seat and binding generation, which the hook writes into the pane's client context as an ordinary lifecycle check-in does. A refusal leaves today's diagnostics unchanged. Lost reply: before find_seat, every hook event checks the instance-scoped continuity intent journal and replays a pending intent under the same operation key; the daemon's idempotent replay returns the recorded seat and generation.
(5) Check order in the daemon lifecycle decision (owned here): A4 agent-to-human refusal (ht-rzi.3) → C1 reattachment on a held/unowned target → existing hold refusal / ordinary path.
(6) Evidence capture (first step): log SessionStart stdin and Herdr 'agent get' from inside a hook for resume, /new and /clear on Claude and Codex, count SessionStart events per resume (Claude Code #24265: startup(new id)+resume(orig id)), record in docs/compatibility/. Never edit ~/.claude, ~/.codex or the shared Herdr server: use a temporary CLAUDE_CONFIG_DIR/CODEX_HOME or --settings and a scratch pane, and a private named Herdr session if a restart is needed. If resume-then-startup is observed, add a dedupe rule: a startup check-in on a seat reattached moments earlier does not overwrite its native_session. Codex: C1 works when the user runs 'codex resume' by hand in the pane; the launch form stays refused by ht-rzi.4.
owns: C1 gate and decision, cooperative_continuity constant, seatless continuity check-in (wire + hook + journal), check order, Herdr diagnostics recording, docs/compatibility evidence note. consumes: B5 migration columns (ht-rzi.1). consumes: lift_baseline_hold_if_clear (ht-rzi.1), pane-agent observation port (ht-rzi.9).
blocked-by ht-rzi.1: consumes hold-lift predicate. blocked-by ht-rzi.9: consumes boundary contract. boundary contract: ht-rzi.9.

**Acceptance criteria (verbatim).** Captured evidence for both harnesses (or the harness excluded from C1
with the reason). Tests (through the hook entry point src/cli/hook.rs, Claude and Codex payloads): resume
with a unique native_session match in a held pane reattaches, pane released, client context gets the seat
and generation; zero / multiple matches leave the seat unresolved and the pane held; retired seats,
plugin_context: sentinels, human-occupant values and NULL/empty never match; source startup/clear/new never
reattaches; Herdr agent_session present-equal, present-different, absent and read-error all reattach on a
unique match and record the comparison in seat inspect; cooperative_continuity appears in seat history and
never on receipts; the open binding is cooperative_top_level; reattaching the last unresolved seat lifts the
hold (needs: ht-rzi.1); daemon restart mid-way is idempotent (replayed intent). Lost reply: daemon commits
the reattachment, the hook drops the reply before writing the client context; the next hook event in the
pane replays the intent and writes the context (no Quiet 'no registered execution').

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Design decisions made here (planner):**
- **Two-step shape.** The continuity check-in is a seatless *reattachment decision* (rebind of the matched
  unresolved seat onto the pane's target + audit row + hold release + lift). The hook then performs the
  **ordinary lifecycle check-in** for the returned seat at the returned generation with the resume event
  (existing `lifecycle_check_in`), which opens the binding with provenance `cooperative_top_level` (so "the
  open binding stays cooperative_top_level") and writes the client context exactly as today. No new binding
  path in the store.
- **Wire.** `Command::ContinuityCheckIn(ContinuityCheckIn { target: HostTargetId, harness: Harness /*
  claude|codex only */, native_session: NativeSessionId, source: String /* must be "resume" */,
  operation: OperationId })`, `deny_unknown_fields`, validated (`source == "resume"`, harness not human,
  session non-empty and not `plugin_context:`-prefixed). Result `CommandResult::ContinuityReattached(
  ContinuityReattachment { seat: SeatId, binding_generation: u64 })`. Dispatch requires peer == owner (as
  cooperative mutations do).
- **Journal.** `IntentScope::Continuity { instance: String, target: HostTargetId }` and
  `SemanticMutation::ContinuityCheckIn { target, harness, native_session, source, event_id }`;
  `IntentKind::ContinuityCheckIn`. Operation keys come from the journal as for every other intent.
- **Match rule (store, deciding transaction).** Candidates: `seats s WHERE s.instance_id=?instance AND
  s.state='unresolved'` whose **latest** binding (`ORDER BY ordinal DESC LIMIT 1`) has `native_session = ?`,
  `harness = ?` (never reattach across harnesses), `native_session <> ''`, `native_session NOT LIKE
  'plugin_context:%'`, `harness <> 'human'`. Exactly one -> reattach; zero -> `NotFound` "no unresolved seat
  matches this resumed session"; more than one -> `Conflict` "resumed session matches several unresolved
  seats; repair with seat rebind --operator". Target must be held (`ExplicitHold`/`BaselineHeld`) or unowned
  (no resolved owner); owned -> `TargetAlreadyOwned`. Fresh current-target observation required (guard like
  the operator rebind's `OperatorTargetGuard`; add a `ContinuityTargetGuard` in `src/ports.rs` mirroring it).
- **Diagnostic.** `OrdinaryIdentity` reads `host.observe_pane_agent(target)` before the decision:
  `Ok(Some(o))` with `o.agent_session == Some(session)` -> `match`; `Ok(Some(o))` with another `Some` ->
  `mismatch`; `Ok(Some(o))` with `None` or `Ok(None)` -> `absent`; `Err(_)` -> `read_error`. Stored in
  `allocation_decisions.continuity_diagnostic`; never consulted for the decision.
- **Check order (daemon).** The continuity command is only sent by the hook when `find_seat` found no
  resolved seat for the pane (held/unresolved/unowned) and the event is a top-level `EventKind::Resume`
  with a native session. A4 (Task 5) applies to *human* lifecycle check-ins and therefore precedes
  naturally; document the order in a comment at the dispatch arm and in the hook: (1) A4 human refusal,
  (2) C1 reattachment on a held/unowned target, (3) existing hold refusal / ordinary path.

**Steps (TDD):**

A. Evidence capture (do first; record what you can).
1. Follow bead item (6) exactly (temporary `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `--settings`, scratch pane,
   private named Herdr session; never touch `~/.claude`, `~/.codex` or the shared Herdr server). Install a
   capture-only hook that appends stdin and `herdr agent get $HERDR_PANE_ID` output to a scratch file. For
   each harness capture: fresh start, `/clear` (Claude) / `/new` (Codex), and resume (`claude --resume` /
   `codex resume` by hand), and count SessionStart events per resume. Write
   `docs/compatibility/cooperative-continuity-resume.md`: harness versions, exact commands, redacted
   payloads (session ids replaced by `<orig>`/`<new>` tokens), event counts and order, whether `agent get`
   reports `agent_session` and its value relation to the payload session id.
2. If resume-then-startup (startup carrying a new id after the resume) is observed for a harness, add the
   dedupe rule in this task: in `register_cooperative` (`src/store/seats.rs`), a lifecycle check-in whose
   event the hook marks as `startup` on a seat with a `cooperative_continuity` decision in the last 10 s
   keeps the binding's `native_session` (needs the source on the wire — add `source` to the bridge's
   lifecycle event id or a flag; keep it minimal and test it). If startup-then-resume or a single resume is
   observed, no dedupe rule; say so in the note.
3. If the environment cannot run a real harness or Herdr (no binary, no auth), do not fake evidence: write
   the note with what was attempted and the exact error, cite the existing captured fixtures that already
   show SessionStart `resume` payload shapes (`tests/fixtures/claude-2.1.286`, `tests/fixtures/codex-0.158.0-live`,
   `docs/compatibility/claude-lifecycle-probe.md`, `docs/compatibility/codex-probe.md`), mark the per-resume
   event count "not captured", and flag it in your report as a concern. Do not disable C1 on your own.

B. Store decision.
4. Failing store tests in `tests/store/cooperative_checkin.rs` (copy the snapshot/hold seeding from
   `tests/store/control.rs` hold tests): `continuity_reattaches_unique_session_match_on_held_target`
   (seat `s-old` unresolved, latest binding `claude`/`sess-1`/`cooperative_top_level`; target `w1:p3`
   ExplicitHold; decide -> seat resolved on `w1:p3`, generation +1, `recovery_holds` row released, one
   `allocation_decisions` row kind `cooperative_continuity`, `operator_label` NULL, diagnostic stored;
   result seat + generation), `continuity_on_unowned_target_reattaches`,
   `continuity_refuses_zero_and_multiple_matches` (seat stays unresolved, hold stays),
   `continuity_never_matches_retired_sentinel_human_or_empty_sessions`,
   `continuity_refuses_owned_target`, `continuity_of_last_unresolved_seat_lifts_hold` (marker matched ->
   `baseline_hold_unclaimed=0`), `continuity_replay_returns_recorded_seat_and_generation` (same operation
   key -> same result, no second row), `continuity_diagnostic_values_round_trip` for
   `match|mismatch|absent|read_error`, `continuity_never_writes_receipts`.
5. Implement `pub fn decide_continuity(context, conn, elected_instance, request: ContinuityRequest,
   budget) -> Result<CommandResult, ApiError>` in `src/store/seats.rs`, structured like the rebind branch of
   `mutate_operator`: `schema::execute_idempotent_transaction` with scope `continuity:{instance}` and digest
   `canonical_digest(&("continuity_check_in", instance, &target, &harness, &native_session))`; precheck
   (observation matches guard; target held-or-unowned; unique candidate); decision: consume the guard
   against the fresh effective observation, rebind the seat (same `UPDATE seats ...` as operator rebind,
   end open bindings, delete `warning_offer`, `ensure_unavailability_episode`, structural proof update),
   insert the audit row
   `INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label,continuity_diagnostic) VALUES (?1,?2,?3,'cooperative_continuity',?4,?5,?6,?7,NULL,?8)`,
   release the target's `recovery_holds`, record `recovery_baseline_releases` / baseline disposition as
   operator rebind does (factor that tail into a shared fn used by both), `apply_eligibility_transition`,
   revisions, then `lift_baseline_hold_if_clear` unconditionally. Result
   `CommandResult::ContinuityReattached { seat, binding_generation: new seat generation }`.
   `pub const COOPERATIVE_CONTINUITY_PROVENANCE: &str = "cooperative_continuity";` in
   `src/protocol/authority.rs` with the A3 doc ("seat rebinds only; never on receipts"); use it for the
   kind string.
   Port: `StorePort::decide_continuity(&self, request: ContinuityRequest, budget)` (default
   `Err(Unsupported)`), `SqliteStore` impl; `ContinuityRequest { command: ContinuityCheckIn, guard:
   ContinuityTargetGuard, diagnostic: &'static str }` in `src/ports.rs`.
6. Seat inspect: `RepairHistory` gains
   `#[serde(default, skip_serializing_if = "Option::is_none")] pub continuity_diagnostic: Option<String>`
   (`src/protocol/results.rs`); `src/store/queries.rs` (~800) selects it; `src/cli/human.rs` shows
   "continuity (Herdr agent_session: match)" for that kind. Test `seat_inspect_shows_continuity_diagnostic`.

C. Service + wire.
7. Protocol types, validation and `Command` arms (`src/protocol/commands.rs`, `results.rs`); dispatch arm in
   `src/service/dispatch.rs` (peer == owner else `Unauthorized`), calling a new
   `OrdinaryIdentity::continuity(command, budget)` in `src/identity/repair.rs`: replay check first (store
   idempotent replay grants no new authority, mirror `operator`), then `with_observation(&target, ..)`, read
   the diagnostic via `self.host.observe_pane_agent(&target, ..)` (bounded by the same budget), build the
   guard, `store.decide_continuity(..)` under the foreground writer turn.

D. Hook + journal (seatless check-in, lost reply).
8. Failing tests in `tests/cli/hook.rs` (unit, fake `LocalClient`) and `tests/hook_entrypoint.rs` (built
   binary, real daemon, stand-in Herdr from Task 1 where a pane agent is needed):
   - entrypoint, Claude and Codex payloads: `resume_with_unique_session_match_in_held_pane_reattaches`
     (seed via DB: seat unresolved with latest binding session `S`, pane held; SessionStart
     `{"source":"resume","session_id":"S",..}` -> hook stdout has the normal check-in context, DB seat
     resolved on the pane, open binding `cooperative_top_level` with `native_session=S`, client context file
     names the seat and the new generation, hold released);
     `startup_clear_and_new_never_reattach` (same seed, `source` startup/clear (Claude) and startup/clear/
     compact (Codex) -> seat stays unresolved, hold stays, stdout is today's unavailable diagnostic);
     `zero_or_multiple_matches_leave_pane_held_with_todays_diagnostic`;
     `herdr_agent_session_diagnostic_is_recorded_for_each_case` (stand-in panes: equal session, different
     session, no session, `agent_get_error` -> all reattach; `seat inspect --json` shows `match`,
     `mismatch`, `absent`, `read_error`);
     `cooperative_continuity_in_history_never_on_receipts`;
     `reattaching_last_unresolved_seat_lifts_hold` (after the marker is recorded by the running daemon's
     pass: wait for `reconciled_epoch` like other entrypoint waits);
     `lost_reply_is_replayed_on_next_hook_event` (counting wrapper: let the daemon commit the continuity
     decision, then drop the reply — the existing `DROP` fault mode pattern applied to `ContinuityCheckIn`;
     next hook event in the pane is a PreToolUse tool event -> it replays the pending continuity intent, runs
     the lifecycle check-in, writes the context; diagnostic is not "no registered execution");
     `daemon_restart_mid_reattachment_replays_idempotently` (restart the elected daemon between the commit and
     the next event; replay returns the same seat/generation; one `cooperative_continuity` row).
9. Journal: add `IntentScope::Continuity`, `SemanticMutation::ContinuityCheckIn`, `IntentKind`, `to_command`,
   scope matching for retry and pending listing (`src/cli/journal.rs`; `src/cli/retry.rs` pairs it with
   `CommandResult::ContinuityReattached`); a journal unit test that a continuity intent round-trips and is
   found by `(instance, target)`.
10. Hook (`src/cli/hook.rs::check_in`): after building `client` and before `find_seat`:
    - **Replay:** open the intents journal; if a pending `Continuity` intent exists for `(instance,
      target)`, replay it with `retry::run_retry_api_to_writer` (same operation key). On
      `ContinuityReattached { seat, binding_generation }` run `lifecycle_check_in` for that seat/generation
      with a resume `LifecycleEvent` rebuilt from the intent (`kind: Resume`, `source: "resume"`,
      `native_session`, `harness`, `role: TopLevel`, `event_id: format!("continuity:{}", operation)`,
      capability as the harness parser sets it) and return its output (the current event's own check-in is
      then the next event's job; for a lifecycle current event, run it after the replayed one). On a
      definitive rejection complete (discard) the intent and fall through.
    - Refactor `find_seat` to return an enum `PaneSeat::{Resolved(seat, generation), HeldOrUnresolved(String),
      Unowned}` (keep messages identical).
    - **Attempt:** when the result is not `Resolved` and `event.role == TopLevel && event.kind ==
      EventKind::Resume && event.native_session.is_some()`, record a continuity intent and submit
      `Command::ContinuityCheckIn` via `retry::run_new_api_to_writer_discarding_rejection` (scope
      `Continuity`). Success -> `lifecycle_check_in(event, contexts(seat), .., &seat, binding_generation, ..)`
      (the ordinary lifecycle path seeds the context from the service mapping and writes it). Any rejection ->
      return exactly today's outcome for that `PaneSeat` value (held/unresolved -> `Failure::Unavailable`
      with the same text; unowned -> `Failure::Quiet`).
    - Keep `bridge.rs` / `context.rs` changes minimal: only what the rebuilt resume event needs (e.g. a
      constructor for the event or a public `PendingCheckIn` field). `harness/claude.rs` / `codex.rs` change
      only if the evidence note requires a recipe/scope string update.
11. Run `--lib store::` (cooperative_checkin), `--lib cli::hook`, `--lib cli::journal`,
    `--features test-support --test hook_entrypoint` (continuity tests), then the gates. Commit:
    "Cooperative continuity reattachment (ht-rzi.2)" (+ a separate docs commit for the evidence note).

**Deliverable / tests:** evidence note; `ContinuityCheckIn` wire command, journal scope and hook flow with
lost-reply replay; store reattachment decision with audit row, diagnostic, hold release and lift; seat
inspect diagnostic. Tests named in steps 4, 6, 8, 9.

---

## Task 8

**Bead:** ht-rzi.7 — Docs: describe shipped B5 guards (README, operations, agent-usage, TRUST-POLICY status).

**filesTouched:** README.md, docs/operations.md, docs/agent-usage.md, TRUST-POLICY.md,
integrations/skill/SKILL.md

**Bead description (verbatim).** Goal: the user-facing docs describe the shipped behaviour. Update
docs/operations.md, docs/agent-usage.md and README for: restore-hold lift, seat retire / rebind --replace and
the collision refusal (ht-rzi.1), cooperative continuity reattachment (resume-only, Herdr agent_session
shown as a diagnostic in seat inspect) (ht-rzi.2), me init refusals (ht-rzi.3), launch live-agent / wake
harness / codex resume refusals (ht-rzi.4), DaemonBootChanged and availability across daemon restart
(ht-rzi.5). Flip TRUST-POLICY.md's Status line and each shipped guard's **required** marker to implemented,
and drop 'today …' wording in C4/A2. Also update README's restore-hold bullet (remove 'planned').
owns: user-facing docs for B5. consumes: each guard's shipped behaviour.
files: README.md, docs/operations.md, docs/agent-usage.md, TRUST-POLICY.md, integrations/skill/SKILL.md if
it names affected behaviour.
Also document: Codex reattachment by running 'codex resume' by hand while the launch form stays refused (per
ht-rzi.2's compatibility note); the human-seat wake rule. TRUST-POLICY.md: flip every B5 guard's
**required** marker; leave C5's marker (owned by B4, ht-p03.2) and name C5 in the Status line as the one
guard owned elsewhere.

**Acceptance criteria (verbatim).** Every behaviour listed is documented with its command/error;
TRUST-POLICY.md no longer says shipped guards are not implemented; no doc describes pre-B5 behaviour as
current (grep for 'planned', 'not yet implemented', 'today' in the trust sections). The grep check excludes
C5, which stays marked required and is named in the Status line as owned by B4.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Steps:**
1. Read the merged code first (this task runs after tasks 1-7): `herdr-threads seat retire --help`,
   `seat rebind --help`, `me init --help`, the exact error texts (grep `src/` for "seat retire",
   "--replace", "me init --operator", "second agent", "DaemonBootChanged"/"daemon_boot_changed", "refuses the
   Codex `resume` form"), and `docs/compatibility/cooperative-continuity-resume.md`. Document what shipped,
   quoting commands and error codes exactly.
2. `docs/operations.md`: a "Restore holds and repair" section — when holds lift (instance-wide once no
   unresolved seat remains and the restored baseline is reconciled; per target on rebind / fresh seat /
   continuity), `seat retire SEAT --operator`, `seat rebind OLD --pane P --replace NEW --operator`, the
   collision refusal and its two argv, `seat inspect` repair rows (`operator_retire`, `operator_rebind`,
   `cooperative_continuity` with the Herdr `agent_session` diagnostic, `operator_human_override`);
   `DaemonBootChanged` (exit 3, retry after re-reading the descriptor, nothing applied) and availability
   across daemon restart (C4 carry-forward).
3. `docs/agent-usage.md`: cooperative continuity (resume-only; Claude `--resume`; Codex `codex resume` run by
   hand in the pane while the managed launch form is refused); `me init` refusals (agent env markers,
   Herdr-reported agent, live agent binding) and `me init --operator`; launch refusal when the bound agent is
   live elsewhere; wake only to the bound harness; human seats never woken.
4. `README.md`: restore-hold bullet without "planned"; one line each for retire/replace and continuity.
5. `TRUST-POLICY.md`: Status line -> "Status: adopted 2026-10-01; B5 guards implemented (epic `ht-rzi`).
   C5's guard is owned by B4 (`ht-p03.2`) and is the one guard still marked **required**."; replace every
   B5 `(**required**...)` marker with `(implemented)` (C2, C3 x2, C4, A2, A4 x4, Accepted limits x2); drop
   "today it lapses until the agent's next check-in" (C4) and "today only the response is checked" (A2);
   leave C5 untouched. Decision record table unchanged except where it says "planned".
6. `integrations/skill/SKILL.md`: update only if it names `me init`, launch of `codex resume`, wake, or
   restore holds (grep first); otherwise leave it.
7. Verify: `grep -n -i "planned\|not yet implemented\|today" README.md docs/operations.md docs/agent-usage.md TRUST-POLICY.md`
   shows nothing in the trust/B5 sections except C5; `cargo test --locked --all-features --test package`
   and any doc-snapshot tests that read these files (`grep -rn "operations.md\|agent-usage.md\|TRUST-POLICY" tests/`)
   pass. Commit: "Docs: shipped B5 trust guards (ht-rzi.7)".

**Deliverable / tests:** updated docs; grep check above; doc-reading tests green.

---

## Task 9

**Bead:** ht-rzi.8 — Integration sweep: B5 trust policy guards end to end. Also the seam integration for
ht-rzi.9.

**filesTouched:** tests/service.rs, tests/service/trust_policy.rs, tests/integration.rs,
tests/integration/trust_policy.rs (plus any small inline fixes the sweep finds; list them in the report)

**Bead description (verbatim).** Root integration sweep (implements what it finds). Verify the goal's main
flows end to end with deterministic stand-in agents, add the integration tests no per-bead acceptance
covers, and sweep for unwired values; fix small gaps inline, file blockers for big ones.
Required end-to-end scenario (F6 walking skeleton): Herdr incarnation change → saved seats unresolved and
baseline targets held → a resumed Claude-style hook check-in reattaches its seat (cooperative_continuity in
seat history, binding cooperative_top_level) → a collision resolved via seat retire and another via rebind
--replace → last unresolved seat resolved → baseline_hold_unclaimed cleared → ordinary seat resolve / me
init on a new pane succeeds → a send to the reattached joined seat has no recipient_unavailable warning.
Cross-bead checks: me init over a cooperatively reattached seat is refused (ht-rzi.2 × ht-rzi.3); launch onto
a reattached seat whose agent is live is refused (ht-rzi.2 × ht-rzi.4); a binding carried forward across a
daemon restart still matches on its session id after a later incarnation change (ht-rzi.5 × ht-rzi.2); the
CLI's expected_boot fill and the carry-forward together give no UnknownOutcome and no warning across a daemon
stop/ensure.
files: tests/service/ (new trust_policy.rs or similar), any small inline fixes.
Also serves as the seam integration for ht-rzi.9 (pane-agent observation port: verify .2/.3/.4 read it
consistently). Extra scenarios: a Codex-style reattachment through the hook entry point with no Herdr hint,
followed by a wake to that seat; a restore where every seat is structurally reconfirmed leaves no hold.

**Acceptance criteria (verbatim).** The F6 end-to-end test and each cross-bead check exist and pass; fmt +
check + the new tests green.

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]`; `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only.

**Steps:**
1. Survey what tasks 1-7 shipped (git log on the integration branch, the new commands/errors) and the
   existing harnesses: `tests/integration/sweep.rs` (built binary + stand-in Herdr `FakeHost` with
   Task 1's `agent_pane` scripting), `tests/service/composition.rs` (`ActualNativeFixture`, elected daemon,
   `restart()`), `tests/hook_entrypoint.rs` (installed hook). Choose per scenario the lowest harness that
   exercises the real wiring: CLI/hook flows through the built binary (`tests/integration/trust_policy.rs`,
   registered in `tests/integration.rs` with `#[path = "integration/trust_policy.rs"] mod trust_policy;`),
   restart/incarnation flows through the elected daemon (`tests/service/trust_policy.rs`, registered in
   `tests/service.rs`). The stand-in Herdr must be able to change incarnation (restart the `FakeHost` on a
   new socket / new process witness; if `FakeHost` cannot, extend it minimally in the new test file, not in
   `sweep.rs`, unless the change is tiny).
2. `f6_walking_skeleton` (macOS-gated if it needs the incarnation witness): seats A (Claude, session `SA`),
   B, C joined to a thread with D; Herdr incarnation change -> A, B, C unresolved, baseline targets held;
   Claude SessionStart `resume` with `SA` through the installed hook in A's restored pane -> A reattached
   (`seat inspect` shows `cooperative_continuity`; open binding `cooperative_top_level`); B's restored pane
   owned by a new seat N1 -> `seat rebind B --pane P_B` refused listing both argv -> `seat retire B
   --operator`; C's pane owned by N2 -> `seat rebind C --pane P_C --replace N2 --operator`; now no
   unresolved seat -> `baseline_hold_unclaimed = 0` (after the daemon's reconciliation pass records the
   marker) -> `seat resolve --pane <new pane>` and `me init` in another new pane succeed; D sends to A ->
   A's receipt `available_at` set, no `prepared_unavailable_warnings` for A.
3. Cross-bead tests: `me_init_over_reattached_seat_is_refused` (2x3);
   `launch_onto_reattached_seat_with_live_agent_is_refused` (2x4, stand-in Herdr reports `claude` in A's
   pane, launch targets a different pane resolving to A... or the same seat via rebind — pick the reachable
   shape and document it); `carried_forward_binding_still_matches_session_after_later_incarnation_change`
   (5x2: restart daemon -> carry-forward keeps `native_session`; then incarnation change; resume with the
   same session reattaches); `expected_boot_and_carry_forward_give_no_unknown_outcome_across_stop_ensure`
   (`daemon stop` + `daemon ensure` through the binary, then `send` -> exit 0, no warning, no
   UnknownOutcome).
4. Extra scenarios: `codex_reattachment_without_herdr_hint_then_wake` (Codex payload, pane with no agent
   record -> diagnostic `absent`, reattached; then wake: with no detected agent the wake is refused, with a
   scripted `codex` agent the wake prompt is submitted — assert via the stand-in's recorded
   `agent.prompt`); `restore_with_every_seat_structurally_reconfirmed_leaves_no_hold`.
5. Seam check (ht-rzi.9): one test asserting `me init` (Task 5), launch (Task 6) and continuity diagnostics
   (Task 7) all observe the same scripted pane-agent values from the stand-in (present/absent/error).
6. Sweep for unwired values: grep that every new constant/kind is read somewhere (`operator_retire`,
   `operator_human_override`, `cooperative_continuity`, `continuity_diagnostic`, `reconciled_boot`,
   `DaemonBootChanged`, `bound_harness`, `observe_pane_agent`); every `record_reconciliation_pass` /
   `lift_baseline_hold_if_clear` call site named in Task 2 exists. Fix small gaps inline (list them); for a
   big gap stop and report it so the coordinator files a blocker.
7. Run the new tests, `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and
   finally the full serial suite `cargo test --locked --all-targets --all-features -- --test-threads=1`.
   Commit: "B5 integration sweep (ht-rzi.8)".

**Deliverable / tests:** `tests/service/trust_policy.rs` and `tests/integration/trust_policy.rs` with the F6
walking skeleton, the four cross-bead checks, the two extra scenarios and the seam check; inline fixes
listed in the report.

---

## Mapping (continued)

Fix loop round 1 (code roast `2026-10-01-b5-trust-policy-guards-roast-pr-1.md`, its step-back
`...-roast-pr-1-step-back.md` and the super-code final review findings in
`docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/scope-filter-round-1-findings.txt`).
Spec: design doc decision 3 as amended by the step-back ("the deciding transaction opens the successor
binding").

| n | bead | task | filesTouched |
|---|---|---|---|
| 10 | ht-rzi.18 | Fix r1: one-step cooperative continuity (redesign) + retryable before reconciliation | src/store/seats.rs, src/protocol/commands.rs, src/cli/hook.rs, src/cli/journal.rs, src/cli/retry.rs, src/harness/context.rs, docs/operations.md, tests/store/control.rs, tests/cli/hook.rs, tests/cli/journal.rs, tests/harness/context.rs, tests/hook_entrypoint.rs, tests/integration/trust_policy.rs |
| 11 | ht-rzi.19 | Fix r1: C4 carry-forward complete (provenance, availability anchor, timers) + reconciliation marker test | src/protocol/authority.rs, src/store/seats.rs, src/store/schema.rs, src/store/messages.rs, src/identity/reconcile.rs, docs/operations.md, tests/store/control.rs, tests/identity/reconcile.rs, tests/service/cooperative.rs, tests/service/worker_health.rs, tests/integration/trust_policy.rs |
| 12 | ht-rzi.20 | Fix r1: C1 diagnostic read off the fenced path | src/identity/repair.rs, src/host/native.rs, tests/service/resolution.rs |
| 13 | ht-rzi.21 | Fix r1: one open-binding query for A4 launch and wake guards | src/protocol/results.rs, src/store/queries.rs, src/store/seats.rs, src/cli/launch.rs, src/cli/human.rs, src/store/wake.rs, src/notification/dispatch.rs, src/host/native.rs, tests/store/queries.rs, tests/cli/launch.rs, tests/store/wake.rs, tests/scheduler/dispatch.rs, tests/cli/hook.rs, tests/cli/cooperative.rs, tests/daemon/control.rs |
| 14 | ht-rzi.22 | Fix r1: single A4 client agent-evidence rule with honored --operator | src/protocol/authority.rs, src/cli/mod.rs, src/cli/me.rs, src/harness/context.rs, src/harness/launch.rs, docs/agent-usage.md, tests/cli/cooperative.rs, tests/harness/context.rs, tests/integration/operator_ux.rs |
| 15 | ht-rzi.23 | Fix r1: protocol version bump for expected_boot + skew docs | src/protocol/wire.rs, src/daemon/lifecycle.rs, src/cli/mod.rs, docs/operations.md, docs/install.md, tests/daemon/lifecycle.rs, tests/cli/cooperative.rs, tests/contracts.rs, tests/daemon/transport.rs |

## Planner notes, fix loop round 1 (read before dispatching)

- **No dependency edges.** All six beads are independent in the bead graph and may run together.
- **Hot files this round.** `src/store/seats.rs` (tasks 10, 11, 13) and `docs/operations.md` (10, 11, 15) are
  each at `hotFileCap: 3`; every edit is local to a different function / paragraph, so rebases stay
  mechanical. Second-tier overlaps: `src/host/native.rs` (12: the `agent.get` read path; 13: the wake
  `bound_harness` check), `src/protocol/authority.rs` (11 and 14 each add one const),
  `src/harness/context.rs` (10 and 14 each add one `ContextJournal` method),
  `tests/integration/trust_policy.rs` and `tests/store/control.rs` (10, 11: appended tests),
  `src/cli/mod.rs` (14: agent-evidence helpers; 15: one protocol check in `connect`),
  `tests/cli/cooperative.rs` (13: one literal; 14 and 15: appended tests) — 3 writers, all additive.
- **Shared helper defined twice on purpose (tasks 10 and 11).** Both need "write the availability anchor and
  start receipt timers" for a binding that just became available. Each task adds the *identical* private
  helper `anchor_seat_availability` (exact body in both sections) directly above `register_available` in
  `src/store/seats.rs`. Whichever merges second keeps one copy during its rebase. Neither task refactors
  `register_cooperative` or `register_available` to call it (keeps task 13's small edit there conflict-free).
- **Task 13 adds a field to `SeatInspection`.** Ten struct literals need `open_binding: None` (two of them in
  `tests/cli/hook.rs` / `tests/cli/cooperative.rs`, which tasks 10 and 14 also edit elsewhere).
- **Task 15 bumps `PROTOCOL_VERSION` to 2.** Tests that hard-code wire/output `"version":1` must use the
  constant; a later rebase of any task that added such a literal does the same.

## Global constraints (fix loop round 1, every task 10-15)

Carried verbatim from the epic `ht-rzi` and `AGENTS.md` (same as tasks 1-9):

- Epic: "Implement the required guards decided in TRUST-POLICY.md (adopted 2026-10-01, branch
  trust-model-invariants). Resolves F6 and the B5 bucket of docs/history/remaining-findings-2026-10-01.md.
  P10 and W5-1 are not here: they are deletions folded into B4 (ht-p03.2). Each child names the policy
  invariant it implements; a change that weakens an invariant must update TRUST-POLICY.md in the same
  commit."
- AGENTS.md: "TRUST-POLICY.md is normative for seat continuity, caller attribution, receipt provenance and
  operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
  `src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
  `src/cli/hook.rs`, launch or wake."
  - "Do not add adversarial caller verification; the model is cooperative and same-user."
  - "Record claims honestly: a new way to attribute an action needs a provenance value defined in the
    policy."
  - "Never merge seats, never move a seat or end a binding on heuristic evidence."
  - "Decide in the daemon against the canonical view (A2); client-local files are hints, never authority."
  - "A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same
    commit."
- Every fix bead says: "Apply the cluster rule to every instance, not only the cited lines." Grep for the
  pattern the cluster names before declaring done.
- Gates for every task: `cargo fmt --all -- --check` and
  `cargo check --locked --all-targets --all-features` must pass; run the task-relevant tests named in the
  section with `cargo test --locked --all-features <filter> -- --test-threads=1`. Several suites are
  macOS-only (`#[cfg(target_os = "macos")]`); run on the host platform.
- Test layout: unit tests live in `tests/<area>/<file>.rs` pulled into `src/` modules with `#[path]`
  (`tests/store/control.rs` from `src/store/control.rs` → filter `store::control::`; `tests/cli/hook.rs` →
  `cli::hook::`; `tests/cli/journal.rs` → `cli::journal::`; `tests/cli/cooperative.rs` → `cli::`;
  `tests/cli/launch.rs` → `cli::launch::`; `tests/harness/context.rs` → `harness::`;
  `tests/store/queries.rs` → `store::queries::`; `tests/store/wake.rs` and
  `tests/store/cooperative_checkin.rs` → `store::`; `tests/identity/reconcile.rs` → `identity::reconcile::`;
  `tests/service/worker_health.rs` → `service::workers::`; `tests/service/cooperative.rs` →
  `service::dispatch::`; `tests/scheduler/dispatch.rs` → `scheduler::`; `tests/daemon/lifecycle.rs` →
  `daemon::lifecycle::`). Integration crates: `tests/service.rs` (`--test service`, includes
  `tests/service/resolution.rs`), `tests/integration.rs` (`--test integration`), `tests/contracts.rs`,
  `tests/hook_entrypoint.rs` (`--features test-support --test hook_entrypoint`).
- When `cargo test` runs inside an agent, `CLAUDECODE` / `CODEX_*` are set in the test process; any test
  that spawns `herdr-threads me init` must `env_remove` them (existing harnesses already do).

---

## Task 10

**Bead:** ht-rzi.18 — Fix r1: one-step cooperative continuity (redesign) + retryable before reconciliation.
TRUST-POLICY C1 (cooperative continuity), pre-reconciliation window part (c).

**filesTouched:** src/store/seats.rs, src/protocol/commands.rs, src/cli/hook.rs, src/cli/journal.rs,
src/cli/retry.rs, src/harness/context.rs, docs/operations.md, tests/store/control.rs, tests/cli/hook.rs,
tests/cli/journal.rs, tests/harness/context.rs, tests/hook_entrypoint.rs, tests/integration/trust_policy.rs
(`src/harness/bridge.rs` only if you choose to put the context-from-reply constructor there; prefer
building the `OccupantContext` literal in `hook.rs`. `src/cli/retry.rs` has no test module of its own; the
`is_continuity_refusal` table test goes in `tests/cli/hook.rs` via `crate::cli::retry::is_continuity_refusal`.)

**Bead description (verbatim).** Redesign (step-back r1, cluster c1-one-step-continuity) plus cluster
pre-reconciliation-window part (c). decide_continuity (src/store/seats.rs ~3745) opens the successor
cooperative_top_level binding (harness, native_session, target, new generation) in the same deciding
transaction, records cooperative_continuity + diagnostic, calls lift_baseline_hold_if_clear, returns
ContinuityReattached{seat, binding_generation} idempotent under the operation key. The hook
(src/cli/hook.rs ~924-947, ~1092, ~1209, ~1356) writes the pane context from the reply, unconditionally
retiring any saved context for that seat or pane; delete the follow-up check_in_seat and
finish_pending_continuity's every-event instance-wide scan; same-hook retry under the operation key only; any
remaining intent read uses O_NOFOLLOW|O_NONBLOCK, is_file, capped read, skips bad entries. While
host_instances.reconciled_boot/epoch lag recovery_boot/epoch, a continuity check-in with no matching
unresolved seat returns a retryable code that keeps the intent (src/cli/retry.rs is_continuity_refusal).
Update docs/operations.md ~127. Dissolves r1 Nits hook.rs:924/journal.rs:580, journal.rs:766/787, evidence
(b). files: src/store/seats.rs, src/protocol/results.rs, src/protocol/commands.rs, src/cli/hook.rs,
src/cli/journal.rs, src/cli/retry.rs, src/harness/bridge.rs, tests/hook_entrypoint.rs,
tests/integration/trust_policy.rs, docs/operations.md.
Roast reports: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-pr-1.md;
step-back: .../2026-10-01-b5-trust-policy-guards-roast-pr-1-step-back.md; spec §3 amended (design doc
decision 3); super-code final review in .superpowers/sdd/ht-rzi-plan/progress.md. Apply the cluster rule to
every instance, not only the cited lines.

**Acceptance criteria (verbatim).** Tests: resume into a pane whose id differs from the seat's saved client
context reattaches and leaves an open cooperative_top_level binding (no 'local context differs' refusal);
lost reply then next event takes the ordinary path with the seat resolved and bound; no per-event intent
scan on tool hooks; daemon kept running across a Herdr restart with resume arriving before reconciliation →
retried, then reattaches once reconciled.

**Cluster rule (step-back, verbatim).** "ContinuityCheckIn is a complete lifecycle check-in: one deciding
transaction rebinds the seat and opens the successor cooperative_top_level binding for the event's session,
returns seat + binding_generation; the hook writes the pane context from the reply, unconditionally retiring
any saved context for that seat or pane; drop the follow-up check_in_seat and the every-event
finish_pending_continuity scan (same-hook retry under the operation key only); any remaining intent read uses
O_NOFOLLOW|O_NONBLOCK, is_file, capped read, skips bad entries; update hook.rs:1209 comment and
docs/operations.md:127." Pre-reconciliation rule: "while host_instances.reconciled_boot/epoch lag
recovery_boot/epoch, readers treat 'not yet reconciled' as pending, not final … a continuity check-in with no
matching unresolved seat returns a retryable code that keeps the intent; test with the daemon kept running
across a Herdr restart." Spec (design doc decision 3, amended): "one transaction rebinds the seat, opens the
successor `cooperative_top_level` binding for the event's harness/session/target with a new generation,
records `cooperative_continuity` and the diagnostic, and lifts the hold if clear; the reply
`ContinuityReattached{seat, binding_generation}` is idempotent under the operation key, and the hook writes
the pane context from it, unconditionally retiring any saved context for that seat or pane. There is no
follow-up check-in. A lost reply is recovered by the committed binding (the next event finds the seat
resolved and takes the ordinary path); the intent is retried only within the same hook, never by a per-event
journal scan. Before the first reconciliation pass of the current recovery epoch (marker of decision 1
lagging), 'no matching unresolved seat' is retryable, not final."

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Fix beads: "Apply the cluster rule to every instance, not only the cited lines."
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]` (module filters are listed in the
  plan's fix-loop Global constraints; the ones this task needs are in its Run step); `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only. Tests that spawn `herdr-threads me init` must
  `env_remove` `CLAUDECODE` / `CODEX_*`.

**Design decisions (planner):**
- *Execution id.* The successor binding needs an `execution_id`. The client chooses it, as for every other
  check-in: add `pub execution: ExecutionId` to `protocol::commands::ContinuityCheckIn` and to
  `cli::journal::SemanticMutation::ContinuityCheckIn` (recorded once in the intent, so every retry under the
  operation key sends the same value), include it in `seats::continuity_digest`. `ContinuityReattachment`
  stays `{seat, binding_generation}`; the hook knows target, harness, session and execution.
- *Retryable code.* Use the existing `ErrorCode::ServiceBusy` with detail
  `"recovery reconciliation of the current host epoch has not finished; retry"` (no new enum variant).
  It is not in `is_continuity_refusal`, so the intent is kept.
- *`StaleHostObservation` is transient for continuity.* Remove it from `is_continuity_refusal` (a fresh
  observation can succeed; during a Herdr restart the daemon's view lags). The hook retries it within its
  deadline; the intent is kept if the deadline passes.
- *What the reattaching hook prints.* After installing the context, present the seat's pending attention
  with one non-durable Current check-in (`bridge::tool_boundary_check_in` with a synthesized
  `EventKind::Tool` copy of the event and `coalesce: false`) — a presentation, not a lifecycle check-in; it
  rotates nothing. Best effort: if it fails, return the reattachment with empty text (the next tool event
  presents).
- *Lost reply across hooks.* No scan. The committed binding is the recovery: the next event finds the seat
  `Resolved`. A tool event with no installed context stays quiet (the existing "no registered execution"
  quiet failure, exit 0); the next lifecycle event (or `herdr-threads retry`) takes the ordinary path. The
  ordinary lifecycle path must not refuse with "local context differs" when the service generation is newer
  than the saved context (step 6). A kept (Uncertain) intent is found again only by the next top-level
  `resume` in the same pane (step 5) or by `herdr-threads retry`.
- *"Saved context for that seat or pane".* Client contexts are per seat (`seat_context_dir`), and a pane is
  located through the daemon's mapping, so the reattached seat's journal is the one to replace; another
  seat's stale context for this pane is never consulted for this pane. Say so in the doc comment.

**Steps:**
1. **RED, store.** In `tests/store/control.rs` (next to `continuity_reattaches_unique_session_match_on_held_target`,
   ~9142) add:
   - `continuity_opens_successor_cooperative_binding_in_the_deciding_transaction`: after `decide_continuity`
     the seat is `resolved` on the target and has exactly one open binding with
     `observation_provenance='cooperative_top_level'`, the command's harness, `native_session`,
     `execution_id`, `generation = seats.generation = reply.binding_generation`, `registered_at` set; one
     `cooperative_continuity` decision; `schema::effective_registered_availability(seat)` is `Some`; one
     `seat_availability` row and one `receipt_timer_materialization` work job for it;
     `seats.unavailability_open = 0`.
   - `continuity_replay_returns_the_same_binding` (extend `continuity_replay_returns_recorded_seat_and_generation`):
     a second `decide_continuity` under the same operation key returns the identical result and adds no
     binding/decision row.
   - `continuity_without_match_is_retryable_until_reconciled`: seed `host_instances` with
     `recovery_boot/epoch` ≠ `reconciled_boot/epoch` and no unresolved seat matching → `ServiceBusy`;
     nothing written under the operation key (verify `execute_idempotent_transaction` stores only Ok
     results; if it stores errors, reject before the idempotent write). Set the marker equal → `NotFound`.
   Update the existing continuity tests' `ContinuityCheckIn` literals with `execution`.
2. **GREEN, store** (`src/store/seats.rs`):
   - `continuity_digest`: add `&command.execution`.
   - `decide_continuity` validate closure: when `continuity_candidates` is empty, read
     `recovery_boot,recovery_epoch,reconciled_boot,reconciled_epoch` from `host_instances`; if they differ
     (`IS NOT`), return `api_error(ErrorCode::ServiceBusy, "recovery reconciliation of the current host epoch has not finished; retry")`,
     else the existing `NotFound`.
   - decide closure, after `rebind_unresolved_seat` and the structural-proof update: insert the successor
     binding at `new_generation` —
     `INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation)`
     with `expected_boot`, `expected_epoch_sql`, `expected_generation_sql` (the values `observed_matches`
     just confirmed and `rebind_unresolved_seat` wrote to the seat), `command.harness.as_str()`,
     `command.native_session`, `command.execution`, `COOPERATIVE_TOP_LEVEL_PROVENANCE`, `at.utc.0` twice,
     and terminal/incarnation from the structural proof when present, else from the fresh effective
     observation (`fresh.terminal_id`, `fresh.incarnation`; check what `mapping.reconfirmation_evidence()`
     yields for an ordinary check-in on the same target and match it). Then
     `UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL`,
     `let seq = schema::next_decision_seq(tx, &instance)?;`,
     `anchor_seat_availability(tx, seat, seq, at.utc, new_generation, COOPERATIVE_TOP_LEVEL_PROVENANCE)?;`,
     `UPDATE seats SET unavailability_open=0 WHERE id=?1`, then the existing decision insert,
     `release_claimed_target` (it already runs `apply_eligibility_transition`) and
     `lift_baseline_hold_if_clear`. Keep the `cooperative_continuity` value in `allocation_decisions` only,
     never on a receipt or the binding.
   - Add the shared helper (identical in Task 11; keep one copy on rebase), directly above
     `pub fn register_available`:
     ```rust
     /// A binding just became available: its availability anchor and the
     /// receipt-timer job for every recipient row already staged for the seat
     /// (same rows as a fresh registration writes).
     fn anchor_seat_availability(
         tx: &Transaction<'_>,
         seat: &SeatId,
         seq: u64,
         at: UtcMillis,
         binding_generation: i64,
         provenance: &str,
     ) -> Result<(), ApiError> {
         tx.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES (?1,?2,?3,?4,?5)",
             params![seat.as_str(), seq as i64, at.0, binding_generation, provenance]).map_err(store_error)?;
         let anchor = tx.last_insert_rowid();
         let high_water: i64 = tx
             .query_row("SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE seat_id=?1", [seat.as_str()], |r| r.get(0))
             .map_err(store_error)?;
         tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'receipt_timer_materialization',?2,?3)",
             params![format!("receipt-timer:{anchor}"), anchor.to_string(), high_water]).map_err(store_error)?;
         Ok(())
     }
     ```
   - Rewrite `decide_continuity`'s and `rebind_unresolved_seat`'s doc comments: the continuity transaction
     opens the successor binding; `rebind_unresolved_seat` still ends the old one (operator repair keeps
     leaving the successor to the agent's next check-in).
3. **Wire + journal** (`src/protocol/commands.rs`, `src/cli/journal.rs`): add `execution: ExecutionId` to
   `ContinuityCheckIn` and `SemanticMutation::ContinuityCheckIn`; map it in `to_command`; update every
   literal (`tests/cli/journal.rs`, `tests/cli/hook.rs`, `tests/hook_entrypoint.rs` `commit_with_lost_reply`
   and the supersede test, `tests/store/control.rs`).
4. **Context journal** (`src/harness/context.rs`): add
   `pub fn install_reattached(&self, context: OccupantContext) -> Result<Option<PendingCheckIn>, ContextError>`:
   `validate_context`, then under the lock: if `pending` is set, move it to `abandoned` (as
   `abandon_pending` does) and return it; set `current = Some(context)` unconditionally (whatever target or
   generation the old one had); save. RED/GREEN test in `tests/harness/context.rs`:
   `install_reattached_replaces_a_context_for_another_pane_and_abandons_pending`.
5. **Hook** (`src/cli/hook.rs`), one-step reattachment:
   - Delete `finish_pending_continuity`, its call before `find_seat` (~924) and the post-`check_in_seat`
     `journal.complete` block; `ContinuityOutcome::Reattached` carries `seat`, `generation`.
   - `reattach_by_continuity(event) -> Option<CheckedIn>` (top-level `resume` with a session only):
     (a) open the journal; look up this pane's pending continuity intent with the hardened
     `Journal::pending_continuity` (below). If one exists for the same harness and session, reuse it (its
     operation key and execution); if it is for another session or unreadable, `complete` it (superseded)
     and record a new one. Record the new intent with a fresh `execution: ExecutionId` (UUID v4).
     (b) `submit_continuity` loop until the hook deadline: `Ok(ContinuityReattached)` → done;
     `ServiceBusy`, `StaleHostObservation`, transport/`UnknownOutcome`/`DeadlineExceeded` → sleep with
     backoff (50 ms doubling, capped at 400 ms, never past the deadline) and resubmit the *same* command;
     `is_continuity_refusal(code)` → `complete` the intent, return `None`.
     (c) On reattachment: build `OccupantContext { format_version: 1, instance, seat, target, harness,
     binding_generation, execution, session: SessionReference::Native(session), role: Role::TopLevel }`,
     `seat_contexts(..).install_reattached(context)`; if it returned an abandoned pending request,
     `journal.complete_operation(&OperationId::new(pending.operation_id.to_string()))`; then complete the
     continuity intent; then present (design decision above) and return `Some(CheckedIn)`.
     If the deadline passes while still retryable, keep the intent and return `None`.
   - In the caller: `absent => match call.reattach_by_continuity(event) { Some(done) => return Ok(done), None => return Err(absent.refusal(pane)) }`.
     The `Resolved` arm no longer carries a continuity tuple.
   - Restore/verify the tool-boundary comment (~1209: "writes nothing to the context or intent journal and
     never replays") — it is accurate again once the scan is gone.
6. **Ordinary path never refuses a historical context** (`lifecycle_check_in`, ~1356): when
   `saved.binding_generation < generation` (the service moved the seat past the context, e.g. a reattachment
   whose reply was lost or that installed in another pane), retire the saved context *before* the
   `saved.target != target` comparison, so "local context differs from current service mapping" is
   returned only when the saved context is not older than the service mapping. Apply the same rule to every
   other place that compares a saved context's target with the pane before the generation (grep
   `local context differs` in `src/cli/`; `run_selected` in `src/cli/mod.rs` is task 14's file — only change
   it if the same ordering bug is there, and note it in the report).
7. **Hardened intent scan** (`src/cli/journal.rs` `pending_continuity`, now called only on the resume path):
   open each `*.intent` with `OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)`
   (cfg(unix); same pattern as `private_open`), skip it unless `metadata().is_file()`, read the header
   line through `.take(64 * 1024)`, and `continue` past `NotFound`, read errors and unparsable headers
   instead of failing the scan. Test in `tests/cli/journal.rs`:
   `pending_continuity_skips_vanished_unparsable_fifo_and_symlink_entries` (a FIFO via `libc::mkfifo`, a
   symlink to `/dev/zero`, a garbage `.intent`, plus one valid intent that is still found).
8. **Retry table** (`src/cli/retry.rs`): drop `StaleHostObservation` from `is_continuity_refusal`; update its
   doc comment ("transient, store, host, uncertain and not-yet-reconciled codes keep the intent"). Add, in
   `tests/cli/hook.rs`, `continuity_refusal_table_is_pinned` asserting every `ErrorCode` variant's classification (refusal:
   InvalidRequest, Unauthorized, Archived, Conflict, OperationPayloadMismatch, MembershipRequired, NotFound,
   TargetAlreadyOwned, TargetUnresolved, TargetUnsafe, CallerUnverified, Unsupported, SequenceExhausted;
   kept: everything else incl. ServiceBusy, StaleHostObservation, UnknownOutcome, DeadlineExceeded,
   StoreBusy, HostUnavailable).
9. **Hook tests** (`tests/cli/hook.rs`, fake client): replace
   `resume_sends_the_seatless_request_and_keeps_the_intent_for_the_follow_up` with
   `resume_reattaches_in_one_request_and_installs_the_context` (one `ContinuityCheckIn` request, no
   `CheckIn` lifecycle request, context installed, intent completed); add
   `service_busy_then_reattached_is_retried_within_the_hook` (fake returns `ServiceBusy` twice, then
   `ContinuityReattached`; one intent, same operation key on all three requests);
   `tool_event_never_scans_or_replays_continuity_intents` (a pending continuity intent for the pane exists;
   a tool event in a pane with a resolved seat sends no `ContinuityCheckIn` and the intent file is untouched).
10. **Entry-point tests** (`tests/hook_entrypoint.rs`, continuity module ~3290-3742):
    - Keep `resume_with_unique_session_match_in_held_pane_reattaches`; adjust its first-line assertion to the
      presentation's opening line if it differs, keep the binding/context/hold/intents assertions.
    - Add `resume_into_a_pane_other_than_the_saved_context_reattaches`: write a saved context for seat `saved`
      with target `old-pane` and an old generation into its context dir before the resume; assert
      reattachment, `current.target == PANE`, an open `cooperative_top_level` binding, and stderr without
      "local context differs".
    - Replace `lost_reply_is_replayed_on_next_hook_event` with
      `lost_reply_next_event_takes_the_ordinary_path`: after `commit_with_lost_reply` the seat is resolved
      with one open `cooperative_top_level` binding; a tool event exits 0 and sends no continuity request
      (intent count unchanged, one `cooperative_continuity` decision); a following SessionStart
      (`startup`) exits 0, writes a context for `saved` at the seat's current generation, no "local context
      differs".
    - Rework `a_different_session_supersedes_a_stale_continuity_intent`: the superseding event is now a
      top-level `resume` of session `S-9` in the pane (tool events no longer read intents); the stale `S-1`
      intent is completed, nothing reattached for `S-1`.
    - Rework `daemon_restart_mid_reattachment_replays_idempotently`: after a lost reply and a daemon restart,
      a second `resume` of `S-1` in the pane reuses the pending intent (same operation key) and the daemon
      replays: still one `cooperative_continuity` decision, context installed at the replayed generation.
11. **Integration** (`tests/integration/trust_policy.rs`, macOS-gated like the file):
    `resume_before_reconciliation_with_daemon_kept_running_is_retried_then_reattaches`: add a `World` helper
    that restarts the stand-in Herdr (`self.herdr.restart(panes)`) **without** `daemon stop/ensure`; fire the
    Claude `resume` hook immediately (before the daemon's next capture/pass); assert exit 0, the seat
    reattached (`cooperative_continuity` row, open `cooperative_top_level` binding on the restored pane), no
    intents left. If the first pass reliably completes before the hook's first request, add a store/service
    level check that the lagging-marker path returned `ServiceBusy` (step 1) and say so in the report.
12. **Docs** (`docs/operations.md`): line ~127 — the tool-boundary check-in again "writes nothing to the
    local journals and never replays"; add one sentence: a resumed session's reattachment is decided in one
    daemon transaction that also opens the new binding; if its reply is lost the seat is already bound and
    the next SessionStart registers normally. Line ~190 (Cooperative continuity): resume before the daemon
    has reconciled the restored Herdr is retried within the hook; if the hook gives up the pane stays held
    and the next `resume` in the pane (or `herdr-threads retry`) finishes it.
13. Run: `cargo test --locked --all-features store::control::continuity -- --test-threads=1`,
    `... harness::`, `... cli::hook::`, `... cli::journal::`,
    `cargo test --locked --all-features --features test-support --test hook_entrypoint continuity -- --test-threads=1`
    (and the whole `hook_entrypoint` crate once), `cargo test --locked --all-features --test integration trust_policy -- --test-threads=1`;
    gates (`fmt --check`, `check --locked --all-targets --all-features`). Commit:
    "One-step cooperative continuity; retry before reconciliation (ht-rzi.18)".

**Deliverable / tests:** continuity opens its binding in the deciding transaction; the hook installs the
context from the reply with no follow-up check-in and no per-event scan; tests named in steps 1, 4, 7-11.

---

## Task 11

**Bead:** ht-rzi.19 — Fix r1: C4 carry-forward complete (provenance, availability anchor, timers) +
reconciliation marker test. TRUST-POLICY C4, C2 marker wiring, pre-reconciliation window part (a).

**filesTouched:** src/protocol/authority.rs, src/store/seats.rs, src/store/schema.rs, src/store/messages.rs,
src/identity/reconcile.rs, docs/operations.md, tests/store/control.rs, tests/identity/reconcile.rs,
tests/service/cooperative.rs, tests/service/worker_health.rs, tests/integration/trust_policy.rs
(`src/service/workers.rs` is cited but needs no code change; touch it only if the loop test cannot reach the
wiring through `FaultyObservationStore`.)

**Bead description (verbatim).** Clusters c4-carry-forward-complete and pre-reconciliation-window part (a),
plus r1 [Should-fix] src/service/workers.rs:1323. carry_binding_forward alone defines carrying: the planner
predicate (src/identity/reconcile.rs ~466-481) and applier (src/store/seats.rs ~1793-1799, ~2273-2285) share
one provenance set; the applier returns Unchanged when it moves no row; a carried binding writes the same
availability anchor and starts the same receipt timers as a fresh registration. Sends before the first
reconciliation pass after daemon ensure must not warn recipient_unavailable for a seat that will be
structurally carried (wait for or optimistically assume the carry); choose and document. Add a test for the
worker-loop wiring that skips record_reconciliation_pass after a pass with a refused (Stale) transition.
files: src/identity/reconcile.rs, src/store/seats.rs, src/store/schema.rs, src/store/messages.rs,
src/service/workers.rs, tests/identity/reconcile.rs, tests/service/cooperative.rs,
tests/integration/trust_policy.rs.
Roast reports: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-pr-1.md;
step-back: .../2026-10-01-b5-trust-policy-guards-roast-pr-1-step-back.md; spec §3 amended (design doc
decision 3); super-code final review in .superpowers/sdd/ht-rzi-plan/progress.md. Apply the cluster rule to
every instance, not only the cited lines.

**Acceptance criteria (verbatim).** Tests: native-provenance binding is not re-planned every pass; send
immediately after daemon ensure (before the pass) to a joined structurally-continuous seat has no
recipient_unavailable warning and its receipt timer starts; receipts staged before carry-forward get timers;
worker skips the marker after a Stale-refused pass.

**Cluster rules (step-back, verbatim).** c4-carry-forward-complete: "carry_binding_forward alone defines
carrying; planner predicate and applier share one provenance set; applier returns Unchanged when it moves no
row; a carried binding writes the same availability anchor and starts the same receipt timers as a fresh
registration; tests for native provenance and availability right after carry-forward."
pre-reconciliation-window: "while host_instances.reconciled_boot/epoch lag recovery_boot/epoch, readers treat
'not yet reconciled' as pending, not final: send availability waits for or optimistically assumes structural
carry-forward …". Final-review evidence (a): "after `daemon ensure`, a send before the first reconciliation
pass sees the recipient unavailable and records recipient_unavailable; carry-forward writes no availability
anchor and starts no receipt timers, while TRUST-POLICY C4 and row O1 say implemented."

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Fix beads: "Apply the cluster rule to every instance, not only the cited lines."
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]` (module filters are listed in the
  plan's fix-loop Global constraints; the ones this task needs are in its Run step); `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only. Tests that spawn `herdr-threads me init` must
  `env_remove` `CLAUDECODE` / `CODEX_*`.

**Design decision (planner): optimistic, not waiting.** A send does not block on the reconciliation pass.
While the instance's reconciliation marker lags, a recipient whose open binding *will be* carried
(structurally continuous) is staged as not yet available but **no `recipient_unavailable` warning is
written**; its receipt timer starts when the carry-forward writes the availability anchor (the anchor's job
covers every recipient row already staged). If the pass ends up not carrying it, the existing paths (seat
unresolved / occupant unavailable) open the episode and later sends warn as today. Document this in
`docs/operations.md` (Daemon restart paragraph) and, if C4's text in TRUST-POLICY.md says availability is
continuous, confirm the wording still holds (no policy weakening expected; if it is, update TRUST-POLICY.md in
the same commit).

**Steps:**
1. **One provenance set.** In `src/protocol/authority.rs` add
   `pub const CARRIED_BINDING_PROVENANCES: [&str; 2] = [COOPERATIVE_TOP_LEVEL_PROVENANCE, OPERATOR_HUMAN_PROVENANCE];`
   with a doc comment ("C4: bindings a structural reconfirmation carries to a new host epoch; native
   `verified_current_target` bindings re-register instead"). Use it in **both** places:
   (a) `carry_binding_forward` (`src/store/seats.rs` ~1799): replace the literal `IN ('cooperative_top_level','operator_human')`
   with `IN (?8,?9)` bound from the const;
   (b) the saved-seat snapshot read that derives `bound_epoch` (`src/store/seats.rs` ~1046): the
   `active_binding` query adds `AND observation_provenance IN (?2,?3)` (from the const), so a native binding
   yields `bound_epoch = None` and the planner (`src/identity/reconcile.rs` ~466-481) never plans
   `CarryForward` for it. Add a comment at the planner predicate naming the const as the single definition.
   Grep for any other `'cooperative_top_level','operator_human'` pair and route it through the const.
2. **Applier returns Unchanged.** Make `carry_binding_forward` return `Result<bool, ApiError>` (moved a row).
   In the `CarryForward` apply arm (~2273) call it *before* the `UPDATE seats SET target_generation` and
   return `Ok(ReconciliationOutcome::Unchanged)` when it moved nothing (no seat update, no lifecycle-revision
   bump). In the `Reconfirm`/`ReconfirmStructure` arm the seat still resolves; just use the bool to decide
   whether to anchor (step 3).
3. **Anchor + timers on carry.** When `carry_binding_forward` moved the binding: read the binding's
   `generation` and `observation_provenance`, `let seq = schema::next_decision_seq(tx, instance)?;`, call
   `anchor_seat_availability(tx, seat, seq, at, generation, &provenance)?` (thread `at: UtcMillis` in from
   the caller's `DecisionInstant`), keep `UPDATE seats SET unavailability_open=0`, and
   `UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL`. Add the shared
   helper exactly as below directly above `pub fn register_available` (Task 10 adds the identical helper;
   whichever merges second keeps one copy):
   ```rust
   /// A binding just became available: its availability anchor and the
   /// receipt-timer job for every recipient row already staged for the seat
   /// (same rows as a fresh registration writes).
   fn anchor_seat_availability(
       tx: &Transaction<'_>,
       seat: &SeatId,
       seq: u64,
       at: UtcMillis,
       binding_generation: i64,
       provenance: &str,
   ) -> Result<(), ApiError> {
       tx.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES (?1,?2,?3,?4,?5)",
           params![seat.as_str(), seq as i64, at.0, binding_generation, provenance]).map_err(store_error)?;
       let anchor = tx.last_insert_rowid();
       let high_water: i64 = tx
           .query_row("SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE seat_id=?1", [seat.as_str()], |r| r.get(0))
           .map_err(store_error)?;
       tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'receipt_timer_materialization',?2,?3)",
           params![format!("receipt-timer:{anchor}"), anchor.to_string(), high_water]).map_err(store_error)?;
       Ok(())
   }
   ```
   Check in `src/store/materialization.rs` (read only) that a `receipt_timer_materialization` job with that
   `high_water` materializes timers for recipient rows staged *before* the anchor while the seat was
   unavailable; if it only covers rows staged as available, extend this task's scope minimally there and list
   the file in the report.
4. **Pending carry is not unavailable** (`src/store/schema.rs` + `src/store/messages.rs`): add
   `pub fn carry_pending(db: &Connection, seat: &str, instance: &str) -> Result<bool, ApiError>`: true iff
   `host_instances.reconciled_boot IS NOT recovery_boot OR reconciled_epoch IS NOT recovery_epoch` and the
   seat is `resolved` with an open registered binding whose provenance is in
   `CARRIED_BINDING_PROVENANCES`, whose `host_boot` equals `host_instances.host_boot`, whose `host_epoch` is
   older than `host_instances.host_epoch`, and whose `incarnation` equals the seat's
   `structural_incarnation` (the structural-continuity evidence C4 carries on). In `stage_recipient`
   (`messages.rs` ~430), when `!available` and `carry_pending(...)` is true, skip the warning block (still
   insert the `prepared_recipients` row with `eligible_at_snapshot = false`). Verify first, with a test, that
   `daemon stop` + `daemon ensure` on the same Herdr advances `recovery_boot/epoch` (seats.rs ~2744 sets
   `recovery_epoch` conditionally). If it does not, gate `carry_pending` on "no completed pass for the
   current `host_epoch`" instead (compare the marker with `host_boot/host_epoch`) and say which in the doc
   comment and the report.
5. **Tests (RED first):**
   - `tests/store/control.rs`: `native_binding_is_never_planned_for_carry_forward` (seed a resolved seat
     with a `verified_current_target` binding, publish a newer epoch with the same terminal/incarnation:
     no `CarryForward` transition; applying a hand-built `CarryForward` returns `Unchanged` and does not bump
     `lifecycle_revision`; a second pass plans nothing for it);
     `carried_binding_writes_availability_anchor_and_timer_job` (cooperative binding carried: one new
     `seat_availability` row at the binding generation, one `receipt_timer_materialization` job whose
     `high_water` covers a recipient row staged before the carry, `unavailability_open = 0`).
   - `tests/identity/reconcile.rs`: `carry_forward_requires_bound_epoch_from_a_carried_provenance` (planner
     with `bound_epoch: None` plans nothing; with the cooperative fixture still plans `CarryForward`).
   - `tests/service/cooperative.rs`: `send_before_first_pass_to_structurally_continuous_seat_has_no_warning`
     (marker lagging, binding one epoch behind, same boot/incarnation → no `prepared_unavailable_warnings`
     row; after the carry applies, the receipt has a timer / `available_at`).
   - `tests/service/worker_health.rs`: `stale_refused_pass_skips_reconciliation_marker` — drive
     `spawn_observation_loop` with `FaultyObservationStore` whose page reports `transitions_refused > 0`;
     assert `pass_records` stays 0 for several loop turns and the marker is unwritten; then a clean page
     records it once (mirror `refusal_free_pass_records_marker_once` ~1292).
   - `tests/integration/trust_policy.rs`: in
     `expected_boot_and_carry_forward_give_no_unknown_outcome_across_stop_ensure` (~1002) remove the wait for
     the reconciliation pass before the send (or add a sibling test without it): send right after
     `daemon ensure` → exit 0, no `recipient_unavailable` warning; after the pass the receipt's timer exists.
6. **Docs** (`docs/operations.md` ~201, Daemon restart): a send right after a daemon restart to a seat whose
   binding is about to be carried does not warn; its receipt timer starts when the carry lands (the first
   reconciliation pass). Remove any sentence that implies the binding is available before the pass.
7. Run: `cargo test --locked --all-features store::control:: -- --test-threads=1` (filter the new names),
   `... identity::reconcile::`, `... service::dispatch::send_before_first_pass`,
   `... service::workers::stale_refused_pass`, `--test integration trust_policy`; gates. Commit:
   "C4 carry-forward: one provenance set, availability anchor, pre-pass sends (ht-rzi.19)".

**Deliverable / tests:** as listed in step 5.

---

## Task 12

**Bead:** ht-rzi.20 — Fix r1: C1 diagnostic read off the fenced path. TRUST-POLICY C1 ("Herdr's agent field
may only suggest").

**filesTouched:** src/identity/repair.rs, src/host/native.rs, tests/service/resolution.rs

**Bead description (verbatim).** Cluster diagnostic-off-fenced-path: r1 [Should-fix]
src/identity/repair.rs:237/235 and [Nit] src/host/native.rs:907. The agent.get diagnostic runs after
decide_continuity's guard is consumed (or before the observation), on an adapter path that never bumps the
NativeCli connection epoch on failure; it can neither expire a guard (MAX_PERMIT_MILLIS 250) nor refuse
another call. files: src/identity/repair.rs, src/host/native.rs.
Roast reports: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-pr-1.md;
step-back: .../2026-10-01-b5-trust-policy-guards-roast-pr-1-step-back.md; spec §3 amended (design doc
decision 3); super-code final review in .superpowers/sdd/ht-rzi-plan/progress.md. Apply the cluster rule to
every instance, not only the cited lines.

**Acceptance criteria (verbatim).** Tests: a diagnostic read delayed past 250 ms does not refuse
reattachment; a diagnostic timeout/unavailable does not bump the NativeCli epoch nor fail a concurrent
fenced host call.

**Cluster rule (step-back, verbatim).** "the C1 agent.get diagnostic is a non-fencing read outside every
freshness window and epoch fence; run it after the guard is consumed (or before the observation) on an
adapter path that never bumps the NativeCli epoch on failure."

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Fix beads: "Apply the cluster rule to every instance, not only the cited lines."
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]` (module filters are listed in the
  plan's fix-loop Global constraints; the ones this task needs are in its Run step); `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only. Tests that spawn `herdr-threads me init` must
  `env_remove` `CLAUDECODE` / `CODEX_*`.

**Design decisions (planner):** run the diagnostic **before** `with_observation` (the decision transaction
must record it, so "after consume" would need a second write). `observe_pane_agent` produces no fenced
observation for any caller (`me init`, person-pane selection, the launch guard, this diagnostic), so the whole
method moves to the non-fencing path; launch keeps failing closed on its own `Err` because it propagates the
error, which is unchanged.

**Steps:**
1. **RED, adapter** (`src/host/native.rs` inline `mod tests`, next to `pane_agent_maps_agent_get_with_session`
   ~1300): `pane_agent_failure_never_bumps_the_connection_epoch` — for a socket that times out
   (fixture never replies; budget/limit short), a socket path that does not exist (HostUnavailable) and a
   reply arriving after the limit (parse-time overrun): `cli.epoch()` is unchanged after
   `observe_pane_agent`, and a following `observe_current_target` with
   `HostCallContext { expected_boot: Some(..), expected_epoch: Some(epoch_before) }` passes `check_context`
   (does not return "host context changed").
2. **GREEN, adapter:** split `dispatch` (~509) so the epoch bump happens in a wrapper: add a
   `fenced: bool` parameter (or a `dispatch_unfenced` sibling) — `run`/`run_witnessed` keep bumping on
   Cancelled/DeadlineExceeded/HostUnavailable/StaleHostObservation; add `fn run_unfenced(&self, args, budget, limit)`
   that never bumps. Add `check_after_parse_unfenced` (same checks, no `fetch_add`). `observe_pane_agent`
   uses both unfenced variants and drops `self.check_context(context)?` only if it would refuse a read that
   carries no expected boot/epoch (it does not today; keep it). Doc comment: "a diagnostic/advisory read: it
   carries no fence and never invalidates the connection epoch".
3. **RED, service** (`tests/service/resolution.rs`, built like the existing `OrdinaryIdentity::new(..)`
   fixtures ~1814/2124): `slow_continuity_diagnostic_does_not_expire_the_guard` — a fake `HostPort` whose
   `observe_pane_agent` sleeps (or advances the fake monotonic clock) by 300 ms (> `MAX_PERMIT_MILLIS` 250)
   and returns `Ok(None)`; a seeded unresolved seat whose last binding holds the session; `continuity(..)`
   returns `ContinuityReattached` and the decision row's `continuity_diagnostic` is `absent`. Also
   `diagnostic_read_error_records_read_error_and_reattaches` (fake returns `Err(HostUnavailable)`).
   If the fixtures cannot reach a store with an unresolved seat cheaply, put the test where the existing
   continuity service path is exercised (`tests/hook_entrypoint.rs` stand-in host with a delayed
   `agent.get`) and list the file in the report.
4. **GREEN, repair** (`src/identity/repair.rs` `continuity`, ~221-250): after the replay check, compute
   `let diagnostic = self.agent_session_diagnostic(&target, &command, budget);` **before**
   `self.with_observation(..)`; inside the closure only build the guard and call `decide_continuity`. Give
   the diagnostic its own bounded budget (min of the request budget and 750 ms) so it cannot consume the
   whole request budget. Update the doc comment: the read happens before the fresh target observation, so
   its latency is outside the guard's freshness window; failures map to `read_error` and never fence.
5. Run: `cargo test --locked --all-features host::native:: -- --test-threads=1`,
   `cargo test --locked --all-features --test service resolution -- --test-threads=1`, and the continuity
   entry-point tests (`--features test-support --test hook_entrypoint continuity`); gates. Commit:
   "C1 diagnostic read outside the guard window and epoch fence (ht-rzi.20)".

**Deliverable / tests:** steps 1 and 3.

---

## Task 13

**Bead:** ht-rzi.21 — Fix r1: one open-binding query for A4 launch and wake guards. TRUST-POLICY A4 (launch
and wake guards).

**filesTouched:** src/protocol/results.rs, src/store/queries.rs, src/store/seats.rs, src/cli/launch.rs,
src/cli/human.rs, src/store/wake.rs, src/notification/dispatch.rs, src/host/native.rs, tests/store/queries.rs,
tests/cli/launch.rs, tests/store/wake.rs, tests/scheduler/dispatch.rs, tests/cli/hook.rs,
tests/cli/cooperative.rs, tests/daemon/control.rs
(`src/cli/human.rs` only if its `SeatInspect` rendering destructures `SeatInspection` exhaustively; the last
three test files only gain `open_binding: None` in `SeatInspection` literals.)

**Bead description (verbatim).** Cluster open-binding-direct: r1 [Should-fix] src/cli/launch.rs:483/482 and
[FYI] src/notification/dispatch.rs:137. One store query (exposed on the wire) returns a seat's open binding
(provenance, harness, target, or none); launch drops the 20x50 SeatInspect paging loop and its bound; wake
treats no open binding as refuse (no prompt) instead of skipping the harness check. files:
src/store/seats.rs or queries.rs, src/protocol/commands.rs, src/protocol/results.rs, src/cli/launch.rs,
src/harness/launch.rs, src/store/wake.rs, src/notification/dispatch.rs, src/host/native.rs.
Roast reports: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-pr-1.md;
step-back: .../2026-10-01-b5-trust-policy-guards-roast-pr-1-step-back.md; spec §3 amended (design doc
decision 3); super-code final review in .superpowers/sdd/ht-rzi-plan/progress.md. Apply the cluster rule to
every instance, not only the cited lines.

**Acceptance criteria (verbatim).** Tests: launch on a seat with >1000 history items still evaluates the
guard; wake for a seat with no open binding sends no prompt; none case of each guard covered.

**Cluster rule (step-back, verbatim).** "one store query returns a seat's open binding (provenance, harness,
target, or none) used by every A4 guard; launch drops the SeatInspect paging loop; wake treats no open
binding as refuse/downgrade; test the none case."

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Fix beads: "Apply the cluster rule to every instance, not only the cited lines."
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]` (module filters are listed in the
  plan's fix-loop Global constraints; the ones this task needs are in its Run step); `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only. Tests that spawn `herdr-threads me init` must
  `env_remove` `CLAUDECODE` / `CODEX_*`.

**Design decision (planner): expose it on the existing `SeatInspect` result**, not a new command (no new
`Command` variant, dispatch route or port method). `SeatInspection` gains
`#[serde(default)] pub open_binding: Option<OpenBindingSummary>`; one `SeatInspect` call with `limit: 1`
answers the guard regardless of history length. (`src/protocol/commands.rs` and `src/harness/launch.rs` from
the bead's list need no change under this choice.)

**Steps:**
1. **Wire type** (`src/protocol/results.rs`): add
   ```rust
   /// A seat's open (not ended) binding, the single answer every A4 guard reads.
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(deny_unknown_fields)]
   pub struct OpenBindingSummary {
       pub provenance: String,
       pub harness: String,
       pub target: HostTargetId,
   }
   ```
   and the `open_binding` field on `SeatInspection`. Add `open_binding: None` to every literal (grep
   `SeatInspection {` in src and tests).
2. **One store query** (`src/store/queries.rs`): `pub(crate) fn open_binding(db: &Connection, seat: &str) -> Result<Option<OpenBindingSummary>, ApiError>`
   = `SELECT observation_provenance,harness,target_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1`.
   Fill `open_binding` in both `SeatInspection` constructions in `seat_inspect` (~897, ~944), read in the
   same query transaction as the summary.
3. **Daemon A4 check uses it** (`src/store/seats.rs` `register_cooperative`, ~4398): replace the inline
   `SELECT observation_provenance,harness FROM occupant_bindings ... ended_at IS NULL` with
   `crate::store::queries::open_binding(tx, seat.as_str())?` (a `Transaction` derefs to `Connection`).
   Behaviour unchanged.
4. **Launch** (`src/cli/launch.rs`): delete `open_binding_from_history` (~480-529) and its page bound;
   the resolver's `open_binding` makes one `SeatInspect { limit: 1 }` call and maps
   `inspection.open_binding` to `OpenBinding { target, provenance }`. Tests in `tests/cli/launch.rs`:
   `launch_guard_reads_open_binding_regardless_of_history_length` (fake client answers a `SeatInspection`
   whose history page has `has_more: true` and >1000 items' worth of cursor, with
   `open_binding: Some(cooperative_top_level, claude, other pane)`; exactly one `SeatInspect` call; the live
   agent check runs and refuses with `target_unsafe`) and `launch_with_no_open_binding_skips_the_live_agent_check`.
   Remove/adjust any test that asserted the old page-bound error.
5. **Wake: no open binding → no authority** (`src/store/wake.rs` `cooperative_authority`, ~345): the
   `None => (None, None)` arm returns `Ok(None)` (no cooperative wake authority without an open binding).
   Update the doc comment ("a live binding, when present" → "the seat's open binding is required"). Test in
   `tests/store/wake.rs`: `cooperative_wake_requires_an_open_binding` (resolved seat, verified observation,
   no open binding → no wake candidate/reservation authority). Fix any existing wake test that relied on the
   no-binding path being wakeable (update its fixture to have a binding, and note it).
6. **Defense in depth** (`src/notification/dispatch.rs` ~136): for `ReservedWakeAuthority::Cooperative`
   with `harness: None`, return `Ok(WakeOutcome::Unsafe)` before any prompt. `src/host/native.rs`
   (`agent_matches_wake_target`-style check ~1001): when `target.basis` is `WakeTargetBasis::CooperativeAgent`
   and `bound_harness` is `None`, return `Err("no bound harness for a cooperative wake")` instead of
   skipping the comparison (registered/`VerifiedOccupant` targets keep their session/execution check).
   Tests: `tests/scheduler/dispatch.rs` `cooperative_reservation_without_harness_is_not_prompted` (fake host
   records no `agent.prompt`); native inline test that a `CooperativeAgent` target with `bound_harness: None`
   is refused.
7. `tests/store/queries.rs`: `seat_inspect_reports_open_binding_and_none` (open cooperative binding →
   summary with provenance/harness/target; after the binding ends → `None`).
8. Run: `cargo test --locked --all-features cli::launch:: -- --test-threads=1`, `... store::queries::`,
   `... store::wake`, `... scheduler::`, `... host::native::`, `--test integration trust_policy`
   (launch/wake cross-bead tests); gates. Commit: "One open-binding answer for the A4 launch and wake
   guards (ht-rzi.21)".

**Deliverable / tests:** steps 4-7.

---

## Task 14

**Bead:** ht-rzi.22 — Fix r1: single A4 client agent-evidence rule with honored --operator. TRUST-POLICY A4
(agent-to-human), client-side best-effort checks.

**filesTouched:** src/protocol/authority.rs, src/cli/mod.rs, src/cli/me.rs, src/harness/context.rs,
src/harness/launch.rs, docs/agent-usage.md, tests/cli/cooperative.rs, tests/harness/context.rs,
tests/integration/operator_ux.rs

**Bead description (verbatim).** Cluster a4-client-heuristic-single-rule: r1 [Nit] src/cli/mod.rs:1194/1192,
[Nit] src/cli/mod.rs:229, [Nit] src/cli/mod.rs:601 (+ spot src/cli/me.rs:197). One shared agent-evidence
predicate (narrowed env markers: CLAUDECODE and the Codex variables Codex sets only inside its own process,
not user-set CODEX_HOME; plus Herdr's agent kind; one error policy) serves me init, person-pane selection
and launch. Wherever a refusal names --operator, that override is honored (record operator provenance in the
human context and honor it in selection_from_context, or change the text). No local context is retired before
the daemon accepts the operator check-in. Update docs/agent-usage.md ~65 wording. files: src/cli/mod.rs,
src/cli/me.rs, docs/agent-usage.md, tests for me init.
Roast reports: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-pr-1.md;
step-back: .../2026-10-01-b5-trust-policy-guards-roast-pr-1-step-back.md; spec §3 amended (design doc
decision 3); super-code final review in .superpowers/sdd/ht-rzi-plan/progress.md. Apply the cluster rule to
every instance, not only the cited lines.

**Acceptance criteria (verbatim).** Tests: CODEX_HOME alone does not refuse; after me init --operator
person-pane commands work; a daemon rejection of the operator check-in leaves the agent's local context
intact.

**Cluster rule (step-back, verbatim).** "one shared agent-evidence predicate (narrowed env markers + Herdr
agent kind, one error policy) for me init, person-pane selection and launch; any refusal naming --operator
honors it; no local context retired before the daemon accepts the operator check-in."

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Fix beads: "Apply the cluster rule to every instance, not only the cited lines."
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]` (module filters are listed in the
  plan's fix-loop Global constraints; the ones this task needs are in its Run step); `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only. Tests that spawn `herdr-threads me init` must
  `env_remove` `CLAUDECODE` / `CODEX_*`.

**Design decisions (planner):**
- *Env markers:* an explicit allowlist, never a prefix: `CLAUDECODE`, `CODEX_SANDBOX`,
  `CODEX_SANDBOX_NETWORK_DISABLED` (the variables codex-rs sets on the commands it spawns). If a local
  `codex` is installed and the sandbox allows it, confirm with
  `codex exec --skip-git-repo-check 'env | grep ^CODEX_'` and add any other variable Codex itself sets
  (never `CODEX_HOME` or other user configuration such as `CODEX_API_KEY`); record what you checked in the
  doc comment.
- *Agent kind:* `pub const HARNESS_AGENT_KINDS: [&str; 2] = ["claude", "codex"];` in
  `src/protocol/authority.rs` plus `pub fn is_harness_agent_kind(kind: &str) -> bool`. `me init`, person-pane
  selection and the launch guard (`src/harness/launch.rs` ~524) all use it (me init today refuses any kind).
- *One error policy for the advisory checks (me init, person-pane selection):* a failed Herdr read is not
  agent evidence (the daemon's A4 refusal is the guard; client-local evidence is a hint, A2). The launch guard
  is the required guard itself and keeps propagating its read error (unchanged; say so in the doc comment).
- *Honoring `--operator`:* record it as a per-seat sidecar in the context journal, keyed by the human
  context's execution (same pattern as `attention_mark`/`set_attention_mark`), written only after the daemon
  accepted the operator check-in; `selection_from_context` skips the agent-evidence checks for a Human
  context whose execution matches the recorded operator mark.
- *No retire before acceptance:* the context journal admits a person's Lifecycle request while the current
  context is an agent's (`request.context.harness == Human && current.harness != Human`) without clearing
  `current`; `dispatch` replaces `current` only on success. A daemon rejection therefore leaves the agent's
  context untouched (the pending human request is abandoned as today).

**Steps:**
1. **Predicate** (`src/cli/mod.rs`): replace `agent_marker` with
   `pub(crate) fn agent_env_marker<K: AsRef<str>, V>(vars) -> Option<String>` (allowlist above) and add
   `pub(crate) fn agent_evidence<K: AsRef<str>, V>(env, read_agent: impl FnOnce() -> Result<Option<PaneAgentObservation>, ApiError>) -> Option<String>`
   returning the evidence text (`"environment variable X is set"` / ``"Herdr reports a `K` agent in this pane"``),
   env first, then the read (only `Ok(Some(obs))` with `is_harness_agent_kind`; `Err` → `None`). Both
   `run_me_init` (`src/cli/me.rs` ~190-201) and `selection_from_context` (~1193-1208) call it; delete their
   inline copies. `src/harness/launch.rs` ~524: `.filter(|kind| crate::protocol::authority::is_harness_agent_kind(kind))`.
2. **Operator mark** (`src/harness/context.rs`): `pub fn operator_mark(&self) -> Option<Uuid>` and
   `pub fn set_operator_mark(&self, execution: Uuid) -> Result<(), ContextError>` (private sidecar file in the
   seat context dir, written atomically like the attention mark; absent/unreadable → `None`).
   `run_me_init`: after `run_selected` succeeds with `operator`, read `contexts.current()` and, if it is a
   Human context, `set_operator_mark(context.execution)`. `selection_from_context` gains
   `operator_override: bool` (caller computes `contexts.operator_mark() == Some(context.execution)`); when
   true the agent-evidence check is skipped.
3. **No retire before acceptance:** in `run_selected` (~601) remove the
   `operator && context.harness != Human && selection.harness == Human` branch of the retire filter (keep the
   agent-over-human branch); for that operator case treat the agent context as absent for seeding (so
   `initial` is built from the service mapping) without calling `retire_current`, and skip the
   "local context differs" comparison for it. In `ContextJournal::prepare_locked` (Lifecycle arm) allow a
   request whose `context.harness == Human` while `current.harness != Human` (person over agent: the daemon
   decides, A4) — keep every other check. `dispatch` already replaces `current` on success.
4. **Refusal text:** `agent_evidence_refusal` still names `herdr-threads me init --operator`, now true for
   person-pane commands too; adjust the wording to "override as the local account with
   `herdr-threads me init --operator` (later commands in this pane then run as you)". Check `ME_INIT_HELP`
   for the same claim.
5. **Tests (RED first):** `tests/cli/cooperative.rs`:
   `codex_home_alone_is_not_agent_evidence` (env `CODEX_HOME=/x` → `agent_env_marker` None; `me init`
   selection proceeds), `codex_sandbox_marker_is_agent_evidence`,
   `herdr_read_error_is_not_agent_evidence`,
   `operator_marked_human_context_is_selected_despite_agent_evidence` and the negative (no mark → refused).
   `tests/harness/context.rs`: `person_lifecycle_request_over_agent_context_keeps_current_until_dispatch`
   (prepare succeeds; a dispatcher returning a definitive rejection → `current` is still the agent context;
   success → `current` is the human one); `operator_mark_round_trips_and_is_execution_scoped`.
   `tests/integration/operator_ux.rs` (built binary; `env_remove` agent markers):
   `me_init_operator_then_person_pane_commands_work` (agent binding + `CODEX_SANDBOX` set in the pane's
   environment → `me init` refused; `me init --operator` succeeds; then `inbox` in that pane with the
   same environment exits 0) and `rejected_operator_check_in_leaves_agent_context_intact` (force a daemon
   Conflict, e.g. a stale generation, and assert the agent's `context.json` `current` is unchanged).
6. **Docs** (`docs/agent-usage.md` ~65, "`me init` refusals"): markers are `CLAUDECODE` and the variables
   Codex sets inside its own process (`CODEX_SANDBOX`, `CODEX_SANDBOX_NETWORK_DISABLED`), not `CODEX_HOME`;
   Herdr's report counts only a Claude or Codex agent; a failed Herdr read is not evidence; after
   `me init --operator` person-pane commands in that pane work.
7. Run: `cargo test --locked --all-features cli:: -- --test-threads=1`, `... harness::`, `--test integration operator_ux`; gates. Commit: "One A4 agent-evidence rule;
   honor me init --operator (ht-rzi.22)".

**Deliverable / tests:** step 5.

---

## Task 15

**Bead:** ht-rzi.23 — Fix r1: protocol version bump for expected_boot + skew docs. TRUST-POLICY A2 (expected
boot) upgrade path.

**filesTouched:** src/protocol/wire.rs, src/daemon/lifecycle.rs, src/cli/mod.rs, docs/operations.md,
docs/install.md, tests/daemon/lifecycle.rs, tests/cli/cooperative.rs, tests/contracts.rs,
tests/daemon/transport.rs
(plus any test that hard-codes the wire/output `version` 1 — grep `"version":1`, `"version": 1`,
`version: 1,` excluding `format_version`; replace with `PROTOCOL_VERSION`; list them in the report.)

**Bead description (verbatim).** r1 [Should-fix] src/protocol/wire.rs:20/9: WireRequest gained expected_boot
without a PROTOCOL_VERSION or package-version bump, so an old daemon's deny_unknown_fields decoder drops every
request from the upgraded binary with no mismatch error. Bump PROTOCOL_VERSION so the Health handshake
reports a mismatch (or make the client omit the field until the daemon advertises support), add a skew note
to docs/operations.md (~78) and soften docs/install.md ~238 'Updating'. files: src/protocol/wire.rs,
src/protocol/ (version const), src/daemon/lifecycle.rs, docs/operations.md, docs/install.md, tests for version
skew.
Roast reports: docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-roast-pr-1.md;
step-back: .../2026-10-01-b5-trust-policy-guards-roast-pr-1-step-back.md; spec §3 amended (design doc
decision 3); super-code final review in .superpowers/sdd/ht-rzi-plan/progress.md. Apply the cluster rule to
every instance, not only the cited lines.

**Acceptance criteria (verbatim).** Test: an upgraded client against a daemon with the previous protocol
version gets a DaemonVersionMismatch (or equivalent) instead of a silent drop/'did not become ready'.

**Roast finding (verbatim evidence).** "main `struct Raw` is `#[serde(deny_unknown_fields)]` with no
`expected_boot`; src/client/local.rs:159 always emits it; old transport.rs:300-301 closes the connection on
decode failure; lifecycle.rs:181 `Err(_) => return Ok(None)` swallows it so neither the protocol check nor
DaemonVersionMismatch fires; `ensure` ends in 'did not become ready'. docs/install.md:238 claims the newer
`stop` reaches the older daemon, which is false for this upgrade; docs/operations.md:78 has a skew note for
the earlier Health change and none is added here."

**Global constraints (epic ht-rzi + AGENTS.md, verbatim).**
- "Each child names the policy invariant it implements; a change that weakens an invariant must update
  TRUST-POLICY.md in the same commit." P10 and W5-1 (and C5) are B4's (`ht-p03.2`); do not touch them.
- "Do not add adversarial caller verification; the model is cooperative and same-user." "Record claims
  honestly: a new way to attribute an action needs a provenance value defined in the policy." "Never merge
  seats, never move a seat or end a binding on heuristic evidence." "Decide in the daemon against the
  canonical view (A2); client-local files are hints, never authority." Read TRUST-POLICY.md before changing
  `src/identity/`, `src/store/seats.rs`, `src/store/receipts.rs`, `src/store/control.rs`,
  `src/protocol/authority.rs`, `src/cli/me.rs`, `src/cli/hook.rs`, launch or wake.
- Fix beads: "Apply the cluster rule to every instance, not only the cited lines."
- Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and the
  task-relevant tests (`cargo test --locked --all-features <filter> -- --test-threads=1`). Unit tests live
  in `tests/<area>/<file>.rs` included into `src/` modules via `#[path]` (module filters are listed in the
  plan's fix-loop Global constraints; the ones this task needs are in its Run step); `tests/service.rs`,
  `tests/integration.rs`, `tests/contracts.rs`, `tests/hook_entrypoint.rs` (`--features test-support`) are
  integration crates. Some suites are macOS-only. Tests that spawn `herdr-threads me init` must
  `env_remove` `CLAUDECODE` / `CODEX_*`.

**Design decision (planner): bump `PROTOCOL_VERSION` to 2.** The descriptor already records the daemon's
protocol and `handshake` (`src/daemon/lifecycle.rs` ~138) and `request_stop` (`src/daemon/control.rs` ~223)
already turn a descriptor mismatch into a definite `UnknownWireVersion` error before any request is sent, so
the bump alone makes `daemon ensure`, `doctor` and `daemon stop` report the skew instead of timing out.
`UnknownWireVersion` with the "daemon protocol 1 differs from executable protocol 2 …" text is the
"equivalent" of `DaemonVersionMismatch` (both exit 3; check `src/cli/exit.rs`).

**Steps:**
1. **RED** (`tests/daemon/lifecycle.rs`): `upgraded_client_against_previous_protocol_daemon_reports_mismatch`
   — publish a descriptor with `protocol_version: PROTOCOL_VERSION - 1` for a live stand-in endpoint that
   closes every connection without replying (the old daemon's decode failure) while the owner lock is held;
   `handshake`/`ensure` returns `Err` with code `UnknownWireVersion` (or `DaemonVersionMismatch`) and a detail
   naming both protocols — never `Ok(None)` / "did not become ready". Reuse the existing descriptor-mismatch
   fixtures in that file if one exists (extend rather than duplicate).
2. **GREEN:** `src/protocol/wire.rs`: `pub const PROTOCOL_VERSION: u16 = 2;` with a comment listing what each
   version added (2: `WireRequest.expected_boot`, B5 commands `OperatorRetire`/`OperatorReplace`/
   `OperatorCheckIn`/`ContinuityCheckIn`, `DaemonBootChanged`). Run the full test suite once to find
   hard-coded `1`s (output envelope `version`, contracts fixtures, transport tests) and switch them to the
   constant.
3. **Every CLI client checks the descriptor first** (`src/cli/mod.rs` `connect`, ~113): after
   `published_endpoint`, if `descriptor.protocol_version != PROTOCOL_VERSION` return the same definite error
   text as `handshake` (`UnknownWireVersion`, "daemon protocol X differs from executable protocol Y; run
   `daemon stop` with the matching older executable, then `daemon ensure` …"), so ordinary commands also get
   the mismatch instead of a dropped request. Leave `src/cli/hook.rs` alone (task 10's file; the hook already
   fails quietly) and note it in the report. Test in `tests/cli/cooperative.rs` (path-included from
   `src/cli/mod.rs`, so `connect` is reachable; append at the end, task 14 also adds tests there):
   `cli_command_against_previous_protocol_descriptor_is_refused_before_send`.
4. **Docs:** `docs/operations.md` (~78, after the Health `notes` paragraph): "Protocol 2 (B5) added the
   request's expected daemon boot and the B5 commands. An older daemon cannot decode a protocol-2 request, so
   `daemon ensure`, `doctor`, `daemon stop` and every command from the newer executable stop with
   `unknown_wire_version` and the stop-then-ensure hint instead of reaching it. Stop the old daemon with the
   old executable first." `docs/install.md` (~238, Updating): soften "the `stop` action from the newer install
   reaches and stops the older daemon" to hold only within one protocol version; across a protocol change the
   newer `stop` refuses (`unknown_wire_version`), so step 1 (stop with the installed executable before
   rebuilding) is required.
5. Run: `cargo test --locked --all-features daemon::lifecycle:: -- --test-threads=1`, `--test contracts`,
   `daemon::` transport tests, `cli::` tests, then the full serial suite once
   (`cargo test --locked --all-targets --all-features -- --test-threads=1`) because the constant is global;
   gates. Commit: "Protocol 2 for expected_boot; version-skew docs (ht-rzi.23)".

**Deliverable / tests:** steps 1 and 3; docs in step 4.

---

## Mapping (continued)

Sweep fix (phase-6 sweep at `fce6e130`, run `docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/run.md`).

| n | bead | task | filesTouched |
|---|---|---|---|
| 16 | ht-rzi.24 | Sweep fix: continuity retry test depends on wall-clock budget | src/cli/hook.rs, tests/cli/hook.rs |

## Planner notes, sweep fix (read before dispatching)

- **Root cause.** `PaneCall::submit_continuity` (`src/cli/hook.rs` ~984-1022) decides whether another
  attempt happens from `self.deadline.saturating_duration_since(Instant::now())` and sleeps with
  `std::thread::sleep`. The test builds the deadline with `Pane::call_within(&daemon, 400 ms)` *before*
  `reattach_by_continuity` records the journal intent (an fsync'd file write). Under the slow serial full
  run that setup can eat the whole 400 ms, so the first reply finds `remaining == 0` and returns `Kept`
  after one request: `requests.len() >= 2` fails. The sibling `service_busy_then_reattached_is_retried_within_the_hook`
  has the same dependence (two real sleeps of 50 ms + 100 ms under a 5 s wall-clock window), just with
  more margin.
- **Seam chosen.** The bead allows a seam in `src/cli/hook.rs` "only if a seam for the clock is needed": it
  is, because no test-only change can make "a second attempt happens" independent of wall-clock time (the
  window is measured from before the journal write). The seam is a one-method retry window on `PaneCall`
  (wait the backoff, or say no attempt fits). Production keeps today's behavior exactly (the hook deadline
  and a real sleep, never past the deadline); tests script how many waits fit and sleep for none.
- Single task, no dependency edges, no shared hot file with any open task.

## Task 16

**Bead:** ht-rzi.24 — Sweep fix: continuity retry test depends on wall-clock budget.

**filesTouched:** `src/cli/hook.rs`, `tests/cli/hook.rs`

### Bead description (verbatim)

"Phase-6 sweep at fce6e130 (log: scratchpad sweep-fce6e130.log; run
docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/run.md) failed
cli::hook::tests::continuity_gate::a_retryable_outcome_is_retried_then_keeps_the_intent_under_its_key
(tests/cli/hook.rs ~2075-2100: assert requests.len() >= 2 'retried within the hook' under call_within 400
ms). Passes 5/5 in isolation; under the slow serial full run only one attempt fit in 400 ms. Make the test
deterministic: drive the retry with an injected clock/attempt budget (or the hook's own retry count) rather
than wall-clock time, keeping its assertions (intent kept, one operation key, same execution). Do not weaken
what it checks. Also check its siblings in continuity_gate for the same wall-clock dependence. files:
tests/cli/hook.rs, src/cli/hook.rs (only if a seam for the clock is needed)."

### Acceptance criteria (verbatim)

"Test passes under the full serial suite and in isolation; no wall-clock-sized budget decides whether a
second attempt happens; assertions unchanged in substance."

### Global constraints

The epic and AGENTS.md constraints under "Global constraints (every task)" and "Global constraints (fix loop
round 1, every task 10-15)" above apply verbatim. In particular: `src/cli/hook.rs` is a TRUST-POLICY-normative
file (read `TRUST-POLICY.md` C1 first); this task changes no policy behavior, so `TRUST-POLICY.md` is not
edited. Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`,
`cargo clippy --locked --all-targets --all-features -- -D warnings` if the repo's CI runs clippy (check
`.github/workflows`), and the tests below with `-- --test-threads=1`.

### Steps

1. **Read first:** `src/cli/hook.rs` `struct PaneCall` (~939), `CONTINUITY_BACKOFF_START`/`_CAP` (~961),
   `PaneCall::submit_continuity` (~984), the production `PaneCall { .. }` literal in the hook entry (~912);
   `tests/cli/hook.rs` `mod continuity_gate` (~1670-2320), especially `Pane::call` / `Pane::call_within`
   (~1791-1809), `a_retryable_outcome_is_retried_then_keeps_the_intent_under_its_key` (~2076) and
   `service_busy_then_reattached_is_retried_within_the_hook`. Confirm with
   `grep -n 'call_within\|PaneCall {' src tests` that `call_within` is used only by the failing test and that
   `PaneCall` is built in exactly two places (the hook entry and `Pane::call_within`).

2. **RED (tests first, in `tests/cli/hook.rs`).**
   a. In `mod continuity_gate`, add a scripted window (no sleeping, counts and records waits):
      ```rust
      /// A retry window that admits a fixed number of waits and never sleeps,
      /// so how many attempts happen is decided by the test, not the clock.
      struct ScriptedWindow {
          left: Mutex<usize>,
          waits: Mutex<Vec<Duration>>,
      }
      impl ScriptedWindow {
          fn allowing(waits: usize) -> Arc<Self> {
              Arc::new(Self {
                  left: Mutex::new(waits),
                  waits: Mutex::new(Vec::new()),
              })
          }
          fn waits(&self) -> Vec<Duration> {
              self.waits.lock().unwrap().clone()
          }
      }
      impl RetryWindow for ScriptedWindow {
          fn wait(&self, backoff: Duration) -> bool {
              let mut left = self.left.lock().unwrap();
              if *left == 0 {
                  return false;
              }
              *left -= 1;
              self.waits.lock().unwrap().push(backoff);
              true
          }
      }
      ```
      (`Arc` comes from `super::*`; if not, add `use std::sync::Arc;` to the module's imports.)
   b. Replace `Pane::call_within(client, window: Duration)` with
      `Pane::call_with(client, retry: Arc<ScriptedWindow>) -> PaneCall<'a>`, which builds the literal with
      `deadline: Instant::now() + Duration::from_secs(5)` (still used only for the scripted daemon's
      `CallBudget`, which `Daemon` ignores) and `retry: retry,` (coerces to `Arc<dyn RetryWindow>`).
      `Pane::call(client)` becomes `self.call_with(client, ScriptedWindow::allowing(8))`: a generous,
      sleep-free allowance, so every sibling that calls `pane.call(..)` no longer sleeps or races a 5 s
      window.
   c. Rewrite `a_retryable_outcome_is_retried_then_keeps_the_intent_under_its_key` over the same six
      failures, keeping every existing assertion and making the attempt count exact:
      ```rust
      let pane = Pane::new();
      let daemon = Daemon::new(vec![failure]).repeating();
      let window = ScriptedWindow::allowing(5);
      assert!(
          pane.call_with(&daemon, Arc::clone(&window))
              .reattach_by_continuity(&resume(Harness::Claude))
              .is_none()
      );
      let kept = pane.pending().expect("the intent is kept");
      let requests = daemon.continuity_requests();
      assert_eq!(requests.len(), 6, "retried within the hook until the window closed");
      assert!(
          requests
              .iter()
              .all(|r| r.operation == kept.operation && r.execution == requests[0].execution)
      );
      assert_eq!(
          window.waits(),
          [50, 100, 200, 400, 400].map(Duration::from_millis),
          "backoff doubles from the start and stops at the cap"
      );
      assert!(pane.saved_context("saved").is_none());
      ```
      Keep the existing `// Kills:` comment and extend it: "...and a retry loop whose backoff does not double
      to its cap." (`requests.len() == 6` is strictly stronger than the old `>= 2`; the operation/execution,
      kept-intent and no-context assertions are unchanged.)
   d. In `service_busy_then_reattached_is_retried_within_the_hook`, build the call with
      `let window = ScriptedWindow::allowing(8);` / `pane.call_with(&daemon, Arc::clone(&window))` and add
      `assert_eq!(window.waits().len(), 2, "one wait before each retry");` next to the existing
      `requests.len() == 3` assertion. Everything else unchanged.
   e. Add a test pinning that a window with no waits left means exactly one attempt and a kept intent (the
      production "deadline already passed" case, which is what flaked):
      ```rust
      // Kills: a retry loop that submits again after the window has closed,
      // or discards the intent when no retry fits.
      #[test]
      fn a_closed_window_submits_once_and_keeps_the_intent() {
          let pane = Pane::new();
          let daemon = Daemon::new(vec![Ok(Err(rejection(ErrorCode::ServiceBusy)))]).repeating();
          assert!(
              pane.call_with(&daemon, ScriptedWindow::allowing(0))
                  .reattach_by_continuity(&resume(Harness::Claude))
                  .is_none()
          );
          assert_eq!(daemon.continuity_requests().len(), 1);
          assert!(pane.pending().is_some(), "the intent is kept");
      }
      ```
   f. Sibling audit: grep the rest of `mod continuity_gate` for `Duration::from_millis`, `Instant::now`,
      `call_within` and `thread::sleep`. Every other test there uses `pane.call(..)` with a single scripted
      reply or no request at all, so after step b none depends on wall-clock time; state this in the report
      with the grep output. Do not touch tests outside `continuity_gate` except step g.
   g. Add production-window tests at the top level of `tests/cli/hook.rs` (next to `budgets_follow_event_mode`,
      ~1237), with one-sided bounds only (no assertion depends on how fast the machine is):
      ```rust
      // Kills: a hook deadline window that retries after its deadline, or
      // sleeps a whole backoff past it.
      #[test]
      fn hook_deadline_window_never_waits_past_the_deadline() {
          // Already passed: no further attempt fits.
          assert!(!HookDeadline(Instant::now()).wait(Duration::from_millis(1)));
          // Far away: the backoff is waited and another attempt fits.
          assert!(HookDeadline(Instant::now() + Duration::from_secs(60)).wait(Duration::from_millis(1)));
          // A backoff longer than the window is cut at the deadline, after which
          // nothing fits (whether or not the first wait still fit under load).
          let window = HookDeadline(Instant::now() + Duration::from_millis(20));
          let started = Instant::now();
          let _ = window.wait(Duration::from_secs(30));
          assert!(started.elapsed() < Duration::from_secs(10), "slept past the deadline");
          assert!(!window.wait(Duration::from_millis(1)));
      }
      ```
   Run `cargo test --locked --all-features cli::hook:: -- --test-threads=1`: it must fail to compile
   (`RetryWindow`, `HookDeadline`, `PaneCall.retry` do not exist yet). That is the RED.

3. **GREEN (`src/cli/hook.rs`).** Directly above `impl PaneCall<'_>` (after `wire_harness`, ~964):
   ```rust
   /// How a retryable continuity submission waits between attempts. The hook
   /// waits against its own deadline; tests script how many waits fit.
   trait RetryWindow: Send + Sync {
       /// Wait `backoff` (never past the window) before the next attempt;
       /// `false`, without waiting, when no further attempt fits.
       fn wait(&self, backoff: Duration) -> bool;
   }

   /// The hook invocation's deadline as a retry window.
   struct HookDeadline(Instant);
   impl RetryWindow for HookDeadline {
       fn wait(&self, backoff: Duration) -> bool {
           let remaining = self.0.saturating_duration_since(Instant::now());
           if remaining.is_zero() {
               return false;
           }
           std::thread::sleep(backoff.min(remaining));
           true
       }
   }
   ```
   (Drop `Send + Sync` if nothing needs it; keep the trait and struct private, `#[cfg(test)]`-free — the
   production path uses them.) Add the field to `PaneCall`:
   ```rust
   /// Paces retries of a continuity submission (the hook deadline in production).
   retry: Arc<dyn RetryWindow>,
   ```
   In the production literal (~912) add `retry: Arc::new(HookDeadline(deadline)),`. In
   `submit_continuity` replace the tail of the loop
   ```rust
   let remaining = self.deadline.saturating_duration_since(Instant::now());
   if remaining.is_zero() {
       return ContinuityOutcome::Kept;
   }
   std::thread::sleep(backoff.min(remaining));
   backoff = (backoff * 2).min(CONTINUITY_BACKOFF_CAP);
   ```
   with
   ```rust
   if !self.retry.wait(backoff) {
       return ContinuityOutcome::Kept;
   }
   backoff = (backoff * 2).min(CONTINUITY_BACKOFF_CAP);
   ```
   Update the `CONTINUITY_BACKOFF_START` doc comment if it still says "never past the hook deadline" (now
   `HookDeadline` guarantees that; keep the sentence accurate). Production behavior is identical: same
   deadline, same backoff sequence, same sleep bound.

4. **Verify.**
   - `cargo test --locked --all-features cli::hook:: -- --test-threads=1` passes.
   - Repeat the formerly flaky test under load to show it no longer depends on timing, e.g.
     `for i in $(seq 1 10); do cargo test --locked --all-features cli::hook::tests::continuity_gate -- --test-threads=1 -q || break; done`
     while a parallel `cargo test --locked --all-features -- --test-threads=8` (or `yes > /dev/null` x
     CPU count, killed afterwards) loads the machine.
   - Acceptance asks for the full serial suite: run
     `cargo test --locked --all-targets --all-features -- --test-threads=1` once and report the
     `continuity_gate` lines and the overall result (other pre-existing failures, if any, are reported, not
     fixed).
   - Gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features` (and clippy if
     CI runs it).
   - Commit: "Drive continuity retry tests with a scripted retry window (ht-rzi.24)".

**Deliverable / tests:** a `RetryWindow` seam on `PaneCall` with the production `HookDeadline`; tests
`cli::hook::tests::continuity_gate::a_retryable_outcome_is_retried_then_keeps_the_intent_under_its_key`
(exact attempt count and backoff sequence, no wall clock),
`cli::hook::tests::continuity_gate::service_busy_then_reattached_is_retried_within_the_hook`,
`cli::hook::tests::continuity_gate::a_closed_window_submits_once_and_keeps_the_intent`, and
`cli::hook::tests::hook_deadline_window_never_waits_past_the_deadline`.
