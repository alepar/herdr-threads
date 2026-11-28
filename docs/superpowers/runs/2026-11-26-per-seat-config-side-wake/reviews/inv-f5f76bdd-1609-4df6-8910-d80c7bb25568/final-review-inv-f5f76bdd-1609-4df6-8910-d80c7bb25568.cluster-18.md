# Final review: cluster 18 (src-ports)

Invocation inv-f5f76bdd-1609-4df6-8910-d80c7bb25568, epic ht-z41e, integration branch
super-auto/per-seat-config-side-wake. Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head
c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Source binding
recorded through `dispatch-text.mjs bind-review-source` (exit 0).

## Scope

One file, `src/ports/wake_intents.rs` (slice 000098, +9 lines), three new port fields:

- `WakeIntentPreparation.side_sessions: Vec<NativeSessionId>`: the target's process-mates, persisted as
  `wake_intents.side_sessions` (JSON array, NULL when empty).
- `WakeQueueBinding.side_sessions: Vec<NativeSessionId>`: the other top-level sessions of the root's
  process, carried from selection to the intent for diagnostics.
- `WakeQueueSelection.programmatic_enabled: bool`: the seat's `programmatic_wake` as read inside the
  selection transaction; unreadable reads `false`.

## Spec alignment

Design doc `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`
has a requirement baseline (B1-B10, S1). This cluster carries data for B1 (the process-wide queue flag
is replaced by a per-seat read) and B8 (queue targets the process root, side sessions are never
targets). Neither row is delivered by this file alone; the port types are the seam between the store
and the notification queue.

Production trace at head, checked only to confirm the fields are produced and consumed as documented:

- `src/notification/queue.rs` `attempt` calls `queue_wake_selection(r, use_queue, ..)`, then returns
  `QueueDispatch::Native` when `!use_queue || !selection.programmatic_enabled || blockers.programmatic`,
  after the native-fence check. Native fences are therefore still honoured when the seat flag is off.
- `src/store/wake_intents/selection.rs` `queue_wake_selection` reads
  `seat_config::programmatic_wake_on(&tx, seat)` in the same transaction as the binding census and
  returns `programmatic_enabled: false` when eligibility is not needed or when the value is off or
  unreadable. `programmatic_wake_on` maps every error to `false`, which matches the port's fail-closed
  doc comment. `tests/store/wake_queue_cost.rs::seat_config_read_failure_selects_native` covers this by
  renaming the table, and `flag_toggle_applies_on_the_next_selection` covers the toggle applying per
  attempt with no restart, as spec row 118 requires.
- `process_root` gives back `(root, sides)` from the adapter's `wake_process_root`, and the sides reach
  `WakeQueueBinding.side_sessions`. The lone-binding branch refuses a `concurrent` or `auxiliary` row,
  as the code-cycle-1 amendment requires. `queue.rs` copies `binding.side_sessions` into the
  `WakeIntentPreparation`, and `src/store/wake_intents.rs` writes NULL for an empty list and a JSON
  string array otherwise (migration 0021, nullable TEXT). Readback decodes the same shape.
  `tests/store/wake_activation_recovery.rs` round-trips a non-empty list.
- The canonical-check preparation inside `select_binding` uses `side_sessions: Vec::new()`. That is
  correct because `canonical` compares target identity, and the migration comment says fulfillment
  matches `target_json` alone, so the diagnostic list never takes part in matching.

Every struct-literal constructor of the three types at head (`queue.rs`, `wake_intents.rs`,
`selection.rs` x4, `tests/fixtures/wake_intent.rs`) sets the new fields. The types have no `Default`
derive that could silently zero `programmatic_enabled` to a permissive value; its zero value is the
safe one.

The field name matches the design text ("side sessions"). No harness name enters the neutral port
(`/side` appears only in a doc comment, as an example), and the port is not part of the peer wire, so
`check-wire-compat` does not apply.

## Findings

- none

## Concerns

- `src/store/wake_intents.rs` readback maps a malformed `side_sessions` JSON value to `corrupt()`, so a
  diagnostic-only column can make the whole intent row unreadable, which affects recovery and
  fulfillment of that intent. Only this code writes the column, through `serde_json`, so the case needs
  outside corruption. Tolerant decoding (an empty list on a parse error) would match "diagnostic only"
  more closely. This is a nit and does not block.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No test asserts the order of `WakeQueueBinding.side_sessions` when there are two or more sides. Order follows `ORDER BY ordinal DESC` and is diagnostic only; `tests/store/wake_queue_cost.rs` (around line 173) checks the contents.

## Deferred OK
- Decode `wake_intents.side_sessions` leniently on readback, so a corrupt diagnostic value cannot fail the intent load (currently `corrupt()`).
- End-to-end delivery of B1 and B8 crosses `src/notification/queue.rs`, `src/store/wake_intents/selection.rs` and the Codex adapter, outside this cluster. The seam reviewer traces it; this review only confirmed that the port fields are produced and consumed as documented.
