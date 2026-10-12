# Q3 registry / dispatch / transport — scoped source review

Verdict: **REVISE — known final active-send ownership gap remains unresolved.** The inspected mode selection, lease/replay/confirmation gates, passive pending-bit handling and post-writer success hint conform to the revised specification by source inspection. No additional material defect was identified in this scoped pass. This is not an overall PASS, runtime verification or integration clearance.

Reviewed quiescent candidate files: `src/ports.rs`, `src/service/mod_channels.rs`, `src/service/dispatch.rs`, `src/daemon/transport/watch_connection.rs` in nested `.worktrees/upstream-composition-v090`. Read the existing worker error/backoff and presentation Pull/Confirm callers only to verify the contracts on which these changes depend. Other authors' active edits and the pending ordinary publication-scanner decision are outside this review. Wire W1/W2 corrections are parent-reported, not re-reviewed here. This is a source review, not a design roast; counters are unchanged.

## Known blocking gap: active final notification still lacks expected owner

`observe_fingerprint` checks `work.channel` and `work.mode` under its first state guard (`src/service/mod_channels.rs:596`), releases that guard, then calls `notify_binding(binding, version, now)` at line 639. The final helper (line 662) checks only the current binding entry's Live state. Replacement between those locks therefore lets C1's result enqueue into C2 and set C2's `last_attention_push`. This includes replacement across Legacy/PresentationV2 modes. The earlier exact check and mode-bound lease do not close that interval.

The new per-registry test callback runs after the first guard is dropped and before the final helper; it can expose the actual gap without pretending a pre-observation store hook covers it. Parent is deliberately obtaining that causal RED before the production fix. Keep it unresolved until the final active helper accepts expected channel/mode and compares them under the same lock as enqueue and health bookkeeping. Keep generic current-seat `notify` semantics explicit when adapting that helper; do not accidentally use a stale captured owner or silently weaken its contract. The passive final branch already compares channel and mode at the enqueue lock.

## Mode and authority assessment

| Contract | Source assessment |
|---|---|
| Initial selection | `dispatch.rs:1034` derives mode solely from explicit `request.presentation_v2` before the first store view: Legacy calls `mod_binding_view`, V2 calls `presentation_seat_view`. The existing role, push-channel harness, binding liveness, seat/hold/instance checks remain common. A mode agreement does not add caller authority. |
| Immutable entry | `ModChannelRegistration.mode` is internal and typed. Registration installs it into a fresh entry, creates a lease only for V2, resets pending/replay state and remains serialized under the existing seat-owner permit. Same-binding replacement selects its own mode; siblings are untouched. The 64-entry admission cap is retained. |
| Worker source split | `ChannelWork` snapshots channel, mode, binding, seat and session. Legacy uses its historical view/version with no V2 refresh. V2 uses the bounded view and refresh. Applying observations checks exact channel plus mode before changing fingerprint/version. Existing replacement and canonical binding checks are preserved, subject to the final active-send gap above. |
| Lease lookup / owner-live | Both require Live PresentationV2. The owner predicate retains seat, binding map key, generation, native session and exact lease checks. Legacy and reconnect/rebind Grace cannot obtain V2 authority. Existing service admission/store checks supply canonical execution fencing; the registry does not replace them with a new heuristic. |
| Replay | Snapshot and application both explicitly reject non-V2 or non-Live entries and retain exact lease/generation/session/binding scope. Replay's full scope and ledger incarnation/stamp checks remain. Replacement starts with no saved replay; no new hint refills it. |
| Pull / Confirm | Existing `delivery_presentation` call sites use `presentation_owner_live` before Watch store work. Therefore the newly mode-aware predicate gates both routes without modifying their canonical admission or rollback behavior. The presentation health recorder separately requires V2, Live and exact seat/binding/generation/session/lease under its write lock. |
| Accepted lease | `watch_connection.rs:159` requires request opt-in and all four advertised capabilities, then filters the registry's channel-specific lease through `advertised_presentation_lease`. Legacy Accepted omits the optional lease; no synthetic lease is minted. A replacement between registration and lookup can yield no old lease and a queued Close, consistent with the documented existing reconnect/fallback handling. |

## Pending hint and failure behavior

`presentation_delivery_may_have_changed` (`mod_channels.rs:946`) visits only `state.seat_bindings(seat)` from the bounded live registry. It marks only Live V2 entries, stores one boolean per entry, releases the state lock, marks the worker dirty and invokes `wake_worker`. This is the existing mod-worker pacer callback, **not** `kick_wakes`/native wake. It does no store work, historical consumer scan, receipt action, offer allocation or health write. Repeated eligible successes can cause repeated pacer kicks while coalescing the same boolean, as allowed; they do not accumulate an unbounded worklist.

At pass snapshot time, `mem::take` clears the pending bit under the registry lock only for Live V2 entries. A hint arriving after that snapshot sets the live entry's bit and is not cleared by the eventual successful observation. Store/view/refresh failure restores the captured bit by setting true only for the same Live channel and mode; this is OR behavior, not assignment from a stale false snapshot. `pass` returns its error, and the inspected existing worker (`workers.rs:1180`) applies pacer failure/backoff plus `mark_dirty`, so restore need not independently create an immediate wake loop.

The passive branch enqueues under the final exact channel/mode/Live lock. A failed push sets the same capable entry's pending bit true without dirtying/kicking solely for the closed receiver; existing safety passes and reconnect initial drain supply retry. A successful push does not clear a newer concurrent bit. Pure private-version changes and captured delivery hints combine into at most one passive frame per observation. When the active fingerprint also changes, the current code sends one active frame; failure preserves a capable recheck, but successful active send remains subject to the known replacement gap.

Transport still treats enqueue separately from socket write: a later write failure unregisters the exact channel, and reconnect's initial drain handles recovery. The concrete sink remains unbounded mpsc and bool enqueue/receiver status; no bounded-queue or exactly-once claim has been introduced. Grace receives no new hint authority, and replacement does not inherit an old pending bit.

## Success classification and writer release

`DeliveryRecheckClass` selects only Ack/AckDisplayed, AckModDelivered and DeliveryConfirm before consuming the mutation. It pairs the class with the admitted request's seat. Ordinary ACK reports qualify when either acknowledged list is nonempty. Mod and hook confirmation reports qualify when at least one item is Settled or AlreadySettled. Errors, wrong result variants, empty/all-refused/retryable/stale reports, unrelated mutations and physical presentation confirmation do not qualify. Lazy success is intentionally included without falsely claiming an ordinary receipt delta or issuing post-commit item classification queries. Retained successful results may conservatively hint.

`cooperative_mutation` now obtains the typed result inside an inner scope holding `_turn`; the scope ends before `class.succeeded` and registry notification (`dispatch.rs:836–887`). Store errors propagate through `?` without a callback. This lexically releases the foreground turn after the synchronous canonical store operation returns and before acquiring registry locks. Existing `ack_mod_delivered` calls `record_ack` afterward for the same successful result classes, including AlreadySettled: the new hint adds no health timestamp and does not suppress established ACK health policy.

## Remaining gate and evidence limits

The parent must obtain the deterministic active final-gap RED, apply its exact-owner fix and verify both mode directions without replacement frame/health contamination. Runtime controls must still establish pending-bit races/restoration, failure/reconnect behavior, result classification and writer release, and the admitted transport/legacy wire pairing. Source inspection of these paths is not a replacement for those checks. The ordinary publication scanner is intentionally unchanged pending independent causal evidence; this report does not endorse or clear that future delta.

Reviewed SHA-256 values:

| File | SHA-256 |
|---|---|
| `src/ports.rs` | `0dd3de6092764630a7f7648cf75630e0fe81be5a50beba3ec7db71c21bb79d79` |
| `src/service/mod_channels.rs` | `897ddebd5f15290c35e66c4176c9ab79221d1297dcfbd276bd29c0984bc5e69d` |
| `src/service/dispatch.rs` | `58a3fbf3df6a647decf83e4774f3a15922fe4abbb13c3de5b6565dc5c364f1e0` |
| `src/daemon/transport/watch_connection.rs` | `7487c8e04f28af99d6f28a28642321ab2f3b5aba00053c37c1f8e81989d02aa2` |

No Cargo, runtime, SQL execution, Git command/write, source/index mutation, Herdr command or delegation occurred. Only this review report was written.

## 2026-10-11 applied final-fence and publication corrections — narrow re-review

Verdict: **PASS for these applied source corrections.** This closes the previously identified final active-send gap by source inspection and accepts the inspected ordinary/Lazy scanner's canonical selection and declared source-work bounds. The earlier REVISE remains historical. Runtime GREEN, physical query-work verification, full Q3 correctness and integration clearance remain pending; no formal roast counter changes.

Verified the exact requested hashes:

| File | SHA-256 |
|---|---|
| `src/service/mod_channels.rs` | `b6c54cd96d0d2ed39e6523fa37d9ef2cd547a1eb875e2b973a7d527edbcf589f` |
| `src/store/invitation_content.rs` | `014b31080186266aa2d5f03db99ff5d13ac493c60bba1f79cf9262df5ebb4d62` |
| `TRUST-POLICY.md` | `1b45e7bc780b6a19cb31d766a96fd5510054a29fcfe20d32eeed5d363ed1766a` |

### Final exact-owner notification fence

`notify_binding` now accepts the captured channel and immutable mode. It compares both plus Live state under the same registry state lock used to enqueue and record the existing successful active-push health timestamp. `observe_fingerprint` passes its captured `ChannelWork` identity. The other call site, direct seat `notify`, captures `(binding, channel, mode)` under its initial lock and passes that tuple into the final helper; it no longer silently retargets an intervening replacement. A source search found only these two call sites.

False enqueue records no health and preserves a recheck only for the exact capable entry under that lock. The existing extra restore in `observe_fingerprint` is idempotent and exact-owner guarded, so replacement between failed enqueue and restore cannot inherit the bit. The passive branch retains its equivalent final identity lock and no-health behavior. New hints arriving during store work remain pending; no new native wake, timer, receipt or replay authority is added. The deterministic observation-gap test hook remains outside the first registry guard.

### Message-first publication proof and 100/200/3 source bound

The scanner retains an instance-scoped, ordered range over immutable `send_manifests`, with the explicit `send_manifests_instance_decision` index and at most 100 fetched manifests. Each fetched row increments visits and advances the private position before filtering, so rejected candidates spend the same source budget.

The first point query selects immutable delivery mode by exact message id while checking the manifest's instance, thread and decision sequence and `kind='ordinary'`. A missing/mismatched message cannot produce an addressed event. Only a matching mode selects the second query:

- Ordinary probes the unique `(preparation_id, seat_id)` prepared-recipient key and verifies its thread equals the manifest/message thread. The manifest already supplies the preparation-to-message relationship; `prepared_recipients` has no message-id column, and the implementation does not invent one.
- Lazy probes its unique `(preparation_id, seat_id)` key and verifies both message and thread against that same published manifest/message tuple.

Schema inspection confirms the two recipient uniqueness constraints, unique manifest message association and unique instance/decision index. No effective-receipt lookup, audience walk, reader scan or extra mode fallback query appears in this loop. One message probe plus at most one recipient probe per visited row gives at most 200 point statements; one possible head capture, one range seek and one possible trailing head check give at most three seeks. Unknown delivery mode fails as corrupt rather than broadening the query. Matching any number of addressed rows increments the two existing private counters once per quantum with checked arithmetic; it does not touch the captured scan generation, canonical reader, receipt state, native attention, offer history or channel health. The canonical subsequent Pull still decides pending versus settled-unread eligibility.

These are source and schema claims, not measured VM/elapsed-time/disk-work claims. The revised TRUST-POLICY states that distinction explicitly and describes the same message-first proof and existing allowance. Shared production query constants support testing the actual SQL. The inherited baseline's prospective-query test failed with `no such table: main.invitations`; that is fixture failure and supplies no physical-work proof. Actual indexed EXPLAIN/VM verification remains a required parent check.

### Neighboring Notice correction: scoped consistency check

The independently reviewed Notice predicate now preserves a carried canonical warning through either its physical warn message or an exact virtual-publication chain. The virtual arm ties the recipient digest's seat/warning/thread and event sequence to the instance/thread manifest, bounds the prepared warning offset by the manifest, matches that preparation/offset/warning id, and requires the same recipient attribution generation. It does not ask current condition actionability or advance the notice frontier to rediscover output. The replay caller uses the predicate only to retain or retire an existing descriptor, preserving the existing token/slot bookkeeping and adding no materialization, ACK, health or wake action. This is a consistency check of the adjacent predicate; its independent detailed review and runtime tests retain their own gate.

### Actual RED provenance and remaining verification

I read `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/run.log`. Both active replacement controls actually produced an old `Attention { version: 3 }` and a health timestamp on the replacement. The independent direct-store-ACK publication control actually observed `(0,0)` instead of `(1,1)`. The virtual Notice replay control actually lost the retained Notice while keeping the same presentation token. These reached failures justify the respective narrow corrections; they are prior-build evidence, not current results. The run also contained unrelated failures and setup errors, so its aggregate result is not blanket evidence for these changes.

The next verification must demonstrate the final-gap controls now pass in both mode directions, the independent scanner oracle advances without a service hint, actual mixed ordinary/Lazy/foreign-prefix SQL work stays bounded, and the Notice preserves its exact token/slots through replay and confirmation. Preserve unrelated unresolved failures and the earlier store-fixture setup qualifications. No claim of current runtime GREEN is made.

No Cargo, runtime, SQL execution, Git command, source/index mutation, Herdr command or delegation occurred in this re-review. Only this appended assessment was written.
