# Claude/opencode watch notice handoff audit

Read-only source audit at `d108f38dc341ce6b5cf6723dad16e60909596655`, 2026-11-15. Inspected files were clean against HEAD. No production edits, builds, runtime probes, credentials, services or real-seat thread mutations were performed. The only durable mutation is narrowly scoped bug **ht-0w99** (P2, open). Beads skill was read and context refreshed before filing; searches for “notice loss” and “watch notice” found no existing issue.

## Result

**Both the Claude mod and opencode plugin have the same automatic notice-delivery loss window before native presentation.** Their ordinary-message append/ACK discipline does not protect notices. A watch page can already be canonically confirmed while its notice is still queued in the plugin. A stream interruption/reconnect or plugin reload can then discard the only automatic replay copy. The underlying warning/message history is not deleted; this finding is loss of automatic presentation to the binding, not destruction of canonical history or corruption of an ordinary receipt.

**The pi-specific bounded-queue overflow eviction is not established for either integration.** Both limit individual append batches to 32 items/64 KiB but leave remainder queued; neither inspected core trims the notice queue to a fixed record/byte budget. Other bounded caches contain IDs or diagnostic lines, not a notice-body eviction policy. Do not label this as their queue-overflow bug. Reload/reconnect is sufficient to prove the shared gap.

## Shared canonical path and exact boundary

| Evidence | Consequence |
|---|---|
| `src/cli/watch.rs:391-402`, `probe_mod_watch` | The normal watch client selects V2 presentation when the daemon advertises presentation, watch-presentation and binding cursors. Neither plugin must explicitly opt in to receive this behavior. |
| `src/cli/watch.rs:699-705`, `DeliveryPullV2` | The watch child pulls with the Watch consumer and live lease. |
| `src/store/invitation_content.rs:2755-2762` | Pull calls `attention::settle_carried_notices` before returning the page. |
| `src/store/attention.rs:1353-1377`, `settle_carried_notices`; `:1333-1347`, `settle_offered_notices` | The exact carried oldest prefix is validated, then its binding's `digest_notice_offer.offered_ordinal` advances. No downstream plugin receipt is involved. |
| `src/store/attention.rs:1282-1304`, `notice_offer_page_for` / `notice_offer_page_above` | Fresh notice selection requires projection ordinal greater than that binding's frontier. A settled notice is not freshly selected for the same binding. |
| `src/store/invitation_content.rs:2841-2855`; `:1710-1724` | The nonordinary page gets a persistent outbox token; replay is available only while that token remains. This initially protects the already-advanced notice frontier. |
| `src/cli/watch.rs:735-758` | Watch serializes lines, writes and flushes stdout, then immediately calls `DeliveryPresentationConfirmV2`. There is no acknowledgement from the reading plugin or native session in this decision. |
| `src/store/invitation_content.rs:947-990` | Control subsets are validated, but every non-control descriptor, including a Notice, is retired/confirmed. `emitted_controls` is not per-notice acceptance evidence. |
| `src/store/invitation_content.rs:855-883`, `retire_outbox` | Completion clears outbox text/descriptors. `last_descriptors` retains control stamps only, not notice bodies. |

Thus successful stdout flush plus canonical confirmation is weaker than successful native append. A same-scope reconnect cannot recover this notice through fresh selection or outbox replay. A lifecycle may seed frontiers under its normal rules; this audit does not require a scope change and does not claim a different binding can never independently see the historical warning.

## Opencode

- Notice acceptance into memory: `integrations/opencode/plugin/herdr-threads.js:625-689`. `notice` is allowed, its body enters `S.queue`, and `ackable` is true only for `lazy` or `message` (`:687`). `wakes` is suppressed for notices (`:680`); they are passive (`:173`).
- Append is asynchronous and serialized: `:870-887`. A live append or append backoff prevents the next batch. The stream parser at `:1171-1194` invokes line callbacks without waiting for native append completion, so watch can continue writing/confirming pages.
- Native stored-row evidence: `:813-838`, `append`, calls `session.prompt(noReply:true)` and requires `r.data.info.id`; error/no-ID does not count as successful presentation. `:887-905` then calls `complete(batch, r.id)` only after success.
- `:703-733`, `complete`, records delivered IDs and ACKs only `ackable` items. A notice cannot use this ordinary ACK path to delay the earlier canonical watch confirmation.
- `:487-492`, `dropQueued`, removes all non-inflight queued items; `:571-576`, `onChildExit`, calls it. `:518-523`, `switchChannel`, also clears the queue.
- `:1059-1062`, `dispose`, stops the old owner. `:1255-1269`, `makePlugin`, creates a new core even when a successor waits for ownership. Its `S.queue` and per-session records are in-memory (`:249-252`); the shared guard transfers ownership, not queued bodies. A notice never appended by the predecessor is not retained by the successor.

Stored-row append narrows the vulnerable interval; it does not close it. If append already succeeded, disposal before the plugin records completion does **not** prove loss: the native session already has the body. The supported scenario explicitly uses a notice still queued, with no native append submitted for that notice.

## Claude

- `integrations/claude/mod/hooks/register.js:351-416` accepts and enqueues notices; `ackable` is limited to messages/lazy rows. `:700-707` classifies notices passive and selects them for `pumpLazy`.
- `:705-735` skips pumping when another passive append is in flight or backoff is active; append resolves asynchronously. Only an accepted append reaches `completeBatch`.
- `:434-477`, `complete`, saves delivered IDs and ordinary ACK state after delivery. `:210-215`, `saveRec`, persists `S.rec`, not `S.queue`. `emptyRec` at `:118` holds delivered/unacked/row/version maps, no unpresented bodies.
- `:224-230`, `dropQueued`, removes non-inflight queued items; `:281-286`, `onChildExit`, invokes it. `:1016-1040`, `onSessionEnd`, resets pending append/queue state.
- `:1110-1113`, `dispose`, stops the old child. Core construction initializes `queue: []` at `:150`; `:792-837`, `onLoad`/`loadSession`, restores turn/provisional/submission metadata and delivered records, not queued notice bodies. `:1253-1262` disposes the prior core at session-start/reload entry.
- The predecessor submission/provisional logic is not a notice-body journal. Notices take passive append, not the ordinary context/submit path; an unsubmitted queued notice has neither native stored content nor a restorable queue entry.

Claude accepts an append result without a row ID under its existing cooperative semantics (`:120-124`); opencode requires a returned stored ID. That difference is irrelevant to this finding because the loss schedule stops **before the notice's append** in both integrations.

## Concrete supported loss schedule

1. Same connected top-level binding has a pending, still-applicable programmatic notice N. Another append A is in flight, or a prior append refusal has put the plugin in backoff.
2. Normal V2 watch pull carries N, advancing its canonical notice frontier and retaining an outbox body.
3. The child writes/flushed N to stdout and confirms its token successfully. The plugin enqueues N but cannot append it yet. There is no backpressure/acceptance message coupling watch confirmation to its native append.
4. The watch stream exits while N is not in flight, causing `dropQueued`, or the plugin reloads and constructs a fresh core before N's append.
5. Same-binding reconnection finds N below its notice frontier and no outbox body. Neither plugin has retained N's body for presentation. It can remain unpresented indefinitely despite a successful earlier watch confirmation.

This schedule needs no full plugin crash, malformed frame, giant notice or ambiguous late append. Stream interruption alone plus the explicit non-inflight queue drop suffices. The native SDK can be healthy and all receipt ACK predicates can remain correct.

## Scope and follow-up

Filed **ht-0w99 — Watch notices can be lost before Claude/opencode append on reload or reconnect**. Requested deterministic coverage should hold append A, allow canonical notice N confirmation, interrupt/reload before N appends, reconnect and require N recovery. Include append-refusal/backoff and ordinary receipt non-regression. That test has **not** been run in this audit.

Repair belongs outside the pi epic. A future repair must keep the notice recoverable until the relevant native presentation predicate, whether through explicit canonical deferred settlement or a carefully bounded durable downstream handoff. Merely retaining ordinary ACK IDs or waiting for their stored-row evidence cannot protect non-ACKable notices. The pi canonical proposal is a design input, not authorization to alter Claude/opencode now.
