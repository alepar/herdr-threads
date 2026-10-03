> **Superseded (2026-10, ht-p03.2 / B4):** the adversarial verification layer described here was removed; see "Cooperative reality (2026-10)". Where this document says "native" acceptance, or mentions caller proof or verification evidence, read it as the cooperative claim: acceptance and ACKs are the top-level agent's cooperative claim, recorded as `cooperative_top_level`; see "Cooperative reality (2026-10)" in the [seat identity design](../herdr-threads/2026-09-27-herdr-threads--seat-identity-design.md) and [root design](../herdr-threads/2026-09-27-herdr-threads-design.md).

## Goal

Expose the system session to a programmatic graph caller and demonstrate the complete native acceptance and notification flow through production daemon wiring.

Parent: [approved amendment](design.md). Bead: ht-4is.32. Earlier siblings: [connection](connection-design.md), [store](store-design.md). Client construction consumes ht-4is.29; runtime integration consumes those siblings' implemented interfaces.

## Client contract

Provide a typed persistent client object that registers once, serializes one request at a time and exposes service operations. No credential file or fake native claim. Store exact pending mutation envelope/operation key/payload durably before submission using the existing local intent conventions scoped to durable programmatic author. On reconnect validate instance/boot/protocol, register anew and expose unresolved intents. Recovery queries or explicit exact-key retry retain payload and key. Never automatically resend a mutation after ambiguous submission. Registration is not itself a durable mutation intent; close an ambiguous registration before retrying its claim.

Per-call deadlines, bounded frames/correlation/output, unknown-outcome classification and authoritative errors follow existing clients. Idle connection lifetime is independent of per-call timeout. An error causing connection loss invalidates its session handle; service operations require a new registration. Document that ordinary one-shot CLI invocations cannot retain authority for a later process. A CLI diagnostic/recovery path may be one-shot; service calls use the persistent programmatic client. No new resident model or daemon solely to hold a CLI token.

## Native surface and composition

Route reserved service commands from registered transport metadata, not user-supplied author claims. Route native acceptance/ACK through existing cooperative caller guards. Extend native acceptance input and retained intent to carry observed requirement revision: the displayed required invitation and the explicitly accepted revision must agree. Surface pending requirement for joined voluntary members; stale acceptance response asks the agent to reread. Startup/check-in teaches agents to accept required invitations explicitly, explains unleaveable status and never auto-accepts.

Wire system event attention into existing scheduler/native delivery; no service author native targeting. Rendering includes durable author, management owner, pending/accepted requirement and release/retirement status with existing bounded continuations. Native participants still read public system channels and discuss/ACK native messages normally. Document same-UID cooperative limitations, explicit disconnect recovery, info/warn behavior, teardown release, and graph's responsibility to reconcile notification effects.

## Validation

A real local daemon fixture runs the persistent client and ordinary client together through register, create, required invite, explicit acceptance, rejected leave, attributed notify, disconnect/reconnect/replay, requirement release and successful leave. Validate ordinary invitations into populated ordinary threads and required rejection there; validate unauthorized topic/archive, no forged service/built-in author, and stale operator disconnect. Distinguish published notice from agent action; inspect stored actor/requirement/receipt records.

Both Codex and Claude native configurations must explicitly accept a displayed required invitation and encounter leave refusal using their actual integration. A hook output, launch/prompt success or mock does not count as native acceptance. Verify ACKs on ordinary native messages still require explicit model calls, while programmatic events create none. Reuse the existing native validation facilities and acceptance gates; do not duplicate or waive the original epic's broader native proofs. Package/build checks include the new public API and documentation.

## Deliverables

D1 owns persistent client and durable intent/recovery API with fake-transport tests plus caller documentation; consumes ht-4is.29. Files: client and intent helpers/docs. Fake transport covers idle/session handling, instance/boot changes, framing/correlation, commit-response loss, exact payload retention and explicit replay.

D2 owns production service/store/transport routing, bounded native CLI/check-in output and required-acceptance revision propagation; consumes D1, B1/B2 and C2/C3. Files: service/app/CLI/harness presentation. Real daemon integration tests verify end-to-end wiring and malformed native/service authority separation. Its exact dependency IDs name implemented interfaces, not narrative order.

D3 owns actual Codex/Claude demonstrations, release documentation/checks and stored-evidence report for this addition; consumes D2. Files: native integration tests/fixtures and docs. Exercise each native configuration early once runnable, before terminal sweep. Maintain original validation evidence requirements; bead closure is never itself proof.
