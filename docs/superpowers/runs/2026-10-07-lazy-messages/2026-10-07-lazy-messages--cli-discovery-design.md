## Goal

Expose lazy sends and canonical read-only markers without changing ordinary sends or summary rendering/cache.

Parent: [approved lazy design](../../specs/2026-10-07-lazy-messages-design.md). Bead: `ht-big.4`. Mode B.

## Problem description

Implement send --lazy validation and capability refusal plus canonical lazy markers on selected history/body/search pages while keeping summaries stable.

## Main challenges

Keep ordinary compatibility while separating passive delivery from receipt obligations. Bound work, preserve frozen actor identity and recover after partial output or mutation failure.

## Key decisions made

Adopt parent contracts: per-message lazy recipient progress, explicit v2 discovery, canonical metadata, existing durable mutation envelope, independent mixed-page ACK and completion intents.

## Decision points

Separate output proof from mutation replay; reusing receipts would fabricate obligation, while a high-water-only read cursor would hide partial chunks. Keep existing summary render/cache interfaces unchanged and query delivery mode separately. Installer seam is cli::actor_route::InvocationActor {Agent,Human}, split_actor_argv(&[String])->Result<(InvocationActor,Vec<String>),ApiError>; journal::classify_original_actor(&IntentScope,&SemanticMutation)->io::Result<OriginalActor> {Agent,HumanOrOperator}. Retry preflight classifies frozen origin before try_completed_retry; Human namespace is immediate argv[1] human with routing flags afterward, --human formatting only. Top-level coordinates active owner integration and exact upstream revisions.

## Deliverables and acceptance

### Lazy send CLI validation and compatibility refusal

Expose --lazy on native send with CLI validation and unsupported-daemon refusal before intent; preserve ordinary omission/digest and frozen actor classification, keep service-owner/compound handoffs ordinary.
owns: CLI lazy send validation and capability preflight.
consumes: lazy send boundary contract.
Files: CLI send parsing/execution; narrow owner-coordinated grammar and guide paragraph.
Acceptance: preserve parent invariants; focused failing regression tests for the stated failure cases; no shared server/profile changes.

### Read-only lazy markers on history body search pages

Annotate selected history/body/search pages using ≤100 exact-ID canonical mode queries; preserve old-daemon ordinary read-only fallback and complete-content summary inclusion with no renderer/chunker/cache/lease or model-work changes.
owns: canonical CLI [lazy] selected-page annotation.
consumes: MessageDeliveryModes handler.
Files: CLI history/body/search printing and focused compatibility tests.
Acceptance: preserve parent invariants; focused failing regression tests for the stated failure cases; no shared server/profile changes.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
