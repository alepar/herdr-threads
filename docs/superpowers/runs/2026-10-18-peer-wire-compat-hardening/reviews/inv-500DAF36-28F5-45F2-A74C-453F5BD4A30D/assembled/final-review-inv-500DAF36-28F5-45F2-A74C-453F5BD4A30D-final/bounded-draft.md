# Final review (bounded): ht-j87z peer wire compat hardening

Invocation: inv-500DAF36-28F5-45F2-A74C-453F5BD4A30D
Captured source: base f78f076ab076a7efc939415ee780084fb56185f3, head af71d4e6ad54ebb47db93f43b14992f56a98863e. Both verified with `git rev-parse --verify <sha>^{commit}`. The caller checkout was at head and clean when this review started; nothing in it was edited, switched or restored.

Route: BOUNDED. The selection sidecar `review-inputs.json` reports status unavailable, sourceAvailable true, wholeRangeReady false, no omissions and four retained provenance gaps. No whole-range package was built or loaded. I read the full 118-path changed inventory. I inspected the hunks listed in bounded-coverage.json from captured Git. Every other path and hunk is unreviewed.

## Step 1: my own view (branch diff against design)

What the design asks for and what I found at head:

- Prefix invariant (design section 1). `EntryPrefix` writes the first 68 bytes of `Body::canonical_bytes`. The `MAGIC` doc comment states the cross-generation invariants: the 68-byte prefix, SHA-256 and ed25519 `verify_strict`, hello envelope generation 1, and HTJRNL01 genesis.
- Opaque entry. `OpaqueEntry::new` checks shape in this order: length, magic, counter (counter 1 gives `NotGenesis`), then the prefix against the header. `verify` checks SHA-256 over the raw bytes, then `verify_strict`. On ingest the signature is checked through `chain::check_next` against the genesis key of the held log. An opaque entry is never genesis: `check_next(None, ..)` reaches `genesis_pubkey`, which returns `NotGenesis`.
- Wire form and gate (section 3).
  - `WireItem` decodes by hand: a probe for an `opaque` key, falling back to `WireEnvelope`.
  - A known item is byte-identical to 2.1.1, pinned by `entries_frames_golden_bytes`.
  - Only two outbound sites build `Entries` or `BranchEntries`: the serve job and the branch serve in `serve.rs`. Both pass through `gate_batch(…, entry.opaque_cap())`, so a link without the capability gets only the known prefix.
  - A batch cut to empty parks the log in `withheld_logs` for the life of the link. A `want` from before the cut lifts the park. A want at or past the cut stays parked, so there is no hot loop.
  - The branch reply ends with `more: false` at the cut. An empty prefix is answered `branch_none`.
- Oversize (section 3, plus ht-j87z.22).
  - The coordinator moves an over-bound opaque item into an `OversizeClaim` before hex-decoding it, and keeps the entries before it.
  - `authenticate_oversize` binds the claim to the frame's log and to `contiguous + 1`. It checks prev against the head, the prefix, SHA-256 over the raw bytes and the signature against the genesis key.
  - Only a verified claim gives `IngestOutcome::Oversize`. That outcome adds the log to `oversize_logs`, an in-memory pause until restart; the size line is logged once and no source is penalised. An unverified claim penalises only its source.
- Storage (section 4).
  - Migration 0004 creates `journal_opaque`: the raw CHECK is 68..=393216, `branch` is 0 or 1, and a partial unique position index covers `branch=0`.
  - It adds immutable and append-only guard triggers, with the `journal_opaque_promoting` exemption.
  - Cross-table position guards run in both directions.
  - `CHAIN_ROWS` and `BRANCH_ROWS` are the union sources. `verify_invariants`, `hash_at`, `entries_from` (serve, branch serve, `entry_at_position`), `next_ready_head`, `mark_opaque_heads`, the `sync_status::verify` chain-link check and the incarnations readers (`held_at`, `branch_prev`, the CANDIDATES opaque arm) read them.
  - `Rows::KnownPrefix` keeps the store-level `Envelope` readers known-only.
  - Query plans checked with the system sqlite3 3.51.0 against the head migrations. The `mark_opaque_heads` and `hash_at` lookups use the `(log_ix,counter)` indexes. `next_ready_head` flattens the UNION ALL and scans journal via its covering index. The base query also scanned `journal` with no stats, so I see no plan regression.
- Apply stall (section 2). An opaque head is skipped (`c.opaque=0`) and marked `blocked_on='opaque_layout'`. `applied_through` is unchanged and no `has_more` spin follows. Dependents block on the existing `deps …` text. `LogRow.blocked_on` (serde default), `LinkRow.withheld` and `RepairRow.cannot_reissue` reach `sync status` text, and each is printed only when non-empty.
- Resync and re-issue. Salvage uses `serve_entries`, so opaque entries cross a resync byte-exactly, and a stop on an opaque entry is named. Re-issue partitions opaque candidates into `cannot_reissue` and refuses an opaque source as StoreCorrupt.
- Mixed versions and docs (sections 5 to 8). `OPAQUE_CAP` is in the hello by default and `advertise_opaque: false` is test-only. The TRUST-POLICY accepted limit, `docs/operations.md`, release.md 8a and 8b, the CHANGELOG, AGENTS.md and the CI lint step are present.
  - CI now sets `HT_REQUIRE_TAGS=1` and fetches v2.1.1 and v2.0.4. This closes the silent skip of `frozen_copy_matches_tag_source` (ledger Task 10, a misgraded minor). It is resolved at head.
  - `scripts/check-wire-compat --base f78f076` ran read-only at head: ok, 0 violations, 0 stale, 255 fields, 18 variants, 268 baseline entries.

Cross-task seams found:

- **Node schema number collides with main (blocking landing).**
  - The branch registers `NodeDelta { version: 4, sql: 0004_journal_opaque.sql }` in `src/store/schema.rs`.
  - Observed: `main` is at 9f1ca770d, 24 commits past the merge base f78f076. It already registers version 4 as `migrations/node/0004_sibling_delivery.sql` at the same position in `NODE_DELTAS`.
  - Merging gives a textual conflict in `schema.rs`. Two different version-4 schemas would also exist: a store opened by a binary built from this branch would be `IncompatibleSchema` for main's binaries, and the other way round.
  - docs/release.md 8b predicted this ("release/2.2.0 already ships its own 0004"). The ledger graded that line an unverifiable minor (Task 11). It is now verified: the collision is on main itself, not only on a release branch.
  - The new test `journal_opaque_delta_upgrades_highest_main_store` (tests/store/schema.rs) hard-codes `NODE_DELTAS[..2]` and `user_version == 3` (ledger Task 13, hard-coded-slice). After a renumber it no longer tests the "highest main store" upgrade it is named for.
- **Promotion decision is not settled by its owner.**
  - The design's Post-Implementation Notes contradict each other. The first note says promotion keeps the `journal_opaque` row with a `promoted` flag. The third defers promotion to ht-evlm.
  - The shipped 0004 builds the delete model: `promote_opaque` deletes the row under the `journal_opaque_promoting` exemption. `promote_opaque`, `apply::unpark`, `LayoutParser` and `understood_layout` have no production caller (`#[allow(dead_code)]`).
  - The ledger's ht-j87z.13 BLOCKED line says the choice needs the owner. The deferral was recorded by the autonomous run, not by the human, and it leaves R4's "apply after upgrade" half undelivered. ht-evlm is open.
  - Schema 0004 is a one-way upgrade. A later model that keeps the row needs a further delta, which is acceptable, but the human should accept the deferral explicitly.
- **Opaque entries in own-log re-issue (minor, edge).**
  - The CANDIDATES opaque arm removes opaque rows from re-issue. Known main-chain entries above the freeze that follow an opaque entry are still re-issued, so the re-issued chain skips the opaque position.
  - A forked known candidate whose prev is an opaque branch row (`branch=1`) can never satisfy `forked_ready`, so it stays `waiting` indefinitely.
  - Reaching this needs an own-log opaque entry, which only a downgraded node of a future release could hold. It is reported through `cannot_reissue` rather than dropped silently.
- **2.1.1 peers and a parked log (by design).** A 2.1.1 node whose sole source is a new node that withholds an opaque entry keeps seeing a higher `have` head. Its 30 s rx-stall rule re-selects and re-wants. The want hits the parked rule, so this costs one frame per stall window and never closes the link. That node's sync round for that log never completes, which is the documented "does not cross a 2.1.1 node" limit.

## Step 2: ledger triage (progress.md)

- No `parked`, `parked finding`, `Recurring minor:`, `Recurring blocker:` or `BLOCKED-AUTH` lines exist. Metrics: 15 merges, 15 completions clean, 0 fix passes. The Metrics line reports `gaps: ledger/Git identities differ`, so ledger completeness is unverified for this range.
- ht-j87z.13 is BLOCKED and needs an owner choice. It is triaged as must-fix (owner decision); see above.
- Task 11 [unverifiable-claim], release.md 8b. It is now verified and blocking: main already has a node version 4.
- Task 13 [hard-coded-slice]. It becomes wrong after the renumber, so it is fixed with the renumber.
- Task 10 [silent-skip-guard]. This minor names a skipped test, so it was misgraded. It is resolved at head (`tag_gate` plus `HT_REQUIRE_TAGS=1` and the CI tag fetch).
- These lines are resolved at head: Task 5 [deferred-opaque-serve] (both), because `entries_from` and `branch_entry` serve opaque rows under `Rows::All`; Task 7 [capability-gate-deferred], replaced by `gate_batch`; Task 8 [missing-edge-case-check], fixed by `authenticate_oversize` (ht-j87z.22).
- Task 12 [suite-over-budget]: 404 s of nextest, over the 300 s budget, driven by `failpoints_are_compiled_out_of_release_builds` (312 s, a test this epic did not touch). The budget is not a release gate under the user's memory rule. Deferred, with a fixer.
- Task 12 [test-leak-flags]: 6 nextest LEAK flags, in tests this epic did not touch. Deferred.
- The remaining minors are deferred OK. They are listed below.

## Verdict
not ready (node schema version 4 collides with main's 0004_sibling_delivery; bounded review only; whole-range review and full-suite at final head outstanding)

## Must fix before landing
- Rebase onto current main (9f1ca770d, which already registers node delta version 4 = 0004_sibling_delivery.sql), rename migrations/node/0004_journal_opaque.sql to the next free number and renumber its NODE_DELTAS entry, then update the migration header, docs/release.md 8b, CHANGELOG and tests/store/schema.rs journal_opaque_delta_upgrades_highest_main_store (hard-coded NODE_DELTAS[..2] and user_version 3) and rerun the schema tests
- Record the human owner's decision on ht-j87z.13 promotion (ledger BLOCKED: needs owner choice): accept shipping R4 without apply-after-upgrade (ht-evlm), and reconcile the contradictory Post-Implementation Notes (kept row with promoted flag vs deferral) with the delete model that 0004 and promote_opaque encode
- Complete the outstanding whole-range final review; this bounded review covers only the hunks in bounded-coverage.json

## Untested scope
- Whole-epic review incomplete: bounded inspected hunks only; all paths and hunks absent from bounded-coverage.json remain unreviewed
- No full-suite measurement of this branch at the final head exists; the sweep is deferred to the caller and must rerun after the main rebase (the ht-j87z.12 evidence covers tree 2c9b5a718 plus the ht-j87z.5 branch, 7805 passed, 404 s nextest)
- I ran no cargo build, clippy or test; only scripts/check-wire-compat (read-only) and sqlite3 EXPLAIN QUERY PLAN against the head migrations with system sqlite3 3.51.0, not the rusqlite-bundled build
- Provenance gap: base and head docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d, invalid failed assembly archive (failed bounded archive contains successful assembly or split claims)
- Provenance gap: base and head docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f, invalid failed assembly archive (failed bounded archive contains successful assembly or split claims)
- Ledger/Git identity gap: the Metrics git-only task list differs from the ledger, so ledger completeness for this range is unverified
- Unreviewed product paths: src/sync/coordinator.rs test module, src/sync/memory_journal.rs, src/sync/incarnations.rs, src/sync/incarnations/worker.rs, src/test_support/*, scripts/check-wire-compat beyond its header and run, scripts/freeze-peer-wire, scripts/tests/test_check_wire_compat.py, scripts/wire-compat-baseline.txt, tests/fixtures/peer_wire/*, every tests/** suite, evidence/**, and docs/superpowers/runs/** other than the design doc
- Partially reviewed: src/sync/frame.rs (the last test hunk was not read), src/sync/envelope.rs (the hex and test hunks were not read), src/sync/chain.rs (the last three test hunks were not read), src/store/incarnations.rs (the hunks after branch_progress were not read), src/sync/coordinator.rs (non-test hunks only)
- check-wire-compat has run only under macOS awk; Linux gawk or mawk on CI is untested (ledger Task 4)
- scripts/freeze-peer-wire --check reproducibility is not in CI; its byte-identical rerun is shown in integration-sweep logs only (ledger Task 3)
- Behaviour against a real 2.1.1 binary is untested by design: it relies on the frozen shadow decoder and the fixtures
- An own-log opaque entry in re-issue is untested: a known forked candidate behind an opaque branch row waits forever, and known entries after an opaque above-freeze entry are re-issued over the gap

## Deferred OK
- LinkStatus.withheld counts parked logs, not withheld entries, and survives an unwant (ledger Task 8 status-semantics)
- oversize_logs is in memory only: the stall recurs and is logged again after a restart (documented in docs/operations.md and TRUST-POLICY)
- An own-log opaque entry is reported through cannot_reissue rather than re-issued; it is reachable only by a node that downgraded from a future release
- A 2.1.1 sole source re-wants a parked log once per 30 s stall window, which is cheap, not a hot loop, and inside the documented non-relay limit
- WireItem::from_json parses each known item twice (probe, then WireEnvelope), a small cost per frame
- promote_opaque, apply::unpark, LayoutParser and understood_layout are dead in production builds until ht-evlm
- The suite runs over its 300 s budget because of failpoints_are_compiled_out_of_release_builds (312 s), which this epic did not touch (ledger Task 12)
- 6 nextest LEAK flags in tests this epic did not touch (ledger Task 12)
- The integration-sweep.md line-3 tree description is stale (ledger Task 12 doc-drift)
- Process-global understood-layout registry in test builds, safe under nextest one-process-per-test (ledger Task 2)
- Local check-wire-compat noise until main carries the baseline file (ledger Task 4)
- MANIFEST generator sha drift is by design, since --check ignores that line (ledger Task 3)
- Fixture append-only enforcement relies on MANIFEST hashes, which could be edited along with the files
- Missing red-phase runs reported in Tasks 2, 7, 13, 14 and 15
- Pin-only tests and the stale clippy run before a rebase (Task 5)
- The changed branch-walk assertion, the out-of-brief file and the Batch.closed tightening (Task 13)
- No MemoryJournal unit tests, which the contract suite covers (Task 14)
- The brief command mismatch and the per-change reissue-log dedup (Task 7)
- The reconstructed hello in forward_compat and the leak-check pid note (Task 10)
- Ingest tests live in coordinator.rs mod tests (Task 15)
