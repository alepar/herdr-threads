# Final review (bounded) — epic ht-j87z, peer wire compat hardening

Invocation: inv-A50B5B16-478E-47B7-A35A-E9FB071E2127
Captured source: base f78f076ab076a7efc939415ee780084fb56185f3, head bcd8be103399c6646d36ef84ae3d21948550092f (both verified with `git rev-parse --verify <sha>^{commit}`).
Route: BOUNDED. The review-inputs sidecar is `status: unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, no omissions, five retained provenance gaps. No whole-range package was built or loaded. I read named paths and hunks from captured Git (`git diff base..head -- <path>`, `git show head:<path>`). `bounded-coverage.json` lists exactly the hunks I inspected. Every other changed path is unreviewed.

Caller checkout observed: branch super-auto/peer-wire-compat-hardening, HEAD bcd8be103, clean at session start. The review never switched or edited it. The bind-review-source step returned exit 0 and echoed the invocation, base and head.

## Scope inspected

- Product paths read hunk by hunk: src/sync/coordinator/serve.rs (the Opaque capability gate), src/sync/frame.rs (WireItem, OpaqueWire, the hand-written decode, and the golden-byte and wire-item tests), src/sync/coordinator/ingest.rs (the oversize-claim authentication from ht-j87z.22), src/sync/chain.rs, src/store/journal.rs (CHAIN_ROWS/BRANCH_ROWS, the chain-row reader, ingest_entries, store_forked_entries, insert_opaque, promote_opaque, verify_invariants), migrations/node/0004_journal_opaque.sql, src/store/schema.rs, src/store/apply.rs (opaque_layout stall, mark_opaque_heads, next_ready_head), src/store/sync_status.rs, src/protocol/sync_admin.rs, the non-test hunks of src/sync/coordinator.rs, src/sync/incarnations/worker.rs (changed lines), and the non-test hunks of src/sync/resync.rs.
- Wiring and policy read: .github/workflows/ci.yml, TRUST-POLICY.md, tests/combined.rs.
- Context read but not counted as diff coverage: the design doc and run.md at head, the header of scripts/check-wire-compat, tests/fixtures/peer_wire/2.0.4/MANIFEST.md, an outline of tests/sync/peer_wire_fixtures.rs, and the skip guard in tests/support/legacy_wire_v211.rs (around line 403).
- Independent check: I ran `EXPLAIN QUERY PLAN` with system sqlite3 3.51 on a scratch DB built from the baseline `sync_logs`/`journal`/`journal_forked` DDL plus migration 0004. The `entries_from` chain read streams as `MERGE (UNION ALL)` with index seeks on both arms, with no full sort, so serving many batches does not become quadratic. `next_ready_head` scans the whole journal, but the base query already did (`SCAN j`), so this is not a regression. The production build bundles SQLite through rusqlite, whose version differs from 3.51, so this is indicative only.
- Tracker state (`bd list --parent ht-j87z --all`): ht-j87z.5 (two-node opaque relay integration tests), ht-j87z.9 (docs: operations limits, trust note, release checklist, changelog), ht-j87z.12 (integration sweep) and ht-j87z.13 (promotion) are still open. The other tasks and their reviews are closed.

## Own findings (step 1, before the ledger)

- Important (spec gap / incomplete epic): the spec's integration requirement is not on this branch. Design §10 calls for "two-node session where the author emits HTJRNL99: a capability-advertising relay forwards it, a hello without the capability withholds it". That is bead ht-j87z.5, still open. Coordinator-level unit tests cover the gate (`withheld` counts, hello caps), but nothing exercises the end-to-end relay over two nodes, which is the epic's headline goal.
- Important (spec gap): docs from design §5/§6 are missing. `docs/release.md` does not gain the "run scripts/freeze-peer-wire <new version>" step. Without it, the next release (2.2.0 is already branched) can ship without its fixture directory, and the "every release with a fixture dir stays decodable" guarantee silently stops growing. `docs/operations.md` does not document the 2.1.1 non-relay limit or the 384 KiB oversize stall. Bead ht-j87z.9 is open. TRUST-POLICY.md did gain the opaque-relay accepted-limit entry.
- Important (test silently skipped in CI): `tests/support/legacy_wire_v211.rs` `frozen_copy_matches_tag_source` (line ~403) prints `skipped: ...` and returns success when git or the `v2.1.1` tag is absent. The CI test job uses `actions/checkout` with the default shallow, tagless clone (only the workflow_dispatch validator job sets `fetch-depth: 0`). So in CI the check that the shadow decoder really is a frozen copy of 2.1.1 never runs, and the forward-compat half rests on an unenforced claim.
- Decision needed: promotion (ht-j87z.13) is open. `promote_opaque` and `apply::unpark` exist but have no caller (`#[allow(dead_code)]`). This release only ever stores opaque entries, which is safe because nothing authors a non-HTJRNL01 layout yet. Landing without .13 is defensible, but the human should explicitly defer it, since design §4 treats promotion as part of the epic.
- Minor: `LinkStatus.withheld` counts parked logs on the link, not withheld entries (design §3 says "a per-link diagnostic counter records the withheld entries"). The task review recorded the same thing as status-semantics.
- Minor: the oversize stall (`Actor::oversize_logs`) is in-memory only. It lifts on restart, which then re-fetches and stalls again, and `sync status` does not show it: the log line is the only diagnostic. Acceptable once docs record it (ht-j87z.9).
- Minor (efficiency): `WireItem::from_json` deserializes every known item twice, first as a `Probe` that scans for an `opaque` key and then as a `WireEnvelope`. Every `entries` frame from every peer now pays a second JSON pass.
- Minor (CI): the R4 delta step runs `git fetch --depth=1 origin "$base" || true` and then falls back to a run with no `--base`. If `origin/<base>` is not created (fetch failed), R4 quietly falls to its default base `main`, which does not resolve in that clone, so it prints "R4: no base, delta skipped" instead of failing the job.
- Verified correct: known-layout items encode byte-identically to 2.1.1 (golden test in frame.rs, captured before WireItem existed). `gate_batch` is the only place serving checks `envelope.opaque.v1`, and it cuts both ordinary and branch batches before the first opaque entry. A batch cut to empty parks the log per link, and a `want` from before the cut un-parks it. `OpaqueEntry::new` refuses counter 1 (NotGenesis), the HTJRNL01 magic, and a prefix/header mismatch. `chain::check_next` verifies the signature before Append or Conflict, so a forged opaque entry is Corrupt, never a fork. Ingest checks that every entry names the frame's log. An oversize claim stalls a log only after checking the same log, the next position, prev = head, a prefix match, SHA-256 over raw, and verify_strict. Otherwise only the source is refused (this resolves the ledger's .4 minor). The `journal_opaque` guard triggers enforce immutability, append-only with a marker-scoped delete, and cross-table position uniqueness in both directions. `verify_invariants` checks that the promotion marker table is empty. An own-log opaque entry is reported as cannot-re-issue rather than dropped. Every commit carries the `Outside-adapter:` trailer.

## Ledger triage (step 2)

- No `parked`, `parked finding`, `Recurring minor:`, `Recurring blocker:` or `BLOCKED-AUTH` lines. Metrics: 11 merges, 11 completions review-clean, ledger-check ok, but a "ledger/Git identities differ" gap with a long git-only list from older epics. So absence in the ledger is not proof of a clean history for this range.
- Misgraded, must-fix: Task 10 (ht-j87z.7) `[silent-skip-guard]` names a test that passes while skipped, the same issue as my own finding above.
- Resolved on the branch (checked in the captured source): Task 5 (ht-j87z.11) `[deferred-opaque-serve]` both lines. `entries_from` and `branch_entry` now read opaque rows under `Rows::All`, and a missing branch row is corrupt rather than skipped. Task 7 (ht-j87z.10) `[capability-gate-deferred]`: replaced by `gate_batch` with `envelope.opaque.v1`. Task 8 (ht-j87z.4) `[missing-edge-case-check]`: the `(None, Some(claim))` arm now goes through `authenticate_oversize`, which requires the next position (ht-j87z.22).
- Deferred OK: the rest. These are missing red runs (Tasks 2, 7, 13, 14, 15), process-global understood-layout registry (safe under nextest's one-process-per-test), local-run noise until main carries the baseline, the stale generator sha in MANIFEST (by design), freeze --check evidence in prose only, pin tests, the stale lint run before a rebase, the changed branch-walk assertion (consistent with the KnownPrefix rule), the out-of-brief file, `Batch.closed` tightening, the hard-coded `NODE_DELTAS[..2]` slice, no MemoryJournal unit tests (the contract suite covers it), the brief command mismatch, reissue-log dedup, withheld status semantics, the reconstructed hello, the leak-check pid note, and tests in coordinator.rs.
- Untested scope from the ledger: Task 4 (ht-j87z.8) `[single-awk-verified]`. check-wire-compat's awk is verified only on macOS awk. CI Linux (gawk/mawk) will be its first run.

## Verdict
not ready (epic incomplete: relay integration tests ht-j87z.5 and docs ht-j87z.9 open, a CI-skipped shadow-copy test, no full-suite run, bounded review only)

## Must fix before landing
- Land ht-j87z.5: a two-node opaque relay integration test (a capability relay forwards HTJRNL99; a peer without the capability is withheld and its have-head stays behind), the end-to-end proof of the epic's main goal from design §10.
- Land ht-j87z.9: add the "run scripts/freeze-peer-wire <new version>" step to docs/release.md, and document the 2.1.1 non-relay limit and the 384 KiB oversize stall in docs/operations.md, before the next release is cut.
- tests/support/legacy_wire_v211.rs frozen_copy_matches_tag_source passes while skipped in CI (shallow tagless checkout): make it fail, fetch the v2.1.1 tag in the test job, or replace it with a check that needs no tag (ledger Task 10 silent-skip-guard, misgraded minor).
- Get an explicit human decision on ht-j87z.13 (promotion). promote_opaque and unpark have no caller; either land .13 or record the deferral in the design's Post-Implementation Notes.

## Untested scope
- No full-suite measurement of this branch exists. The sweep (ht-j87z.12) is open and owned by the caller, so the branch must not be treated as tested.
- Bounded review only: every changed path not listed in bounded-coverage.json is unreviewed. That covers scripts/check-wire-compat, scripts/freeze-peer-wire, scripts/tests/test_check_wire_compat.py, scripts/wire-compat-baseline.txt, src/sync/envelope.rs hunks outside the prefix/OpaqueEntry/StoredEntry region, src/sync/incarnations.rs, src/sync/incarnations/sink.rs, src/sync/journal_port.rs, src/sync/memory_journal.rs, src/sync/link.rs, src/store/incarnations.rs, src/protocol/output_compact.rs, src/service/*, src/test_support/*, the coordinator.rs test hunks, the frame.rs final test hunk, the resync.rs test hunks, AGENTS.md, all tests/** files and fixtures, docs and evidence logs.
- Provenance gap: head run.md:27 malformed collection record (record outside approvals or invalid prefix).
- Provenance gap: base and head docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-264d-4506-beaa-307c9600bb8d, an invalid failed assembly archive (a failed bounded archive that contains successful assembly or split claims).
- Provenance gap: base and head docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:58 inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f, an invalid failed assembly archive (a failed bounded archive that contains successful assembly or split claims).
- Ledger/Git identity gap: the Metrics git-only task list differs from the ledger, so ledger completeness for this range is unverified.
- check-wire-compat has not run under Linux gawk/mawk (ledger Task 4 single-awk-verified). The first CI run is its first test there.
- scripts/freeze-peer-wire --check reproducibility is not run in CI, and its byte-identical rerun evidence is prose only (ledger Task 3).
- End-to-end mixed-version behaviour against a real 2.1.1 binary is untested by design; it relies on the shadow decoder, whose tag check is skipped in CI.
- The SQL plan check used system sqlite3 3.51, not the rusqlite-bundled SQLite version.

## Deferred OK
- LinkStatus.withheld counts parked logs rather than withheld entries (design §3 wording; ledger Task 8 status-semantics).
- The oversize-stall set is in-memory only and missing from sync status; acceptable once docs/operations.md records it (part of ht-j87z.9).
- WireItem::from_json parses each known item twice (Probe, then WireEnvelope): a small per-frame cost worth a later optimization.
- CI R4 delta silently skips when origin/<base> is not fetched; harden later.
- Fixture append-only enforcement relies on MANIFEST hashes, which a reviewer must notice being edited alongside the files.
- Missing red-phase runs reported in Tasks 2, 7, 13, 14 and 15.
- Process-global understood-layout registry in test builds (safe under nextest's per-test processes).
- Local check-wire-compat noise until main carries the baseline file.
- MANIFEST generator sha drift (by design; --check ignores it) and freeze --check evidence in prose only.
- Pin-only tests, the stale clippy run before a rebase, the changed branch-walk assertion, the out-of-brief file, Batch.closed tightening, and the hard-coded NODE_DELTAS slice.
- No MemoryJournal unit tests (the contract suite covers it), the brief command mismatch, reissue-log dedup, the reconstructed hello in forward_compat, the leak-check pid note, and ingest tests living in coordinator.rs.
- Resolved on the branch: Task 5 deferred-opaque-serve (both lines), Task 7 capability-gate-deferred, and Task 8 missing-edge-case-check (fixed by ht-j87z.22).
