## Goal

Explicit setup/status/unsetup and requested doctor fix manage an independent permission component; hook ownership never authorizes a missing grant.

# Setup Permission Lifecycle

Epic: `ht-uwd.7`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.

## Problem description

Choose two consumers of the shared permission API: explicit lifecycle/doctor, and installer reconciliation/consent. Missing component requires explicit permission consent with ordinary read/write grant and human/operator exclusion. Setup request explicitly selected permissions is consent; broad existing --setup installer consent must be updated to name the new grant. Merely hooks-owned auto-update cannot grant missing permissions. Noninteractive absence reports missing with next action. Installer reconciliation re-inspects exact fingerprints and component state after consent rather than boolean-only equality; hook and permissions shared settings writes are serial and inspect latest bytes. Status distinguishes hooks/skill/permissions and external broad grant diagnostics; unsetup exact manifest removal survives absent harness binary. Doctor fix is requested, uses same component APIs and consent rules, never chooses approval/sandbox/network modes. Use landed component architecture; no adapter-registry guesses.

## Main challenges

Preserve exact existing provenance/canonical authority, owned-byte invariants and approved scope. No real config, shared host, native models, full suite or unrelated adapter changes.

## Key decisions made

The leaves below own separate merge-worthy mechanisms. Shared interfaces and files stay assigned to one owner per concern; serialized merges rebase and revalidate common files. Common per-change gates: focused meaningful RED/GREEN all-feature tests, fmt, clippy/default guard, diff and UUID cleanup.

## Decision points and task ownership

### ht-uwd.7.1: Explicit permission setup status unsetup and requested doctor fix

Wire backend component into explicit request/response/status/remove, with active isolated roots and independent component result. owns: explicit permission lifecycle API. consumes: Claude permission backend; Codex permission backend; component API. Acceptance: actual Rust consumer install/status/remove round trip, absent backend/config and owned-state diagnostics, requested doctor only, stronger/foreign state preserved. Files: src/cli/setup.rs lifecycle and src/cli/doctor.rs component dispatch; setup CLI declaration slots reserved by ht-uwd.1.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.5: consumes declared producer artifact
blocked-by ht-uwd.6: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.5).

### ht-uwd.7.2: Installer component reconciliation and explicit permission consent

Reconcile permissions separately from hooks/skill; require specific missing permission consent, retain ownership-only updates and truthful per-component state/exit. Reinspect exact state after consent and serialize JSON mutation plans. owns: independent permission consent/reconciliation. consumes: explicit permission lifecycle API. Acceptance: missing/declined/noninteractive/hooks-owned cases; consent says ordinary reads AND writes, human/operator withheld; mixed hook/permission status and changed-during-consent refusal; Rust consumer witness. Files: src/cli/installer.rs reconciliation, tests/installer_integrations.rs.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.4: consumes boundary contract
blocked-by ht-uwd.7.1: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.4).
boundary contract: ht-uwd.4

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
