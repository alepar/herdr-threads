# V090 Mod integration failures: read-only triage

Scope: frozen `upstream-composition-v090`, supplied 9a/5158 inventory, actual run `/private/tmp/ht-C-v090-final-composed-657/nextest-run.log`. Read-only source inspection; no Cargo, Git, Herdr, runtime, source, index or test changes. This report is the only write. Read `TRUST-POLICY.md`, including A2 and cooperative attribution, before tracing identity. Proposed fixture repairs are not runtime-verified.

## 1. Four notice-frontier SQL failures: confirmed obsolete schema

Actual failures are at `tests/integration/mod_delivery.rs:2594`, `3179`, `4430`, `4920`, each `no such column: o.binding_generation`. The queries are at 2591, 3177, 4427 and 4917. Migration `migrations/0035_binding_attention_frontiers.sql:23` defines `digest_notice_offer(binding_id PRIMARY KEY, seat_id, offered_ordinal)`: generation/execution now belong to `occupant_bindings`. `store/attention.rs:1219` settles only the supplied exact BindingId, preserving projection rows and every other binding frontier.

Minimal proposal: resolve the tested Watch's exact canonical binding once from its claim key, then query the frontier only for that BindingId. For relay tests use the actually captured Pull/Confirm claim. For the other two tests the key is the rig's explicitly served `SESSION_B`, seat B, Claude, top-level, empty agent ID. Never use the seat-level `SEAT_NOTICE_BINDING_SQL`, `generation=seats.generation`, `ORDER BY ... LIMIT 1`, or any newest-binding selector to replace that exact key.

Suggested exact resolution SQL (no fallback or ordering):

```sql
SELECT b.binding_id,b.generation,b.execution_id,b.native_session,
       b.harness,b.target_id,b.liveness,b.observation_provenance
FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id
WHERE b.seat_id=?1 AND b.harness=?2 AND b.native_session=?3
  AND b.agent_id='' AND b.role='top_level' AND b.ended_at IS NULL
  AND s.instance_id=?4 AND s.state='resolved';
```

Require exactly one row. Independently compare its generation, execution, target and native session to the actual captured claim wherever available; require connected top-level identity. For non-relay cases retain the explicit session key and independently compare execution to the expected session-derived UUID (`harness::context::session_execution` using this rig's canonical instance), including existing `execution != SESSION_B`. This is a test expectation, never an authority decision. `store/bindings.rs:98` defines the canonical key; V32 allows several sibling bindings with the same generation.

Then use:

```sql
SELECT o.binding_id,b.binding_id,o.offered_ordinal,d.ordinal,
       b.generation,b.execution_id,d.thread_id
FROM occupant_bindings b
JOIN digest_notice_offer o ON o.binding_id=b.binding_id AND o.seat_id=b.seat_id
JOIN digest_programmatic_warnings d ON d.seat_id=b.seat_id
WHERE b.binding_id=?1 AND b.seat_id=?2 AND d.warning_id=?3;
```

Compare offered BindingId to the separately resolved expected BindingId. Retain the claim-generation/execution comparisons via `b`; V35 no longer duplicates those columns on `o`. Retain `frontier >= ordinal`, positive ordinal where already asserted, exact warning/thread/schema/body, no ACK marker, different session/execution, scoped emitted Attention epoch and retraction, hook suppression, live channel and running Watch. Do not reduce these checks to a seat-wide frontier comparison.

For 3177 preserve its independent covered-notice boolean, using the old Pull's exact captured claim to resolve the BindingId and `LEFT JOIN digest_notice_offer o ON o.binding_id=b.binding_id AND o.seat_id=b.seat_id`. Keep `d.ordinal <= COALESCE(o.offered_ordinal,0)` and the exact warning. The original Pull must settle its own Warn even though its held Confirm was lost; replacement lease, physical replay, original receipt remaining pending and uncommitted outbox oracles all remain independent.

Additional non-failing diagnostic: line 2970 still reads `digest_notice_offer WHERE seat_id=?1` as a scalar. V35 can have several rows per seat; qualify that diagnostic by the actual current tested Watch BindingId rather than silently choosing an arbitrary sibling. This is diagnostic-only and does not replace any assertions.

## 2. `/clear` Watch: exact session/execution mismatch in the fixture

Actual 657 failure at `wait_line` 1384 sees Connected then Closing(reason=error,exit=1), no item, empty stderr. Empty stderr is consistent with `watch_presentation_loop` mapping a refused/other response to generic Closing; it does not prove a raw stderr-reader failure.

The dataflow establishes a concrete mismatch:

1. `clear_rebind_pending_item_restreamed_to_new_session` 4476 creates SB2 using the SessionStart payload but preserves SESSION_B's process fingerprint, correctly modeling `/clear` in one process.
2. `Rig::command_with` 669-676 always uses `as_agent(...,session_of(pane))`; `session_of(PANE_B)` is fixed SESSION_B (57-62). Thus `rig.watch("SB2")` supplies environment SESSION_B despite argv `--session SB2`.
3. `cli/caller.rs:149` derives execution from that environment's native session. `watch.rs:1111` rewrites only native session/role/agent ID/liveness, leaving execution SESSION_B's UUID. Registration resolves by BindingKey, so it can report Connected for SB2. V2 `invitation_content.rs:285` requires the exact binding's stored execution to equal the Pull claim and refuses the mixed claim.
4. The final `rig.ack_result("SB2",...)` follows the same `locate_claim` path (watch.rs:1214), so correcting only the new Watch would leave the ACK mismatch.

Minimal fixture correction: for the new Watch and final SB2 ACK, set `CLAUDE_CODE_SESSION_ID=SB2` while retaining `CLAUDE_PID` and `HT_TEST_HARNESS_PID` for SESSION_B's process. An explicitly session/process-aware helper shared by these two calls can do this; do not update the daemon to accept mixed claims. Existing `watch_with(..., env)` can already override the session environment for Watch, but ACK needs the corresponding override too. Do not call `as_agent(...,"SB2")` without restoring the old process fingerprint: that changes the independent same-process `/clear` scenario.

Before invoking the new Watch, assert the exact SB2 canonical row has its distinct expected execution, its liveness PID/start equals the original process, and the old SESSION_B row is disconnected/superseded. Preserve closing reason binding_changed, old exit 0, rebind grace, still-pending receipt, exact restreamed body, new channel Live and explicit final settlement. No added grace, timeout, retry, incidental ACK or native wake.

## 3. Real-store channel test: not repairable by canonical identity alone

Failure `tests/service/mod_channels.rs:1413`: initial A frames `[]`, expected `[Attention{version:5}]`. Fake-store counterpart passed. The following boundaries explain it:

- `registry.pass` now unconditionally calls `presentation_seat_view` (`service/mod_channels.rs:368`). SQLite overrides it through `mod_seat_view_mode(...,false)` (`store/seats.rs:4915`), which zeros pending receipts, unread, lazy and other pending fields (4974-4992). Its initial fingerprint has no pending state.
- Trait fake defaults `presentation_seat_view` to `mod_binding_view` (`ports.rs:2643`); the fake retains a nonzero fingerprint, so its pass is not evidence for the real-store path.
- Initial `observe_fingerprint` uses `has_pending()` (560), hence false even when attention version is 5. Passive refresh also cannot signal this fixture's ordinary mail: `advance_ordinary_publications` (invitation_content.rs:658) increments mod_version only after an exact lazy_recipient and a message with `delivery_mode='lazy'`. `domain::deliver` (1367) seeds ordinary manifests/prepared_recipients, not those lazy rows. With no invitation, passive version stays 0.
- On the final A-only cursor update (1433), presentation_seat_view again removes binding unread. Refresh tracks publication/invitation work, not that cursor. The test's independent requirement A frames length 3 / B length 2 cannot fire through the current boundary.

An independent exact-claim fixture defect exists: `request_for("native2")` (964) changes session but leaves execution `...aa`; `add_binding` stores `...bb`. Correct that exact known execution before any V2 Pull is added. It does not explain or fix 1413: registration resolves the key, and refresh reads each canonical row's own execution.

No honest fixture-only proposal preserves all the existing real-store assertions. Do not manufacture lazy rows, write passive versions, call synthetic notify, drop A-only cursor oracles, replace them with seat-wide notifications, or restore a full legacy digest as a test shim. Those would hide the boundary failure or change ordinary-mail semantics. Keep this test failing until the parent decides whether the presentation integration must carry equivalent bounded binding-owned change evidence or explicitly rehomes the legacy behavior while preserving its complete oracle coverage. This is a distinct runtime/design decision; no implementation proposed or authorized here.

## Parent-owned verification after authorized repair

Run the four named notice integration tests and `/clear` test separately, then relevant nearby mod tests; parent remains sole Cargo owner. For the real-store failure, verify the first-send, stable-no-repeat, both-second-send and A-only-cursor transitions independently. Retain flush-before-confirm, owner/lease provenance, no incidental ACK/accept/lazy completion, no invented wake, no origin erasure and existing work/size caps. Per-change lint/harness-neutral checks remain required. This report makes no pass/completion claim.

## Authorized candidate-only repair, 2026-10-11

Parent subsequently authorized the four failed SQL sites and the `/clear` Watch/final ACK fixture correction, tests only. Editable path is V090 `tests/integration/mod_delivery.rs`; ROOT's older source was only read and never edited. No production code or `tests/service/mod_channels.rs` changed.

Implementation deliberately retains every original notice tuple and assertion: the offer's generation/execution now come from `occupant_bindings offered` joined through `o.binding_id`, while the expected `b` binding is pinned by seat, Claude, the exact served/captured native session, empty agent ID, open connected top-level state and **`b.binding_id=o.binding_id`**. This exact-ID join is the identity oracle under V35; the retained generation/execution equality assertions are now redundant because both describe that exact canonical row. They no longer pretend the frontier table stores duplicated identity columns. A sibling's frontier cannot satisfy the join even if the sibling shares generation or execution. The first activation site's existing generation/execution comparisons to the captured claim remain independent. Replacement's covered-notice query uses the original captured claim's exact native session and a left join on the matching BindingId, retaining the original COALESCE and boolean.

`Rig::ack_with` adds only optional environment overrides, with `ack` delegating using an empty slice, preserving every other call. `/clear` overrides only `CLAUDE_CODE_SESSION_ID=SB2` on new Watch and final ACK; both retain old process PID/fingerprint supplied by `command()`. Explicit ACK exit-code and exact result-line checks remain unchanged in effect. Existing body, pending receipt, close reason, grace, channel and final settlement assertions remain. Timing and retries did not change. Diagnostic-only query 2970 remains untouched because the edit authorization covered four failed notice SQL sites.

Formatted only the owned file with `rustfmt --edition 2024 --config skip_children=true`; reviewed the complete non-Git diff against `/private/tmp/v090-mod-delivery-before-triage-repair.rs`. The diff has only those authorized changes; no model-visible strings added. Before SHA-256: `97c273b3605f23cc15f22f10cbe947d2c35cab02682e96f5fcc53b08fd40cb68`. Quiescent after SHA-256: `bc0957128e577619ca1a20fec8e51f4b67592c41c4b4cba373ce294f15340b46`.

No Cargo or runtime verification performed; parent owns those checks. Source ownership is relinquished at this hash.
