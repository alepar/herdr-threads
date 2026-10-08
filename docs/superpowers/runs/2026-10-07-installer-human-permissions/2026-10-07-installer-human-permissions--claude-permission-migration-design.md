## Goal

Claude grants positive ordinary forms independently of hooks, with exact recoverable migration of owned historical broad rules and no foreign-policy adoption.

# Claude Permission Migration

Epic: `ht-uwd.5`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.

## Problem description

Choose three mechanisms: pure positive renderer, hook decoupling, and ownership transfer. Renderer uses catalog plus validated path/routing set and conservative text representability (ambiguous literal glob/quotes/backslashes refuse those forms; safe bare coverage remains). Hook completeness ignores new permissions but still reads historical proof for migration/removal. Transfer uses prepared source/destination manifest identities and old/new settings fingerprints; source owns until prepared transfer durable, proposed settings published, destination ownership published, then historical ownership retired. Resume accepts exact prepared states; edited or contradictory bytes refuse. Config roots use verified absolute keys with historical literal lookup. Both components serialize shared JSON writers and re-read latest bytes; retain deny/ask/managed and unrelated hooks. Pre-existing broad rules remain foreign with explicit status disclosure; no claim of complete external exclusion.

## Main challenges

Preserve exact existing provenance/canonical authority, owned-byte invariants and approved scope. No real config, shared host, native models, full suite or unrelated adapter changes.

## Key decisions made

The leaves below own separate merge-worthy mechanisms. Shared interfaces and files stay assigned to one owner per concern; serialized merges rebase and revalidate common files. Common per-change gates: focused meaningful RED/GREEN all-feature tests, fmt, clippy/default guard, diff and UUID cleanup.

## Decision points and task ownership

### ht-uwd.5.1: Claude positive ordinary permission renderer

Generate positive ordinary-family and exact supported pinned/output rule forms, never arbitrary middle wildcard covering human. owns: Claude rule renderer and representability contract. consumes: ordinary command catalog; PermissionInputs. Acceptance: ordinary reads/writes and --human formatting covered; immediate human/legacy operator excluded by grammar plus ask forms; literal-metachar/space/quote/body/compound fixture scope explicit. No native model evidence. Files: src/harness/permissions/claude.rs, src/harness/claude.rs matcher fixtures.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.4: consumes boundary contract
Acceptance consumes declared producer artifact (needs: ht-uwd.4).
boundary contract: ht-uwd.4

### ht-uwd.5.2: Decouple hook completeness and writes from new permissions

Remove broad automatic permission grant/requirement from new Claude hooks install/status while preserving historical OwnedPermission fingerprint/pre_existing/superseded/resume proof. owns: independent hook completeness and legacy ownership evidence. consumes: separate permission manifest schema. Acceptance: hook-only update cannot add broad or missing permission; hooks status independent; historical removal refuses edited proof and preserves foreign/pre-existing rules. Files: src/harness/setup.rs hook install/inspect/remove internals, setup unit tests.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.4: consumes boundary contract
Acceptance consumes declared producer artifact (needs: ht-uwd.4).
boundary contract: ht-uwd.4

### ht-uwd.5.3: Crash-safe Claude permission ownership transfer and lifecycle

Implement independent install/inspect/remove and durable prepared source/destination transfer from exact historical broad/retired entries, never two live owners or lost removal evidence. owns: Claude lifecycle and transfer state machine. consumes: Claude rule renderer and representability contract; independent hook completeness and legacy ownership evidence. Acceptance: interrupt every publication boundary then resume/remove; no broad restore; malformed/edited/foreign/partial/symlink/race cases safe; shared JSON fresh-base preservation and stronger policy intact. Files: src/harness/permissions/claude.rs lifecycle, src/harness/setup.rs transfer adapter only, tests/installer_integrations.rs.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.4: consumes boundary contract
blocked-by ht-uwd.5.1: consumes declared producer artifact
blocked-by ht-uwd.5.2: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.4).
boundary contract: ht-uwd.4


## Round 1 settled ownership/publication decisions

ExactHistoricalOwned is an existing logical permission component, recognized only by exact non-pre-existing historical proof and current matching bytes; an authorized update narrows/transfers it even when new grant consent declines. Native matching scope never expands without consent: bare historical herdr-threads grants do not authorize ht/absolute forms, retired export-only rules do not authorize direct ordinary grants. Missing/edited/foreign/pre-existing evidence cannot add permissions. Shared OwnedConfigWriteGuard serializes all cooperating owned writers across processes from refreshed validation through ownership/settings/recovery publication. Noncooperating edits in final-check→rename interval are an explicit accepted limit. Historical recovery fault model is process termination on functioning filesystem with existing file+parent sync barriers. New-process interruption witnesses cover each promised transfer stage and lock release. Generic fresh/update/remove retain honest partial refusal, not universal automatic recovery.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
