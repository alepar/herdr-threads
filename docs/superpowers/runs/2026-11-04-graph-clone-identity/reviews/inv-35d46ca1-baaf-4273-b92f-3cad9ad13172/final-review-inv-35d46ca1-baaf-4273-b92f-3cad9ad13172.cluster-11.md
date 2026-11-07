# Final review, cluster 11 (src-cli), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both resolve as commits. The caller checkout was at the head with a clean status at the start and before writing this file, so it was read in place. It was not switched or modified apart from this file and the authorized ledger binding.

Scope: `src/cli/{caller,caller_location,commands,hook,journal,launch,me,mod,retry,site_capability}.rs` (slices 000058 to 000067). Other sources were opened only to check a named risk: `src/store/seats/{continuity,key_verdict,resolution}.rs`, `src/identity/repair.rs`, `src/service/dispatch.rs`, `src/host/{named,hc/mod}.rs`, `src/cli/{seat_resolution,watch,exit}.rs`, `src/harness/{adapter,context}.rs`, `TRUST-POLICY.md` and the test files named below.

## What the cluster must do

Spec section 4.2, client side:
- One helper reads `HERDR_THREADS_CONTINUITY_KEY` for top-level, non-subagent callers only.
- The key is sent only when `seat.continuity_key_v1` is advertised.
- It travels on `ContinuityCheckIn`, `ResolveSeat` and `CheckIn`.
- `ResolveSeat` carries it only for the caller's own pane, never for `launch`.
- A conflict leaves the hook successful with no enrollment and no new text.
- `ServiceBusy` keeps the intent and never falls back to a keyless enrollment.
- `ContinuityKeyPending` is definitive, so nothing is kept.
- A kept claim keeps its key. A digest with no key is unchanged.

## Trace of the hook path

- Production entry is `run_admitted_hook_since`, then `check_in`, then `PaneCall::reattach_by_continuity`, then `reattach_and_install`, then `continuity_intent` and `submit_continuity`. `SemanticMutation::ContinuityCheckIn::to_command` carries the frozen key.
- A reserved, unbound key on an unowned pane:
  - The claim's verdict is `EnrollThenBind`, which falls through to a keyless decision.
  - `SelectedSeat::None` answers `NotFound`, so a lifecycle event runs `resolve_seat` with the key.
  - `settle_key` binds it, and `check_in_seat_with_turn` sends the key again with a `NoOp` verdict.
  - `key_sent_only_when_advertised` in `tests/cli/hook.rs` pins that all three commands carry the key.
- A key bound to an unresolved seat at a pane with no owner: the verdict is `Reattach`, and the store selects `SelectedSeat::Keyed`, recording `cooperative_continuity_key`. The hook installs the context from the reply as for any C1 claim.
- Hold-bearing conflicts:
  - `KeyDecision::refuse` answers `TargetUnresolved` with the `continuity key conflict ` prefix, and `is_continuity_refusal` includes that code.
  - The intent is completed, and `api_failure` returns `Failure::Quiet` on the prefix.
  - A keyless caller on a held pane hits the same prefix through `resolution.rs:222`, also quiet.
  - `conflict_leaves_hook_successful_and_silent` pins the quiet path and the re-presentation.
- Alert-only conflicts answer `TargetAlreadyOwned`, which the resolved arm of `check_in` turns into the owner's ordinary check-in, so T proceeds as the table requires. `resolve_observed` also lets them through and records them with `settle_key`.
- `ContinuityKeyPending` is added to `is_continuity_refusal`, and a claim always has `has_claim` true, so only a claimless `ResolveSeat` meets it. `run_new_api_to_writer_discarding_rejection` completes a `ServiceAllocation` intent on any answered rejection, so nothing is kept there.

## Keyed lifecycle probe of a resolved pane

`probes_resolved_pane` is the late addition recorded in the spec's post-implementation note.
- It runs only for `Lifecycle` mode, with a key that is presented. `Tool` and `Compact` are `Current` mode, so steady turns make no claim; `keyed_start_on_a_current_mapping_takes_the_ordinary_check_in` pins that.
- Each verdict was followed through `decide_continuity`:
  - Lag with an owner gives `ServiceBusy`. The daemon keeps the claim with its key, the hook goes quiet through `pending_failure`, and `decide_kept_continuity` decides later with no event. `cold_restart_keyed_claim_during_lag_needs_no_later_event` drives this on a real daemon.
  - No lag and a current mapping gives `TargetAlreadyOwned` from `target_free`, and the ordinary check-in follows.
- The probe uses `reuse = false`, so a stale replay is never installed on a resolved pane.
- `TRUST-POLICY.md` lines 289-291 state the new probe.

## Unread capability

`continuity_key_presentation` separates `Absent`, `Present` and `Unknown`. `enrollment_key` turns `Unknown` into a quiet end with nothing journaled, on both enrollment routes (`check_in` and `enroll_pane`), so a failed read mints no second seat. `failed_capability_read_ends_quiet_with_no_enrollment` and its enroll-only sibling pin that. A send-only caller reads `Unknown` as no key, which is the one soft edge; it is listed under the deferred items below.

## Plain CLI and own-pane gate

- Every production `SemanticMutation::ResolveSeat` producer was enumerated: `launch.rs::resolve_result`, the `seat resolve` arm, `allocate_recipient_seat` and `me.rs`.
- `launch`, `resolve_named` and `DaemonSeatResolver` pass `None`. The other three go through `own_pane_continuity_key`.
- `MutationSpec::into_command` with `continuity_key: None` is not a live route for resolve: `CallerNeed::None` sends it to the journaled arm in `run_parsed_in_pane`.
- `allocate_recipient_seat` has two call paths, both under an `ActiveGuard`: `mod.rs:1189` (new) and `mod.rs:1681`. The guard added on the operator branch changes no other reader, because no other `caller::active` consumer runs inside that block. `seat resolve` and `me init` run outside the guard and derive the origin through `resolve_arm_key`.
- `Deferred.host` is `Some` only for an hc terminal, whose `tN` id cannot equal a Herdr pane id, and `relative_pane` is `None` for a terminal. So the no-flag branch of `own_server_under` cannot present a key for another server's pane.
- `seat_filter` blocks the variable for library and test callers. `from_lookup` drops an empty value, and `text` drops one that is not UTF-8.

## Named and hc seats

`key_decision` returns `Unreserved`, hence `Ignore`, for any host whose kind is not Herdr, so the daemon ignores a key from either. A keyed lifecycle event on a resolved named or terminal seat does now send one claim. `NamedHost::observe_current_target` answers from a name without dialling anything, and the claim ends in `TargetAlreadyOwned`, so the ordinary check-in follows. This route has no direct test; see the untested items below.

## Journal and wire shape

- The new `continuity_key` fields carry `serde(default, skip_serializing_if)`, so keyless intents keep their bytes.
- `SemanticMutation` has no `deny_unknown_fields`.
- `validate` and `to_command` both carry the key.
- `changed_key_does_not_reuse_the_intent` and `a_keyless_intent_is_not_reused_by_a_keyed_claim` pin intent reuse.
- `ContinuityKeyPending` maps through `api_exit_code` like other request failures.

## Token rules and harness neutrality

No new model-visible string. The new text is `Failure::Quiet` detail, which reaches stderr or a diagnostic only, as the commit message for 7e47b7eb2 states. `enroll_pane` output is a diagnostic only, so its `ServiceBusy` staying `UnavailableDetail` shows the model nothing. No harness name or branch was added to the core files in this cluster.

## Requirement baseline, for this cluster

- B1: delivered on the client side from real entry points: the hook, the watch claim (`watch.rs:1291`, `CallerEnv::for_site`), the plain CLI and `me init`. Daemon judgment, reservation and the mod and Hermes forwarding leave this cluster and are the seam reviewer's to trace.
- B2: the cluster has no retirement logic. Operator `seat retire` reaches the store unchanged as `SemanticMutation::OperatorRetire`, and `tests/store/reversible_retirement.rs` covers finalization by the operator. The retirement path itself is the seam reviewer's to trace.
- B3: no CLI change is involved; memberships belong to the seat. `agent_exit_with_pane_open_keeps_the_seat` exists in the sweep. The seat behaviour is the seam reviewer's to trace.
- B4: the cluster adds no Herdr request or dependency beyond the existing `current_pane` and target reads.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The tests named in this review were read at the head and not re-run, which avoided a separate build; their pass on this tree rests on the caller's full-suite run.
- A keyed lifecycle event on a resolved named or herdr-core terminal seat has no direct test. The code read says it ends in `TargetAlreadyOwned` and the ordinary check-in, but a host with lasting reconciliation lag would turn it into a silent skip of that lifecycle check-in, and this review did not run that case.
- A keyed lifecycle event on a resolved pane during lag where the pane's seat is genuinely current (a daemon restart with Herdr still up) is covered only for the claim and its silence; a fresh idle agent there gets no binding once the worker settles the claim as owned, until its next event.
- `seat resolve`, `me init` and recipient allocation with a real key are covered at the helper level (`site_capability.rs` tests, `on_own_server_tests`), not end to end through the binary for a key bound to another seat.

## Deferred OK
- On `PresentedKey::Unknown` the hook's `continuity_key` reads as no key, so `continuity_intent` supersedes a pending keyed intent and sends a keyless claim for that one event (`src/cli/hook.rs:2504`); enrollment still ends quietly and the next event reads again, but a fresh idle agent whose first capability read fails during lag keeps no key until a later event.
- A subagent or refused-origin caller running `seat resolve` on its own unowned pane presents no key by design and can mint a keyless seat that a later keyed claim then meets as `pane_owned_by_other_seat`; this matches spec 4.2's top-level-only rule, and the path is unusual enough to leave.
- `is_own_pane` compares against the environment's pane id, so a moved pane resolved under its new id through plain `seat resolve` or recipient allocation presents no key; the hook path is not affected because it sends the key with the current target.
- A keyed caller on a daemon without the capability makes a fresh `Capabilities` call on each `continuity_key` read, up to about four per event; this exists only in a mixed-version window between hook and daemon.
- Stale comments: the `reattach_by_continuity` doc in `src/cli/hook.rs` still says it runs for resume and for an event without a resolved pane, and the `ContinuityCheckIn` variant doc in `src/cli/journal.rs` still says resume-only.
