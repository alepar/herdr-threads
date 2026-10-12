# Q3 production reader quality review — PASS (static)

2026-10-11. Scoped review of the reader implementation in the nested `upstream-composition-v090` candidate: `src/store/presentation_reader.rs`, `read_cursors.rs`, `control.rs`, `lazy_delivery.rs`, `first_contact.rs`, and the new module declaration in `mod.rs`. Compared against the revised Q3 design, production interface plan section 4 and TRUST-POLICY's canonical-binding, consumer-ownership, captured-state and rollback rules. This is not a formal code roast; counters unchanged. Other concurrent production owners were not reviewed.

No static correctness blocker found in this reader slice. No Cargo, runtime/SQL execution, Git command or mutation, source edit, Herdr or subagent was used. Parent's preserved 19-FAIL baseline remains unchanged; GREEN, lint and measured SQL work are pending. The verdict covers the source snapshots below, not the entire dirty candidate.

## Findings checked

**Canonical scope and isolation.** `presentation_reader.rs:67–92` resolves the exact BindingId by the canonical binding/seat join, including native session, instance, generation and execution. It does not infer a sibling from seat/generation or take caller-local state as authority. Native session is owned by that exact canonical binding; there is correctly no invented consumer native-session column. The helper is maintenance after the owning operation's admission, so it does not impose a new top-level/V2 admission gate on established zero-consumer or subagent cursor operations.

At lines 93–125 the helper visits only Hook and Watch by their complete fixed keys. It skips absent rows, seat/instance mismatch and rows with both scope headers stale. The primary-current OR refresh-current comparison matches the agreed initialization rule; it neither repairs headers nor revives stale history. `invitation_presentation_consumers` has the `(binding_id,consumer)` primary key in migration 44. No consumer insertion, seat-history update or new binding enumeration appears.

**Checked mutation and preserved state.** Counter values are read as SQLite Values, require nonnegative integers, and use checked addition. Exhaustion yields SequenceExhausted; applicable malformed counters yield StoreCorrupt. Each UPDATE writes exactly `ordinary_passive_generation` and `mod_version`; it repeats the exact key, seat/instance, matching scope and previous counters and requires one affected row. Captured scan generation, cursor, carry, partial body, tokens/outbox, read position and logical offer stamps are untouched by this helper. No receipt, invitation, native wake, health or ledger authority is added.

**Coalescing.** `read_cursors::advance_through_displayed_with_change` preserves the original display walk and returns the actual monotonic-write bool without another cursor lookup. The old public helper delegates and preserves its return contract. Neither low-level advance path performs invalidation. `control::advance_cursors` ORs movement across its by-thread loop and calls once for the resolved binding. `lazy_delivery::complete_displayed_for_binding` ORs movement separately for each already-selected target and calls once per changed target; existing historical target selection and Lazy update-hook touches remain. `first_contact::fast_forward` accumulates movement across every gap, including gaps beyond MAX_THREADS, then invalidates once. No-change paths perform no private update.

**Transactions.** AdvanceCursors invokes the helper inside its existing idempotent transaction before returning the result. Full-display completion receives `&Transaction`; all helper errors propagate with `?`. Fast-forward retains `&Connection` as designed, but its established production calls in `seats.rs` pass their deciding transaction and propagate its error. It does not create a nested transaction or swallow an error. Therefore an overflow/failed Watch update after the Hook update aborts the same canonical transaction containing cursor advancement. This is source reasoning; execution must still prove rollback and retained-result behavior.

**Work accounting.** Per changed operation/binding: one canonical lookup statement touching two indexed canonical relations, two exact consumer point reads and at most two exact updates. The code does not deserialize full presentation payloads or call a state saver. N already-selected historical completion targets add N times that cost. Existing display/fast-forward source walks remain potentially larger; their old work must not be attributed to the new constant per-binding allowance. Static query shape is consistent with the plan; actual EXPLAIN/VM evidence is still required before claiming measured physical work.

## Integration and validation conditions

The helper moved from the proposed `invitation_content.rs` location into private `presentation_reader.rs`, wired by `mod.rs:20`. This is a sound scope split with the same internal interface; record that path deviation in the implementation notes and commit trailer.

Before completion, retain focused zero/one/two consumer, primary/refresh/both-stale/foreign scope, sibling isolation, exact nonempty-state preservation, no-op/replay, both-counter exhaustion and late-write rollback controls. Multi-thread AdvanceCursors, existing multi-target completion and more-than-MAX_THREADS fast-forward should demonstrate one invalidation per changed operation/binding, including zero-consumer/subagent success. Use the repaired controls' meaningful preconditions rather than treating empty snapshots as proof of populated carry/outbox preservation. The parent must run these, required per-change checks, SQL work evidence and contract/accounting updates. No additional implementation change is requested by this review.

## Reviewed SHA-256 snapshots

- `src/store/presentation_reader.rs`: `d98f3f1867193dba41e659f3a3c010072e5f4249076f435cd565de04240e57fa`
- `src/store/read_cursors.rs`: `03f3f89370cbf162733aa6925b68cfdf24f241f727686facaf8221cf2832d218`
- `src/store/control.rs`: `1187c3b45049ccb66c824edd6d25bccad5896675d6ef704055e2a2c48167371a`
- `src/store/lazy_delivery.rs`: `be9775f973992ba97c7a40633563a58531727a5f9a37edc866216046b07dbce0`
- `src/store/first_contact.rs`: `2c11f48b54c123d7a70c220e96cfcfbf3c5bbcbf127936f5fde7e9e32fc40131`
- `src/store/mod.rs`: `dcd276d9da9037a5c0fd8759c85ac16603d262412df6fb0c7c0192c03c8a034a`

**Verdict: PASS for this static reader review. Runtime GREEN and complete Q3 integration remain unproven.**
