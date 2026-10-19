# Final review (bounded): epic ht-76u6, bytes-first journal

Invocation inv-239E1565-1CFB-44FF-9208-D3750DC138F8. Captured source: base e6815f803bb2cd2b8d30c90b62d53c80cf0216fb (main, v2.2.1), head fbe242d66522392ab1ace4225243c2243fc7c31a (super-auto/bytes-first-journal). Both verified with `git rev-parse --verify <sha>^{commit}`. The caller checkout was at head with a clean status when this review started.

## Scope and route

This is a bounded review. The review-input selection sidecar (`review-inputs.json`) reports status unavailable, sourceAvailable true, wholeRangeReady false, 159 selected paths, 0 omitted and 4 gaps. The gaps cover `docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md` lines 51 (inv-8759cc82-264d-4506-beaa-307c9600bb8d) and 58 (inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f), at both base and head. Each one is "invalid failed assembly archive: failed bounded archive contains successful assembly or split claims". No whole-range package was built or loaded. I read the full 159-path inventory and inspected the hunks listed in `bounded-coverage.json` from captured Git only.

The range covers two epics. Main was not merged between them, so the unmerged peer-wire-compat-hardening branch (ht-j87z, 2.3.0 opaque relay) sits below the bytes-first epic (ht-76u6). Both land together. I focused on the bytes-first product changes: delta 0005, the rebuild runner, the journal row decoder and serving, ingest, promotion, apply stall, re-issue, sync status, resync salvage and docs. I read the peer-wire serve gate only where bytes-first types flow through it.

## Step 1: own review against the spec

The spec is `docs/superpowers/runs/2026-10-18-bytes-first-journal/2026-10-18-bytes-first-journal-design.md`.

What composes correctly:
- Delta 0005 rebuild runner (`src/store/schema.rs`). `foreign_keys` is set off before `BEGIN IMMEDIATE`, and the prior value is restored on every path. The steps run in order: `sql`, the Rust `copy`, `sql_after`, an empty `pragma_foreign_key_check` checked from Rust, then the `user_version` bump. Any error rolls back. The `expected_objects` replay runs `sql` and then `sql_after`, so the renamed-table text matches.
- `copy_rows_with_raw` (`src/store/journal.rs`). I checked the column indexes against both INSERT lists. They match. `ingest_seq` and `forked_ix` are preserved, so the `thread_registers` and `sync_reissues` foreign keys stay valid. The `sqlite_sequence` high-water mark is carried over. Every row must satisfy SHA-256(canonical_bytes) == hash, or the copy aborts with StoreCorrupt.
- The new DDL matches the baseline. The `apply_state` set is a superset of the old one (`parked_layout` is added). Indexes `journal_applied_seq`, `journal_unapplied`, `journal_creates`, `journal_parked_on` and `journal_forked_creates` are recreated. The trigger set matches spec §3. The all-or-none projection CHECK and the CHECK `parked_layout` iff `kind IS NULL` make promotion an atomic two-field move. `journal_envelope_immutable` refuses any change to signed columns and any projection change once the projection is filled.
- Classification by raw magic (`KNOWN_SQL`). It is used in `row_select`, the re-issue candidates, `pending_reissues`/`cannot_reissue` and the invariant ordering. Apply alone uses projection presence (`mark_opaque_heads` and `next_ready_head` use `kind IS NULL`/`IS NOT NULL`). `OpaqueEntry::new` refuses the HTJRNL01 magic, so an HTJRNL01 row with an empty projection can only come from corruption.
- Promotion (`promote`, `promote_parked`). It runs in a SAVEPOINT. It runs only on `parked_layout` rows with an empty projection and only for magics this build registered, and production registers none. It checks that the parsed envelope's header matches the stored log, counter, prev, hash and signature. It sets `pending` and clears `blocked_on=opaque_layout` only when the row is the log's next position. A failure is logged once and the row stays parked. It runs at store open after migrations and at the start of each apply pass.
- Serving. `serve_entries`/`serve_branch_entries` return Opaque from `raw` for every row without the HTJRNL01 magic, promoted or not, and the capability gate cuts at the first opaque entry. Known rows are still built from the projection, so the known wire is unchanged.
- Resync salvage. It detects a schema-4 backup by the missing `journal.raw` column and rebuilds each entry from the projection. An undecodable row stops only its own log.

Findings (own):
- [Important] `CHANGELOG.md` @@ -2,6 +2,8 @@. The two new feature bullets ("Peer wire carries entries of future layouts", "Wire compatibility checks") sit under the existing `## v2.2.1` heading. v2.2.1 is already tagged and published, and `git show v2.2.1:CHANGELOG.md` does not have these bullets. Landing this would rewrite the notes of a released version and credit unreleased 2.3.0 work (design §8 ships it as 2.3.0) to v2.2.1. This looks like an artifact of merging main v2.2.1 into the peer-wire branch. Fix: move the bullets under a new heading for the next release.
- [Minor] `src/store/incarnations.rs` `forked_ready`. A known forked re-issue candidate whose predecessor is an opaque `journal_forked` row can never become ready. Opaque forked rows are never re-issued, and `forked_ready` accepts only a main-chain predecessor or an already re-issued forked one. Such a candidate is counted as `waiting` on every pass and in `pending` forever. It does not set `has_more`, so there is no hot loop. The edge needs a fork branch of an own frozen log that carries a future-layout entry.
- [Minor] `src/store/sync_status.rs` `rehash_entries` (J3). A Known row is checked as compute_hash(projection) == hash. Nothing checks that its `raw` still equals the projection's canonical bytes, so the "raw is truth" invariant goes unaudited for known rows. Today `raw` is written only by INSERT and is immutable under the trigger, so this is defence in depth.
- [Minor] Promotion does not fold the promoted entry's HLC into the node clock (ingest skips the fold for opaque rows too). This matches restart behaviour, where `Hlc::resume` reads only self logs, so it is consistent rather than a regression.

## Step 2: ledger triage

The ledger is `.superpowers/sdd/ht-76u6-plan/progress.md`. It has no parked completions, no parked findings, no BLOCKED-AUTH lines and no recurring blockers. Metrics: 9 merges, 9 review-clean completions, 0 fix passes. The ledger and Git identities differ, which is recorded as a ledger gap and not read as clean.

- Recurring minor `[missing-evidence]` (×3 across ht-76u6.1, .4 and .5; tasks 6, 8 and 10 report the same pattern). This is a process cluster: task reports keep logs under /tmp and carry no structured evidence records. It is not a product defect. Deferred, but the caller's full-suite sweep must produce retained evidence.
- `[test-process-global-state]` ht-76u6.4. The HTJRNL99 parser registry (`understood_layout`) is process-global. `tests/store/apply.rs` registers it, and that file compiles into the lib unit-test binary, alongside the opaque-stall tests in `tests/store/journal.rs` and `tests/store/sync_status.rs`. Under plain multi-threaded `cargo test` (the documented per-change command), a promotion test can promote rows that a concurrently running stall test expects to stay parked, which makes failures intermittent. Under nextest, each test runs in its own process. The comment in the test says to run them by filter. Deferred, but a follow-up should isolate the registry, for example per thread or per store context.
- `[known-null-projection-gap]` ht-76u6.10. It cannot be reached without corruption: `OpaqueEntry::new` refuses the HTJRNL01 magic and the CHECK ties `parked_layout` to an empty projection. Deferred.
- `[test-support-null-kind]` ht-76u6.2. `multinode::journal_entry` returns None for a `parked_layout` row, because the decode fails and `.ok()` swallows it. That is harmless for current callers. Deferred.
- `[restart-not-reopen]` ht-76u6.6. Promotion at store open is covered by the unit test `promotion_runs_at_store_open` (tests/store/apply.rs). Deferred.
- `[deferred-to-later-task]` ht-76u6.4. The capability vs non-capability wire after promotion is covered by `tests/sync/opaque_promotion.rs::upgrade_promotes_relayed_entry_and_successors_apply`. Resolved.
- `[unverified-reference]` ht-76u6.6. `served_known_frames_byte_identical_across_migration` exists at head (tests/store/journal.rs:2678). Resolved.
- `[suite-over-budget]` ht-76u6.8. The 371 s comes from a cold release build in `manifest::failpoints_are_compiled_out_of_release_builds`. The user's standing guidance is that suite time does not gate a release. Deferred.
- `[unbounded-set]`, `[duplicated-batch-logic]`, `[fragile-string-derivation]` (guarded by a test), `[decode-before-limit]` (pre-existing), `[nested-transaction-edge]` (an untested defensive branch), `[fixture-raw-stub]`, `[duplicated-helper]`, `[scope-drift]`, `[test-placement-deviation]`, `[tdd-red-missing]`/`[missing-red-run]`, `[reissue-tests-not-added]` and `[missing-edge-case-test]` (compact `cannot_promote` rendering): code-quality or process items. Deferred.
- `[salvage-schema4-gap]` ht-76u6.2. Closed by ht-76u6.5: `legacy_entries` is present at head and tested.

No full-suite measurement of this branch exists for this invocation. The caller runs the sweep afterwards. The ht-76u6.8 integration-sweep note reports 7856 passed, measured on a base tree described as code-identical. This review did not re-run or verify that result, so the branch is not treated as tested here.

## Verdict
not ready (bounded review only, whole-range review and full-suite sweep outstanding; CHANGELOG puts the unreleased 2.3.0 features under the released v2.2.1 heading)

## Must fix before landing
- CHANGELOG.md: move the "Peer wire carries entries of future layouts" and "Wire compatibility checks" bullets out of the released `## v2.2.1` section into a new section for the next release (2.3.0).

## Untested scope
- Whole-range review outstanding: this bounded review covered only the hunks listed in bounded-coverage.json; every other changed path and hunk of the 159-path inventory was not reviewed.
- Provenance gap: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d invalid failed assembly archive (failed bounded archive contains successful assembly or split claims), at base and head.
- Provenance gap: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f invalid failed assembly archive (failed bounded archive contains successful assembly or split claims), at base and head.
- Unreviewed product code: src/sync/envelope.rs (read in part, hunks not covered), src/sync/frame.rs, src/sync/coordinator.rs, src/sync/coordinator/ingest.rs, the last hunk of src/sync/coordinator/serve.rs, src/sync/chain.rs, src/sync/incarnations.rs and its sink/worker, src/sync/journal_port.rs, src/sync/memory_journal.rs, src/sync/link.rs, src/protocol/output_compact.rs, src/protocol/sync_admin.rs, src/service/sync_admin.rs and src/test_support/*.
- Unreviewed tooling and CI: scripts/check-wire-compat, scripts/freeze-peer-wire, scripts/release-qualify, scripts/tests/test_check_wire_compat.py, scripts/wire-compat-baseline.txt, .github/workflows/ci.yml and .github/workflows/full-suite.yml.
- Unreviewed tests and fixtures: everything under tests/ (store, sync, service, cli, support, combined.rs and the peer_wire fixtures).
- Unreviewed run records and evidence: docs/superpowers/runs/2026-10-18-peer-wire-compat-hardening/**, the remaining 2026-10-18-bytes-first-journal run docs, docs/superpowers/specs/INDEX.md and evidence/**.
- No full-suite measurement of head fbe242d66 exists for this invocation: the caller's deferred nextest sweep plus leak checks must run before landing.
- The ledger and Git task identities differ (git-only task ids listed in Metrics), so ledger completeness is unverified.

## Deferred OK
- incarnations.rs forked_ready: a known forked candidate behind an opaque forked predecessor stays waiting/pending forever (no hot loop); follow-up bead.
- sync_status.rs J3 does not check raw == canonical bytes of the projection for known rows (raw is INSERT-only and trigger-immutable); follow-up bead.
- Promotion does not fold the promoted entry's HLC (consistent with restart resume semantics).
- Process-global HTJRNL99 registry can make lib-binary tests fail intermittently under plain multi-threaded cargo test (nextest isolates per process); follow-up to isolate the registry.
- Recurring missing-evidence cluster in task reports (process only).
- known-null-projection-gap: cannot be reached without corruption (CHECK plus OpaqueEntry refusing HTJRNL01).
- test-support journal_entry returns None for parked_layout rows (harmless).
- promote_logged set grows per failed row per process (production registers no parsers, so it stays empty).
- legacy_entries duplicates the Batch limit logic; known_sql string derivation (tested); decode-before-limit (pre-existing).
- Untested non-autocommit refusal branch in apply_delta; compact cannot_promote rendering untested.
- Test placement, helper duplication, fixture literal duplication and missing RED runs (process and code quality).
- Suite at 371 s over the 300 s budget from a cold release build (not a release gate).
