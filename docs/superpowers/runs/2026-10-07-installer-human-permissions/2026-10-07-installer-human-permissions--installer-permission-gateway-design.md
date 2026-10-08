## Goal

Installer delivers only its validated owned executable spellings to independently consented native permission setup, with truthful update/removal and gateway evidence.

# Installer Permission Gateway

Epic: `ht-uwd.8`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.

## Problem description

Installer already owns canonical installed binary, herdr-threads symlink and ht alias protection (4bc613bb). Do not rediscover foreign PATH entries. Supply raw path/routing inputs via namespace-owner reserved internal CLI args; Rust validation checks bounded absolute spellings, ownership markers/symlink target and canonical target before accepting them. Keep invoked validated spelling and canonical executable separately. Claude representability can conservatively exclude ambiguous path forms with explicit diagnostics; Codex renders exact escaped strings. Two leaves separate input consumer from shell transport so each has actual entry-point witnesses. Update installer consent text to ordinary read/write permissions excluding human/operator and preserve --no-setup/declined/noninteractive semantics; --setup explicit grant is not hooks ownership inference. Uninstall exact permissions unsetup precedes owned link removal, absent binaries/removal refusals remain truthful. No package/version/release changes.

## Main challenges

Preserve exact existing provenance/canonical authority, owned-byte invariants and approved scope. No real config, shared host, native models, full suite or unrelated adapter changes.

## Key decisions made

The leaves below own separate merge-worthy mechanisms. Shared interfaces and files stay assigned to one owner per concern; serialized merges rebase and revalidate common files. Common per-change gates: focused meaningful RED/GREEN all-feature tests, fmt, clippy/default guard, diff and UUID cleanup.

## Decision points and task ownership

### ht-uwd.8.1: Validate installer executable inventory at Rust setup consumer

Consume parser-reserved raw paths/routing and validate exact owned canonical/link/alias forms; reject foreign or mismatched ownership and ambiguous controls; direct setup allows only safe bare names + verified current canonical binary unless explicit owned inputs validated. owns: installer inventory validation/transport consumer. consumes: PermissionInputs and permission component lifecycle. Acceptance: actual Rust internal installer invocation positive owned inputs and foreign/relative/control/symlink mismatch/race negatives, spaces/metachars represented or diagnostic. Files: src/cli/installer.rs input consumer, src/cli/setup.rs SetupEnv inputs; commands parser owned by ht-uwd.1.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.4: consumes boundary contract
blocked-by ht-uwd.7: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.4).
boundary contract: ht-uwd.4

### ht-uwd.8.2: Pass owned installer paths and consent through shell gateway

Forward installed_binary/link_path/alias_path and pinned state/socket to actual installer-integrations consumer; update missing permission consent and truthful summaries/exit/next steps; preserve foreign/force/PATH protections and uninstall ownership. owns: shell inventory gateway and permission consent wording. consumes: installer inventory validation/transport consumer. Acceptance: fake-host shell fixture actual gateway arguments plus Rust consumer round trip; fresh/update/uninstall/declined/no-setup/noninteractive/custom-prefix/foreign/symlink cases; no real config. Files: scripts/install.sh, tests/release/install_test.sh.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.7: consumes declared producer artifact
blocked-by ht-uwd.8.1: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.7).


## Round 1 settled consent decision

Exact non-pre-existing matching historical owned permission proof is existing state, not Missing merely because new manifest absent. Narrow/transfer within old native match scope without expanded grant consent; ht/absolute/new direct or export-only broader grants need explicit consent, decline leaves missing desired forms with truthful incomplete status. Hook-only/pre-existing/foreign/edited/missing rule states never authorize additions. Consent before shared config lock; revalidate under OwnedConfigWriteGuard before mutations.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
