# Automatic one-to-one threads: handoff

Parent: [root spec](2026-10-10-automatic-one-to-one-threads-design.md) · bead ht-e99.3

## Goal

Compose automatically selected pair channels into existing and topology handoff plans with exact frozen replay and canonical child attachment.

## Problem description

This subtree provides one precisely owned boundary of the root automatic-channel behavior. The root spec is normative for syntax, authority, invitation consent, lifecycle and compatibility.

## Main challenges

Existing V1 plans are byte-sensitive, and automatic selection must attach to a canonical fence before restricted child effects. New-tab handoff discovers its recipient only after guarded topology attachment.

## Key decisions made

One shared leaf owns the new automatic plan representation, canonical ensure child registration/attachment and compatibility. Existing-peer and topology producers then consume that shared boundary independently; neither rewrites V1.

## Decision points

### Shared automatic handoff contract

The shared leaf owns an explicit automatic-channel variant and new-only version handling in strict protocol/journal validators, default-inert adapters, Ensure child registration, canonical resulting thread attachment and namespace equality. It leaves V1 exact bytes/hash/launch reports accepted under their historical grammar. The shared leaf owns complete canonical deciding adapters and daemon automatic-handoff capability activation when Ensure attachment and restricted effects are usable. Local CLI producer presence never gates the daemon descriptor; each producer independently owns capability negotiation before local publication/first effects, including new-tab before topology creation. Child invite/send can only reference the attached pair result. Shared journal/version edits belong here; consumer leaves call the decided helpers rather than independently changing serialized layout.

### Existing-recipient producer

The existing/pane leaf owns parser omission semantics and existing-peer/pane/native-launch handoff preparation and staged execution. It freezes target and automatic selection before publication, uses the shared child keys/attachment, preserves explicit override forms, and runs one fake-host invocation each for --existing --seat, --existing pane, and launch-into-pane. It tests lost ensure/invite/send replies and completed historical presentation, with no acceptance, ACK or duplicate start.

### New-tab producer

The topology leaf owns --new-tab omission: preserve bootstrap topology fences/unknown recovery, wait for canonical recipient attachment, then form automatic downstream plan and resolve/attach pair through the same shared contract. It tests one full fake-host new-tab request, child-complete/outer-incomplete recovery and possible-start fences. It must not depend on the existing-recipient producer's work: shared automatic plan/adapter boundary is the needed artifact.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
