## Goal

Maintain one connection-bound system authority per instance while preserving ordinary one-request clients, bounded request processing and truthful recovery.

Parent: [approved amendment](design.md). Bead: ht-4is.30. Consumes ht-4is.29 compiled contract. Its authority gate is normative; no independent check-then-commit implementation is permitted.

## Protocol and lifecycle

The first bounded frame selects ordinary request handling or explicit system registration. Ordinary handling stays one request/response and closes. Registration negotiates the existing protocol version plus the service-session capability; reject unsupported clients/servers visibly. Validate kernel UID, expected instance and request correlation before claiming the slot. Allocate a monotonically changing connection generation within the daemon boot, bind the reserved durable author, and return the boot/generation. Registration response loss requires the client to close and reconnect; it never proves registration failed.

A successfully registered socket reads sequential request/response frames. No multiplexing or server push in this release. Keep the existing encoded frame bound and five-second per-operation budget. Idle time between complete requests has no ordinary request deadline. Once the first byte of a new frame arrives, apply the ordinary bounded read/dispatch/write budget; partial frames cannot pin execution indefinitely. Invalid frames terminate registration. Long-lived idle registration consumes one bounded connection slot, never a writer or search slot; health and ordinary clients must retain capacity. Registration takes the existing bounded admission path; no unbounded waiting claim queue.

The transport owns EOF observation and generation revocation, including during dispatch. A service request may stage outside the transaction, then acquire DB write ownership and the authority guard in that order. The guard validates the exact active boot/generation and remains held through commit/rollback. Revocation takes only the authority gate, never a DB lock. A transaction already holding the guard finishes before revocation becomes effective. No await of host/network/preparation is allowed under the guard. A disconnect before the deciding guard rejects uncommitted publication; after commit it creates an unknown outcome to recover, not rollback. Cancellation and ordinary service request budgets are checked at the existing store decision boundary.

Use explicit owner identity for cleanup: EOF/error/shutdown/drop revokes only its own generation. A late completion or cleanup cannot clear a successor. Restart reconstructs durable author state but initializes an empty live slot. The socket must be closed when a service call times out; resolve ambiguity with existing stable operation keys on a fresh registration. Preserve ordinary request unknown-outcome behavior.

## Recovery and diagnostics

Same-UID operator inspection reports instance, boot, connected/disconnected, connection generation and registration time; no native-agent liveness claim. Explicit operator disconnect includes expected boot/generation and compare-and-revokes that generation only. Reject stale observations. Close the owned socket and cancel uncommitted work; retain already committed history. Persist an audit event through the store audit interface supplied by the contract without acquiring DB ownership under the authority gate: revoke first, enqueue a bounded audit write afterward, report audit failure visibly. A lost audit write must never be represented as successful durable audit; disconnect outcome and audit outcome are distinguishable.

Recovery is deliberate, never automatic takeover based on elapsed idle time. Same-UID trust means an agent can deliberately invoke the operator path; documentation must not call this authenticated human approval.

## Deliverables

B1 owns registration, sequential frame loop, generation gate runtime and disconnect/commit fencing; consumes ht-4is.29. Files: daemon transport/authority integration. Real UDS tests cover two claims/one winner, separate instances, idle persistence, partial/oversized frames, ordinary-client progress, EOF before decision versus response loss after commit, restart, and stale cleanup.

B2 owns observed-generation operator recovery and bounded registration diagnostics; consumes B1's registry/revocation interface. Files: daemon control/health/diagnostics and typed operator routing. Tests exercise stale and matching disconnect, active request cancellation, committed request survival and visible audit failure. No fake native authority or agent launching.
