# Task 2 scoped re-review, fix round 1

## Finding Verdicts

- **I1 — Duplicated setup ownership engine: ADDRESSED.** `src/harness/owned_files.rs:181`, `:301` and `:374` now hold the shared install/recovery, conservative removal and status classification logic. Opencode delegates with its existing permission metadata (`src/harness/opencode/setup.rs:263`, `:324`, `:378`); pi delegates with empty metadata and its existing directory layout (`src/harness/pi/setup.rs:102`, `:129`, `:133`, `:176`). Both copied engines are removed. Rendering, manifest paths, diagnostics, output projections and permission decisions remain adapter-owned. The extraction introduces no HarnessAdapter policy or harness-name conditional.
- **M1 — Malformed trust-section insertion: ADDRESSED.** `TRUST-POLICY.md:334` restores the complete original inline Accepted limits reference. The unchanged pi amendment now begins at a real heading (`TRUST-POLICY.md:2334`) before the existing Accepted limits section (`:2396`); C1 continuity prose is no longer interrupted.

## New Breakage in the Fix Diff

None found.

- Checked the extracted engine against both removed implementations in the supplied diff. Read bounds, symlink handling, conflict checks before file publication, current/pending digest ownership, intent-first publication, metadata-change detection, finalization, modified-file retention and reverse empty-directory cleanup retain the prior behavior (`src/harness/owned_files.rs:181`, `:231`, `:264`, `:301`). The new shared status function retains the same missing/unowned/modified/stale/installed predicates (`:374`).
- Checked the manifest compatibility change: flattened adapter metadata occupies the original position between binary version and file entries (`src/harness/owned_files.rs:35`). Opencode's boolean remains required and adapter-owned; pi's empty metadata adds no serialized field (`src/harness/opencode/setup.rs:263`, `src/harness/pi/setup.rs:102`).
- Checked the actual historical-producer regression (`tests/harness/opencode_setup.rs:809`). It requires an explicit producer, checks its 3.2.0 version, invokes old setup in isolated directories, and generates both permission variants rather than reconstructing JSON (`:812`, `:829`). It checks raw manifest round-trip equality, stored digests, unchanged owned bytes after new setup, setup/status/doctor projections, removal and foreign/modified preservation (`:875`, `:891`, `:911`, `:915`). Missing producer fails explicitly when this normally ignored gate is selected. Pi's regression separately pins the five-field/two-file manifest and byte-stable reinstall (`tests/harness/pi.rs:336`, `:381`).

## Evidence and Scope

- Reviewed direct tree-to-tree package `review-fix1-ca4474c06..197669a99.diff` once in sequential, nontruncated chunks, against the prior reviewed tree `ca4474c06` and final tree `197669a999f8555a76d41987e4ef9067f2c0d10f`. The authorized history reorder means the former is not an ancestor. The supplied commit metadata separates helper/opencode (`7db3f97d3`) from pi conversion (`4a784faa9`); the report supplies the old/new SHA map, unchanged-tree comparison and standalone ordering evidence. No git commands were rerun.
- Read the complete fix brief and report. Reported final results identify 101 focused tests plus 20 public/golden tests, the separately selected actual-3.2.0 gate for both permission variants, the expected missing-producer failure, and passing fmt/clippy/default-feature/neutral/wire/scoped-leak checks. The reported producer hash and tag/source comparison establish the stated fixture provenance; I did not independently execute or hash the historical binary. No golden fixture changes appear in the fix diff.
- No tests/builds were rerun: the diff raised no specific unresolved behavioral doubt requiring a new execution. No outside-diff source lookup, subagent, threads write, credential/configuration access or source/index/branch mutation occurred. Only this review report was written.

## Out-of-Scope Observations

None. Native pi wake strategy and Task 3 qualification are outside this fix and remain unreviewed here.

## Verdict

**Fix round: All findings addressed, no new Critical/Important breakage. Approved for this scoped Task 2 repair.** This is not final integrated, native-runtime or release qualification.
