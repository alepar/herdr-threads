# v0.9.0 store binding composition author

State: authored, static checks passed, QUIESCENT. No Cargo, runtime, Git index, commit, Herdr command or process spawn performed.

Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`, original uncommitted merge HEAD `f330bdab` plus Main `b6dd5dd1`.

## Authored paths

- `src/store/queries.rs`
- `src/store/receipts.rs`
- `src/store/seats.rs`
- `tests/store/attention.rs`
- `tests/store/receipts.rs`
- `src/store/mod.rs` — parent explicitly transferred forwarding ownership after schema author confirmed zero edits and quiescence.

No other production/test paths edited. This report is the requested outside-checkout run artifact.

## Composition

Read actual checkout AGENTS.md, merged TRUST-POLICY.md and immutable Main TRUST-POLICY.md; compared index stages 1/2/3. Main migrations 1–35 untouched.

Receipt settlement keeps Main `ack_binding_id` attribution in physical and sparse receipts while retaining candidate A's original projected deadline before settlement. Catch-up extensions remain separate; waived/terminal receipts do not gain invented clocks. Candidate discovery/origin/backfill changes and Main binding-aware attribution tests coexist.

The inbox preserves Main Reader::Bound, unread streams, binding cursor fences, own warning frontier, own lazy predicate, seat receipt ACK eligibility and prospective reader semantics. Candidate invitation metadata, lazy confirmation/no deadline, ordinary presentation source, exact due keys and body hydration coexist. Capable normal traversal captures raw lazy high-water, seeks one raw lazy/prepared recipient before filtering, and advances rejected candidates: it does not use Main's potentially long filtered unread range walk. Internal presentation refuses an unknown binding reader. Public inbox behavior retains Main traversal.

Exact replay uses canonical admitted BindingId. Ordinary eligibility is pending seat receipt OR that binding's unread addressed publication; a settled ordinary message remains visible to an unread sibling, with no ACK candidate. Lazy eligibility uses the shared binding unread predicate, including legacy_pending; legacy `state` alone cannot hide another binding's unread row. Receipt pending state alone never advances a binding cursor.

Seat views retain Main exact binding lookup and unread/frontier fields. The bounded presentation view skips legacy digest/unread counting and returns zeros for those aggregate fields. Legacy mod view preserves binding-specific counts, candidate confirming-invitation-only wake classification, and candidate invitation content version. Hook reconnect uses Main cooperative mapping/resolve algorithm, then selects bounded or legacy view by an internal boolean; it adds no new continuity inference.

## Agreed interfaces

- `seats::presentation_seat_view(db, instance, seat, ModBindingRef)`.
- `seats::hook_plain_reconnect(context, conn, instance, claim, presentation: bool)`.
- `queries::ordinary_presentation_page(db, context, instance, claim, cursor, budget)`.
- `queries::ordinary_due_context(db, instance, seat, binding, work)`.
- `queries::ordinary_presentation_message_by_id(db, instance, seat, binding, id, due_context, work)`.
- Store forwarding matches agreed ports additions: presentation view BindingRef; refresh BindingId; pulled-notice settlement BindingId; hook reconnect boolean. `src/store/mod.rs` retains both Main liveness methods and candidate presentation methods.

Watch author and frontend author agreed interfaces before edits. Attention author owns the exact-ID settlement implementation; this lane adapted its tests/forwarding.

## Indexed probe accounting

Parent explicitly accepted a composed worst-case 1803 keyed probes: setup 3 + 100 attempts × 18. Ordinary pending branch remains 13 (message 1 + existing effective receipt envelope 10 + hydrate 2); nonpending branch adds 5 canonical unread relation probes, maximum 18. Lazy maximum 11 (initial 1 + recipient/manifest/message/binding/cursor/cursor-membership/membership 7 + hydrate 3). No claim that old 1303 ceiling survives this addition. Parent owns plan/TRUST/budget contract updates and independent review.

100 attempts, 32 hydration/body/output blocks, 100 availability seeks, 433 source visits, 352 sizing operations and 65536 output bytes were not increased. Physical source selection precedes binding filtering. No outside-owned effective helper modification or unbounded discovery added.

## Tests retained and added

All stage-2 attention/receipt test function names remain. All stage-3 attention names remain. The sole stage-3 receipt name absent is `send_does_not_start_timer_from_stale_registered_binding`, already replaced by candidate `stale_registered_binding_keeps_availability_absent_with_send_time_deadline`; its unavailable assertion is preserved with the new original send-time deadline assertion.

New regressions:

- `presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement`: settled receipt plus modern displayed lazy remain readable by B while A has advanced; no ordinary ACK candidate; no mutation to ACK binding or B cursor. Exact accounting assertions B=32 and fully read A=27.
- `pulled_notice_settlement_uses_exact_binding_when_sessions_match_across_harnesses`: same native session under a second harness cannot redirect settlement to the newer sibling binding.

Existing due-key SQL-plan/VM test gains exact binding input, a retained cursor for lazy membership semantics, and composed 18-per-attempt bound. Existing notice tests gain admitted binding IDs. Candidate receipt fixtures using old warning_offer/catch_up columns were adapted to Main binding_id schema.

## Static verification and remaining work

Passed `rustfmt --check --edition 2024 --config skip_children=true` on all six authored Rust paths; passed `git diff --check` scoped to those paths; no merge markers in those paths. Static test function inventory compared against stages 2 and 3 as above.

No type-check, clippy, test, SQL runtime/VM probe, migration run or harness-neutral command was run by this author. Parent is sole Cargo/runtime owner. Remaining risks require parent verification: whole-tree signature alignment with active authors, migration fixtures/constraints, exact SQL plans and composed counter assertions, new sibling unread and notice isolation tests, plus per-change lint/neutral/default-feature checks required by AGENTS.md. No success/runtime claim is made.
