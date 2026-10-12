# Q3 store causal corrections — author report

2026-10-11. Prepared the requested patch in ignored WS while the parent's baseline kept source frozen. After the parent reported the run complete and explicitly authorized source unfreeze, applied only the prepared `tests/service/delivery_pull.rs` contents. All original selectors remain. No production, Git/index, Cargo, test/SQL runtime, Herdr, configuration or seat command was run. No processes or agents were started. The author is quiescent for the parent's compiler and causal run.

Base owned-file SHA-256 (parent's count correction included): `004087d446b95bd2d40d2b5914bfa323f48e4e1f05e45922aa4e62aba4b4f80b`.

Applied source SHA-256: `7a3d88fc5fb5af1d6e794b38b3e7190abfc0cb1e70e114d703efbb8b9452ade2`.

Patch: `resume-v090-q3-store-causal-corrections.patch`, SHA-256 `16fc97fcfc5526d169a405e8742ddaf99bbf745238f1da3d959f38711d228ceb`. Its `a/` and `b/` paths name only `tests/service/delivery_pull.rs`. The formatted candidate copy remains in WS; rustfmt of that copy exited 0 before application. Application checked that source bytes still exactly matched the sealed base, then wrote the formatted candidate. No Git patch/apply command was used.

## Shared receipt causal correction (CQ1)

The original selector `presentation_q3_shared_ack_rechecks_read_captured_body_but_keeps_unread_sibling` now constructs 257 real canonical sends: 255 small bodies, the 256th a 58,000-byte NUL body, and a small successor. The long encoded body requires multiple actual advertised continuations. Its inherited 8 KiB truncation is emitted as a real logical offer while the raw body's skip-body cursor remains captured. Draining stops only at actual more:false/Unchanged; the fixture explicitly asserts all 256 real full stamps, a truncated offered ID256, no ID257 offer, and the exact source-created captured skip-body cursor/offset.

B's full public body chain uses the existing maximum public encoded response, `MAX_PAGE_BYTES`, to retain the existing helper's 58,000-byte fixture ceiling and >=1024-byte partial progress rule for NUL escaping. The old helper remains a 4096-byte wrapper; other existing tests retain their requests and loop bound. No timeout, canonical output limit, production budget or sleep was enlarged.

Before A ACKs, `q3_consume_frames_to_quiet` consumes every publication/own-reader frame and its actual continuation. The test then proves exact Pending/full-ledger capture retention, no queued frame and unchanged full offer stamps. A subsequently ACKs only the offered captured ID256 through DomainService. This is the only intervening canonical event; own-reader signals cannot be borrowed by the decisive frame. B's prior display plus that shared terminal ACK frees one slot, permitting the normal Q1 continuation check to retire the exact skip-body and emit ID257. Unread B keeps all stamps and the captured body.

Every frame wait/maintenance sweep retains `ceil(M/100)+2` stable-prefix passes, with no concurrent arrivals or due-clock change. Maintenance pulls occur only after captured frames and follow only actual Page.more. The captured setup is an unexecuted hypothesis: failure to obtain the required full-ledger skip-body/more:false preconditions is explicitly labeled SETUP, not evidence for a missing receipt hint. The parent should validate separation with reader invalidation alone and then with the receipt hint.

## Independent publication control

Added `presentation_q3_ordinary_publication_generation_observes_early_store_ack_without_service_hint`. It uses a real canonical store permit and mutation for A's ACK, bypassing DomainService's post-ACK hint entirely. B's exact consumer is already quiet. After ordinary publication, A's own read/AdvanceCursors and early store ACK, the real materializer must keep native attention unchanged. A direct bounded real-store presentation_refresh must count the manifest and increment both existing private publication counters once. This is the scanner's independent RED oracle; no synthetic notification/version or new authority supplies it.

## Reader retention and coalescing corrections (CQ3)

The original zero/one/two-consumer selector now asserts exact pre-mutation consumer cardinality. Its fixture materializes the real ordinary Pending receipt; before/after snapshots compare exact physical/sparse state, ACK binding and ACK observation rather than row counts alone. Existing scope/rollback/no-op selectors remain.

New selectors under `service::delivery_pull::tests::`:

- `presentation_q3_ordinary_publication_generation_observes_early_store_ack_without_service_hint`
- `presentation_q3_advance_cursors_coalesces_many_real_thread_gaps`
- `presentation_q3_complete_inbox_delivery_coalesces_many_real_thread_gaps`
- `presentation_q3_fast_forward_coalesces_every_gap_beyond_description_cap`
- `presentation_q3_reader_change_preserves_nonempty_progress_outbox_and_offers`

The many-thread fixture creates and joins twelve additional threads through public writers, publishes two bodies per thread, and reads the bodies fully through public continuation RPCs. A single AdvanceCursors or CompleteInboxDelivery operation spans every new thread. Both require one private increment per existing current consumer, regardless of the moved gap count, preserve every other private byte and sibling cursor/row, and compare exact receipt evidence. Completion additionally proves every Lazy row displayed and every intended cursor at the canonical head; retained replay and a fresh no-op add no private signal.

The lifecycle fast-forward variant exercises thirteen joined gaps (original thread plus twelve new threads), a real SessionEnd and lifecycle reconnect. It independently asserts eight returned descriptions, five additional descriptions omitted, and all thirteen actual cursor moves. It covers zero and two presentation consumers, preserves sibling state and exact receipt evidence, and requires one signal per existing current consumer. It does not equate returned descriptions with mutation count. These are new public-operation oracles, not production helper counters or source-text checks.

The nonempty retention fixture allocates real ordinary logical offers, creates a large-body continuation, then produces a canonical Lazy invitation and genuine outstanding Hook outbox/token. Preconditions explicitly require the nonempty cursor, outbox descriptors/token and nontrivial full offer stamps. Own reader success, retained replay and fresh no-op preserve every other consumer column and full stamp, sibling rows, and exact receipt evidence.

Rollback failures now classify actual checked exhaustion separately from the injected SQL-trigger detail; unrelated admission failures cannot satisfy the expected oracle. Atomic snapshots remain. This does not claim to observe an earlier Hook row write before the Watch trigger: it verifies the complete rolled-back outcome.

All fourteen existing WatchRequest fixtures in this owned file explicitly request PresentationV2, because they consume presentation leases/pages. The one direct ModChannelRegistration constructor declares `ModChannelMode::PresentationV2`. No intentional legacy oracle in this file was reclassified; the legacy domain and registry tests stay with their owners.

## Evidence and limits

Static API inspection and successful rustfmt parsing only. Compilation and runtime outcomes are pending the parent. No passing source/test claim or production-clearance claim is made. The parent owns exact selectors, Cargo/nextest execution, process/leak checks, source freezes and fresh review. The register guard still proves bounded registry reads only; DomainService's unwrapped StorePort is outside that guard's interception scope.

Historical unbound multi-target completion, subagent fast-forward, physical helper point-read/update instrumentation, due-clock comparisons and worker/transport race controls remain outside this patch. The many-thread operation counters prove coalescing behavior, not measured helper SQL work. The existing original author report remains as historical first-stage evidence; this correction supersedes its overbroad CQ1/CQ3 coverage claims.
