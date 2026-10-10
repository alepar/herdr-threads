# Automatic one-to-one threads: pair-boundary

Parent: [root spec](2026-10-10-automatic-one-to-one-threads-design.md) · bead ht-e99.1

## Goal

Provide the canonical pair identity, atomic resolver and immutable enrollment/audience invariant consumed by direct send and handoff.

## Problem description

This subtree provides one precisely owned boundary of the root automatic-channel behavior. The root spec is normative for syntax, authority, invitation consent, lifecycle and compatibility.

## Main challenges

Canonical pair identity must be unique and indexed, while ordinary entry points must never enroll a third seat or send outside the pair. Response loss must replay the original channel without fresh effects.

## Key decisions made

Split the compilable schema/protocol contract from its canonical transaction and membership enforcement. The first leaf supplies an inert capability boundary; the second activates only after all guards exist. Neither client consumes a partially implemented resolver.

## Decision points

### Contract ownership

The contract leaf owns the next additive migration with pair uniqueness and FKs, strict EnsureDirectThread command/result/permit/hash types and default-omitted SendMessage.direct_recipient field, capability token, dispatch adapter signature and compatibility fixtures. Its default runtime returns unsupported until the guarded implementation is installed; it must not advertise support early. Existing serialized commands/results remain unchanged. Use the ordinary ThreadCreated/ID result if its semantics fit; no new result fields are needed just for presentation.

### Transaction and enrollment

The mechanism leaf owns the actual accountable transaction using the contract, create/invite event/projection wiring, reversed concurrency convergence, archived/left/rejected guidance and historical replay. It also owns every pair-only check in ordinary/service/operator enrollment and send preparation/publication, including direct_recipient validation/audience staging independent of ACK-only invited_recipients, plus TRUST-POLICY A5. Resolve all membership through canonical effective/voluntary semantics, not stale raw state. All checks and activation land together so no advertised resolver can create a porous pair. Existing ordinary APIs stay unchanged for unmarked threads.

### Acceptance and retention

Tests use real store transactions and barriers for reverse concurrent ensure, saved-operation replay and third-participant attempts. Pair retention preserves history; cleanup cannot delete the unique pair identity and permit a new duplicate conversation while its seats/history remain valid. Migration applies only newly allocated schema with no historical mutation.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
