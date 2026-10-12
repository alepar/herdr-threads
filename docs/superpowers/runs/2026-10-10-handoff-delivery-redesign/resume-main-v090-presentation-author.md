# Main v0.9.0 presentation adaptation — author handoff

Source is quiescent. This is an authored implementation and static audit handoff, not a compilation, runtime PASS, or readiness claim.

## Identity and ownership

Shared checkout: `.worktrees/upstream-composition-v090` under the handoff-delivery-redesign root. Uncommitted composition of Main `b6dd5dd122772599a02e2cc7f76b20e5492f0af3` (tree `3a7ba4d535aa4d9556aff9c21207953b4b29f5a6`) and frozen D composition `f330bdab3abee8535213228e5465b085b65be750` (tree `5576d25210259cada7b4b00c484c33693369f228`). Only these assigned files were edited:

| Path | SHA256 |
| --- | --- |
| src/store/invitation_content.rs | c5113b7acb56bf44287623f659d53b67c05b23377fa0fdc7c21d65884658d754 |
| src/store/presentation_relevance.rs | 6f55d44c0e1c6a4ccee7e1c39ab189e982047f033834e67d7a747a23e0dec75b |
| tests/store/invitation_content.rs | d419ceadb625259164ab0a6a9c7a00cefbfcdfc11f198d1c3176071be7bb4ee6 |

## Implemented behavior

The presentation store resolves the claim independently through canonical `bindings::match_claim`, then verifies exact binding ID, seat/instance, binding generation/execution, current target generation, connected/open canonical top-level role, and absence of an active target recovery hold. Wire/internal binding IDs cannot select another binding. No seat-generation equality or observation-provenance heuristic selects a current sibling. The existing canonical binding/session index replaces the removed current-binding index; no published migration was edited.

Presentation consumer state, invitation allocation/revisit/passive state, and warning proofs use binding+consumer keys. Seat remains shared invitation/receipt authority. Proof JSON records the exact binding. Render selection and Confirm independently resolve their claim. A foreign token returns NotCurrentPage without touching the sibling outbox. Physical Confirm still performs no ordinary receipt ACK or cursor mutation. Notice offer/frontier/settlement calls now use that same exact binding.

Normal ordinary pages pass the CallerClaim to the ReaderBound query. Due and owner replay pass canonical BindingId. Reconciliation preserves A-ACKed mail while B is unread; lazy reconciliation uses Main's shared binding-unread predicate, including legacy-pending behavior, rather than historical lazy display state. Missing or ambiguous authority retains capacity. Carry rehydrates and revalidates every retained message against the exact binding, replacing stale body/ACK metadata and dropping canonically ineligible carry. Carry consumes its own quantum even if every item is dropped; it cannot open a fresh ordinary scan in that Pull. Ordinary physical output remains outside the durable control/invitation outbox.

Passive refresh now accepts `binding_id: &BindingId` after seat, reconstructs only the exact canonical top-level claim, and stores passive progress per binding. Public relevance advance/revalidate signatures remain unchanged; their internal canonical admission is shared with presentation.

Publication invalidation is conservatively triggered for an addressed published lazy row regardless of old recipient display state. This preserves native recovery after sibling display without adding cursor probes, tokens, or wakes.

## Work accounting

These are independent envelopes, not an assertion that one counter measures every SQL operation in Pull:

| Category | Maximum |
| --- | --- |
| Offer reconciliation | 256 unique keys; 2560 keyed relation probes; no body hydration |
| Due OR owner replay | setup 3 + 100×18 = 1803 keyed probes; 100 receipt seeks; 32 hydrated messages |
| Carry | setup 3 + 32×18 = 579 keyed probes; 32 receipt seeks; 32 hydrated messages |
| Ordinary publication invalidation | existing 100 visits, 200 probes, 3 seeks |
| Model output | existing 32 blocks / 65536 bytes |
| Fresh discovery/output work | existing 433 source visits / 352 sizing budget retained |

Due, owner replay, carry, and the fresh ordinary page walk are mutually exclusive ordinary source modes. Therefore maximum charged ordinary keyed work is `2560 + max(1803,579) = 4363`; publication work is separate, as are the existing relevance, invitation, notice/control, and physical outbox checks. The new finish guard rejects exceeded ordinary envelopes and carry combined with due/fresh ordinary work inside the existing decision transaction, so rejection rolls back canonical state.

Admission/render/proof reads are fixed overhead outside candidate counters: canonical admission is resolved at Pull, frozen render selection, and relevance advance; each resolution contains one exact session-key read and one exact binding+seat+hold validation statement. Render/state/proof loads and the binding frontier are separately keyed. Existing relevance counters and ceilings, invitation hydration/discovery limits, at-most-32 outbox descriptor validation, token-allocation bounded retry, notice selection, and state/control writes remain their own envelopes. No omnibus whole-Pull SQL count is claimed; `4363` must not be described as covering those operations. No time budget increased.

## Test authorship and intended negative controls

The first binding controls were authored before production adaptation, and carry controls before carry implementation. Runtime execution was explicitly delegated to the parent; no fabricated failing baseline or PASS is claimed.

New real-store test keys under `invitation_content::binding_presentation`:

1. `same_generation_binding_outboxes_and_notice_frontiers_are_independent`: two live bindings on one seat/generation independently receive the notice; tokens differ; sibling-token Confirm refuses; A's Confirm leaves B replayable and ordinary cursors unchanged. Kills seat-keyed state/frontier and foreign-token acceptance regressions.
2. `binding_input_cannot_select_a_sibling_or_canonical_subagent`: mismatched internal binding and canonical subagent with top-level wire claim both refuse before consumer state creation. Kills trusting input.binding_id or wire role alone.
3. `sibling_settled_receipt_remains_in_bound_normal_and_exact_recovery`: B receives A-settled unread ordinary mail without ACK requirement, keeps the exact owner replay stamp including count four, is not reclaimed, and does not mutate the original ACK or B cursor. Kills receipt-only retirement and physical/logical offer conflation.
4. `carried_message_rechecks_own_unread_and_sibling_ack_without_touching_receipt`: pending carry loses stale ACK metadata after A settles it, remains eligible for B until B cursor moves, and then emits no message. Counters must be nonzero and bounded. Kills frozen carry eligibility/ACK reuse and uncharged carry.
5. `carry_quantum_accounts_all_thirty_two_keys_and_refuses_a_thirty_third`: 32 canonical carry rows hydrate exactly 32, emit 32, charge only carry, and forbid fresh/due work; a 33rd persisted row refuses and preserves pre-call state through rollback. Kills no-accounting, mode-combination, and removed-size-guard regressions.

Existing schema characterization fixtures now include immutable released Main32..35 before appended36..45, construct canonical binding parents, and use the correct43→44 statement probe. Renamed old misleading `released_v39_invitation_epoch_alter_records_nested_work_and_preserves_state` to `composed_v43_invitation_epoch_alter_records_nested_work_and_preserves_state`; its assertions/diagnostics now correctly preserve user_version43. These statement characterizations still do not substitute for the production migration runner.

## Verification and limits

Successful source-only checks: `rustfmt --edition 2024 --config skip_children=true` on exactly the three owned files, followed by scoped `git diff --check`. Static caller review covered updated query signatures, store refresh forwarding, Main canonical read predicates, notice frontier/settlement signatures, transaction rollback boundary, schema44/45 conflict keys, and core carry work fields.

No Cargo, build, test, plugin, Herdr write, child process, staging, commit, or merge was executed by this author. No test/helper process was started. Parent must compile the composed source, run the five controls and migration characterization, run causal mutants and existing cancellation/incarnation/control/owner bounds tests, and close the previously observed runtime failures. There is no known unresolved source design block in this lane; runtime and type-check results remain pending.
