# Final review: cluster 4 (docs-sp-specs), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify <sha>^{commit}`). The caller checkout was at HEAD c3e9ab0be with a clean status when the review started. The review-source binding command exited 0.

## Scope

The cluster has one file, `docs/superpowers/specs/INDEX.md` (slice 000050), and the diff adds one row: the 2026-11-26 entry "Per-seat config and Codex /side queue wake routing".

## Analysis

- Link: the relative target `../runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` exists at head (`git cat-file -e` succeeded), and the path resolves from `docs/superpowers/specs/`.
- Placement and format: the row is newest-first, above the 2026-10-31 rows. It has the six columns the header declares (date, title, design link, summary, status, tags). The tags lead with the run slug, as the 2026-10-31 run-spec row does.
- Title: matches the design doc's H1 exactly.
- Summary accuracy, checked against the design at head:
  - `ht config seat list/get/set` plus human-tier writes through `ht human config seat set` matches Section 1 "Commands".
  - "programmatic_wake moves from node to seat" matches "Removing the node setting".
  - "Codex queue wake targets the root session with /side open" matches Section 2 "Selection".
  - "native wake defers on a side chat" matches Section 3.
  - The summary leaves out the `unset` verbs. That is acceptable for a one-line summary.
- Status `draft`: other run root specs that were implemented also keep `draft` (for example the 2026-10-31 environments implementation spec), so the value is consistent with practice. Moving it to `implemented` once the work lands is optional bookkeeping.
- Requirement baseline (B1-B10, S1): an index row delivers none of these rows, and none falls to this cluster. Production delivery of each row runs through the CLI, daemon, store and harness clusters, and tracing it is the seam reviewer's job. This cluster neither contradicts nor narrows any row: the summary's tier split (agent-writable reads and low-risk writes, human-only gated writes) matches B6 and B7.

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- none

## Deferred OK
- INDEX.md row status stays `draft`; update it to `implemented` when the epic lands, if the project wants the index to track landing state (existing run-spec rows do not).
