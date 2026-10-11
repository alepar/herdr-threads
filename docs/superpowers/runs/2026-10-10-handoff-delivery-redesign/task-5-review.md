### Spec Compliance

- ✅ Spec compliant for Task5's local producer contract. The new migration captures physical/prepared high waters once without changing migration0030 or the canonical cutover (`migrations/0031_receipt_deadline_backfill.sql:3`, `src/store/schema.rs:605`). Both repair queries constrain ordinals before LIMIT, charge every returned row, include unpublished prepared rows, and persist progress/completion (`src/store/materialization.rs:65`, `src/store/materialization.rs:92`, `src/store/materialization.rs:126`).
- ✅ Stored clocks and terminal/waived state win; dormant physical Pending receipts derive send1000+duration300, and pending ACK settlement persists that original deadline independently of A6 (`src/store/effective.rs:1910`, `src/store/receipts.rs:453`, `tests/store/effective.rs:1248`, `tests/store/effective.rs:1556`). Historical terminal NULL clocks and ACK provenance remain covered by the preservation/restart matrix (`tests/store/effective.rs:1317`).
- ✅ Repair consumes at most ceil(limit/2), unused allowance returns to due selection, completion resets ordinary cursors before selection in the same decision transaction, and inspected includes repair plus due rows (`src/store/receipts.rs:570`, `src/store/receipts.rs:599`). Rollback/restart and one-shot completion tests exercise the failure seam, old cursor retention, removed tails, all-terminal501-row accounting and empty repair with100 due rows (`tests/store/effective.rs:1332`, `tests/store/effective.rs:1373`, `tests/store/effective.rs:1424`, `tests/store/effective.rs:1477`, `tests/store/effective.rs:1512`).
- ✅ Real old-schema fixture, populated FK graph and physical reopen precede migration; tests retain operation results/preparation/recipient/manifest bytes and the original cutover510 across restart and arrivals (`tests/store/effective.rs:991`, `tests/store/effective.rs:1109`, `tests/store/effective.rs:1248`). Provisional0031 is valid against this task's local base; serial integration must renumber it later.
- ⚠️ Cannot verify the global H/D/I fairness bound, ordinary due-query high-water SQL/full allowance lending, invitation scheduling, blocked wake continuation, multi-originator warning delivery or final default-feature integration here. These remain explicitly Task7/controller-owned; ordinary SQL still applies source high waters after selection (`src/store/receipts.rs:583`, `src/store/receipts.rs:590`, `src/store/receipts.rs:608`). This approval does not establish those downstream contracts.

### Strengths

- The repair is a small indexed producer with durable cursors, not an unbounded startup rewrite; completion is a one-shot transition and terminal-only work terminates (`src/store/materialization.rs:40`, `tests/store/effective.rs:1332`).
- Warning checks retain the existing single coalesced event and verify every intended2/100/500 obligation's exact open receipt-condition association rather than replacing row discovery with aggregate event counts (`tests/store/effective.rs:1044`, `tests/store/effective.rs:1072`, `tests/store/effective.rs:1109`, `tests/store/effective.rs:1424`, `tests/store/effective.rs:1512`).
- ACK-before-repair covers both physical and existing sparse pendingNULL through the public authenticated settlement path; original1300 and effective6000 remain distinct (`tests/store/effective.rs:1556`).
- Named outside-diff preservation risk checked: the reused projector could overwrite running deadlines/provenance or fabricate availability. Focused trace of `project_receipt` shows COALESCE preservation and pending-only updates (`src/store/materialization.rs:697`); the manifest projection derives availability separately and preserves stored deadlines (`src/store/effective.rs:1797`). Named fixture-validity risk checked: `v28_fixture` executes actual immutable migrations0001..0028 and enables FK checking (`tests/store/effective.rs:160`). No broader source crawl.

### Issues

#### Critical (Must Fix)

- None found in the Task5 scope.

#### Important (Should Fix)

- None found in the Task5 scope.

#### Minor (Nice to Have)

- Historical validation budget concern: the report records RED compilation times2m17s and1m48s, exceeding the1-minute incremental budget. Final frozen build42.62s and required lint56.64s meet it; retain the earlier overruns in run evidence rather than claiming all builds met budget (`task-5-report.md`, “TDD and commands so far” and “Final verification and commit”). The initial nice permission diagnostic/canceled attempt and fixture compile error are disclosed separately from RED.

### Assessment

- **Task quality:** Approved.
- **Reasoning:** The local upgrade producer preserves clocks/history, bounds actual visited work and atomically couples its completion to fresh due traversal. The remaining global fairness and warning-routing obligations require the explicitly assigned downstream integration checks.
- **Checks:** Read the full review package, recovering display-truncated portions from the package itself; no source mutation or Cargo reruns. Required test stat: `tests/store/effective.rs`674 added/changed lines and `tests/store/schema.rs`4 changed lines; total674 insertions/4 deletions across2 files. Required test diff preview was deliberately limited to400 lines; the complete package supplied the remaining test hunks. No test deletion, skip/ignore addition or behavioral assertion weakening found: old hardcoded version30 becomes LATEST_VERSION, schema catalog count75 becomes76 and rootpage exclusions add the new table (`tests/store/effective.rs:1699`, `tests/store/effective.rs:1827`, `tests/store/schema.rs:5692`, `tests/store/schema.rs:6266`).
- **Checks:** Reported final evidence is10/10 dormant,45/45 effective,130/130 connection,22/22 materialization and65/65 receipt executions, all passing/zero ignored; clippy exits0 without diagnostics and fmt check exits0. These are reported evidence, not independently rerun results (`task-5-report.md`, “Final verification and commit”). Full suite/default-feature and combined Task7 proof remain unverified here.
