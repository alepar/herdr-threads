# Final review (bounded): epic ht-76u6, bytes-first journal

Invocation inv-4A9435FD-FDDB-4EB7-AF22-DFF91E3F97A3 · range e6815f803bb2cd2b8d30c90b62d53c80cf0216fb..57b39343caa74230890066a15659a8b49a25b4e2 (168 changed paths, +36339/-802) · integration branch super-auto/bytes-first-journal

## Scope and route

- Bounded captured-source review. review-inputs.json reports status unavailable, sourceAvailable true, wholeRangeReady false, 168 selected, 0 omitted, with 4 provenance gaps: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md lines 51 (inv-8759cc82-264d-4506-beaa-307c9600bb8d) and 58 (inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f), at base and head, each an invalid failed assembly archive. No whole-range package was built or loaded.
- Both SHAs verified with `git rev-parse --verify`. The caller checkout stayed at HEAD 57b39343c with a clean status for the whole review, and nothing was switched or restored.
- Earlier bounded review inv-239E1565 covered head fbe242d66: the migration SQL, schema runner, journal.rs storage, apply and promotion, sync_status and parts of serve.rs. This review focuses on (a) the fbe242d66..57b39343c delta, which holds the roast-pr-1 fixes ht-76u6.18 to .22, and (b) product code that inv-239E1565 listed as unreviewed: src/sync/envelope.rs, frame.rs, chain.rs, coordinator.rs (non-test), coordinator/ingest.rs, coordinator/serve.rs and journal_port.rs.
- Inspected hunks are in bounded-coverage.json. Test-file hunks were read only in part (the new tests of the roast fixes, and the registry-hold sweep) and are not claimed as inspected.

## Step 1: own review against the spec

Spec: docs/superpowers/runs/2026-10-18-bytes-first-journal/2026-10-18-bytes-first-journal-design.md (one table, raw as truth, NULL projection plus parked_layout for unknown layouts, promotion as a projection update, no wire change).

Roast-pr-1 fixes (fbe242d66..57b39343c), checked against the roast's confirmed findings:
- CHANGELOG (Should-fix): fixed. The opaque-relay and wire-check bullets moved into a new `## Unreleased` section above `## v2.2.1`. Cargo.toml stays at 2.2.1, to be bumped at release time. This settles the earlier review's only must-fix.
- promote_parked full scan (Should-fix): fixed. `any_layout_understood()` (production: a constant false) returns before any query, and PARKED_SQL adds `apply_state<>'applied'` so the partial index `journal_unapplied` applies. An EXPLAIN QUERY PLAN test pins the index use. The sibling `sync status` cannot_promote query (src/store/sync_status.rs per_log, `WHERE apply_state='parked_layout'`) still scans the whole journal on every `sync status` call. It was deliberately left (ledger Task 12 out-of-scope-nit). It is a Nit on an operator path, not the apply hot path.
- Opaque forked predecessor (Should-fix): fixed. The `fork_cut` recursive CTE seeds on every non-HTJRNL01 journal_forked row of a frozen own log and follows `counter+1`/`prev_hash` through journal_forked. Its known descendants leave the src=1 candidate arm and `pending_reissues`, and are counted under `cannot_reissue` unless an older binary already re-issued them. `UNION` keeps the recursion finite. A new test asserts waiting=0, pending 0 and cannot_reissue 3 across two passes. Parameter numbering checks out in all three uses (candidates_sql uses ?1 for the kind, pending_reissues and cannot_reissue use ?1 for log_ix). Residual, unchanged from base design: a known fork row chained on an opaque main-chain row above the freeze passes `forked_ready` through `held_at` and is re-issued while its opaque parent is cannot_reissue. This matches the main-chain arm's accepted behaviour, so it is not a new defect.
- user_version re-check (FYI): fixed. `run_delta_transaction` re-reads user_version after `BEGIN IMMEDIATE` and rolls back with Ok(false) when the version is already at or past the delta. `apply_deltas` re-reads the version after each delta before `verify_at`. On the rebuild path, foreign_keys is still restored on every path. A two-connection WAL race test covers it.
- Process-global layout registry (Nit): fixed for the lib test binary. A thread-reentrant `LAYOUT_GATE` serializes `mark_layout_understood` guards against `hold_layout_registry()` holds. The token is !Send, so it cannot drop on another thread. Every lib test that both stores an HTJRNL99 row and runs a registry-sensitive path takes a hold: the registry is read only by journal::open (promote_parked), the apply pass and the sync_status cannot_promote count. A heuristic scan of the lib-included test files found unheld HTJRNL99 users only in pure envelope/frame/chain unit tests (no registry read) and in tests/store/journal.rs rows with a zeroblob magic (never registered). opaque_promotion runs as its own [[test]] binary, and the combined binary registers nothing.
- operations/TRUST-POLICY "2.1.1" wording (Nit): fixed to "any release before 2.3.0 (2.2.1 and older)".

Previously unreviewed product code (own pass):
- envelope.rs: EntryPrefix parse/write (68 bytes, big-endian), the CANONICAL_MAX/RAW_MAX const assertion (design §1), and OpaqueEntry::new: prefix, size bound, non-HTJRNL01 magic, counter at least 2, prefix equal to header. verify is SHA-256(raw) == hash, then verify_strict. StoredEntry is closed (two variants), and `genesis_pubkey` refuses opaque entries. The 2.3.0 design §6 is satisfied.
- frame.rs: `OPAQUE_CAP = "envelope.opaque.v1"`. WireItem::Known serializes exactly as the bare WireEnvelope, and a golden-bytes test pins entries and branch_entries bytes captured before WireItem existed (no wire change). Items are decoded by hand from RawValue. `into_stored` checks the size before decoding hex. Known spot-confirmed nit: a malformed `opaque` item falls back to a WireEnvelope error message (roast-pr-1 unverified nits).
- chain.rs: check_next is unchanged in order and only generalised over StoredEntry, so opaque entries follow the same duplicate, fork, gap and freeze rules. A forged opaque entry is Corrupt, never a fork. Tests cover each arm.
- coordinator/ingest.rs: an oversize opaque item travels as an unauthenticated claim after the decoded prefix. It may stall a log only after authentication: same log, `counter == contiguous+1`, prev == head, prefix match with a non-HTJRNL01 magic, SHA-256(raw) == hash, and verify_strict against the log's key. Otherwise only the source is refused. The PastFreeze outcome maps to Frozen. `(None, None)` is unreachable because the actor never queues an empty batch without a claim. I found no defect.
- coordinator/serve.rs: `gate_batch` is the only place serving reads the capability. A capless peer gets the known prefix. An all-opaque cut parks the log per link (`withheld_logs`), and a `want` from before the cut lifts the park. Branch replies use the same gate, and `more` ends at the cut. Inbound `branch_entries` with an oversize opaque item decode as malformed. That is fork-path only and consistent with the frame cap, so it is no finding.
- coordinator.rs (non-test): OPAQUE_CAP is advertised by default. `CoordinatorOptions` is compiled unconditionally (known roast nit). `oversize_logs` pauses a log until restart, as documented. held_genesis_pubkey filters to known entries.
- Spec composition: decoder classification by raw magic (journal.rs row_select/StoredRow::decode), the parked_layout insert, and promotion (fills the projection, moves parked_layout to pending, clears `blocked_on` only when the row is the next position, never writes raw/hash/signature, and runs in a savepoint) agree with design §§3-5. A promoted foreign-layout row stays Opaque for chain, verify and serve (Key decision 3).

No new Critical or Important defect was found in the inspected scope.

## Step 2: ledger triage

- Recurring minor, missing-evidence (×6 across ht-76u6.1, .4, .5, .6, .8, .19): process only, with no product impact. It does mean the per-task green claims rest on prose, which is one more reason the caller's deferred full-suite sweep is required. Deferred OK.
- No `parked`, `parked finding`, `Recurring blocker` or `BLOCKED-AUTH` lines exist. The fix-pass line (Task 14, ht-76u6.21) records that the registry race was first "half fixed". The seam fix (commits 109943464 and 0601a91fc) added the holds, and the scan above found no remaining registry-sensitive test without a hold.
- Task 12 test-self-skip (tests/store/journal.rs early-return test silently returned when another test held a registration): resolved at head. The test now takes `hold_layout_registry()` and asserts `!any_layout_understood()`.
- Task 4 test-process-global-state: resolved by ht-76u6.21 (see above).
- Task 15 stale-test-filter: the post-rebase green run may not have included `second_connection_skips_delta_applied_while_it_waited`. The full-suite sweep covers it. It is not a defect.
- Task 10 known-null-projection-gap, Task 5 duplicated-batch-logic, Task 9 known_sql derivation and decode-before-limit, Task 4 unbounded promote_logged set and untested compact cannot_promote rendering, Task 1 untested non-autocommit refusal branch, Task 6 restart-not-reopen (promotion-on-open is not exercised by the relay restart test), Task 13 optional guard test, Task 12 design-deviation (any_layout_understood reads the registry in test builds, which is correct behaviour), and the scope/placement/duplication nits are all Deferred OK.
- Task 8 suite-over-budget (371 s from a cold release build): per project memory, not a release gate. Deferred OK.
- Metrics: ledger-check ok, but ledger and Git task identities differ (git-only ids from the bundled peer-wire and earlier runs), so do not read absence in the ledger as clean.

## Verdict
not ready (bounded review only: whole-range coverage and provenance incomplete; no full-suite measurement of head 57b39343c exists yet)

## Must fix before landing
- none

## Untested scope
- Whole-range review outstanding: only the hunks listed in bounded-coverage.json were reviewed; every other changed path and hunk of the 168-path inventory is unreviewed by this invocation.
- Provenance gap: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (failed bounded archive contains successful assembly or split claims), at base and head.
- Provenance gap: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (failed bounded archive contains successful assembly or split claims), at base and head.
- No full-suite measurement of head 57b39343c exists: the caller's deferred `cargo nextest run --locked --all-targets --all-features` sweep plus check-no-leaked-processes with --run-id must run before landing; the branch is not to be treated as tested.
- Unreviewed product hunks: src/sync/envelope.rs hunks at -500, -517 and the test hunk -1087; src/sync/frame.rs test hunk -874; src/sync/coordinator.rs test hunks from -2480 on; src/sync/incarnations.rs, src/sync/incarnations/sink.rs, src/sync/incarnations/worker.rs, src/sync/resync.rs, src/sync/memory_journal.rs, src/sync/link.rs; src/store/journal.rs hunks other than -911 and -1608 (this invocation); src/store/incarnations.rs hunks other than the eight listed; src/store/schema.rs hunks other than -344 and -362; src/store/sync_status.rs hunks other than -80 and -98; src/store/apply.rs, src/store/connection.rs; src/protocol/output_compact.rs, src/protocol/sync_admin.rs, src/service/sync_admin.rs; src/test_support/*.
- Unreviewed migrations in this invocation: migrations/node/0005_bytes_first_journal.sql and 0005_bytes_first_journal_after.sql (covered by inv-239E1565 at fbe242d66; unchanged since).
- Unreviewed tooling and CI: scripts/check-wire-compat, scripts/freeze-peer-wire, scripts/release-qualify, scripts/tests/test_check_wire_compat.py, scripts/wire-compat-baseline.txt, .github/workflows/ci.yml, .github/workflows/full-suite.yml, Cargo.toml, AGENTS.md, RELEASE.md.
- Unreviewed tests and fixtures: everything under tests/ (only the new roast-fix tests and the registry-hold sweep were read in part), including tests/fixtures/peer_wire/** and tests/support/**.
- Unreviewed run records and evidence: docs/superpowers/runs/2026-10-18-peer-wire-compat-hardening/**, docs/superpowers/runs/2026-10-18-bytes-first-journal/** other than the design and roast-pr-1, docs/superpowers/specs/INDEX.md, evidence/**.
- Ledger completeness unverified: ledger and Git task identities differ (git-only ids in Metrics).
- Promotion at store open is not exercised by the relay restart test (Task 6 restart-not-reopen); only store-level tests cover open-time promotion.

## Deferred OK
- src/store/sync_status.rs per_log cannot_promote query lacks the `apply_state<>'applied'` idiom, so each `sync status` call scans the journal (operator path; one-line follow-up).
- Known fork row chained on an opaque main-chain row above the freeze is re-issued while its parent counts cannot_reissue (same as the accepted main-chain arm).
- WireItem::from_json reports a malformed opaque item with a known-envelope error (diagnostics nit).
- CoordinatorOptions/start_with compiled unconditionally though only test_support uses it.
- Recurring missing-evidence cluster in task reports (process only).
- known-null-projection-gap: unreachable without corruption.
- promote_logged set grows per failed row per process (production never registers a parser).
- legacy_entries duplicates Batch limit logic; known_sql string derivation (tested); decode-before-limit (pre-existing).
- Untested non-autocommit refusal branch in apply_delta; compact cannot_promote rendering untested; failed-promotion log line untested.
- Test placement, helper duplication, fixture literal duplication and missing RED runs (process and code quality).
- Suite at 371 s over the 300 s budget from a cold release build (not a release gate).
