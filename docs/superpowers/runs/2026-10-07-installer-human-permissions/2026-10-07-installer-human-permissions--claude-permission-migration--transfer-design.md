## Goal

Interrupted Claude permission transfers retain exact ownership and resume safely; independent lifecycle never restores broad grants or removes foreign policy.

# Claude Permission Migration  Transfer

Epic: `ht-uwd.5.3`. Parent: [Claude migration](2026-10-07-installer-human-permissions--claude-permission-migration-design.md). Mode B autonomous.

## Problem description

Implement a prepared transfer journal with bounded schema: source hook manifest identity/fingerprint, destination permission manifest identity, exact before/after settings fingerprints, old owned rules and new owned rules plus pre-existing flags. States: Prepared (durable record, settings old); SettingsPublished (settings new, source proof retained); DestinationPublished (destination manifest exact, transfer record retains source evidence); SourceRetired (historical hook permission ownership retired); Complete (remove prepared marker last). Derive resumable stage from exact artifacts, not guessed current rule presence. Write-ahead Prepared precedes settings replacement; destination ownership precedes source retirement. Resume advances only matching combinations; unknown bytes/duplicate conflicting owner refuse. Removal consults exact prepared records and removes only owned current entries, leaving pre-existing/foreign rules. No real config transaction guarantee: interruption is recoverable, every stage preserves evidence. Hook and permission APIs serialize writers against refreshed base; compare/recheck settings and manifests before each publish; preserve unrelated changes only by a freshly rederived safe plan, never overwrite stale bytes. Lifecycle uses this state machine for migration, normal independent installation uses prepared own-file update where needed. Strict manifest version validation and legacy literal path lookup coexist with normalized verified absolute root identity.

## Main challenges

Preserve exact existing provenance/canonical authority, owned-byte invariants and approved scope. No real config, shared host, native models, full suite or unrelated adapter changes.

## Key decisions made

The leaves below own separate merge-worthy mechanisms. Shared interfaces and files stay assigned to one owner per concern; serialized merges rebase and revalidate common files. Common per-change gates: focused meaningful RED/GREEN all-feature tests, fmt, clippy/default guard, diff and UUID cleanup.

## Decision points and task ownership

### ht-uwd.5.3.1: Durable exact Claude ownership transfer engine

Implement validated prepared record, exact stage derivation and idempotent resume/publication state machine from historical source to independent destination. owns: prepared ownership transfer engine. consumes: Claude rule renderer and legacy hook evidence. Acceptance: interrupt each boundary then resume/retry/remove, at least one durable removal proof always, no two contradictory owners, unchanged foreign/preexisting bytes; corrupt/edited/race states refuse. Files: src/harness/permissions/claude_transfer.rs, src/harness/setup.rs transfer proof adapter only; focused transfer fixtures.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.5.1: consumes declared producer artifact
blocked-by ht-uwd.5.2: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.5.1).

### ht-uwd.5.3.2: Independent Claude install status remove with shared JSON safety

Wire backend lifecycle to exact transfer engine and current ownership manifest, refreshed settings plans and representability diagnostic; status separates hooks/native permissions/external broad rule warning. owns: independent Claude lifecycle. consumes: prepared ownership transfer engine. Acceptance: fresh/update/unsetup plus migration/partial/edited/foreign/symlink/race matrix; unrelated hooks/deny/ask survive and removed broad never returns. Files: src/harness/permissions/claude.rs lifecycle, tests/installer_integrations.rs.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.4: consumes boundary contract
blocked-by ht-uwd.5.3.1: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.4).
boundary contract: ht-uwd.4


## Round 1 settled ownership/publication decisions

ExactHistoricalOwned is an existing logical permission component, recognized only by exact non-pre-existing historical proof and current matching bytes; an authorized update narrows/transfers it even when new grant consent declines. Native matching scope never expands without consent: bare historical herdr-threads grants do not authorize ht/absolute forms, retired export-only rules do not authorize direct ordinary grants. Missing/edited/foreign/pre-existing evidence cannot add permissions. Shared OwnedConfigWriteGuard serializes all cooperating owned writers across processes from refreshed validation through ownership/settings/recovery publication. Noncooperating edits in final-check→rename interval are an explicit accepted limit. Historical recovery fault model is process termination on functioning filesystem with existing file+parent sync barriers. New-process interruption witnesses cover each promised transfer stage and lock release. Generic fresh/update/remove retain honest partial refusal, not universal automatic recovery.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
