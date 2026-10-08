## Goal

Root accountable commands refuse inferred Human before effects; human communication and check-in retain honest existing provenance and canonical guards.

# Runtime Actor Routing

Epic: `ht-uwd.3`. Parent: [root](2026-10-07-installer-human-permissions-design.md). Mode B autonomous.

## Problem description

Person attribution presently comes from current contexts during selected/cooperative dispatch; run_selected can retire a context before dispatch. A parser-only namespace cannot guard these effects. Choose one common route check before any identity/intent/context change; retain existing explicit agent selection and A2 daemon validation. Reject mixing Human route with explicit agent cooperative selectors rather than reinterpreting claims. Accountable own inbox preserves Human no-display-ACK behavior; explicit foreign-seat inbox remains read-only. Person lifecycle uses existing Human binding/provenance, --operator only accepted through Human route and existing A4/UID logic. Native hooks continue agent-only enrollment; no new hook routing field.

## Main challenges

Preserve exact existing provenance/canonical authority, owned-byte invariants and approved scope. No real config, shared host, native models, full suite or unrelated adapter changes.

## Key decisions made

The leaves below own separate merge-worthy mechanisms. Shared interfaces and files stay assigned to one owner per concern; serialized merges rebase and revalidate common files. Common per-change gates: focused meaningful RED/GREEN all-feature tests, fmt, clippy/default guard, diff and UUID cleanup.

## Decision points and task ownership

### ht-uwd.3.1: Guard accountable caller selection before local effects

Own route validation in derive_caller/derive_selection/run_selected/run_cooperative/run_operator and own-inbox handling. owns: runtime actor boundary. consumes: InvocationActor route. Acceptance: root inferred Human refuses before context retirement, intent mkdir or daemon mutation; explicit agent selector remains lawful; human agent-selector mismatch refuses; Human own inbox never creates display ACK. Files: src/cli/mod.rs caller/dispatch sections, tests/cli/cooperative.rs, tests/cli/human.rs.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.1: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.1).

### ht-uwd.3.2: Human check-in and person communication preserve provenance

Wire human me init/check-in and human communication through existing Human claims and operator semantics, preserve agent-to-human A4 guard unless existing operator UID decision permits. owns: human lifecycle/communication route integration. consumes: runtime actor boundary. Acceptance: RED/GREEN Human init/communication/operator repair, no fabricated agent binding, legacy root refuses before effects, canonical stale/hold guards still decide. Files: src/cli/me.rs, tests/cli/human.rs; commands grammar owned by ht-uwd.1.
Files and acceptance are scope bounded; use isolated fixtures, test-support spawn and no live model.
blocked-by ht-uwd.1: consumes declared producer artifact
blocked-by ht-uwd.3.1: consumes declared producer artifact
Acceptance consumes declared producer artifact (needs: ht-uwd.1).

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
