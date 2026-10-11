# Reconnect activation failure triage

Scope: read-only diagnosis of `mod_delivery::reconnect_within_grace_keeps_wake_suppressed_restreams_and_late_ack_settles_once` at source `082385c7`, in `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`. No tests, Cargo, Herdr commands, or production/test source edits were performed by this triager. This report is the only authorized write.

## Finding

The failure is a production compatibility regression in the activated V2 Watch path, not a demonstrated obsolete test assumption. A replacement Watch is admitted and connected, but the shared ordinary offer ledger suppresses an already-streamed, still-pending message. It treats recovery by a new transport owner as another logical offer subject to the 60-second delay. The old contract explicitly promises pending-message replay on reconnect, and no inspected later accepted document explicitly waives that promise.

The execution log `/private/tmp/ht-resume-activation-082385c7/nextest-run.log:296` records this test failing after 21.379 seconds. Its timeout at `tests/integration/mod_delivery.rs:1288` reports message `mOsdH03MS` missing; the replacement stream contains only `status:0`, `state: connected`. The enclosing activation matrix reports 291 passes and three failures. This triage addresses only the reconnect failure.

## Source-supported causal chain

1. The fixture sends a real addressed require-ACK message: `tests/integration/mod_delivery.rs:842` invokes `send <thread> --body <body> --require-ack <recipient>`. At `:3474-3478`, the first Watch emits the item, is killed, enters reconnect grace, and the fixture verifies the canonical receipt is still pending. Thus neither missing audience nor premature receipt settlement explains the failure.
2. Capability negotiation selects V2 when the daemon advertises presentation support and registration supplies a valid lease: `src/cli/watch.rs:350-359,414-428`. The V2 loop emits connected status, then issues `DeliveryPullV2`: `:574-631`.
3. The first V2 pull records every ordinary `page.offered` identity in the shared binding-generation ledger: `src/service/delivery_presentation.rs:214-230`. This occurs when the service returns its allocated page, rather than on a later receipt ACK.
4. Replacement registration allocates a fresh channel ID and presentation lease but retains the registration's canonical binding generation/native session: `src/service/mod_channels.rs:573-598`. Reconnect ends grace and resets the registry fingerprint; it does not rotate the binding or reset the offered ledger.
5. `OfferedLedger::seat_mut`, `src/delivery/ledger.rs:234-291`, discards offers on binding-generation change, not on Watch lease change. The replacement therefore sees the first Watch's offer.
6. V2 constructs `blocked_messages` from every offer that is not due: `src/service/delivery_presentation.rs:164-185`. `Offer::is_due` requires at least 60,000 milliseconds since its last logical offer and count below three: `src/delivery/ledger.rs:18-39`.
7. The canonical allocator skips blocked ordinary identities at `src/store/invitation_content.rs:2290-2295`, consuming the scanned item without emitting it. The fixture reconnects immediately and waits at `tests/integration/mod_delivery.rs:3482`; its 31-second clock advance occurs later at `:3489` and is never reached.
8. Frozen presentation recovery cannot recover this message. Fresh ordinary output at `src/store/invitation_content.rs:2309-2318` adds offered/truncated IDs without an outbox descriptor. Recovery explicitly rejects an ordinary Message descriptor at `:1630`. Notice/invitation/control outbox recovery and receipt-message recovery are consequently different paths.
9. ACK handling does not explain the missing output. `src/service/dispatch.rs:813-836` admits mod ACKs against a live channel or reconnect/rebind grace and records successful ACK health. The fixture's late/repeat ACK calls are after the failed replay assertion and have not executed.

The shared-ledger application to Watch originated in `4c4988d9` (`Implement bounded canonical in-turn delivery`), as established by blame of `src/service/delivery_presentation.rs:164-201`. The immediate reconnect replay expectation originated in `12252c833`, not in a recent fixture adjustment.

## Contract assessment

The original mod design, `docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md:55`, states that reconnect re-streams everything still pending and the mod deduplicates by message ID. Its D7 clause at `:259` states that an item streamed but never ACKed remains pending and is re-streamed on reconnect or reaches the agent through native fallback. Reconnect grace at `:254-257` suppresses native wake while replacement registration is expected to restore the stream.

The original 60-second/three-offer design appears under `DeliveryPull` / `DeliveryConfirm` in `docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--delivery-policy-design.md:53-57`; it described the HookPull offered ledger. The later accepted capacity refinement, `capacity-design-round1.md` in this workspace, preserves those logical offer limits and states that a successful store allocation followed by lost socket still records its ordinary offer (`:108`). It does not explicitly replace the existing Watch reconnect promise with delayed or absent recovery.

The legacy path has a fresh `EmitState`/Assembler per Watch process (`src/cli/watch.rs:391`) and drains pending inbox data immediately after connection (`:489-500`). Its deduplication therefore does not suppress another process's initial replay. V2 retains the same accepted registration/grace behavior but replaces that process-local delivery behavior with shared logical offer suppression.

This is a delivery regression and activation blocker for the reconnect contract. It is not evidence that grace registration or canonical receipt attribution is broken. It should not be resolved by weakening the test to wait 60 seconds, adding another unrelated producer, or clearing the ledger.

## Why a timeout increase is not a correction

After a V2 pull returns no further page, `src/cli/watch.rs:706-731` waits for an Attention frame; read timeouts only perform parent-liveness checks. The mod worker performs safety passes (`src/service/workers.rs:1131-1180`), but ordinary unchanged fingerprints do not trigger new Attention (`src/service/mod_channels.rs:470-508`). The offer becoming due does not itself guarantee another Watch pull. An unrelated canonical change or eventual stall handover could permit later progress, but neither restores immediate reconnect delivery.

## Minimal correction direction

Separate recovery of an existing logical offer by a new Watch owner from allocation of a new logical offer. Maintain bounded per-lease eligibility for previously offered pending identities, revalidate them against current canonical audience and receipt/lazy state, and emit each recovery candidate at most once for that owner. Recovery must preserve the existing Offer first/last timestamps, count, truncation state and capacity occupancy. A later ordinary retry remains subject to the original 60-second spacing and three-offer cap.

Use the existing per-seat presentation-owner serialization and exact lease check. A stale lease must neither emit nor consume the replacement's recovery eligibility. Recovery must fit existing per-call work, item and encoded-byte limits, including when all 256 logical offer slots are occupied. It must not reset normal ordinary cursor/carry/partial state or the entire ledger. Pending logical receipt and lazy state remain unchanged until their existing ACK/completion authority runs. Truncated pending messages still require their existing explicit read/ACK handling.

This is a correction direction for the independently assigned design role, not an implemented or tested algorithm. In particular, a lease-local physical replay must be distinguished explicitly from another logical offer so the shared bounded-reoffer policy remains intact.

## Meaningful regression requirements

- Keep the current elected-daemon/real-CLI integration's immediate reconnect replay assertion and its late ACK, repeat `already_settled`, exactly-one ACK event, expired-old-grace and no-native-wake assertions. It is already a causal failing regression.
- Add a source-level service regression with a real replacement lease and unchanged binding generation. Show that the still-pending item reaches the replacement immediately while all logical Offer stamps remain identical.
- Prove repeated pulls or Attention within the same lease do not duplicate physical recovery and do not modify offer counters/timestamps. Reject the old lease without consuming successor recovery state.
- Exercise replacement recovery at full 256-offer capacity while preserving normal discovery carry. Recovery of known pending IDs must not allocate unknown identity 257 or reset the original three-offer/60-second history.
- Include a pending lazy message, verifying physical recovery preserves lazy framing, causes no native wake, and performs no implicit lazy completion, receipt ACK or invitation acceptance.
- Retain ordinary due-time second/third offer and parking controls, canonical terminal-reclamation controls, output bounds, and truncated-message behavior.

Verification limitation: only existing source, history, contract documents and the supplied execution log were inspected. No local reproduction or correction verification was run by this triager.
