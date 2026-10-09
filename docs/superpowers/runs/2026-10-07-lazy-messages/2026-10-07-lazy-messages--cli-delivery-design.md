## Goal

Default text inbox settles only fully flushed contiguous lazy bodies with independent durable frozen-origin recovery.

Parent: [approved lazy design](../../specs/2026-10-07-lazy-messages-design.md). Bead: `ht-big.5`. Mode B.

## Problem description

Implement explicit CLI v2 inbox with durable contiguous display journal and independent frozen ACK/completion intent recovery.

## Main challenges

Keep ordinary compatibility while separating passive delivery from receipt obligations. Bound work, preserve frozen actor identity and recover after partial output or mutation failure.

## Key decisions made

Adopt parent contracts: per-message lazy recipient progress, explicit v2 discovery, canonical metadata, existing durable mutation envelope, independent mixed-page ACK and completion intents.

## Decision points

Separate output proof from mutation replay; reusing receipts would fabricate obligation, while a high-water-only read cursor would hide partial chunks. Keep existing summary render/cache interfaces unchanged and query delivery mode separately. Installer seam is cli::actor_route::InvocationActor {Agent,Human}, split_actor_argv(&[String])->Result<(InvocationActor,Vec<String>),ApiError>; journal::classify_original_actor(&IntentScope,&SemanticMutation)->io::Result<OriginalActor> {Agent,HumanOrOperator}. Retry preflight classifies frozen origin before try_completed_retry; Human namespace is immediate argv[1] human with routing flags afterward, --human formatting only. Top-level coordinates active owner integration and exact upstream revisions.

## Deliverables and acceptance

### V2 explicit inbox output and contiguous display journal

Render bounded v2 lazy chunks and continuation pages; produce durable occupant-bound fully-displayed candidates only after full selected page write/flush, contiguous offset zero through length. JSON/machine/explicit-seat remain read-only; skips/partial writes/flush failure/cancel/binding change leave pending.
owns: v2 CLI output; occupant-bound contiguous chunk proof and fully-displayed candidate interface.
consumes: v2 batch handler.
Files: CLI inbox output and isolated chunk journal plus failure tests.
Acceptance: preserve parent invariants; focused failing regression tests for the stated failure cases; no shared server/profile changes.

### Independent frozen ACK and lazy completion intent recovery

Persist both frozen intents before either submission; submit independently despite peer failures; retain exact retry refs on partial journal failure or lost reply and clear successful set only; lazy IDs never ACK. Preserve saved SemanticMutation+CallerClaim harness+IntentScope before replay/completed-result presentation/cleanup. Canonical completion idempotent; human top-level allowed, declared subagents read-only.
owns: completion SemanticMutation/journal envelope; independent settlement/retry/cleanup; frozen actor routing.
consumes: fully displayed candidate journal; canonical completion handler; installer classifier seam.
Files: CLI journal mutation retry and thin inbox settlement plus injected failure tests.
Acceptance: preserve parent invariants; focused failing regression tests for the stated failure cases; no shared server/profile changes.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
