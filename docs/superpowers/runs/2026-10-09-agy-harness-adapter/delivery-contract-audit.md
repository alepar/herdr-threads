# Delivery contract audit

Date: 2026-10-10 (redone after the main v0.6.0 absorb). Base SHA: `e60263e5` (`super-auto/agy-harness-adapter`). Beads: ht-poh.7, ht-poh.17.

Contract: root spec section "Delivery contract" (six rules), as copied into `AGENTS.md`.

## Method

Read, for each rule, the producer (CLI send), the store (message validation, attention, wake candidate and batch
window), the scheduler (wake and poke admission, mod routing), the service (mod channel flag, digest), the notification
dispatcher (idle and composer gating) and the hook/watch delivery paths. Existing tests that pin each rule were located
by searching `tests/scheduler/`, `tests/service/`, `tests/store/`, `tests/cli/` and `tests/integration/`. No test was
run for the audit; the verdicts rest on the cited code and on the cited tests being present at the base SHA (every named test was
re-located by `rg` for this redo). Line numbers are at the base SHA, so they differ from the first audit, which
predates the v0.6.0 absorb; the changes of this task (`hook_inbox_page`, the `control.rs` guard) are cited at
their post-change lines.

Verdict vocabulary: `holds`, `divergence - fixed here`, `divergence - recorded`.

## Rule 1. Messages are lazy by default

Enforcement:
- `src/cli/commands.rs:2202-2209`: `send` is `Lazy` unless `--nudge` or an explicit ACK recipient
  (`--require-ack`, `--require-ack-pane`) is given.
- `src/cli/journal.rs:304,439` and `src/cli/follow.rs:1421` build `Lazy` sends.
- `src/cli/handoff.rs:181` builds an `Ordinary` send, but it always invites its recipient (an ACK-required message),
  so rule 5 makes it nudge-class; consistent with the contract.
- Wire default: `DeliveryMode` derives `Default` as `Ordinary` (`src/protocol/commands.rs:766-770`) and the field is
  omitted from the wire when ordinary (`:845-846`). A request that omits the field (an older client) is ordinary.
  This is a frozen wire shape, not a producer default; the CLI always states its mode explicitly. See DC-1.

Pinning tests: `tests/cli/lazy_send.rs` `lazy_send_default_and_nudge_ack_selection`,
`lazy_send_default_refuses_deadline_and_explicit_mode_conflicts`, `ordinary_send_omits_mode_and_preserves_digest`.

Main v0.6.0 changed nothing here. Verdict: holds for every producer in this repository; the wire default is a recorded, accepted divergence (DC-1).

## Rule 2. Lazy messages never wake an idle session (unless `wake_on: all`)

Enforcement:
- `src/store/messages.rs:87-97` (`validate_delivery_options`): a lazy send cannot carry invited recipients or a
  deadline, so a lazy message can never become an invitation, a receipt obligation or a deadline.
- Wake attention for a default (`nudge`) seat is built only from pending invitations, pending ACK-required receipts and
  actionable warnings (`src/store/attention.rs:492` `pending_receipts` filters `ack_required=1`; `wake_seat_attention`
  at `:1133`; `src/store/wake.rs` `load_candidate` at `:44`, `reason_labels` at `:437`). Lazy recipients live in
  `lazy_recipients` (`src/store/lazy_delivery.rs`).
- What main v0.6.0 changed: the lazy digest key (`AttentionDigestQuery.lazy`, `src/harness/bridge.rs` `read_digest`
  at `:1041`, and the delivery digest at `:1180`) moves only the hook's change marker, never wake attention: the
  wake frontier carries a lazy key only for a `wake_on: all` seat (`src/store/attention.rs:1147-1153`, `nudge`
  gives `None`), pinned by `tests/integration/lazy_attention.rs`
  `lazy_attention_moves_the_lazy_digest_marker_but_never_wake`.
- The `wake_on: all` exception now has code (ht-poh.3.6): `src/store/attention.rs:1067-1069` (`has_wake_rows` for an
  `all` seat), `:1147-1153` (lazy wake frontier), `src/store/wake.rs:482-485` (the reservation re-reads the lazy
  frontier and `wake_on`) and `:947-952` (`batch_window` batches the oldest pending lazy row like an ordinary receipt;
  `oldest_ordinary_at` is at `:853`). This replaces the first audit's "Note for ht-poh.3.5/3.6". A `nudge` seat
  never reaches any of these lazy branches, so the default is unchanged.
- Wake batching (`src/store/wake.rs:911` `batch_window`) otherwise looks only at invitations and ACK-required
  receipts.
- Lazy pending work is surfaced as a count in the attention digest (`lazy_pending`, `src/ports.rs:2531`,
  `src/store/seats.rs:4623,4638`), which hooks and the mod read on a turn boundary.

Pinning tests: `tests/integration/lazy_attention.rs` `lazy_attention_no_wake_poke_prompt_retry_deadline_warning`,
`lazy_attention_no_indirect_outstanding_send_work`, `lazy_attention_mixed_ordinary_positive_controls` (positive
control); `tests/store/wake_on_wake.rs` and `tests/store/lazy_inbox.rs` `lazy_inbox_wakes_follows_wake_on_and_mod_ack_completes_it`
(the `all` opt-in and the `nudge` default); `tests/cli/lazy_send.rs` `lazy_send_rejects_ack_seats_panes_and_deadline`;
`tests/store/lazy_schema.rs` `lazy_pending_scan_bounds_unpublished_work`.

Verdict: holds.

## Rule 3. Turn hooks and channels deliver every pending item directly into context

Enforcement:
- Claude mod channel: `src/cli/watch.rs:564` emits ordinary items and `LazyMessage` items (bodies) into the watch
  stream; `src/service/dispatch.rs:817` sets `mod_channel_live` on the digest so hooks do not duplicate it.
- What main v0.6.0 changed: standard hooks now carry lazy bodies. For Claude without a live mod, Codex and Hermes the
  presenting top-level hook (marker advanced, or a lifecycle check-in) reads one bounded v2 inbox page
  (`src/cli/hook_inbox.rs:55` `read_page`) and appends the complete items to its context
  (`src/cli/hook.rs:2114-2124` at a tool boundary, `:2176-2189` at a lifecycle check-in), then completes the lazy rows
  shown whole with `cooperative_hook_context` after its stdout was flushed. A live mod channel keeps the page out
  (`src/harness/bridge.rs:1218-1226`, `src/cli/hook.rs:2588-2593`).
- Codex turn start (ht-mu3, 2026-10-11): a text-only Codex turn makes no tool call, so with only SessionStart and
  Bash PreToolUse owned, a pending lazy item waited for the next tool call, wake or session start. The Codex adapter
  now also owns `UserPromptSubmit` (`src/harness/codex.rs` `DECLARATION`), decoded as the same Current check-in as a
  tool boundary and answered under its own `hookEventName`; a quiet turn coalesces to empty output. A Codex that never
  fires it degrades to tool-boundary delivery. Pinned by `tests/hook_entrypoint.rs`
  `lazy_mail_reaches_the_next_codex_user_prompt_hook`.
- Claude hooks-only turn start (ht-turnstart, 2026-10-23): without the mod, a woken Claude got the `inbox` marker and
  spent a Bash `inbox` round trip, because no turn-start hook injected. The Claude adapter now also owns
  `UserPromptSubmit` (`src/harness/claude.rs` `declared_hook_groups`), the same Current check-in as the Bash boundary
  (a live mod channel still keeps the page out), and a `cooperative_top_level` binding's native wake is the
  content-free `herdr-threads: wake up`, as for Codex (rules 3 and 6; rules 1, 2, 4 and 5 unchanged: wake eligibility,
  idle and draft guards are untouched). Hermes' native wake text follows the same rule (`pre_llm_call` injects). An
  install that predates the hook keeps working: the stale-install finding asks for `setup claude`, and a bare wake
  falls back to the skill's one `inbox` call. Pinned by `tests/hook_entrypoint.rs`
  `lazy_mail_reaches_the_next_claude_user_prompt_hook`, `tests/scheduler/wake_text.rs`
  `hook_evidence_keeps_recovery_and_parked_inbox_hints` and the opt-in wake e2e case `hooks-wake/group`.
- Codex turn end (ht-end-of-turn, 2026-10-23): an item at p <= T (or a receipt, invitation or actionable warning)
  that arrived after the turn's last tool boundary waited for idle plus the native wake (30 s spacing, 60 s more on a
  focused pane). The Codex adapter now owns `Stop`: the hook asks the daemon `TurnEndDue` (`WakeCandidate::owes_turn_end`,
  the idle-wake predicate without the reservation frontier) and, only when due, checks in like a tool boundary and
  answers `{decision:"block", reason: <page>}`; Codex records the reason as a user-role hook prompt and samples again
  in the same turn, without running `UserPromptSubmit`. Rules: 1-2 (a p > T item never continues a turn; the page then
  carries every pending item, rule 3), 4 (in-turn, no nudge; the delivered items are then offered, so no idle wake
  follows), bounded (no continuation when `stop_hook_active`, so at most one per turn; a payload without the flag is
  treated as continued), quiet stops print nothing. Pinned by `tests/hook_entrypoint.rs`
  `codex_stop_continues_once_for_a_waking_item_and_stays_quiet_otherwise`.
- Hook-pull seats (agy) carry bodies only through `DeliveryPull` (`src/service/delivery_pull.rs`), completed by
  `DeliveryConfirm`. The decision is `src/delivery/routing.rs:35` `hook_inbox_page`: a hook-pull seat's standard hook
  reads no page at either boundary, and the daemon enforces it
  (`src/store/control.rs:1961-1971` refuses `via: hook_context` from a hook-pull binding). This closes the double
  delivery that the merge left at the lifecycle check-in (a hook-pull seat would have received its lazy row both as
  an appended page and as a `DeliveryPull` offer).
- Ordinary (non-lazy) bodies still arrive as digest and ready commands for hook harnesses
  (`src/harness/bridge.rs:1218-1291`, `src/cli/hook.rs:2588-2630`); a hook-pull seat's non-lazy bodies come through
  `DeliveryPull` as well.

Pinning tests: `tests/cli/watch.rs`, `tests/protocol/lazy_inbox_v2.rs`, `tests/service/mod_channels.rs` and
`tests/service/mod_digest_flag.rs` (DP10-frozen), `tests/integration/mod_delivery.rs` `lazy_row_appended_then_completed`,
`tests/integration/token_diet.rs`. Hook bodies: `tests/hook_entrypoint.rs` `lazy_mail_moves_the_digest_and_the_next_tool_hook_injects_inbox_rows`
(Claude) and `lazy_mail_reaches_the_next_codex_tool_hook`, `tests/cli/hook.rs`
`hermes_qualified_turn_injects_inbox_page_and_defers_lazy_completion`, `tests/cli/hook_inbox.rs`. Hook-pull exactly
once: `tests/delivery/routing.rs` `hook_inbox_page_is_never_read_for_a_hook_pull_seat`, `tests/cli/hook_pull_lazy.rs`
(`hook_pull_lifecycle_check_in_reads_no_inbox_page`, `hook_pull_tool_boundary_reads_no_inbox_page`),
`tests/service/lazy_hook_pull.rs` (`hook_pull_lazy_row_is_delivered_exactly_once_through_pull`,
`hook_pull_seat_text_inbox_completion_still_works`, `claude_seat_hook_context_completion_per_main`).

Verdict: holds for the Claude mod channel, for Claude-without-mod, Codex and Hermes hooks (lazy bodies in context), and
for hook-pull seats (lazy bodies through `DeliveryPull`, exactly once). Non-lazy bodies on the hook harnesses arrive as
digest plus ready commands, "as far as the harness allows": recorded (DC-2, narrowed); the root spec lists delivery
policies for Codex and Hermes as a non-goal.

## Rule 4. A nudge wakes a session only when it is idle

Enforcement:
- Wake targeting: `TargetState::may_hint` requires `HostUiState::Idle` (`src/notification/policy.rs:104-110`); the
  cooperative wake identity excludes `ActiveTurn` and `ApprovalOrQuestion` (`src/ports.rs:2062-2067`); typed input is
  refused by the dispatcher (a composer must be empty; drafts are never stashed for a wake).
- Scheduler gate: `WakeRunner::try_candidate` (`src/scheduler/mod.rs:676`) reserves nothing for a seat without
  actionable work, a resolved continuity or a target, and skips a seat served by a live mod channel (`:703`). A
  `wake_on: all` seat's lazy row reaches this gate like any other wake reason, so it is subject to the same idle
  gate.
- Pokes (deadline reminders typed into the composer): `poke_eligibility` (`src/notification/policy.rs`) skips every
  `ActiveTurn` observation and no turn-time capability exists (ht-poh.42), so an active-turn poke stays pending until
  the seat is idle. Turn-time pokes are parked as an idea.
- Is a deadline poke a "nudge" under rule 4? Yes: it is a typed prompt into the composer that interrupts the model
  exactly as a wake does, so rule 4 applies to it. It also carries only ACK-required receipts (rule 5), never lazy
  mail. Rule 4 is enforced in `poke_eligibility` itself, with no exception (DC-3, fixed).
- Claude mod channel: whether the mod submits a turn only when idle is decided in the mod (frozen at v0.5.0, R9) and
  is not verifiable from core code (DC-4).
- Served in-turn (ht-705, 2026-10-11): a nudge or invitation a standard hook showed whole inside the recipient's
  turn stayed pending (the hook never ACKs or accepts), so the idle wake fired after that turn and the model read
  it again (found on Codex by `scripts/wake-e2e.py` busy-nudge and busy-invite). The hook's post-flush
  `AdvanceCursors` now carries `in_turn` and the shown invitations at a tool boundary or turn start
  (`src/cli/hook.rs`, `src/cli/hook_inbox.rs`), the daemon records the displayed pending receipts and invitations
  in `in_turn_offer` for that top-level binding (`src/store/control.rs` `advance_cursors`), and
  `WakeCandidate::has_actionable_work` skips pending receipts or invitations only when all are recorded for
  every binding the wake could reach (`src/store/wake.rs` `served_by_all`); the native dispatcher rechecks the
  binding the composer chose right before typing and refuses a served one pre-send
  (`ReservationCheck::owes_wake`, `wake::wake_owed_to_binding`), so neither a served foreground binding is
  woken nor an unserved sibling's wake hidden (review of 2026-10-12). The items stay pending; a lifecycle
  check-in clears that binding's records only. Warnings were already covered by the check-in's warning offer
  frontier. Pinned by `tests/integration/lazy_attention.rs` `in_turn_display_of_a_nudge_owes_no_idle_wake`,
  `in_turn_display_of_an_invitation_owes_no_idle_wake` and
  `a_sibling_lifecycle_check_in_resets_only_its_own_in_turn_service`, and `tests/store/wake.rs`
  `in_turn_service_is_decided_for_the_binding_the_composer_reaches` and
  `an_in_turn_offer_after_reservation_cancels_that_bindings_wake`.
- One-process lazy wakes (ht-yha, 2026-10-15, main review M9): since priority delivery, a threshold-admitted
  delivery row (reason `lazy`, what a bare `--nudge` leaves, as it makes no receipt) is a seat-attention reason
  (`WakeReservation::for_seat_attention`, `src/ports.rs:2184-2195`), so a lazy-only wake to several bindings of one
  process (ht-327 `/side` bindings) skips the dispatcher's pane-idle gate (`src/notification/dispatch.rs:265-270`
  `gated`, `:287` `ComposerChoice::Process`), as receipt and invitation wakes already did. Only the adapter's final
  composer read before submission (TRUST-POLICY A4) refuses typed input or a busy agent there, so rule 4 holds
  through that read alone (DC-5). The pre-send recheck (`ReservationCheck::owes_wake`, `dispatch.rs:411`) owes such
  a wake while any reached binding still has the frontier row unread (`wake::wake_owed_to_reached`,
  `src/store/wake.rs:643`; ht-yha.62). Pinned by `tests/scheduler/dispatch.rs`
  `one_process_composer_wake_does_not_need_a_composer_classified_idle`,
  `one_process_composer_wake_serves_a_threshold_admitted_delivery_row` and
  `one_process_composer_wake_stays_refused_outside_seat_attention`, and `tests/store/wake.rs`
  `a_sibling_display_never_blocks_a_one_process_lazy_wake`.

Pinning tests: `tests/scheduler/dispatch.rs` `active_attention_is_always_deferred_then_idle_delivers`,
`blocked_or_human_input_target_is_never_prompted`, `nonempty_composer_skips_before_stash_or_restore`,
`active_turn_pokes_always_defer_receipts_until_idle`; `tests/scheduler/poke_flow.rs`
`active_turn_pokes_remain_pending_until_idle`, `drafts_never_stash_even_with_test_only_capabilities`;
`tests/scheduler/policy.rs` (`eligibility_matrix`, `active_turn_always_defers`).

Verdict: holds in production wiring and in `poke_eligibility` (DC-3, fixed by ht-poh.42); mod-side idleness recorded (DC-4).

## Rule 5. Messages that require an ACK are always nudge-class

Enforcement:
- `src/cli/commands.rs:2202-2207`: any `--require-ack`, `--require-ack-pane` or `--nudge` yields `Ordinary`.
- `src/store/messages.rs:87-97`: the reverse direction; `Lazy` plus recipients or a deadline is rejected.
- Wake reasons are invitations, ACK-required receipts and actionable warnings (`src/store/wake.rs:437`
  `reason_labels`; a `wake_on: all` seat adds its lazy rows as a wake reason by its own opt-in, rule 2); an ACK-required receipt is always `Ordinary`. Invitations (including handoff, `src/cli/handoff.rs:181`)
  and warnings are also nudge-class, which extends rule 5 without violating it.

Pinning tests: `tests/cli/lazy_send.rs` `lazy_send_default_and_nudge_ack_selection`,
`lazy_send_rejects_ack_seats_panes_and_deadline`; `tests/store/messages.rs`; `tests/store/mod_ack.rs`.

Verdict: holds.

## Rule 6. Who performs an idle wake is the only per-harness variation

Enforcement:
- `Channel`: `src/service/dispatch.rs:70` `seat_has_mod_channel` (`ModChannels::seat_live`), `:817` digest flag.
- Scheduler routing: `mod_suppressed` (`src/scheduler/mod.rs:610`) returns true when the channel's policy says the channel wakes
  an idle session (`ModChannels::wake_suppressed`, `src/service/mod_channels.rs:589`, `IdleWake::Channel`, ht-poh.3.3)
  and the channel is not stalled; it gates `try_candidate` (`:703`), `drive_pokes` (`:290`) and `try_poke` (`:936`). A stalled, absent or removed channel
  falls back to the `Native` path with the ladder untouched, and channel removal kicks the wake lane.
- Wake admission, batching, retry ladder and the idle gate (rule 4) are the same code for every harness; only the
  `Native` marker typing differs per adapter.

Pinning tests: `tests/scheduler/mod_routing.rs` (`wake_not_reserved_while_channel_live_and_ladder_unchanged`,
`wake_suppressed_when_the_seat_has_a_channel_of_another_generation`, `native_ladder_runs_while_channel_stalled`,
`wake_resumes_after_channel_removed`, `poke_skipped_while_live_and_runs_while_stalled`,
`try_poke_rechecks_the_channel_before_reserving`).

Verdict: holds. The channel flag is a mod-channel registry query keyed by the adapter's declared `idle_wake`
(ht-poh.3.3), not a harness-id branch, so a future `Channel` adapter needs no core change.

## Divergences

| ID | Rule | Site | Verdict | Fix commit or reason | Punch-list owner |
|----|------|------|---------|----------------------|------------------|
| DC-1 | 1 | `src/protocol/commands.rs:766-770,845-846` wire default `Ordinary` | divergence - recorded | Frozen wire shape: an absent field must decode as ordinary for pre-lazy clients (R9/wire compat). Every in-repo producer states its mode. Permanent. | none (permanent) |
| DC-2 | 3 | `src/harness/bridge.rs:1218-1291`, `src/cli/hook.rs:2588-2630` | divergence - recorded (narrowed) | Lazy bodies now reach Claude-without-mod, Codex and Hermes through the standard hook's inbox page and a hook-pull seat through `DeliveryPull` (see rule 3). Only ordinary (non-lazy) bodies on the hook harnesses remain digest plus ready commands, "as far as the harness allows"; ordinary body delivery for Codex/Hermes is a root-spec non-goal. | follow-up (per-harness delivery policy work) |
| DC-3 | 4 | `src/notification/policy.rs` `poke_eligibility` `ActiveTurn` branch (removed) | fixed | ht-poh.42: the `ActiveTurn` submit branch and the `poke_during_turn` capability (and Claude's exact-version declaration) were removed (human decision 2026-10-10). Turn-time pokes are parked as an idea; a revival needs a delivery-policy method with a neutral default and must keep rule 4. | none |
| DC-4 | 4 | Claude mod idleness decision (mod side) | divergence - recorded | Not verifiable from core; the mod is frozen at v0.5.0 (R9, DP10). | none (R9) |
| DC-5 | 4 | `src/notification/dispatch.rs:265-270,287` (pane-idle gate), `src/ports.rs:2184-2195` (`for_seat_attention`) | divergence - recorded | One-process seat-attention wakes (now including `lazy` rows) skip the pane-idle gate; the adapter's final composer read is the only busy check. Accepted with ht-327: one process is one composer. | none |

No divergence met the fix criteria (small, inside `src/scheduler/**` or `src/service/**`, no change to the v0.5.0 mod
wire or behaviour, no TRUST-POLICY change), so no Rust was changed for DC-1 to DC-4 (DC-3 was later fixed by ht-poh.42 on a human decision).

Fixed by ht-poh.17 (not a numbered divergence, found by this redo): the lifecycle check-in of a hook-pull seat read
an inbox page and could complete the same lazy rows that `DeliveryPull` offers (two completion sources, rule 3
delivery twice). Fix: `src/delivery/routing.rs:35` `hook_inbox_page`, used at both hook boundaries, plus the
daemon guard `src/store/control.rs:1961-1971`; TRUST-POLICY A3 (`cooperative_hook_context` row) and A8 (completion
sources) updated in the same commit.

The `AGENTS.md` "Delivery contract" rule 2 text said "once `wake_on` exists"; `wake_on` exists now (ht-poh.3.6), so
the clause was removed. No other rule text was inaccurate.

## Punch list for the run report

- DC-1 (rule 1): wire default `DeliveryMode::Ordinary` is frozen for compatibility; accepted, permanent.
- DC-2 (rule 3): lazy bodies now arrive in context for Claude-without-mod, Codex and Hermes (inbox page) and hook-pull
  seats (`DeliveryPull`); ordinary (non-lazy) bodies on the hook harnesses remain digest plus ready commands, accepted
  as "as far as the harness allows"; revisit only with per-harness delivery policies.
- DC-3 (rule 4): resolved by ht-poh.42; `poke_eligibility` always defers an active turn and the `poke_during_turn` capability is gone.
- DC-4 (rule 4): the Claude mod's own idle-vs-in-turn decision cannot be audited from core; frozen by R9.
- DC-5 (rule 4): one-process lazy wakes rely on the adapter's final composer read for idleness (ht-yha, main review M9); recorded.
- Resolved by ht-poh.3.6 (was a note for 3.5/3.6): rule 2's `wake_on: all` exception now has code and tests (see rule 2).
- Resolved by ht-poh.17: a hook-pull seat's lazy rows are delivered once, through `DeliveryPull` only (rule 3).
