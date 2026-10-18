# Final review (bounded): ht-j87z peer wire compatibility hardening

Invocation inv-79743B9E-C426-482C-B438-4B73A762CC61. Captured source: base f78f076ab076a7efc939415ee780084fb56185f3, head 1dcb7473c16728b559d0b9f56a73058b748e3e0c. Both were verified with `git rev-parse --verify <sha>^{commit}`. The caller checkout was at head 1dcb7473c with a clean status when I read it, and I did not switch it. I ran the lint in a review-owned detached worktree at the head (scratchpad `review-wt-79743B9E`).

Route: bounded. review-inputs.json has sourceAvailable true, wholeRangeReady false, status unavailable, 90 changed paths, 0 omitted and 4 gaps. The gaps are the two failed assembly archives in the 2026-10-17 run.md, at lines 51 and 58, listed at both base and head. I did not build or load a whole-range package. Inspected hunks are listed in bounded-coverage.json, and every other path or hunk is unreviewed.

Spec read: `bd show ht-j87z`, the design doc (sections 1-9, key abstractions and post-implementation notes) and the epic's child beads. Open children at head: ht-j87z.5 (two-node relay integration tests), ht-j87z.9 (docs), ht-j87z.12 (integration sweep) and ht-j87z.13 (promotion).

## Step 1: own view (before the ledger)

- **Promotion storage seam (Important).** Migration `migrations/node/0004_journal_opaque.sql` ships three guards:
  - `journal_opaque_immutable`: no UPDATE of any row.
  - `journal_opaque_append_only`: no DELETE, except through the `journal_opaque_promoting` marker.
  - `journal_position_taken_by_opaque`: no `journal` insert at a position held by a branch-0 opaque row.

  `journal::promote_opaque` (src/store/journal.rs:1963) deletes the opaque row. Only tests call it and `apply::unpark` (tests/store/apply.rs:2188), so it has no production caller. The design's post-implementation note (the 2026-10-18 ht-50fc decision) says something different: promotion keeps the `journal_opaque` row and its raw bytes with a `promoted` flag. 0004 has no such column, and its immutable trigger forbids setting one. Design section 4 still describes the delete-and-insert path, so the design text, the post-implementation note and the code disagree. Node deltas are frozen once they ship. Whichever ht-j87z.13 option is chosen (A/B/C in the ledger BLOCKED line) therefore has two consequences:
  - It either needs a 0005 delta, or 0004 must be settled before it reaches any release.
  - `promote_opaque` and `unpark` are dead or contradictory API until then.
- **Epic not complete at head.** ht-j87z.5 is open, so no two-node relay test exists. That test is the end-to-end proof of the epic's main goal: a capability relay forwards HTJRNL99, and a non-capability peer has it withheld. ht-j87z.9 is also open, and `docs/release.md` and `docs/operations.md` are unchanged in the range. Design section 6 requires the `scripts/freeze-peer-wire <version>` release-checklist step. Section 5 requires the documented limits in operations.md: no relay through 2.1.1, and the 384 KiB oversize stall. TRUST-POLICY.md does carry the opaque relay accepted limit.
- **Capability gate composition.** I verified this by reading the code.
  - `gate_batch` (src/sync/coordinator/serve.rs) is applied to both ordinary serve and fork-branch serve.
  - A batch cut to empty parks the log per link in `withheld_logs`. A `want` from before the cut lifts the park.
  - `ready` excludes parked logs.
  - `send_branch_entries` ends a cut branch with `more: false`, and an empty prefix becomes `branch_none`.
  - Known items encode byte-identically to 2.1.1 (golden test `entries_frames_golden_bytes`).
  - `OPAQUE_CAP` is advertised by default (CoordinatorOptions).

  I found no seam defect here.
- **Oversize authentication.** I verified this by reading the code. `authenticate_oversize` (src/sync/coordinator/ingest.rs) checks, in order: frame log, `counter == contiguous + 1`, prev == head, prefix (non-HTJRNL01 magic, log, counter, prev), SHA-256 of the raw bytes, and `verify_strict` against the log pubkey. Only then can it return `Oversize`. Any other result refuses only that source. This closes the earlier ledger edge case (Task 8).
- **Chain and store union.** I verified this by reading the code.
  - `CHAIN_ROWS` and `BRANCH_ROWS` back these readers: `verify_invariants` (contiguous, head hash, empty promotion marker), `hash_at`, `entries_from`, `serve_branch_within`, `next_ready_head`, `mark_opaque_heads` and the sync-status verify.
  - Opaque appends go to `journal_opaque` with no HLC fold.
  - `check_next` treats opaque and known entries the same way, and an opaque genesis is `NotGenesis`.
- **Apply hot path (unmeasured, Minor).** `next_ready_head` now runs `mark_opaque_heads` plus a union-joined head query on every `apply_pass` loop iteration. Each prepares an uncached statement. EXPLAIN QUERY PLAN ran on Python's SQLite 3.53.4 with no stats, not the rusqlite-bundled build. It shows the head query split into two UNION ALL arms, each with a temp B-tree for ORDER BY. The old query was a single journal scan, so the new one is still a journal scan with extra constant work per applied entry. No catch-up benchmark exists.
- **Restore-rotation log line (Minor).** In src/sync/incarnations/worker.rs, the "own log ... unknown layout; cannot re-issue" line is logged each time the restore path meets an opaque own entry, with no dedupe. `report_cannot_reissue` dedupes per count change.
- **Lint.** `scripts/check-wire-compat` at head reports 0 violations, 0 stale, 255 fields, 18 variants and 268 baseline entries. `--base` against the epic base also passes. `test_check_wire_compat.py` ran 36 tests, all OK. All runs were on macOS awk. The new fields `LogRow.blocked_on`, `LinkRow.withheld` and `RepairRow.cannot_reissue` carry `serde(default)` and are not in the baseline. `OpaqueWire` fields are baselined as required, as designed.
- **CI tags.** Both workflows set `HT_REQUIRE_TAGS=1` and fetch v2.1.1 and v2.0.4. `git ls-remote` shows both tags on origin, and `frozen_copy_matches_tag_source` now panics instead of skipping when the tag is missing.

## Step 2: ledger triage

- Recurring minor and Recurring blocker lines: none in the ledger.
- Parked completions and parked findings: none. BLOCKED-AUTH: none.
- Task ? (ht-j87z.13) BLOCKED, "recorded decision contradicts another acceptance requirement": must-fix. It needs the owner's choice of A, B or C, recorded as an amendment, and it is tied to the 0004 storage seam above.
- Task 10 [silent-skip-guard]: this was misgraded as minor, because it was a skipped test in CI. It is resolved at head by ht-j87z.25 (code and CI verified).
- Task 8 [missing-edge-case-check]: resolved by ht-j87z.22 (verified).
- Task 7 [capability-gate-deferred]: resolved by ht-j87z.4, since the branch serve now goes through `gate_batch` (verified).
- Task 5 [deferred-opaque-serve], both lines: resolved. `serve_branch_within` reads opaque branch rows through `branch_entry` over `BRANCH_ROWS`, and `batch_journal_range` no longer exists at head (verified).
- Task 4 [single-awk-verified]: still untested on Linux gawk/mawk until the CI job runs.
- Task 8 [status-semantics], Task 2 [global-test-state], Task 4 [local-run-noise], Task 3 [doc-drift] and [missing-evidence], Task 5 [pin-passes-without-change] and [stale-lint-run], Task 13 (ht-j87z.14) items, Task 14 [missing-unit-tests] and [tests-not-red-first], Task 7 [brief-command-mismatch] and [reissue-log-dedup], Task 10 [reconstructed-fixture] and [test-hygiene-note], Task 15 items and every missing-red-run note: deferred OK. None names a skipped test, a known product defect or a data-loss path.

## Verdict
not ready (epic incomplete: ht-j87z.5/.9/.13 open, promotion storage vs migration 0004 undecided, bounded review only, no full-suite run)

## Must fix before landing
- Get a recorded human or owner decision on ht-j87z.13 (option A, B or C from the ledger BLOCKED line). Align migration 0004's triggers, `promote_opaque` (which deletes the row) and the design's post-implementation note (which keeps the row with a `promoted` flag that 0004 cannot hold) before 0004 ships in any release, or record that a 0005 delta will follow.
- Land ht-j87z.5: a two-node opaque relay integration test. A capability relay forwards HTJRNL99, and a non-capability peer has it withheld, its have-head staying behind and the link staying up. This is the end-to-end proof of the epic goal.
- Land ht-j87z.9: add the `scripts/freeze-peer-wire <new version>` step to docs/release.md, and document the 2.1.1 non-relay limit and the 384 KiB oversize stall in docs/operations.md (design sections 5 and 6).
- Run the deferred full-suite sweep (ht-j87z.12) on this branch and keep its evidence. No full-suite measurement exists yet.

## Untested scope
- No full-suite measurement of this branch exists. The sweep is deferred to the caller, so the branch must not be treated as tested. I ran no cargo build, clippy or tests in this review.
- This review is bounded. Only the hunks listed in bounded-coverage.json were inspected. Unreviewed paths: src/sync/coordinator.rs, src/sync/memory_journal.rs, src/sync/incarnations.rs, src/sync/incarnations/sink.rs, src/store/incarnations.rs, src/test_support/*, scripts/check-wire-compat source (run only), scripts/freeze-peer-wire, scripts/tests/test_check_wire_compat.py (run only), scripts/wire-compat-baseline.txt, all tests/** files and fixtures, docs/superpowers/runs/2026-10-18-peer-wire-compat-hardening/** (the design and the earlier review were read as inputs only) and evidence/**.
- Partially reviewed: src/sync/chain.rs (first 200 diff lines), src/sync/resync.rs (non-test hunks) and src/store/journal.rs (the hunks listed in the coverage file only).
- Provenance gaps: docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md line 51 (inv-8759cc82-264d-4506-beaa-307c9600bb8d) and line 58 (inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f), at both base and head. Each is an invalid failed assembly archive: the failed bounded archive contains successful assembly or split claims.
- Ledger/Git identity gap: the Metrics git-only task list differs from the ledger, so ledger completeness for this range is unverified.
- check-wire-compat has run only under macOS awk. The first CI Linux run (gawk or mawk) is its first test there (ledger Task 4).
- `scripts/freeze-peer-wire --check` reproducibility is not run in CI, and its byte-identical rerun evidence exists only as prose (ledger Task 3).
- Mixed-version behaviour against a real 2.1.1 binary is untested by design. It relies on the frozen shadow decoder and the fixtures.
- The apply-pass cost of the CHAIN_ROWS union in `next_ready_head` and `mark_opaque_heads` is unmeasured. The query plan was checked on Python's SQLite 3.53.4 with no stats, not the bundled rusqlite build.
- Whether a 2.1.1 fork.v1 peer handles a fork ref whose conflicting side is opaque (it gets `branch_none` or a known prefix) is unverified beyond the gate code.

## Deferred OK
- `LinkStatus.withheld` counts logs parked on the link rather than withheld entries (ledger Task 8 status-semantics).
- The oversize-stall set (`oversize_logs`) is in memory only and logged once per process, so the stall recurs after a restart. This is acceptable once ht-j87z.9 documents it, and TRUST-POLICY already states the stall.
- In src/sync/incarnations/worker.rs, the restore-rotation "cannot re-issue" line is not deduplicated per log.
- `WireItem::from_json` parses each known item twice (Probe, then WireEnvelope), a small cost per frame.
- Fixture append-only enforcement relies on MANIFEST hashes, which can be edited together with the files, so review has to catch such edits.
- check-wire-compat prints every baseline line as `baseline +` until main carries the baseline file (ledger Task 4 local-run-noise).
- The understood-layout registry is process-global in test builds. This is safe under nextest's one process per test (ledger Task 2).
- MANIFEST generator sha drift is by design, since `--check` ignores that line (ledger Task 3).
- Missing red-phase runs reported in Tasks 2, 7, 13, 14 and 15.
- Pin-only tests, the stale clippy run before a rebase, the changed branch-walk assertion, the out-of-brief file, the Batch.closed tightening and the hard-coded NODE_DELTAS slice (Tasks 5 and 13).
- No MemoryJournal unit tests (the contract suite covers it), the brief command mismatch, the reissue-log dedup, the reconstructed hello in forward_compat, the leak-check pid note, and the ingest tests living in coordinator.rs.
