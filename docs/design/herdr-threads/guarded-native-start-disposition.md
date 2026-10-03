# Guarded native start: Task11 triage disposition

The coordinator read Task11's complete report and the report-only triage at `.superpowers/sdd/ht-4is-plan/task-11-triage.{json,md}`. Adopt RESOLVE for source implementation. Full native acceptance and ht-910 remain open. The previous goal turn made progress through committed contract rulings, a resumed receipt worker and a fresh deadline-driver review.

## Availability boundary

> **Superseded (2026-10, ht-p03.2 / B4):** the adversarial verification layer described here was removed; see "Cooperative reality (2026-10)".

Managed launch may use either a proven EmptyShell observation followed by direct native start, or an explicitly supported host operation that checks shell ownership and prompt readiness before it starts the agent. Add a narrow adapter-owned typed launch capability distinguishing these cases from Unsupported. It is not client-asserted authority. Unknown observation remains Unknown; only the supported guarded-start path may delegate that availability check to Herdr. Known occupied/editor/agent or blocked UI rejects before launch.
<!-- end superseded -->

Both paths first resolve the durable seat and recovery holds, make a fresh ordered explicit-target structural read, compare expected terminal/generation and required incarnation/epoch fences, and reject known invalidations. The host guard does not provide expected terminal/generation/incarnation compare-and-swap. Preserve the approved optimistic observation/start race and document it. This change removes the unnecessary requirement for a separate prompt-ready field when the start operation itself checks readiness; it does not remove any incarnation or current-execution gate.

Use direct argument arrays with explicit pane, preserve native arguments and permissions, add owned hook configuration and Codex --no-daemon under the existing launch contract. No commands are pasted into an unknown occupant. A finite launch-specific ceiling of 30 seconds is clipped to the caller's absolute budget; the existing two-second prompt ceiling does not apply to startup. If the supported host cannot accept the remaining startup timeout, reject before submission. Cancellation bounds cleanup and late responses cannot change state. Confirmed pre-start rejection stays a rejection; timeout, cancellation, malformed response or missing correlated readiness after possible start returns OutcomeUnknown. Never automatically duplicate a start or treat it as handoff receipt.

## Ownership and validation

Task43 owns the narrow shared capability/seam and fixtures; Task11 owns actual transport/readiness/error mapping; Task30 owns configured launch preflight and argument composition. Task11's existing Unsupported lifecycle response is an explicitly allowed degraded path; periodic reconciliation still needs its own production tests. No native lifecycle support or coherent closure follows from it.

Test both launch paths, unsupported capability, guarded Unknown delegation, known busy/blocked refusal, target and fence changes, explicit argv, correlated startup, possible-start response loss, timeout/cancellation and resource cleanup. Invitations/receipts remain untouched. Pin the host implementation evidence and perform isolated native tests before enabling/claiming the capability for a supported release; mutable source documentation and fake helpers alone are insufficient. Current native incarnation/execution proof remains unavailable. Both harness and model-ACK/SQLite evidence requirements remain unchanged. No native launches or global/shared-host mutations are authorized by this source-work disposition.

This is an autonomous within-scope correction of the preflight mechanism, consistent with the original approved optimistic policy. Existing design roast counters and low-coverage qualification remain; final code review must include this change and its evidence limitations.
