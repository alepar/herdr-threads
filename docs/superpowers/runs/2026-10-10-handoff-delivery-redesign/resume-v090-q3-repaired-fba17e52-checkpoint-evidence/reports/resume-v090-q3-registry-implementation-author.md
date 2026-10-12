# Q3 registry/admission author handoff

2026-10-11. Parent supplied actual baseline `/private/tmp/ht-C-v090-q3-baseline-compile-corrected/run.log`: 26 selected, 7 PASS, 19 FAIL, 1.023 seconds; parent leak scope `7275a27f-d923-4f3e-9d72-0c35e34149f8`, reported zero scoped leaks. The nextest run ID printed in the log is separately `23d49601-eee9-481c-b6b6-01c6f7f76304`. This author read the failure log and did not execute Cargo or tests.

Initially prepared unapplied patches in this ignored WS. Parent subsequently released `src/service/mod_channels.rs` after its test-seam author and explicitly authorized applying the proven four-file scope. Applied only `src/ports.rs`, `src/service/mod_channels.rs`, `src/service/dispatch.rs`, and `src/daemon/transport/watch_connection.rs`; then ran owned-file `rustfmt --edition 2024 --config skip_children=true` successfully. No test/source outside those four files, Git/index, Cargo, application/SQL/test runtime, Herdr command, new agent or spawned server was touched. Formatting proves parsing only, not compilation or behavior.

## Applied scope

- Internal `ports::ModChannelMode::{Legacy, PresentationV2}` and required `ModChannelRegistration.mode`. No default mode hidden in the constructor.
- Legacy Watch admission/pass uses `mod_binding_view` and bypasses private refresh. Explicit capable admission/pass keeps `presentation_seat_view` plus the existing bounded refresh. No publication-scanner change.
- Registry generates an optional lease only for capable mode. Exact mode gates lease lookup, Live-owner admission, replay snapshot/application and presentation-confirmation health. Existing grace ACK, ordinary ACK health, liveness and wake-suppression rules remain.
- Entry-local `pending_delivery_recheck` is atomically taken into `ChannelWork` before store reads. New events during a pass remain pending. Store errors restore captured work only to its exact Live channel/mode, using existing worker error handling. Passive false enqueue restores the exact owner's bit without an immediate dirty/wake loop; no hint-generated health is added.
- Seat-local completion hint visits only current registry entries, within the existing global 64-entry cap. It marks Live capable entries and wakes the existing mod worker after releasing the registry mutex. It does not call the native wake callback, scan durable consumer history or write receipt/private counter/ledger state.
- `cooperative_mutation` captures eligible mutation class and admitted claim seat, runs existing permit/store mutation under an inner foreground-writer scope, then classifies the successful result and hints after that scope drops. Errors never reach the hint. Existing `ack_mod_delivered` outer `counts_as_mod_ack -> record_ack` remains unchanged, including AlreadySettled.
- Transport advertises an Accepted lease only for request opt-in plus all four capabilities (`MOD_WATCH`, `MOD_WATCH_PRESENTATION_V2`, `DELIVERY_PRESENTATION_V2`, `BINDING_CURSORS`) and the registry's actual optional lease. Wire owner supplies the bool and new capability.

## Exact successful-result classification

| Mutation | Accepted typed result for conservative hint |
|---|---|
| `PermitMutation::Ack` or `AckDisplayed` | `CommandResult::Acknowledged` with nonempty `acknowledged` OR `already_acknowledged` |
| `PermitMutation::AckModDelivered` | `CommandResult::ModDeliveryAcked` with at least one `ModAckOutcome::Settled` OR `AlreadySettled` |
| `PermitMutation::DeliveryConfirm` | `CommandResult::DeliveryConfirmed` with at least one `DeliveryConfirmOutcome::Settled` OR `AlreadySettled` |
| Every other mutation/result or `Err` | No hint |

Settled includes ordinary settlement and Lazy completion. Mixed batches with an accepted success qualify; all-refused/retryable/stale batches do not. Retained successful replies may conservatively hint again. No classification asserts a new durable receipt delta, and no post-commit item-query API was added. Presentation confirmation does not qualify. Fixed-eligibility quietness and legitimate DP5 due retries still need their dedicated controls; health comparisons involving mod ACK snapshot after its existing handler has returned.

## Deliberately held correction and preserved seam

`notify_binding(binding, version, now)` remains **binding-only** in the applied stage. The active final-send replacement bug is deliberately present for the parent's actual race RED. Its return type is now bool so failed active enqueue can restore the captured capable work without inventing health. The passive branch retains its channel check and adds immutable mode comparison; it holds the entry lock through push and bit restoration.

The test author's `#[cfg(test)] before_final_push: Mutex<Option<BeforeFinalPush>>` and one-shot callback remain intact immediately after observation unlock and before active/passive dispatch. `observe_fingerprint` now receives `&ChannelWork` but keeps local `binding` and `channel` values for the exact callback contract. The type remains `Box<dyn FnOnce(&BindingId, ModChannelId, bool) + Send>`; tests can capture intended modes in their closures.

`resume-v090-q3-registry-final-fence-held.patch` is a separate **unapplied** apply_patch-format patch. After actual race RED and a fresh parent release, it adds expected channel/mode to the active helper, compares them under the final push/health lock, snapshots exact owners in direct seat-wide notify, and retains capable failed-enqueue progress for that owner. Do not apply it before the parent records the race. The two-patch intermediate state is not a completed release candidate.

`resume-v090-q3-registry-proven.patch` records the already-applied first-stage edits in apply_patch format, before rustfmt. Do not reapply it to the candidate. Both patches use candidate-relative `src/...` paths; source application must run against the candidate, not the outer repository.

Publication extension remains held. The early sibling-ACK failure can be satisfied by the new completion hint, so it does not uniquely prove a need to change the scanner. Independent canonical scanner-only RED is required first. Existing ordinary-pending/Lazy publication and capable bounded-read controls were already PASS in this baseline and are preserved.

## Source release hashes and remaining ownership

Quiescent after successful owned-file rustfmt:

| File | SHA256 |
|---|---|
| `src/ports.rs` | `0dd3de6092764630a7f7648cf75630e0fe81be5a50beba3ec7db71c21bb79d79` |
| `src/service/mod_channels.rs` | `897ddebd5f15290c35e66c4176c9ab79221d1297dcfbd276bd29c0984bc5e69d` |
| `src/service/dispatch.rs` | `58a3fbf3df6a647decf83e4774f3a15922fe4abbb13c3de5b6565dc5c364f1e0` |
| `src/daemon/transport/watch_connection.rs` | `7487c8e04f28af99d6f28a28642321ab2f3b5aba00053c37c1f8e81989d02aa2` |

The parent owns constructor election sweeps and all Cargo/validation. Direct `ModChannelRegistration` constructions were observed in `tests/service/{delivery_pull,mod_channels}.rs`, `tests/protocol/watch.rs`, and `tests/delivery/push_channel.rs`; choose Legacy for historical channel tests and PresentationV2 only for capable behavior. Existing V2 WatchRequest constructions likewise need explicit election; never change a claim-only legacy oracle to hide its failure.

Further registry or dispatch edits require parent source release because the next stage runs causal race/other controls and adds the remaining tests. Remaining coverage includes final active/passive races in both modes, hint-during-store success/error/replacement, honest result classification and writer-release ordering, post-enqueue transport loss, and fixed/due DP5 comparison. This author claims no PASS for the applied changes and is quiescent.

## Second authorized stage: final fence and independently isolated publication

Parent subsequently reported composed baseline `3511`: 51 selected, 33 PASS and 18 failures (12 assertions plus six hanging CLI tests interrupted with SIGINT), scoped leaks zero under `bdd0e0c7-6938-469e-b383-19db93367101`. Both actual final-gap mode directions failed because the replacement received old Attention 3 and its health timestamp. The independent scanner control used a direct canonical store ACK, established unchanged native state, then failed private counter 0 versus 1. This author did not run or independently execute that baseline. Parent explicitly released three files and authorized both corrections from these causal outcomes.

Applied the previously held final-fence patch with apply_patch, preserving the exact cfg(test) seam. Final active helper now receives expected channel and mode; it checks them and Live state under the same lock as enqueue and health mutation. Seat-wide notify snapshots binding/channel/mode before sending. Failed capable enqueue restores the exact current owner's pending bit without immediate kick. The existing passive final fence remains.

Changed only publication probing within `src/store/invitation_content.rs`: first select mode from the exact message id with instance/thread/decision-sequence/ordinary-kind checks, then execute one selected ordinary or Lazy recipient point query. Ordinary checks preparation/seat/thread against the captured canonical manifest; Lazy additionally checks its stored message id. Unknown canonical mode refuses; missing or mismatched message/recipient facts do not create a passive change. Existing cursor traversal, checked private increments and transaction behavior remain. No effective-receipt, materialization, audience/history query or native attention/health write was added.

Actual production SQL constants exposed `pub(super)` for the query-control owner:

- `PUBLICATION_MESSAGE_SQL`: exact message key and canonical scope, returning delivery mode.
- `PUBLICATION_ORDINARY_RECIPIENT_SQL`: `prepared_recipients` preparation/seat key, returning thread id.
- `PUBLICATION_LAZY_RECIPIENT_SQL`: `lazy_recipients` preparation/seat key, returning message/thread ids.
- Existing `PUBLICATION_RANGE_SQL` is unchanged.

The source allowance remains at most 100 manifest visits, 200 point probes and three range/head seeks. Both recipient relations have the existing unique preparation/seat key. Query-plan/VM execution must measure these actual constants before claiming physical work; no measured runtime result is claimed here. Parent's newly applied Notice predicate (`notice_replay_applicable`) and `notice_replay_tests` module were preserved. TRUST-POLICY now describes the implemented ordinary/Lazy probe order, its source-only accounting, explicit channel mode and the final enqueue fence, preserving the concurrent reader/Notice additions.

Only `src/service/mod_channels.rs`, `src/store/invitation_content.rs`, and `TRUST-POLICY.md` changed in this stage. Owned Rust files passed `rustfmt --edition 2024 --config skip_children=true` parsing. No Cargo, application/test/SQL runtime, Git/index, Herdr or nested agent operation was run.

Final source release hashes supersede the first-stage registry hash:

| File | SHA256 |
|---|---|
| `src/service/mod_channels.rs` | `b6c54cd96d0d2ed39e6523fa37d9ef2cd547a1eb875e2b973a7d527edbcf589f` |
| `src/store/invitation_content.rs` | `014b31080186266aa2d5f03db99ff5d13ac493c60bba1f79cf9262df5ebb4d62` |
| `TRUST-POLICY.md` | `1b45e7bc780b6a19cb31d766a96fd5510054a29fcfe20d32eeed5d363ed1766a` |

The file named `resume-v090-q3-registry-final-fence-held.patch` is now an **already-applied historical patch**, despite its retained filename. Do not reapply it. Source is quiescent for parent compilation, actual controls and review; this author makes no GREEN claim.
