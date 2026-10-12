# Main v0.9.0 composition: independent quality review

2026-10-11. Static review of the frozen composition, not a runtime PASS and not a roast severity round.

## Source identity and scope

- Source checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`.
- HEAD: `f330bdab3abee8535213228e5465b085b65be750`; HEAD tree: `5576d25210259cada7b4b00c484c33693369f228`.
- MERGE_HEAD / released Main: `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`.
- Reviewed composition tree: `e85bfcb4ac2575c67f97dc4ef511fbf26d0d111b`, read with immutable `git show tree:path` and tree diffs. All source line references below refer to that tree.
- Parent compilation/gates were in progress; their results were not evidence for this review. No Cargo, builds, tests, runtime processes, Git mutations, source edits, subagents, or Herdr actions were performed. This review artifact is the only write.

Reviewed the actual delta from Main and the merged service/store call chains, including presentation owner permits, registry leases/replay, binding ledger stamps, canonical binding proof, ordinary page/exact-key/carry eligibility, reclamation, physical confirmation, Hook reconnect, Watch admission, schema upgrade tests, and relevant frontend/test integration. Read the quality impact and delivery author notes as context, then inspected code independently. The cooperative same-user trust model is preserved; findings do not request adversarial caller verification.

## Findings

### Q1 — Persisted ordinary body continuation can permanently block V2 after a legitimate full read

**Needs correction before composition clearance.** The Main binding-unread source introduces a changing eligibility predicate into D's durable ordinary continuation, without a recovery/skip transition for a message the same binding has since read.

The concrete path is:

1. Publish a sufficiently large ordinary message to a joined thread, followed by another message. A V2 pull can emit its inherited 8 KiB truncated representation while retaining an inbox `Source::Unread` body continuation in `ordinary_cursor` and `ordinary_skip_body`.
2. The same binding follows the body continuation and reads the message whole. This is a supported production action: `src/cli/mod.rs:1344` calls `AdvanceCursors` after a full body read; `src/store/control.rs:2596` advances that binding's cursor through the displayed message without requiring receipt ACK.
3. The next V2 normal quantum calls `ordinary_presentation_page` with the retained cursor (`src/store/invitation_content.rs:2283`). The inbox continuation precheck (`src/store/queries.rs:7042`, Source::Unread branch) requires `unread_ordinal` to remain present. `unread_ordinal` (`:6303`) includes `UNREAD_EXTRA`, requiring the message sequence to be greater than this binding's cursor (`src/store/read_cursors.rs:176`). The legitimate full read has made that predicate false, so the call returns `InvalidRequest("continued body is not captured addressed content")`. The unread scan has a second equivalent failure at `queries.rs:6633`.
4. The enclosing presentation decision fails and saves no updated continuation (`src/store/mod.rs:1296`). `ordinary_done` is still false, so the restart branch at `invitation_content.rs:2265` cannot clear the stale cursor. Later pulls repeat the same error; reconnecting Watch also keeps the same durable binding/consumer state. A pending ACK is not a remedy: the cursor advanced independently, and physical Confirm neither repairs this cursor nor performs a message read.

This is a new interaction between Main's full-read cursor advancement and D's persisted traversal, rather than a generic invalid public inbox cursor. D owns this private continuation and must make progress when its captured item becomes read. Retire/skip the exact continued body or safely restart the bounded traversal after canonical revalidation, keeping other carry, offer stamps, and sibling state intact. Do not weaken public cursor identity checks globally or turn physical Confirm into read/ACK authority.

Add a regression using an actually published ordinary message, a real first V2 quantum with `Source::Unread` body continuation, a real same-binding `AdvanceCursors`/full-body read between quanta, and subsequent delivery of the following message. Assert bounded work, no repeat truncated display, no fabricated ACK, and successful subsequent pulls. Cover Hook and Watch if their durable paths differ. Existing `presentation_v2_keeps_truncated_ordinary_cursor_progress_across_quanta` (`tests/service/delivery_pull.rs:3623`) uses physical receipt fixtures and no intervening cursor advancement; it does not exercise this seam. This finding is established by source control flow; the reviewer did not execute its reproduction.

### Q2 — Retained reclamation tests still assert pre-Main terminality and probe counts

**Test adaptation is incomplete in the reviewed tree.** These are concrete stale assertions, not a request to weaken the new binding-unread semantics:

- `assert_v2_offer_window_reclaimed` (`tests/service/delivery_pull.rs:7481`) expects 768 lazy / 1024 ordinary reconciliation probes for 256 keys. The actual composed reconciliation charges 1+7 per lazy key and 1+3+5 per published terminal ordinary key, respectively 2048 and 2304. Its Hook/manual-ACK variant also needs to distinguish receipt settlement from this binding's full read before expecting reclamation.
- `presentation_reclamation_legacy_terminal_delta_has_exact_bounded_work_on_unchanged` (`:7967`) expects 1280 probes for 256 terminal legacy receipts. The composed path charges 1+3+1+5 per terminal physical key, giving 2560.
- `presentation_reclamation_preserves_logical_precedence_and_explicit_waiver_flags` (`:8062`) publishes a message, edits shared receipt/waiver state, but does not advance the binding cursor. It still expects terminal shared receipt state alone to reclaim the binding's offer. The composed canonical unread test correctly preserves that offer. This fixture needs explicit read/unread cases, with genuine per-binding display completion for the reclaiming case.
- `presentation_reclamation_requires_published_addressed_lazy_completion` (`:8121`) likewise treats a direct seat-wide `lazy_recipients.state='displayed'` update as proof that the current binding completed it, despite the composed binding-lazy predicate. Related neighboring probe assertions still reflect the smaller prior envelope.

Audit these neighboring reclamation fixtures as a group. Keep physical/sparse precedence, missing-key conservatism and waiver coverage, but use canonical binding read state as well as receipt state and update exact counters to the approved envelope. A test merely changing its expected result without a paired still-unread sibling control would lose its original regression value. Parent runtime gates may already expose these failures; no result is claimed here.

## Supported static conclusions and remaining limits

- Service admission resolves the claim to a canonical binding before registry/ledger use. `binding_scope` independently resolves the claim and validates connected top-level placement/execution; `pull` compares the internal `PresentationInput.binding_id` hint with that result before effects. Foreign hint/snapshot validation is explicit.
- Durable consumer/item/proof ownership is keyed by binding, while the per-seat permit only serializes decisions. Pull/Confirm retain the owner permit through store work, ledger installation and replay application. Register/replacement/close/disconnect use that permit and target exact registry entries. Store work is outside registry/ledger mutexes; those mutexes are not nested by the replay path.
- Replay scope includes BindingId, lease, generation, execution and ledger incarnation; complete Offer stamps fence progress. The registry rejects sibling snapshots and abandons a pass after incarnation loss rather than recapturing it in the same lease. Ledger capacity remains 4096 binding entries with 256 offers each; registry capacity remains 64 Watch entries with at most 256 captured stamps per owner. Per-seat permit capacity is a separate bounded weak cache.
- Normal ordinary queries pass a bound claim; exact-key hydration accepts pending obligation OR this binding's unread ordinary state. Reclamation tests canonical unread before retiring settled ordinary offers. Lazy eligibility uses binding cursors. Physical Confirm retires presentation descriptors/control masks and does not move message cursors or settle receipts. Foreign/current/last tokens and emitted-control subsets are checked before effects.
- V1 and V2 Hook share plain reconnect; V2 selects the bounded presentation view, without the legacy ordinary digest. Watch requires an already connected binding and exact Live lease. New sibling tests cover independent outboxes/carry, foreign tokens, foreign due snapshots and replay scopes; registry tests cover sibling replacement and owner permit serialization. Those source assertions are not execution evidence.
- The explicit reconciliation/due/carry limits are 2560/1803/579 respectively. Carry and normal/due work are guarded as mutually exclusive. 4363 is the combined ordinary reconciliation-plus-due envelope, not an all-SQL count; relevance, publication, invitation and output accounting remain separate. Several older tests have not yet caught up, as Q2 identifies.
- Main migration files 1..35 are byte-unchanged in the tree diff; only 36..45 are added. The new `published_v35_upgrade_preserves_exact_ack_bindings_and_delivery_history` test constructs an actual V35 schema, retains physical and sparse `ack_binding_id`, deliberately fails V37's post-copy FK check, checks rollback to V36 and exact retained data/catalog, then retries and reopens. This is meaningful source coverage; its runtime result remains a parent gate.
- Health evidence has two intentionally distinct routes in the current tests: strict fresh physical Watch Attention confirmation, and the retained legacy observer socket-push cutoff (`tests/service/delivery_pull.rs:8685`). The strict test explicitly excludes an observer pass. Accordingly, this review does **not** assert that all health state originates only in physical Confirm; it did not classify the deliberate legacy behavior as a new composition defect.

No final composition, runtime, merge-readiness, or parser-guard PASS is given. Q1 requires implementation and causal regression coverage; Q2 requires fixture/counter adaptation and focused execution. Later source trees and parent gates require their own verification.
