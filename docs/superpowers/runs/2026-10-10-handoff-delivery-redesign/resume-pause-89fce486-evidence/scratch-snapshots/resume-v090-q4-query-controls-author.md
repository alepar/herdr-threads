# Q4 actual query-control scratch handoff

2026-10-11. Quiescent scratch-only test assembly against the installed Q4/Q5 source. No candidate edit, Cargo/build/test, runtime SQL, Git/index, Herdr, config or nested-agent action. Scoped rustfmt --edition2024 --config skip_children=true and its --check both exited0 on the scratch module. This is syntax/format evidence only, not compilation or runtime PASS. Parent owns source application, baseline/causal failure attribution, focused execution and reviews.

Files:

- `resume-v090-q4-query-controls.patch`: 81c92843e18fb546f19162b6f42b0c08589558c766e1d9d6281e739c6ad7c211
- `resume-v090-q4-query-controls.rs`: c6802b591f410654d74aa2e4ce6831cd99887ad8ac942ab8e388646598d87e67
- `resume-v090-q4-query-controls-source-seal.json`: records module/mount/production source hashes and exact selectors.

Patch only mounts `tests/store/query_work_q4.rs` from existing `tests/store/queries.rs`, plus the new module. Mount base queries test SHA256 be00df205deaaa6dc115a84a48161d03ff7ec19424fecff058e00b06b9738395. Candidate remained untouched. No existing counter oracle is adapted.

Five authored selectors:

- `store::queries::tests::query_work_q4::q4_episode_private_and_reconciliation_points_have_constant_sqlite_work`
- `store::queries::tests::query_work_q4::q4_scalar_agreement_and_strict_retirement_use_exact_scope`
- `store::queries::tests::query_work_q4::q4_three_event_keys_and_actual_floor_have_constant_sqlite_work`
- `store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers`
- `store::queries::tests::query_work_q4::q4_actual_continued_tail_checks_scope_once_and_charges_retirement`

Actual constants are imported, with no copied predicate SQL. A complete migrated store fixture keeps foreign keys, schema constraints and triggers. Canonical-shaped raw fixture rows isolate SQL behavior; they do not prove that public writers produce each branch. The selected100 keys are fixed:25 current-episode eligible,25 pre-episode excluded,25 nonpair excluded,25 voluntary-terminal excluded. Noise populations128/8192 increase same-seat unrelated pairs/invitations, retained older episodes and timeline events without moving the exact selected current episode or message scalars.

Physical witness expectations are ten indexed alias accesses in fresh eligibility, one messages point plus nine aliases in scalarized reconciliation with zero second messages lookup, and six aliases in scope-only continuation. Duplicate cursor accesses count twice. SQLite's constant outer SELECT EXISTS row is explicitly distinguished from a retained relation SCAN. WITHOUT ROWID cursor PRIMARY KEY searches are accepted as indexed point accesses. Same-key/same-branch VM and FullscanStep expectations use the real statements; logs print representative EXPLAIN plans, SQLite version and actual production source SHA256. No statement count, one-row result or source counter alone is called a physical proof.

The optimized second reconciliation query rejects mismatched message/seat/binding/instance/thread/publication binds with no row, preserving the distinction from Some(false) terminal evidence. Sequence comes from the preceding exact canonical message query in the same transaction; the installed query deliberately has no second sequence lookup/base_sequence comparison. The fixture asserts that first row's literal thread,sequence,decision before supplying it. Strict retirement uses valid state='retired' with retired_at set; no CHECK-disabled contradictory resolved/retired fixture is used.

All three fixed event keys use the actual event SQL. A separately labeled robustness fixture retains all three recognized keys for one invitation and requires the actual floor helper to choose the earliest valid sequence. This proves neither that a public duplicate appends all three events nor that every writer path was exercised; service/native/operator public writer coverage remains the behavioral owner’s responsibility.

Actual private raw discovery must visit100, charge300 added scope probes and render only25; actual exact-ID due helper must charge14 and leave fresh/reconciliation counters untouched. Due setup3 is outside this exact-ID helper invocation and is not hidden in14. Whole-helper VM controls compare identical inputs against both noise populations. These are private-source helpers, not a complete presentation433/source envelope or a public/completion operation latency bound.

A real long UTF-8 body produces the captured continuation cursor. Connection-local SQLite tracing counts actual statements: first page one fresh eligibility and zero continuation; the tail zero fresh and one continuation, including encoding/sizing. The tail separately requires one physical visit,three added scope charges and zero ordinary continuation probes. Explicit fixture cursor movement preserves the same captured ID/offset; loss of modern cursorless invitation scope retires that captured body without emission. The trace guard owns boxed per-connection callback state and unregisters before freeing it, with no process-global failpoint/hook or spawned process. Runtime must verify all these expectations.

Known limits/pending gates:

- Parent compilation and real fixture setup have not occurred for this patch. Any failure must be attributed before changing source or expectations.
- Exact relation/VM witness does not establish a constant elapsed-time/disk-I/O bound across arbitrary SQLite implementations.
- No real256-key reconciliation/lexical-tail capacity test is added here; that remains service-owner coverage against actual10/key/2560 limits. The point witness does not certify the full caller’s hidden SQL count.
- This module does not revise due1803/carry579/reconciliation2560 caps, public/nonprivate filtering-before-LIMIT or inherited completion traversal costs.
- Full ordinary continuation's separate15-probe behavior is not reimplemented; Lazy tail expects its own independent scope charge.

Assembly is quiescent, with no processes started or left behind.
