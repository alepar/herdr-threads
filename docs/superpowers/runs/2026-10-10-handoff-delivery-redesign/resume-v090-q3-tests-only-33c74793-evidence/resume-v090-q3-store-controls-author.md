# Q3 publication and reader controls — tests-only author

2026-10-11. Parent-authorized tests-only authoring after the materialization run finished (reported 2/2 PASS, scoped leaks 0). Candidate is `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`; parent-selected base HEAD `16ede4ed2eaf3c24b62077f8f38cb0faf24b23cc`, tree `9b0f6151d01e5b071b4cfbda28e9af62286da9b3`. No independent Git attestation was performed.

Read the full candidate TRUST-POLICY, design `resume-v090-q3-channel-design.md`, and both appended specification/quality PASS re-reviews. Used systematic-debugging and test-driven-development for hypothesis-driven controls. The design remains a tests-only gate; this report grants no production clearance.

## File ownership and seal

Only candidate `tests/service/delivery_pull.rs` was changed, plus this outer-ROOT ignored WS report. The source edit adds the Q3 helper/control block before `TestClock`; existing tests and numeric test/runtime budgets are unchanged. No production, Git/index, Cargo, test runtime, SQL runtime, Herdr, seat or configuration command was run. No subprocess or server was started; no delegated agents were spawned. The author is quiescent for the parent's format/compile/baseline stage.

Source SHA-256 after parent-requested owned-file formatting (`rustfmt --edition 2024 tests/service/delivery_pull.rs`, exit 0): `7656622f9e72066c927f3c5913a861ae0890326066d67a2c09b84b3c0f0c02a9`.

## Exact new selectors

All are library test names under `service::delivery_pull::tests::`:

1. `presentation_q3_quiet_ordinary_sibling_ack_before_materialization_drains_passively`
2. `presentation_q3_quiet_pending_and_lazy_publications_have_distinct_authority`
3. `presentation_q3_parked_reader_then_ack_resumes_existing_id257`
4. `presentation_q3_parked_ack_then_reader_resumes_existing_id257`
5. `presentation_q3_shared_ack_rechecks_read_captured_body_but_keeps_unread_sibling`
6. `presentation_q3_reader_change_is_binding_local_coalesced_and_noop_stable`
7. `presentation_q3_reader_change_respects_current_primary_or_refresh_scope`
8. `presentation_q3_reader_change_failure_rolls_back_cursor_and_private_state`
9. `presentation_q3_capable_registry_uses_only_bounded_real_store_reads`

For a single bounded baseline selection, the shared name fragment is `presentation_q3_`. The parent owns all exact Cargo/nextest choices, leak scopes, required checks, formatting and subsequent source freezes.

## Causal construction

`Q3Channel` uses a real SqliteStore, canonical fixture claims, lifecycle check-ins, real DomainService registration/lease, a recording sink and real V2 Pull. It consumes initial Accepted-equivalent drain/continuation before observing and clearing registry frames. `GuardModStoreReads` delegates actual bounded view/refresh/stall reads; invoking its legacy aggregate view panics. No fake store or default bounded-to-legacy fallback supplies registry evidence. Existing implicit-V2 registration fields compile as-is; explicit mode election is the only planned later API adaptation.

The publication race creates connected same-seat Claude siblings with distinct process fingerprints, sends through the actual producer, verifies no physical `receipts` row exists for B yet, completely reads and advances only A, and ACKs through the actual service before attention materialization. It then runs the real bounded attention job and requires the native attention scalar to remain unchanged. Subsequent success requires a normal registry frame before B's Pull, exact passive body/no ACK/no wakes/no controls, terminal receipt preservation and unchanged B cursor. A registers its own genuine Watch owner and is checked separately for no settled body past its completed cursor.

The ordinary-pending/Lazy positive uses actual producers and ordinary materialization, then verifies the differing resulting message flags and canonical authority. Transport Attention is treated solely as a request for Pull. Lazy presentation does not create receipts or complete its row; ordinary presentation leaves Pending intact.

Both parked controls send all 257 identities before freeing a slot. A real first-message attention materialization generates their initial channel input; only that captured frame and actually advertised continuation justify issuing the first 256 through real V2 allocation. They obtain actual `more:false`/Unchanged quietness, settle registry observation and freeze clock below DP5 due. The first event is either own full read plus AdvanceCursors or manual ACK; neither may evict an unread/Pending offer or alter the parked tuple. The second canonical event must cause an actual frame, then ID257 output with no new send, synthetic notification, private counter bump, reconnect, unseen-ID257 display or logical-history reset. Remaining 255 stamps must remain exact.

The captured-body control creates an ordinary partial through two real 58,000-byte sends, first-message attention materialization and the resulting channel-driven page. It snapshots the real current partial, reads full public continuations and advances only B when selected, proves Pending still retains the capture, and then ACKs through sibling A's actual service route. After publication observation is already consumed, a normal registry frame and Pull must retire the exact read/terminal capture. The unread-B variant must retain or output it. Native attention stays unchanged over the deliberate shared-settlement interval. Actual setup/retirement outcomes are pending execution; no runtime Q1 or failure claim is made.

Every `frame_then_pull` has the explicit finite bound `ceil(M/100)+2` successful registry passes, where M is the read-only count of this fixture's already-published instance manifests. This is the existing stable-prefix publication contract, with no concurrent arrivals, new invitations or deadline transitions; it also bounds the zero-prefix/current-entry hint case. It never polls Pull looking for a desired identity. Continuation follows only an actual Page.more=true, stops immediately on more=false/Unchanged, and retains the existing offer-window fixture's 256-pull ceiling. The publication controls have M=1, the captured control M=2, and the parked controls M=257. A first-event maintenance Pull in the parked tests is performed only when an actual first-event frame exists.

Reader controls use real full body read plus AdvanceCursors and zero/one/two existing canonical consumer rows. They preserve every column except the two allowed private signal counters, all logical offers, sibling rows/cursor and canonical receipt/acceptance/completion authority. Retained replay and fresh no-op must be stable. Scope fixtures deliberately alter only negative/control headers: primary-current, refresh-only-current, both stale and foreign-instance. Overflow fixtures deliberately exhaust each existing checked counter; a SQLite trigger aborts a later Watch row update to prove cursor and earlier private updates roll back together.

## Existing proof controls retained

No older legacy oracle was removed. Existing service controls retain Lazy publication/foreign-prefix budgets and ordinary pending delivery. Existing library selector `store::queries::tests::presentation_publication_range_uses_exact_index_and_bounded_sqlite_work` provides actual production publication SQL/index/SQLite VM work evidence for 128 versus 8192 retained manifests; the parent should retain it when assessing a scanner correction. Mode-specific point-query evidence and mixed ordinary/Lazy probe ordering must be assessed with the eventual correction; the current author adds no production SQL or enlarged allowance and claims no executed physical-plan result.

## Evidence limits / next action

Static source/API inspection plus successful owned-file rustfmt parsing only. No compilation or test outcome is asserted. Parent must compile first, classify admission/fixture/API errors separately from causal RED, and execute the controls on this candidate. Expected missing-signal assertions do not establish a defect until the parent observes them. A passing causal hypothesis removes its proposed production correction rather than motivating a manufactured failure.

The reader control owns AdvanceCursors coverage here. Many-thread fast-forward, full-display coalescing work counts, service success classification/writer release, registry pending-bit/replace/sink races, and JS idle/context behavior belong to the parent's other scoped authors; this file does not claim to cover them.
