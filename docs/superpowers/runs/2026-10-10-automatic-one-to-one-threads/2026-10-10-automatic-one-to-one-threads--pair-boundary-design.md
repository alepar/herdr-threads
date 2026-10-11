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

- 2026-10-10 — Migration 0029 retains one indexed sorted seat pair per instance and a unique thread reference. Accountable Ensure atomically creates the joined caller, ordinary pending invitation and registry row or reuses the exact pair; it never adopts a name/topic match or reenrolls a rejected/left seat. The direct_recipient marker feeds the ordinary Send audience independently of ACK-only invitation fields. Canonical pair and publication guards bar third-seat/service enrollment and corrupted audiences; resolved unbound and Human targets preserve ordinary semantics. The passive direct_threads table is registered in the SQLite update-hook classifier.
- 2026-10-10 — Main's ordinary Reject now emits an attributed warn notice using frozen membership high-water. That notice is an effect of Reject itself, not automatic pair retry; pair membership restriction bounds its audience to the two reserved seats. Existing historical info replay and explicit reinvite/join/reopen recovery remain. Main's case-sensitive literal name-or-topic search and name/all revision are discovery only and cannot replace the registry identity.
