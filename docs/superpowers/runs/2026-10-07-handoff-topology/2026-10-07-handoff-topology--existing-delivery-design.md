## Goal

An existing canonical peer receives exact staged work and replayable completion without native launch or identity replacement.

Parent: [approved root](2026-10-07-handoff-topology-design.md). Bead: ht-qhz.5. Autonomous Mode B.

## Decisions

Inherit the root spec exact grammar, immutable identities, authority and failure contracts verbatim; no independent namespace or retry policy. Split only independently reviewable artifacts with actual consumed boundaries.

### Exact existing recipient and durable staged-work delivery

Implement --existing selector grammar, canonical seat lookup and keyed shared staging with immutable DeliveryPlan.
owns: DeliveryPlan staged-work execution and existingpeer selector resolution;
consumes: topology/delivery boundary contract and existing canonical Handoff fences.
Files: src/cli/handoff_delivery.rs, src/cli/commands.rs (existing mode), src/cli/handoff.rs (shared staging extraction).
Acceptance: actualCLI RED/GREEN no allocation/rebind/start/preflight/registration, held/unresolved/retired/foreign refusal, canonical joined/pendinginvited/unboundreporting, exact invitation episode reuse and send children; original Handoff serialization/golden unchanged.

### Delivery retry and completed presentation cleanup

Integrate journal retry and terminal reporting for existing-peer durable delivery.
owns: DeliveryPlan retry and absorbing terminal presentation;
consumes: DeliveryPlan staged-work execution.
Files: src/cli/handoff_delivery.rs (retry/report), src/cli/retry.rs.
Acceptance: RED/GREEN changedcurrentbinding/archive/restart completion cleanuponly, failedflush/removal retry, exactorigin classifier before all paths, zero duplicate invitation/message or nativeeffects; retained report corruption refuses.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
