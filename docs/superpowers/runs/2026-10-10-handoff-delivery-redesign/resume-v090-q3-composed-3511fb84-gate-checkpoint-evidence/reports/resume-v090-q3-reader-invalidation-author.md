# Q3 reader invalidation source handoff

2026-10-11. Owned candidate files only: `src/store/read_cursors.rs`, new
`src/store/presentation_reader.rs`, `src/store/control.rs`,
`src/store/lazy_delivery.rs`, and `src/store/first_contact.rs`. Parent owns
`src/store/mod.rs` wiring and all Cargo/runtime/test execution. No test, Cargo,
runtime SQL, Git/index, Herdr, config, or nested-agent action was performed.

Read the revised Q3 channel design and production-interface plan section 4;
inspected the actual causal RED log at
`/private/tmp/ht-C-v090-q3-baseline-compile-corrected/run.log`. Its three reader
controls fail because current-scope counters remain zero and overflow does not
abort the canonical cursor operation. This report does not claim GREEN.

The public `advance_through_displayed` u64 API delegates to an internal companion
returning `(reached, actually_changed)`. The companion uses the existing cursor
read and the existing monotone write's bool; no extra cursor query or private
invalidation occurs in the low-level helpers.

`control::advance_cursors` ORs actual changes across all existing joined-thread
walks and invalidates its one canonical binding once. The existing idempotent
transaction replay bypasses this mutation closure. A fresh no-change operation
does not invoke the private helper. `lazy_delivery::complete_displayed_for_binding`
ORs across each already selected target's thread walks and invalidates that
changed target once. Existing Lazy completion and update-hook touches remain.
`first_contact::fast_forward` ORs all low-level cursor writes and invalidates once
after every gap, including gaps beyond the MAX_THREADS description cap.

The helper resolves the exact canonical BindingId through occupant_bindings and
seats, reading seat, instance, generation, execution, and native session. Native
session is owned by that canonical BindingId; consumer rows have no duplicate
session field, and no binding is selected by seat/generation/session heuristic.
Each fixed Hook/Watch key is read separately. Only matching seat/instance plus
current primary generation/execution OR current refresh generation/execution
is applicable. Both-stale/foreign rows remain unchanged; no header is repaired.
For an applicable row, generation/counter corruption is StoreCorrupt and checked
counter exhaustion is SequenceExhausted. Exactly the two passive counters are
updated, guarded by exact key, current scope, and old counter values. No consumer
is inserted, no full-state saver runs, and no captured scan, token, offer stamp,
cursor, carry, partial body, receipt, acceptance, health or wake authority is
rewritten by the helper.

Source-derived additional work per changed operation/binding is one canonical
lookup statement joining two indexed relations, two fixed consumer point-read
statements, and at most two exact-key update statements. Existing membership
or message/gap traversal is unchanged. Historical unbound completion already
selects N targets: added work is N times this per-binding envelope, not a constant
bound for the whole operation. Physical EXPLAIN/VM measurements remain required;
this author performed no SQL runtime and claims no measured VM bound.

All three owning production paths supply an existing transaction. Any helper
failure propagates from the owning closure, so cursor writes, earlier tentative
Hook/Watch counter changes, existing completion writes, and later transaction
writes roll back together. The helper does not add a nested transaction or
require V2/top-level/connected admission. Zero consumers, including legitimate
subagent reader use, adds no durable presentation writes. Private-only counter
changes retain the existing safety pass; no notification table is added.

Remaining cursor writers are enrollment/re-adoption `seed`,
`init_for_new_binding`, `copy_lineage`, and membership `init_for_join`. They are
unchanged per the selected section-4 scope. The parent was told that widening
reader invalidation to those initialization/membership operations requires an
explicit boundary decision; no hidden per-low-level-write invalidation was added.

Formatted only the five owned files with
`rustfmt --edition 2024 --config skip_children=true`. Parent module wiring
`mod presentation_reader;` was observed in src/store/mod.rs after handoff.
No processes were started or left running by this author. Source is quiescent.

SHA256:

```
03f3f89370cbf162733aa6925b68cfdf24f241f727686facaf8221cf2832d218  src/store/read_cursors.rs
d98f3f1867193dba41e659f3a3c010072e5f4249076f435cd565de04240e57fa  src/store/presentation_reader.rs
1187c3b45049ccb66c824edd6d25bccad5896675d6ef704055e2a2c48167371a  src/store/control.rs
be9775f973992ba97c7a40633563a58531727a5f9a37edc866216046b07dbce0  src/store/lazy_delivery.rs
2c11f48b54c123d7a70c220e96cfcfbf3c5bbcbf127936f5fde7e9e32fc40131  src/store/first_contact.rs
```
