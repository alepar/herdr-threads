# Delivery contract audit

Date: 2026-10-10. Base SHA: `42593b0b` (`super-auto/agy-harness-adapter`). Bead: ht-poh.7.

Contract: root spec section "Delivery contract" (six rules), as copied into `AGENTS.md`.

## Method

Read, for each rule, the producer (CLI send), the store (message validation, attention, wake candidate and batch
window), the scheduler (wake and poke admission, mod routing), the service (mod channel flag, digest), the notification
dispatcher (idle and composer gating) and the hook/watch delivery paths. Existing tests that pin each rule were located
by searching `tests/scheduler/`, `tests/service/`, `tests/store/`, `tests/cli/` and `tests/integration/`. No test was
run for the audit; the verdicts rest on the cited code and on the cited tests being present at the base SHA. Line
numbers are at the base SHA.

Verdict vocabulary: `holds`, `divergence - fixed here`, `divergence - recorded`.

## Rule 1. Messages are lazy by default

Enforcement:
- `src/cli/commands.rs:2443-2450`: `send` is `Lazy` unless `--nudge` or an explicit ACK recipient
  (`--require-ack`, `--require-ack-pane`) is given.
- `src/cli/journal.rs:303,438` and `src/cli/follow.rs:1421` build `Lazy` sends.
- `src/cli/handoff.rs:245` builds an `Ordinary` send, but it always invites its recipient (an ACK-required message),
  so rule 5 makes it nudge-class; consistent with the contract.
- Wire default: `DeliveryMode` derives `Default` as `Ordinary` (`src/protocol/commands.rs:752-756`) and the field is
  omitted from the wire when ordinary (`:780-781`). A request that omits the field (an older client) is ordinary.
  This is a frozen wire shape, not a producer default; the CLI always states its mode explicitly. See DC-1.

Pinning tests: `tests/cli/lazy_send.rs` `lazy_send_default_and_nudge_ack_selection`,
`lazy_send_default_refuses_deadline_and_explicit_mode_conflicts`, `ordinary_send_omits_mode_and_preserves_digest`.

Verdict: holds for every producer in this repository; the wire default is a recorded, accepted divergence (DC-1).

## Rule 2. Lazy messages never wake an idle session (unless `wake_on: all`)

Enforcement:
- `src/store/messages.rs:87-97` (`validate_delivery_options`): a lazy send cannot carry invited recipients or a
  deadline, so a lazy message can never become an invitation, a receipt obligation or a deadline.
- Wake attention is built only from pending invitations, pending ACK-required receipts and actionable warnings
  (`src/store/attention.rs:428-438` `pending_receipts` filters `ack_required=1`; `wake_seat_attention` at `:1068`;
  `src/store/wake.rs` `load_candidate` at `:40`, `reason_labels` at `:432`). Lazy recipients live in
  `lazy_recipients` (`src/store/lazy_delivery.rs`) and are not read by any of these.
- Wake batching (`src/store/wake.rs:837` `oldest_ordinary_at`, `:895` `batch_window`) looks only at invitations and
  ACK-required receipts, so a lazy row never opens or extends a batch window.
- Lazy pending work is surfaced only as a count in the attention digest (`lazy_pending`, `src/ports.rs:2494`,
  `src/store/seats.rs:4627`), which hooks and the mod read on a turn boundary.
- `wake_on: all` does not exist yet; ht-poh.3.5/3.6 introduce it. Until then there is no opt-in and no exception.

Pinning tests: `tests/integration/lazy_attention.rs` `lazy_attention_no_wake_poke_prompt_retry_deadline_warning`,
`lazy_attention_no_indirect_outstanding_send_work`, `lazy_attention_mixed_ordinary_positive_controls` (positive
control); `tests/cli/lazy_send.rs` `lazy_send_rejects_ack_seats_panes_and_deadline`;
`tests/store/lazy_schema.rs` `lazy_pending_scan_bounds_unpublished_work`.

Verdict: holds.

## Rule 3. Turn hooks and channels deliver every pending item directly into context

Enforcement:
- Claude mod channel: `src/cli/watch.rs:583-616` emits ordinary items and `LazyMessage` items (bodies) into the watch
  stream; `src/service/dispatch.rs:797-801` sets `mod_channel_live` on the digest so hooks do not duplicate it.
- Hooks (Codex, Hermes, Claude without the mod): `src/harness/bridge.rs:1212,1267` and `src/cli/hook.rs:2474` present
  the attention digest (counts, including lazy) and ready-to-run commands, and skip the offer when a channel is live.
  Bodies are not injected; the model runs the ready command (one extra `herdr-threads` call).

Pinning tests: `tests/cli/watch.rs`, `tests/protocol/lazy_inbox_v2.rs`, `tests/service/mod_channels.rs` and
`tests/service/mod_digest_flag.rs` (DP10-frozen), `tests/integration/mod_delivery.rs` `lazy_row_appended_then_completed`,
`tests/integration/token_diet.rs`.

Verdict: holds for the Claude mod channel. Hook harnesses deliver the digest and commands, not bodies: recorded as an
accepted divergence "as far as the harness allows" (DC-2); the root spec lists delivery policies for Codex and Hermes
as a non-goal.

## Rule 4. A nudge wakes a session only when it is idle

Enforcement:
- Wake targeting: `TargetState::may_hint` requires `HostUiState::Idle` (`src/notification/policy.rs:105-108`); the
  cooperative wake identity excludes `ActiveTurn` and `ApprovalOrQuestion` (`src/ports.rs:2064-2067`); typed input is
  refused by the dispatcher (a composer must be empty; drafts are never stashed for a wake).
- Scheduler gate: `WakeRunner::try_candidate` (`src/scheduler/mod.rs:676`) reserves nothing for a seat without
  actionable work, a resolved continuity or a target, and skips a seat served by a live mod channel (`:703`).
- Pokes (deadline reminders typed into the composer): `poke_eligibility` (`src/notification/policy.rs:355-380`) would
  submit during `ActiveTurn` when the harness declares `poke_during_turn`. In production no harness declares it:
  `ObservedPokeCapabilities::capabilities` returns `PokeCapabilities::NONE` for every harness
  (`src/app.rs:164-167`), so an active-turn poke is skipped and stays pending until the seat is idle. The historical
  Claude declaration (`src/harness/claude.rs:163`, asserted at `src/app.rs:1877-1878`) is test-only data.
- Is a deadline poke a "nudge" under rule 4? Yes: it is a typed prompt into the composer that interrupts the model
  exactly as a wake does, so rule 4 applies to it. It also carries only ACK-required receipts (rule 5), never lazy
  mail. The production wiring already honours rule 4; the dormant `poke_during_turn` branch is the only place the
  policy code would not (DC-3).
- Claude mod channel: whether the mod submits a turn only when idle is decided in the mod (frozen at v0.5.0, R9) and
  is not verifiable from core code (DC-4).

Pinning tests: `tests/scheduler/dispatch.rs` `active_attention_is_deferred_even_with_turn_capability_then_idle_delivers`,
`blocked_or_human_input_target_is_never_prompted`, `nonempty_composer_skips_before_stash_or_restore`,
`active_turn_pokes_defer_receipts_until_idle_even_with_declared_support`; `tests/scheduler/poke_flow.rs`
`active_turn_pokes_remain_pending_even_with_the_capability`, `drafts_never_stash_even_with_test_only_capabilities`;
`tests/scheduler/policy.rs` (poke eligibility matrix, `:579`).

Verdict: holds in production wiring. Dormant policy branch recorded (DC-3); mod-side idleness recorded (DC-4).

## Rule 5. Messages that require an ACK are always nudge-class

Enforcement:
- `src/cli/commands.rs:2443-2447`: any `--require-ack`, `--require-ack-pane` or `--nudge` yields `Ordinary`.
- `src/store/messages.rs:87-97`: the reverse direction; `Lazy` plus recipients or a deadline is rejected.
- Wake reasons are invitations, ACK-required receipts and actionable warnings (`src/store/wake.rs:432`
  `reason_labels`); an ACK-required receipt is always `Ordinary`. Invitations (including handoff, `src/cli/handoff.rs:245`)
  and warnings are also nudge-class, which extends rule 5 without violating it.

Pinning tests: `tests/cli/lazy_send.rs` `lazy_send_default_and_nudge_ack_selection`,
`lazy_send_rejects_ack_seats_panes_and_deadline`; `tests/store/messages.rs`; `tests/store/mod_ack.rs`.

Verdict: holds.

## Rule 6. Who performs an idle wake is the only per-harness variation

Enforcement:
- `Channel`: `src/service/dispatch.rs:67` `seat_has_mod_channel` (`ModChannels::seat_live`), `:797` digest flag.
- Scheduler routing: `mod_suppressed` (`src/scheduler/mod.rs:610`) returns true for a live, non-stalled channel; it
  gates `try_candidate` (`:703`), `drive_pokes` (`:290`) and `try_poke` (`:936`). A stalled, absent or removed channel
  falls back to the `Native` path with the ladder untouched, and channel removal kicks the wake lane.
- Wake admission, batching, retry ladder and the idle gate (rule 4) are the same code for every harness; only the
  `Native` marker typing differs per adapter.

Pinning tests: `tests/scheduler/mod_routing.rs` (`wake_not_reserved_while_channel_live_and_ladder_unchanged`,
`wake_suppressed_when_the_seat_has_a_channel_of_another_generation`, `native_ladder_runs_while_channel_stalled`,
`wake_resumes_after_channel_removed`, `poke_skipped_while_live_and_runs_while_stalled`,
`try_poke_rechecks_the_channel_before_reserving`).

Verdict: holds. The channel flag is a mod-channel registry query, not a harness-id branch; a future `Channel` adapter
other than Claude's mod would need `ModChannels` to be keyed by adapter policy (ht-poh.3.x), which is a design item for
that work, not a divergence today.

## Divergences

| ID | Rule | Site | Verdict | Fix commit or reason | Punch-list owner |
|----|------|------|---------|----------------------|------------------|
| DC-1 | 1 | `src/protocol/commands.rs:752-756,780-781` wire default `Ordinary` | divergence - recorded | Frozen wire shape: an absent field must decode as ordinary for pre-lazy clients (R9/wire compat). Every in-repo producer states its mode. Permanent. | none (permanent) |
| DC-2 | 3 | `src/harness/bridge.rs:1212,1267`, `src/cli/hook.rs:2474` | divergence - recorded | Hook harnesses get digest and ready commands, not bodies, "as far as the harness allows"; body delivery for Codex/Hermes is a root-spec non-goal. | follow-up (per-harness delivery policy work) |
| DC-3 | 4 | `src/notification/policy.rs:375-377` `ActiveTurn` poke branch | divergence - recorded | Dormant: production capability source is `NONE` (`src/app.rs:164-167`), so no poke is typed during a turn. Not in scope for a scheduler/service fix; the branch and the Claude `poke_during_turn` declaration (`src/harness/claude.rs:163`) contradict rule 4 if ever wired. | ht-poh.3.3 (delivery policy) or follow-up: remove the branch or gate it behind the policy trait |
| DC-4 | 4 | Claude mod idleness decision (mod side) | divergence - recorded | Not verifiable from core; the mod is frozen at v0.5.0 (R9, DP10). | none (R9) |

No divergence met the fix criteria (small, inside `src/scheduler/**` or `src/service/**`, no change to the v0.5.0 mod
wire or behaviour, no TRUST-POLICY change), so no Rust was changed.

## Punch list for the run report

- DC-1 (rule 1): wire default `DeliveryMode::Ordinary` is frozen for compatibility; accepted, permanent.
- DC-2 (rule 3): Codex, Hermes and Claude-without-mod hooks deliver a digest plus ready commands, not bodies; accepted
  as "as far as the harness allows"; revisit only with per-harness delivery policies.
- DC-3 (rule 4): `poke_eligibility` still has an `ActiveTurn` submit branch gated by `poke_during_turn`; production
  declares none, so it is dormant. Remove or route through the delivery policy in ht-poh.3.3 or a follow-up.
- DC-4 (rule 4): the Claude mod's own idle-vs-in-turn decision cannot be audited from core; frozen by R9.
- Note for ht-poh.3.5/3.6: rule 2's `wake_on: all` exception has no code today; its introduction must change wake
  attention (`src/store/attention.rs` `wake_seat_attention`) and `batch_window`, and add tests for both the opt-in and
  the default.
