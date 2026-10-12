# Q3 production interface and ownership plan

2026-10-11. Proposals only. Parent reports candidate `33c74793` is clean and tests-only; this author did not run Git or independently attest the commit. All source observations below are from `.worktrees/upstream-composition-v090` under the outer handoff workspace. Parent owns the sole Cargo lane. No production/test/index edit, compilation, test run, SQL runtime, Herdr action or nested delegation was performed for this plan.

**Goal:** Restore the historical Watch channel contract and provide explicitly negotiated, bounded capable delivery with passive progress after publication, own reading and shared settlement.

**Architecture:** Elect immutable mode at registration. Keep historical aggregate reads confined to Legacy channels; PresentationV2 channels use bounded canonical views and private publication/reader signals. A process-local, seat-local completion hint requests a canonical pull without asserting receipt change, health, read progress or retry eligibility.

**Tech stack:** Existing Rust traits/service registry, serde wire types, rusqlite transactions and Tokio unbounded Watch transport. No migration, new timer, queue redesign or JavaScript behavior change.

**Spec:** [resume-v090-q3-channel-design.md](resume-v090-q3-channel-design.md), reviewed revision SHA256 `04b09db182355e44a991088b3055073199b797e32d174d808afcf9c11ab9139c`. Candidate TRUST-POLICY remains normative. This interface map supplements that design; implementation is authorized only by a subsequent parent dispatch after actual causal RED. A passing hypothesis removes its proposed correction.

## Evidence and gates

Read author reports in outer `.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-{store,wire-registry}-controls-author.md`. They report nine new real-store controls and fifteen wire/registry controls, with parsing/format evidence only. Neither report claims compilation or runtime outcomes. The previously observed real-store legacy `service::mod_channels::tests::domain::each_channel_receives_only_its_bindings_deliveries` RED remains distinct from these pending baselines. Its fake-store counterpart must remain.

- [ ] Parent compiles and executes the authored selectors, separating fixture/admission/decoder failures from causal delivery failures. Raw proposed-field decoder RED proves missing negotiation, not parked-ledger or shared-settlement progress.
- [ ] Record the precise first failing assertion for each applicable correction before source changes. An ordinary sibling-ACK control must first establish unchanged native attention and then fail for absent normal channel progress; it must not fail during setup.
- [ ] Add the explicitly deferred race, coalescing, classification and retry controls described below when their real interfaces exist. Missing coverage is not supplied by the count of authored tests.
- [ ] Execute only the parent-selected correction supported by those results, then its positive/negative controls, required lint/harness checks and fresh code review. No full-suite instruction is added here.

## Ownership and dependency map

All paths in this table are relative to the candidate. Assign a file to one author at a time; `src/ports.rs`, `src/service/dispatch.rs` and `src/store/invitation_content.rs` are shared interfaces and must not have concurrent writers.

| Unit | Exact production files | Interface delivered | Depends on |
|---|---|---|---|
| Wire election | `src/protocol/watch.rs`, `src/protocol/capabilities.rs`, `src/cli/watch.rs`, `src/daemon/transport/watch_connection.rs` | Optional false-omitted request flag; new capability; CLI conjunction; mode-bound Accepted lease | Mode field agreed with registry/admission owner |
| Admission/registry | `src/ports.rs`, `src/service/mod_channels.rs`, admission and completion sections of `src/service/dispatch.rs` | `ModChannelMode`, registration field, no-op hint trait method, exact-owner lease gates, bounded pending bit and final send fence | Wire request flag; existing store ports unchanged |
| Publication/reader maintenance | `src/store/invitation_content.rs`, `src/store/read_cursors.rs`, `src/store/control.rs`, `src/store/lazy_delivery.rs`, `src/store/first_contact.rs` | Mode-specific publication point checks; one private reader helper; changed-status companion for display traversal | Actual store RED and physical-query evidence |
| Contract/accounting | `TRUST-POLICY.md` | Publication wording expanded from Lazy to addressed ordinary/Lazy; explicit reader-maintenance accounting and ephemeral hint limits | Final implemented queries/call boundaries |

Existing tests requiring election adaptation are `tests/cli/watch.rs`, `tests/service/mod_channels.rs`, and `tests/service/delivery_pull.rs`; adapt only explicit mode construction/negotiation as the production seam appears. `tests/store/queries.rs` owns the existing production publication SQL/index/VM control. Existing cursor/first-contact test modules should own added operation work-count controls; use their current module locations rather than a new process target. `src/store/receipts.rs` is a classification reference, not a planned delta-reporting API. `src/service/delivery_presentation.rs`, ledger allocation and JavaScript delivery policy need no behavioral changes for this proposal.

## 1. Wire and immutable mode

Add these exact neutral interfaces; the internal enum is not serialized:

```rust
// src/protocol/capabilities.rs; also add to ADVERTISED.
pub const MOD_WATCH_PRESENTATION_V2: &str = "mod.watch_presentation_v2";

// src/protocol/watch.rs: append to existing WatchRequest.
#[serde(default, skip_serializing_if = "is_false")]
pub presentation_v2: bool,
fn is_false(value: &bool) -> bool { !*value }

// src/ports.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModChannelMode { Legacy, PresentationV2 }
// Add to ModChannelRegistration:
pub mode: ModChannelMode,

// src/ports.rs, within ModChannels: inert default remains inert.
fn presentation_delivery_may_have_changed(&self, _seat: &SeatId) {}
```

Place `is_false` at module scope (reuse an equivalent existing helper if present). Preserve `deny_unknown_fields`, claim field order, all historical envelope fields and Accepted's existing optional `presentation_lease`. No protocol-version gate. True adds the 23 JSON bytes `,"presentation_v2":true`; false/absent writes zero new bytes. No model-visible string is added.

In `src/cli/watch.rs`, keep `Probe::Supported { presentation: bool, cursors: bool }`; redefine its internal `presentation` value as the conjunction of generic `DELIVERY_PRESENTATION_V2`, new `MOD_WATCH_PRESENTATION_V2` and `BINDING_CURSORS`, within the existing `MOD_WATCH` branch. `cursors` remains independent for the historical reader claim. Change the private signature to:

```rust
fn open_watch(socket: &Path, instance: uuid::Uuid, claim: CallerClaim,
              presentation_v2: bool) -> Result<(WatchReply, SocketConn), OpenError>;
```

Pass the conjunction into the wire flag and retain it for selecting the V2 loop. A valid but unsolicited Accepted lease cannot elect V2. Both existing loops still drain initially on Accepted; later Attention requests another drain. Discovery failure and missing `MOD_WATCH` continue to open no registration socket.

`DomainService::watch_register` keeps its existing public signature and return tuple. Derive mode from the flag before the first store read: Legacy uses `mod_binding_view`, PresentationV2 uses `presentation_seat_view`. Retain canonical role/liveness/hold/harness/instance checks and pass the canonical binding to `ModChannelRegistration` with the selected mode. The daemon implements and advertises the capability; it does not use a client-supplied version as authority.

`watch_connection.rs` must gate advertised lease on both request opt-in and the complete daemon feature conjunction; retain `advertised_presentation_lease(active: bool, lease: Option<String>)` if useful. Registry lease lookup independently enforces mode, so transport cannot accidentally give Legacy authority. A replaced channel may return no lease; existing close/reconnect behavior handles that without minting a substitute.

## 2. Registry entry, ownership and final send

Add `mode: ModChannelMode` and `pending_delivery_recheck: bool` to private `Entry`; initialize false on every registration/replacement. Prefer `presentation_lease: Option<String>` internally: generate `Some` only for PresentationV2, `None` for Legacy. Keep the existing seat presentation permit around registration/replacement of either mode, since replacing a capable owner with Legacy must serialize against in-flight allocation. Do not acquire that permit from the completion-hint method.

Require `PresentationV2` plus all existing exact-owner checks in `presentation_lease`, `presentation_owner_live`, `presentation_replay_snapshot`, `apply_presentation_replay`, and `record_presentation_attention_push`. Keep `presentation_owner(&SeatId)` as a permit lookup: a seat mutex itself grants no ownership. Preserve `is_live`, existing grace ACK authorization, `record_ack` and wake suppression semantics. Grace never gains V2 pull/replay authority.

Use an explicit work snapshot and one final send helper:

```rust
struct ChannelWork {
    binding: BindingId,
    seat: SeatId,
    channel: ModChannelId,
    mode: ModChannelMode,
    native_session: String,
    delivery_recheck: bool,
}
#[derive(Clone, Copy)]
enum ChannelSignal { Active, Passive }

fn notify_binding(&self, binding: &BindingId, channel: ModChannelId,
                  mode: ModChannelMode, version: u64, now: UtcMillis,
                  signal: ChannelSignal);
fn restore_delivery_recheck(&self, work: &ChannelWork);
```

The helper can return `()` because it owns failed-push restoration under its lock. `restore_delivery_recheck` ORs a captured bit only into the same Live channel and mode; it never writes replacement state. Store-error callers separately use existing dirty/backoff handling. Add mode to `Judged` if that makes the close fence explicit; channel IDs already uniquely identify one registration, but no fallback may close by binding alone after an unlocked read.

Ordering in `pass`:

1. Under the state mutex capture eligible entries. Atomically take the pending boolean for each Live capable entry into `ChannelWork` before any store read. Grace has no usable sink/presentation hint. Do not clear a bit later merely because this work snapshot succeeded.
2. Release the mutex. Select the read by captured mode. Legacy never calls `presentation_refresh`; its fingerprint/version behavior remains historical. Capable uses `presentation_seat_view` and existing bounded refresh. Retain binding and close judgments against the captured channel. Retain `more` maintenance scheduling, separately from sink retry.
3. On store error, restore the captured bit for that same owner before returning the error through existing worker retry/backoff. Do not restore to a replacement. New hints during the read already reside in the entry and must survive both success and failure.
4. `observe_fingerprint` receives captured mode and recheck bit in addition to its current arguments. Under its initial lock compare exact channel/mode before updating last observations. Compute existing active-fingerprint reason separately from passive-version/recheck reason. Combine to at most one frame for this observation: active precedence preserves existing active behavior; a solely passive reason never arms health.
5. After that first lock is released, final `notify_binding` reacquires the mutex and compares binding, channel, mode and Live state together. Hold this mutex through nonblocking `sink.push` and any `last_attention_push` update. Active successful enqueue may record existing health; passive never does. No unlocked clone/send can target a newer owner. A stale observation mutates neither replacement observation fields nor its pending bit/health.
6. If enqueue returns false for a capable signal, set that exact entry's pending bit true under the same lock. Do not immediately mark dirty or kick solely for a dead receiver; the existing safety pass and unregister/reconnect provide recovery. An unrelated newer hint or refresh-more reason may independently schedule legitimate work.

Change existing seat-wide `notify` to capture `(binding, channel, mode)` under one lock and call the fenced helper for each captured owner. Its direct native producer signal remains Active. Passing only binding would retain the replacement race even after fixing `observe_fingerprint`.

`ModChannelSink::push` stays bool. Concrete transport is unbounded Tokio MPSC; false means gone, true means enqueue, neither implies successful socket flush. Post-enqueue transport failure unregisters; a new owner drains on Accepted. The 64-entry registry cap bounds snapshots, per-seat visits and pending bits, not transport queue memory.

## 3. Successful completion hint and writer ordering

Implement `presentation_delivery_may_have_changed` using only `state.by_seat` and current entries. Visit at most the global 64 entries, set the bit on Live PresentationV2 entries, and after unlocking mark dirty/wake the existing mod worker if at least one entry was marked. Do not call `kick_wakes`, issue a frame synchronously, inspect consumer history or write durable state. Repeated hints coalesce until a pass takes the bit.

Keep the classification helper private to `src/service/dispatch.rs`:

```rust
#[derive(Clone, Copy)]
enum DeliveryRecheckClass { Ack, ModAck, DeliveryConfirm }
fn delivery_recheck_class(mutation: &PermitMutation) -> Option<DeliveryRecheckClass>;
fn delivery_recheck_succeeded(class: DeliveryRecheckClass,
                              result: &CommandResult) -> bool;
```

Exact matches: `Ack | AckDisplayed` map to Ack and require `CommandResult::Acknowledged` with nonempty `acknowledged` or `already_acknowledged`; `AckModDelivered` maps to ModAck and requires at least one `ModAckOutcome::{Settled, AlreadySettled}`; `DeliveryConfirm` maps to DeliveryConfirm and requires at least one `DeliveryConfirmOutcome::{Settled, AlreadySettled}`. All other mutations/results are false. `Settled` includes Lazy completion; do not label it fresh ordinary receipt evidence. Retained successful answers may conservatively hint again. Errors, rollback, all-refused/retryable/stale outcomes and presentation confirmation do not hint.

Within `cooperative_mutation`, capture class and `request.claim.seat.clone()` before consuming request/mutation. Keep all existing admission and special send-preparation control flow. Wrap foreground turn, permit issuance, optional bootstrap permit handling and final mutation dispatch in an inner scope that returns `Result<CommandResult, ApiError>`. After the scope and successful `?`, classify the result and call the registry hint. The store transaction has committed and its writer turn is dropped before registry locking. This introduces no reverse registry/store lock order and no post-commit per-item SQL.

`ack_mod_delivered` continues its outer `counts_as_mod_ack -> record_ack` handling unchanged, including AlreadySettled. Hint consumption adds no health; tests snapshot health after the complete ACK handler. Ordinary ACK and isolated hint paths can assert entirely unchanged health. Preserve all existing admission decisions and attribution values; the hint has no provenance because it asserts no durable fact.

## 4. Reader-change interface and coalescing

Current `read_cursors::advance(&Connection, &BindingId, &ThreadId, u64) -> Result<bool, ApiError>` already reports actual change. Current `advance_through_displayed` returns `Result<u64, ApiError>` (reached sequence), not a changed bit. Preserve that API for unrelated callers and tests, and extract its existing implementation into this companion:

```rust
// src/store/read_cursors.rs
pub(super) fn advance_through_displayed_with_change(
    db: &Connection, binding: &BindingId, thread: &ThreadId, displayed: &[u64],
) -> Result<(u64, bool), ApiError>;
// Existing advance_through_displayed delegates and returns tuple.0.

// src/store/invitation_content.rs
pub(super) fn note_binding_reader_change(
    db: &Connection, binding: &BindingId,
) -> Result<(), ApiError>;
```

The companion uses its already-read current/reached values and returns the result of the existing `advance` write as tuple.1; missing seat/no movement returns false. No extra cursor lookup and no private invalidation inside either low-level cursor helper.

`note_binding_reader_change` requires the caller's existing canonical transaction. It is maintenance for an already resolved binding, not a second presentation-admission gate. Resolve that exact binding ID against its canonical seat/instance, native session, generation and execution in one indexed lookup; do not reconstruct a binding by seat/session heuristic. Query only the two fixed consumer keys `(binding_id,'hook')` and `(binding_id,'watch')`, selecting just scope headers and the two private counters. A consumer's seat/instance must match; its primary generation/execution OR passive-refresh mod_generation/mod_execution must match current canonical scope. Both-stale and foreign-instance rows are skipped, not promoted. Native session belongs to the canonical binding, not a nonexistent duplicate consumer column. Zero consumers, including legitimate subagent fast-forward, succeeds.

For each applicable row validate nonnegative counters, checked-add one to `ordinary_passive_generation` and `mod_version`, then update only those columns by exact key/current scope. Do not call the full state saver or reset a header/token/cursor/carry/partial/stamp. Overflow is SequenceExhausted; malformed applicable counters are StoreCorrupt. Both existing cursor writes and any earlier private counter update roll back on failure. No row insertion or migration.

The added bound is one canonical binding/seat lookup statement, two fixed consumer point reads and at most two writes per changed operation/binding. The canonical join includes two indexed relations; document that physical work rather than describing one SQL statement as one relation probe. No all-consumer scan, history traversal or audience enumeration. Verify the actual EXPLAIN/VM work before claiming the bound.

| Owning operation | Exact coalescing placement |
|---|---|
| `control::advance_cursors` | OR tuple.1 across its existing by-thread loop; call private helper once for the resolved binding if true, inside the idempotent transaction before returning `CursorsAdvanced`. Retained transaction result skips mutation and counter changes. |
| `lazy_delivery::complete_displayed_for_binding` | For each already selected target, OR tuple.1 across its existing thread loop; call once for that target if true. Preserve existing Lazy completion and update-hook touches. Historical unbound calls already enumerate N targets: added work is N times the per-binding bound, not constant for the entire call. |
| `first_contact::fast_forward` | OR the bool from every existing low-level `advance`; call once after all gaps have advanced and before returning the summary. MAX_THREADS caps rendered descriptions only. Existing callers in `seats.rs` supply the enclosing transaction; do not add a nested transaction or change zero-consumer/subagent admission. |

Own reads may free capacity only when existing terminal-receipt plus own-display predicates both hold. They cannot retire Pending captures or sibling-unread content. `binding_cursors` is WITHOUT ROWID, so retain the existing safety pass for a private-only change; do not add the presentation table to `MOD_NOTIFY_TABLES` and create a self-kick loop. Shared ACK still needs the live-channel hint: B may have completed its own read earlier while Pending correctly retained the body.

## 5. Publication extension, only if its causal control fails

Keep `advance_ordinary_publications` signature and fixed publication range/head SQL. Preserve the stable prefix cursor, 100 manifest visits and at most three seeks. Change only the recipient proof ordering: for each visited manifest use one exact message-key read to verify instance/thread/decision sequence/kind and determine canonical mode; then one mode-selected exact recipient lookup. Ordinary mode uses `prepared_recipients` keyed by preparation/seat and checks thread identity; this relation has no message-id column, so the captured canonical manifest supplies its message association. Lazy mode also checks its stored message identity. Reject missing/mismatched publication facts. The existing UNIQUE(preparation_id,seat_id) keys support both recipient probes; runtime EXPLAIN/VM evidence must confirm their actual use. No recipient join or membership/receipt/history walk belongs in these two probes.

```sql
-- Probe 1, keyed by the manifest's message id:
SELECT delivery_mode FROM messages
WHERE id=?1 AND instance_id=?2 AND thread_id=?3
  AND decision_seq=?4 AND kind='ordinary';
-- Probe 2a, ordinary mode; compare returned thread with captured manifest:
SELECT thread_id FROM prepared_recipients
WHERE preparation_id=?1 AND seat_id=?2;
-- Probe 2b, Lazy mode; compare both returned identities with manifest:
SELECT message_id,thread_id FROM lazy_recipients
WHERE preparation_id=?1 AND seat_id=?2;
```

Only addressed ordinary or Lazy publication sets the existing changed boolean. Once per quantum increment the existing checked private passive generation/mod version. No native attention version, receipt, cursor, Lazy completion or output is written by discovery. Actual pull remains responsible for current canonical eligibility, so sibling-settled unread mail is passive and still-pending mail retains its ACK requirement. Unrelated recipient publication stays silent. Initial Accepted drain does not replace this ongoing publication signal.

Capture indexed EXPLAIN plus SQLite VM/op evidence for both modes and mixed/unrelated prefixes, retaining `store::queries::tests::presentation_publication_range_uses_exact_index_and_bounded_sqlite_work`. No allowance increase: 100 visits, 200 keyed probes, three seeks. Count the message and selected recipient relation accesses honestly; an extra hidden query would require revising the implementation, not calling it two probes. Preserve refresh160/fresh-pull433 source budgets and all existing output/ledger/continuation limits.

## 6. Focused control ownership and acceptance

| Change | Existing authored control gate | Still required before acceptance |
|---|---|---|
| Legacy and negotiation | Existing real/fake `each_channel_receives_only_its_bindings_deliveries`; nine `cli::watch::tests::q3_` controls | Adapt explicit elections without weakening historical byte/frame assertions; direct daemon claim-only Accepted lease omission |
| Mode/lease ownership | Five `service::mod_channels::tests::domain::q3_` controls | All owner/replay/health paths enforce mode; mixed siblings and both replacement directions remain isolated |
| Publication progress | `presentation_q3_quiet_ordinary_sibling_ack_before_materialization_drains_passively`, `presentation_q3_quiet_pending_and_lazy_publications_have_distinct_authority`, `presentation_q3_capable_registry_uses_only_bounded_real_store_reads` | Actual index/VM evidence for both recipient modes; retain existing Lazy/invitation no-idle-wake integration behavior |
| Reader maintenance | Both `presentation_q3_parked_*_resumes_existing_id257` controls and three `presentation_q3_reader_change_*` controls | Many-gap fast-forward and full-display helper-call counts; zero consumers/subagent; final operation rollback. Existing Q1 Pending/unread guards remain |
| Shared ACK hint | `presentation_q3_shared_ack_rechecks_read_captured_body_but_keeps_unread_sibling` | Typed ordinary/Lazy/mixed/all-refused classification, retained replay, writer-release ordering, no post-ACK hint health |
| Failed enqueue | `q3_closed_receiver_preserves_passive_drain_without_an_immediate_kick_loop` | Same-owner restoration OR, replacement isolation, true enqueue then socket failure/reconnect recovery |
| Final send and concurrent hints | Author explicitly deferred these | Deterministic actual pre-send gap in both mode directions and active/passive branches; hint arriving during store read survives success; failed old work cannot restore into replacement |
| Retry eligibility | Author explicitly deferred this | Fixed clock below 60-second due threshold gives no hint-created output/stamps; threshold crossing matches explicit canonical pull and three-offer policy, with no owner-replay refill |

For the final-send race use a per-registry `#[cfg(test)]` one-shot callback stored in a separate mutex, removed before invocation. Invoke after the observation guard is dropped and before the final helper. Pass the captured binding/channel/mode to the callback; bounded events coordinate pause, replacement and resume. Never call user code under the state mutex and never use a process-global failpoint or sleeps. `FakeStore.on_read` is useful for during-store hint races but is too early to prove this final-send gap. The author's existing closed-receiver test uses real unbounded MPSC; do not rename it overflow coverage.

## Handoff checklist

- [ ] Parent supplies actual RED/PASS attribution for each proposed delta; no production author starts from this document alone.
- [ ] Agree the mode/registration/hint interfaces first. One owner edits shared ports/dispatch; publication and reader changes share one invitation-content owner or run sequentially.
- [ ] Preserve old request bytes and final-send fences through both registry directions, then apply only causally supported publication/reader changes.
- [ ] Complete deferred controls against the actual interfaces, update TRUST-POLICY to the measured work, and perform focused validation plus fresh code review.
- [ ] Append deviations and measured outcomes to the reviewed design; retain Q1, DP5, passive Lazy/no-idle-wake, exact binding scope, rollback and original fake/real legacy controls.

No approvals are requested here. The parent explicitly limits this stage to a written plan while it owns runtime baseline work; this author is quiescent after writing and hashing this document.
