## Goal

Persist frozen lazy audiences atomically with bounded preparation, cleanup and pending scans, without creating actionable work.

Parent: [approved lazy design](../../specs/2026-10-07-lazy-messages-design.md). Bead: `ht-big.2`. Mode B; inherits approved parent goal and non-goals.

## Problem description

Implement additive audited DDL and bounded lazy recipient staging, publication and preparation cleanup with no receipts/warnings/send_attention.

## Main challenges

Preserve ordinary compatibility and canonical claims while separating lazy bookkeeping from actionable receipt evidence. Keep each indexed walk and mutation work-limited and publication-fenced; isolate tests and stop owned children.

## Key decisions made

Adopt the parent design unchanged: separate per-message recipient progress and explicit v2 discovery, never reuse receipts or high-water-only delivery progress. This keeps partial output and concurrent pages recoverable without creating attention. Existing summary formats, rendering and cache stay unchanged.

## Decision points

Use the existing journaled mutation envelope and full frozen actor/claim/scope rather than invent another provenance or namespace. Use composable narrow accessors behind the seam contract rather than broad changes to legacy v1 responses. Migration numbering remains owner-coordinated and unallocated until top-level confirms; inert contract code never needs the slot. Operational deployment and main release gates remain with the caller.

## Deliverables and acceptance

### Audited lazy-delivery schema and store accessors

Introduce the additive mode/recipient schema, immutable identity triggers, pending indexes and bounded accessors with canonical trust bookkeeping documentation.
owns: stored delivery mode; lazy recipient DDL and ordinal/progress accessors; policy bookkeeping definitions.
consumes: delivery-mode and lazy recipient boundary types.
Files: src/store/schema.rs, migration registry plus new store lazy-delivery module, TRUST-POLICY.md.
Acceptance: owner-confirmed migration slot only, ordinary rows default ordinary, DDL audit, exact-ID mode lookup, bounded recipient and unpublished scans seek candidate before publication test, work counts and continuation stable, large displayed/unpublished query-plan tests; no receipt fields/provenance.

### Bounded lazy preparation publication and cleanup

Stage frozen lazy audiences and atomically publish only manifest-visible recipient deliveries while skipping receipt and attention work.
owns: lazy branch of canonical send preparation/publication, replay validation and discard/expiry bounded cleanup.
consumes: schema and recipient accessors.
Files: src/store send/preparation/messages modules plus focused store tests.
Acceptance: daemon independently rejects lazy with ACK seats/panes/deadline; sealed joined nonretired excluding author includes human/unavailable excludes invitees; revision fences/replay/restart; zero receipt/warning/send_attention; cleanup unpublished one bounded unit; ordinary digest and publication positive controls unchanged.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
