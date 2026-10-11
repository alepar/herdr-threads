# Main v0.9.0 composition: independent SPEC and architecture impact

Date: 2026-10-11. Read-only source review; no merge, Git/source change, Cargo, test, plugin, Herdr action or delegated agent was run. This report is the only write. No roast severity adjudication or readiness verdict is made.

## Immutable scope and conclusion

- New Main: `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`, tree `3a7ba4d535aa4d9556aff9c21207953b4b29f5a6` (commit subject: `Release v0.9.0`; external release/publication status not independently established).
- Frozen COMP: `f330bdab3abee8535213228e5465b085b65be750`, tree `5576d25210259cada7b4b00c484c33693369f228`, at ROOT/.worktrees/upstream-composition-478ec4a9. Its working tree was clean when inspected.
- Common ancestor / former Main: `026ad3ab444779216748cc5733e2241e2834ebaf`.
- ROOT: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`.

Parent clarified that b6dd is a source-allocation baseline, not yet a verified release claim. “Main-allocated”/“upstream-reserved” below means preserve its immutable migration prefix during composition; it does not claim a tag, distribution or deployed user database was verified.

This is a substantive identity-model composition, not a migration-number-only update. Main now has many session bindings per seat, independent binding read streams and delivery channels, wire version 7, and schema 35. COMP still has wire 6 and redesign schema 41 based on Main026. Its new durable presentation implementation assumes one current occupant and one Hook/Watch state per seat. Those assumptions must be revised before its V2 capability can truthfully be advertised on v0.9.0.

The previous whole-epic source review remains evidence only for its recorded pre-v0.9.0 candidate. Parent's frozen-f330 diagnostics likewise do not validate a future v0.9.0 composition. No runtime outcomes were collected or independently reverified in this follow-up.

## Main-allocated schema and exact collision mapping

Read each Main-allocated SQL file and the production migration/verification code. Byte comparison through `git show` established that all 31 old-Main migration files are identical in old Main, new Main, and COMP. Main contains exactly 35 migrations; COMP contains exactly 41. Preserve Main **1–35 byte-for-byte**, including existing names and SQL, and preserve Main's data-transforming migration functions and integrity checks. Do not resolve a version collision by retaining both differently named files with the same number.

| Main-allocated migration | Exact Git blob | Main effect |
|---|---|---|
| 0032_session_bindings.sql | `b3f71eb889bc98919c81803adef6dd93d451a170` | Rebuilds occupant_bindings with stable binding_id, session-key uniqueness, role/parent/agent/lineage, separate liveness and fingerprint, migrated markers. Preserves ordinal sequence/history; migrates open agents disconnected and open humans connected; seeds binding_cursors at joined-thread heads. Adds receipts/receipt_state.ack_binding_id and messages.sender_binding_id. |
| 0033_archival_scan_scope.sql | `ce711f97fc57193630f55a3604586332ebbb0b64` | Replaces selected archival fences with membership/liveness/lifecycle relevance predicates; ordinary fingerprint refresh must not restart every archival scan. |
| 0034_binding_delivery_state.sql | `349491bd789c562ae5750c0f90f24f2486d5701a` | Adds lazy_recipients.legacy_pending and indexes; carries manifest-published pending lazy mail across upgrade. Rebuilds catch_up keyed by binding_id, maps old rows deterministically to migrated/newest bindings, removes rows lacking any binding, recreates archival triggers. |
| 0035_binding_attention_frontiers.sql | `55d7cf7894162f2648783507bb1426233011d453` | Rebuilds warning_offer and digest_notice_offer keyed by binding_id; transfers only the historical row's matching generation/execution binding, preferring the migrated open row; drops unmatched obsolete frontiers. |

Main schema.rs:70–74 selects these exact files; :655–669 orders migrations and verifications; :874–898 stamps open migrated bindings' disconnected_at with the sampled upgrade time transactionally. This timestamp and the 24-hour archival grace/re-adoption semantics must survive. Main :904–984 audits the rebuilt binding schema and attribution columns; :693–706 audits exact new frontier definitions. A simple union of schema constants is insufficient.

Proposed reserved numbering, assuming none of the redesign SQL has been published independently:

| Frozen COMP file | Combined file |
|---|---|
| 0032_invitation_confirmation.sql | 0036_invitation_confirmation.sql |
| 0033_receipt_send_time.sql | 0037_receipt_send_time.sql |
| 0034_handoff_body_bounds.sql | 0038_handoff_body_bounds.sql |
| 0035_receipt_deadline_backfill.sql | 0039_receipt_deadline_backfill.sql |
| 0036_invitation_lifecycle.sql | 0040_invitation_lifecycle.sql |
| 0037_invitation_warning_origin.sql | 0041_invitation_warning_origin.sql |
| 0038_receipt_warning_origins.sql | 0042_receipt_warning_origins.sql |
| 0039_automatic_handoff_body_bounds.sql | 0043_automatic_handoff_body_bounds.sql |
| 0040_invitation_presentation.sql | 0044_invitation_presentation.sql |
| 0041_presentation_relevance.sql | 0045_presentation_relevance.sql |

36–43 can preserve their payloads where source/schema audit proves independence, with migration labels/references corrected consistently. 44–45 need semantic adaptation for binding-owned private presentation state; they cannot simply be renamed and called compatible. Since these files are unlanded, adapt them before integration instead of first integrating an incompatible per-seat shape. If private test databases at redesign32–41 exist, treat them as a distinct development schema, not as databases created by Main v0.9.0: do not reinterpret user_version alone or guess ownership. Use fresh isolated stores/explicit development recovery; never rewrite the upstream-reserved prefix.

Update the initializer's empty-store path, every version threshold, user_version assignment, migration transaction, exact SQL audit, expected catalog fixture, downgrade refusal, renamed temporary-table references where actually renamed, and genuine historical fixture cutpoints. A label containing an old design number is not authority to rewrite already Main-allocated SQL. Main32's dropped `occupant_bindings_current` index is a concrete compatibility issue described below.

## Actionable composition findings

### V090-1 — durable presentation ownership cannot remain per seat

Evidence in COMP:

- migrations/0040_invitation_presentation.sql:26–108 keys consumers by `(seat_id,consumer)` and items by `(seat_id,invitation_id,consumer)`; migration41 private warning proofs use the same seat/consumer ownership.
- store/invitation_content.rs:399–439 resets this shared row when a different claim execution/generation arrives. That is correct for the old single-occupant model, but alternating legitimate sessions on one seat would replace each other's durable outbox/carry, replay tokens, notice bookkeeping and context history.
- service/mod_channels.rs:95–98 has `HashMap<SeatId, Entry>`; the finite owner replay capture and permit are attached to this ownership model. service/delivery_presentation.rs:165–185 uses `ledger.seat_mut` and the seat offer capacity.
- Main service/mod_channels.rs:114–118 has `HashMap<BindingId, Entry>`, a by-seat index and binding cooldowns; Main delivery_pull.rs:342 uses `ledger.binding_mut(seat,binding_id,generation)`.

Required adaptation: canonically resolve one BindingId, then carry it through V2 admission, store state/items/proofs, registry owner capture, channel health, ledger, pending capacity, clears, confirmations and replay. Keep seat obligations and canonical invitation/context sources shared. A per-seat mutex may remain a conservative serialization mechanism, but it must not imply shared durable ownership or close/reset another binding's channel. Bind physical replay to the exact binding plus existing live owner incarnation/lease/execution fences. Resume of the same session keeps its binding/offered state; another session gets independent state. Do not use mapping generation as a substitute for binding identity.

The D spec's storage-size claims stated in seats now need an explicit binding-based accounting amendment. Bound per-binding rows/outbox/carry and global live-owner capture independently; define lifecycle cleanup without deleting Main's durable binding/cursor history. Preserve the finite owner capture (including captured-empty, no replenishment, full Offer stamps, no logical re-offer increment on physical replay), output and per-quantum budgets. Do not hide the extra cardinality by retaining only one binding per seat.

### V090-2 — D's admission is both too narrow and missing Main's liveness/role semantics

COMP invitation_content.rs:262–269 requires the caller's declared top-level role and `cooperative_top_level` observation provenance, with no canonical row role or connected predicate. The refresh query at :2663 has the same provenance assumption. presentation_relevance.rs:361–367 performs an old-style claim/generation/session probe. These new files would merge as additions without a textual conflict while retaining the old trust model.

Main TRUST-POLICY A2–A5 and ports.rs:2629–2648 instead admit open, connected, top-level bindings of the appropriate harness, regardless of enrollment provenance. A plain environment-attributed enrollment can remain `cooperative_env_session`; changing its provenance to make D pass would invent evidence. Canonical subagent roles must refuse even if a claim has a top-level shape. Disconnected superseded/end-event rows cannot regain a channel through an ordinary call. Main's permitted fingerprinted plain reconnect after process exit/migration must remain available through its canonical resolver, without supersede, placeholder replacement or fast-forward.

Use Main's binding resolver and ModBindingRef::Claim/binding view rather than reproducing part of it in new SQL. Make store transaction checks agree with service admission and refuse stale/disconnected/subagent scopes before presentation mutation. Preserve binding enrollment provenance separately from action receipt provenance and retain actual ack_binding_id/sender_binding_id attribution. Historical NULL attribution remains unknown; do not backfill it from whichever binding happens to be current.

### V090-3 — readers, notices and ordinary carry must follow the reading binding

Main queries.rs:6202–6257 chooses Reader::Bound from BindingKey::from_claim and pins the reading binding's identity. Its inbox includes that binding's unread settled mail as well as the seat's pending ACK obligations. Main lazy_delivery.rs:41–82 uses binding cursors plus upgrade-only legacy_pending, not a seat's old lazy display state. Main delivery_pull.rs:388–393 asks for notices of the specific binding.

D must thread the claim/binding through all ordinary page sources, persisted continuations, re-offer/replay filtering, partial carry, passive publication changes and relevance decisions. Filling new InboxQuery.claim fields with None just to compile would silently revert to the seat reader and lose the new contract. An ACK by binding A does not make an unread message disappear from B. Settled mail to B is passive and cannot wake B merely because D still classifies it as ordinary; preserve Main's explicit `wakes` distinction for wake_on:all lazy promotion.

COMP invitation_content.rs:2490–2498 settles carried notices using seat/generation/execution. Main migration35 removes those frontier columns in favor of binding_id. Adapt allocation-time notice settlement to the exact binding frontier, and preserve the frozen outbox replay guarantee after settlement. A's CheckIn or bound inbox read must neither settle B's notice nor erase B's captured outbox. D presentation confirmation remains presentation bookkeeping, never receipt ACK or cursor advancement inferred from output bytes alone. Keep Main's actual full-display completion/AdvanceCursors paths and truncated-message handling; read_cursors.rs:234–259 only counts this binding's own display ACKs when walking past a held cursor.

### V090-4 — new relevance SQL forces a removed index

COMP store/presentation_relevance.rs:530 contains `occupant_bindings INDEXED BY occupant_bindings_current`. Main migration32 rebuilds the table and creates session_key/open/history/execution indexes, not occupant_bindings_current; Main effective.rs:1945–1953 already removes this forced index from its own human-waiver query. Reusing the D query unchanged fails to prepare when the evaluator reaches this branch on the Main-allocated schema.

Adapt this probe to the new schema and re-establish its bounded indexed lookup. Audit new SQL outside textual conflicts for the same old assumptions, plus renamed warning/notice frontier columns and catch_up keys. This is a direct source incompatibility, not merely a test-fixture mismatch.

### V090-5 — A/B/C must preserve new session semantics at their clean merge seams

A: receipt deadlines and obligations stay seat-level. Keep the redesign's immutable send-time cutover, real availability timestamp, finite physical/prepared repair, canonical originator warnings and bounded discovery. Main's availability is now derived from any qualifying open connected registered binding, and last disconnection/check-in drives the seat episode. Do not restore a newest-binding/provenance-only availability query when merging receipt-warning code. Carry ack_binding_id through physical and sparse receipt settlement and retain sender_binding_id in all native send paths. Main receipts.rs:597 writes these fields. Origin warning ownership stays the canonical origin seat, not whichever of its bindings reads first; per-binding presentation frontiers do not merge warning origins or ACK obligations.

B: invitations and required episodes remain seat-owned. Preserve Lazy/timed acceptance and initial-only Ensure policy, exact retained policy/deadline/origin on reuse, no implicit accept, no rejected/left resurrection, and the earlier Main026 fresh-required-link/history rule. Main control.rs:660/:1007 additionally initializes binding cursors on direct-thread creation/acceptance (also join, required accept and general creation). Keep those hooks in the combined paths without causing duplicate membership episodes or resetting existing cursors. A subagent cannot accept/invite through a top-level-shaped claim that resolves to a subagent binding.

C: frozen body, always-addressed ACK-required handoff body, fixed routing prompt and diagnostic-only readiness remain valid. New binding seeding at the thread head must not hide the pre-launch pending handoff receipt; pending obligations remain visible irrespective of read cursor. Keep Main's fingerprinted managed_launch placeholder beside existing bindings and lifecycle replacement rule; native structural safety is not the readiness advice being relaxed. Main topology_handoff/persistence.rs:526 changed live validation to cooperative_bound_mapping; preserve that canonical binding resolution in all explicit and automatic retained-operation paths, including hidden subagent-role refusal. Do not restore pane-local journal identity as the caller authority.

Modern automatic Some policy and bounded summaries/recovery remain redesign requirements. Main adds optional agent_id/liveness to CallerClaim, which is embedded in frozen semantic digests and automatic original/child/attachment/recovery envelopes. Re-run serializer-derived admission accounting with the actual combined claim shape and maximum supported fingerprint/IDs; retain 512-KiB modern originals/attachments, 2-MiB modern recovery, historical bounds and request-family-specific wire limits. Preserve absent optional fields' old serialization and original stored digests; no migration may substitute today's fingerprint or binding into a historical frozen operation. Live binding resolution and immutable original identity are separate concerns. Unsupported pending operations must refuse before new effects, while completed historical inspection must remain truthful.

## Wire, capabilities and native hook ownership

Main protocol/wire.rs:37 is version 7; COMP :36 is version 6. Keep Main’s wire skew guard and claim validation. Union Main's binding-cursor and binding-digest capabilities with redesign send-time, invitation, modern Ensure and V2 presentation capabilities only where the combined canonical handlers implement them. V2's explicit recorder/adapter readiness must not be replaced by unconditional advertisement. Old wire peers fail at Main’s guard; this does not justify silent fallback inside a modern request or rewriting retained local/store records.

Main adds agent_id/liveness claims, per-binding query claims, AdvanceCursors and binding lifecycle events. Audit constructors and trait implementations beyond files that conflict. Main hooks also add/expand native SubagentStart/SubagentStop/SessionEnd handling, Claude command attribution rewriting and Hermes witnessed CLI enrollment/child lifecycle; required_hooks is a new adapter-driven stale-install diagnostic seam. Combine those with the redesign's stable event-qualified hook commands and exact ownership manifests. Do not take one side's full native registration array: a stable command lacking Main's required end/subagent event loses liveness/role correctness, and a Main-style event lacking the redesign's registration validation loses the stable ownership contract. Stale install findings stay diagnostic for C's launch readiness; structural event/payload matching and actually-used native configuration ownership checks remain mandatory.

Normative references read: Main TRUST-POLICY's binding vocabulary/A2–A5/A7–A8 and accepted limits; Main session-keyed-seat-bindings design decisions 1–15, sections 2–4 and post-implementation notes; the previously reviewed root/four-child redesign specifications, task13–15 and resume refinements supply A/B/C/D's continuing contracts. Main's post-implementation note explicitly confirms schema32–35, resume-keeps-binding, per-binding delivery and role/liveness gates. Reconcile TRUST-POLICY as a union of these behavioral contracts, preserving the corrected A7 origin-wake exceptions, both check-in/hook opportunities and passive-observer accepted limit; neither complete file can safely replace the other.

## Safest isolated implementation sequence

1. Preserve the frozen candidate and its diagnostic artifacts. Create a separate integration worktree/branch rooted at the immutable new Main; do not mutate or promote ROOT as part of this review. Record both immutable tips and mapping of later automatic fixes. Use old Main026 as the three-way ancestor.
2. Reserve 36–45 and port schema/A/B prerequisites in order on new Main, preserving Main migrations 1–35 byte-for-byte and Main's migration code. Perform explicit schema catalog/audit reconciliation, not a global numeric substitution. Keep genuine v31 and v35 upgrade fixtures distinct. Establish v35→combined and v31→v35→combined compatibility before treating newer migration fixtures as authoritative.
3. Port receipt/invitation/Ensure behavior onto Main's accountable actor/binding/cursor APIs, maintaining seat-level obligations, sender/ACK binding attribution, canonical availability, fresh required links and all first-policy/reuse rules. Review semantic clean merges, especially materialization, control, services and warning wake selection.
4. Port C's body/startup/diagnostic launch, automatic admission/result/recovery and capability paths with Main caller identity, fingerprinted launch and bound mapping. Keep immutable historical serialization and independently account for new claim fields. Existing f330 diagnostic fixes can be imported with exact source mapping after this architectural reconciliation.
5. Amend D's ownership specification explicitly for binding-scoped presentation and bounded cardinality, then implement migrations44–45 and all private storage/service/registry/read/notice seams consistently. Retain Main's per-binding V1 behavior while V2 is incomplete; do not advertise partial V2. Preserve D's strict output/confirmation/physical replay/control budgets and no-ACK semantics.
6. Compose stable hook installation and Main lifecycle/claim derivation as one adapter-driven event inventory. Audit registered-hook membership, native event/payload checks, exact owned manifest migration, foreign/drift refusal and Hermes enrollment. Reconcile policy/docs/help against actual final behavior.
7. Review the resulting entire production diff against both parents before promotion. A textually clean merge of a new D file is a review hotspot, not evidence of compatibility. Only then run the authorized focused verification and exact-tip gates in the parent's isolated validation slot.

Future verification must include real Main schema 35 upgrade/reopen/rollback; exact 1–35 byte comparison; Main31→35→combined histories and migrated re-adoption; two concurrent top-level bindings on one seat alternating Hook/Watch pulls and confirmations; independent notices/outboxes/cursors/capacity/health; stale owner confirm and same-binding resume versus another session; env-enrolled top-level admission and canonical subagent/disconnected refusal; own/sibling display ACK semantics; settled-unread passive delivery and promoted lazy wake; pending pre-launch handoff body despite cursor seeding; preserved originator warnings and fresh-required links; bounded D discovery/replay/carry; maximum combined automatic envelopes, response loss and historical absent fields; and native event ownership/lifecycle inventory. These are required future evidence targets, not claims that they ran here.

Source-scoped result: migration and architecture impact identified with actionable adaptations. The frozen candidate has not been validated against v0.9.0 and is not asserted ready for promotion or final review.
