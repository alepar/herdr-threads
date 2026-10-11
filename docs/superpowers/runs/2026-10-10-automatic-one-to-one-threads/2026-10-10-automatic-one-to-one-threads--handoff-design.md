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

The shared leaf owns shared automatic grammar/parse representation (compilable independently of each destination admission), an explicit automatic-channel variant and new-only version handling in strict protocol/journal validators, complete canonical deciding adapters with fail-closed defaults for unsupported runtimes, Ensure child registration, canonical resulting thread attachment and namespace equality. It leaves V1 exact bytes/hash/launch reports accepted under their historical grammar. The shared leaf owns complete canonical deciding adapters and daemon automatic-handoff capability activation when Ensure attachment and restricted effects are usable. Local CLI producer presence never gates the daemon descriptor; each producer independently owns capability negotiation before local publication/first effects, including new-tab before topology creation. Automatic-only child registration is Ensure→Send, with no Invite; reject raw Invite registration/execution in automatic plans. Send references only the attached pair and revalidates target lifecycle at preparation/publication, so a lost Ensure response followed by rejection/leave cannot reinvite or send. Explicit/V1 plans retain their Invite sequence. Shared journal/version edits belong here; consumer leaves call the decided helpers rather than independently changing serialized layout.

### Existing-recipient producer

The existing/pane leaf owns parser omission semantics and existing-peer/pane/native-launch handoff preparation and staged execution. It freezes target and automatic selection before publication, uses the shared Ensure→Send child keys/attachment without Invite, preserves explicit override forms, and runs one fake-host invocation each for --existing --seat, --existing pane, and launch-into-pane. It tests lost ensure/send replies (and lost invite replies for explicit/V1 modes) and completed historical presentation, with no acceptance, ACK or duplicate start.

### New-tab producer

The topology leaf owns --new-tab omission: preserve bootstrap topology fences/unknown recovery, wait for canonical recipient attachment, then form automatic downstream plan and resolve/attach pair through the same shared contract. It tests one full fake-host new-tab request, child-complete/outer-incomplete recovery and possible-start fences. It must not depend on the existing-recipient producer's work: shared automatic plan/adapter boundary is the needed artifact.

## Post-Implementation Notes

**Changes vs. original design — 2026-10-10:** Implemented additive automatic V2 child/bootstrap plans and all four producers. Automatic plans omit Invite and require exact published ordinary Send and completed-child proof before parent completion. Public historical reconciliation now absorbs a canonically completed child after sender binding change while retaining live full-claim admission and at-most-one native submission. Imported complete snapshot-staging fixture loops preserve existing assertions; modeled producer/real canonical and UUID-wrapper source-tracing limits remain explicit.

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

- 2026-10-10 — The automatic V2 handoff representation freezes one original caller, namespace, recipient, body, keys and selected canonical child. Additive 0030 companion tables register that full original and its exact parent/child linkage; legacy Create/Invite children are refused for automatic plans, while explicit/V1 plans retain their old bytes, keys and report grammar. Ensure creates the initial ordinary invitation, then ordinary Send publishes against current pair lifecycle; exact frozen published Send proof precedes Complete and guarded startup. Active parents protect unknown or selected threads from archival; terminal completion releases only their own protection.
- 2026-10-10 — Existing-seat, existing-pane and launched-pane routes use production-owned parsed producers with injected client/launcher seams. The new-tab route uses the production-used injectable runtime and canonical SqliteStore in its composition tests. New-tab creation/possible-start recovery remains conservative. The additive local OperatorRecoverAutomaticBootstrap V2 semantic reuses the operator intent kind and routes parsed Human recover/retry to canonical inspected decisions, preserving V1 operator records and original agent attribution. A launched-pane lost-Ensure→Reject test models the canonical refusal at the fake protocol boundary; the real canonical Reject and publication refusal are separately exercised in the authority suite.
