# Ordinary seat resolution: recovery contract clarification

The CLI owner found that ResolveSeat carried only target even though ordinary resolution can allocate a durable seat. The canonical CLI contract requires a private operation key before submitting a mutation and exact-key recovery after ambiguous output. The separate service-allocation authority route does not exempt its effects from recovery.

## Ruling

Add `operation: OperationId` to ResolveSeat and typed `ResolveSeat` local intent kind. The CLI routes ordinary seat resolution through durable intent publication, preserving explicit instance and target in the semantic request. Use a narrow service-allocation intent scope with instance and target; do not fabricate a native seat before allocation, a native caller claim, or an operator actor. The daemon continues to establish allocation provenance internally and consume OrdinaryAllocationGuard for a new allocation. Existing kernel/local instance routing remains unchanged. The new journal scope authorizes no other command.

The store binds the operation record to instance plus the ordinary-allocation route and canonical target payload. Exact-key replay returns the immutable original seat result, including after mapping changes or retirement; it neither revives nor reallocates the old seat. Reusing the key with a different target/payload fails. An uncommitted retry obtains a fresh allocation guard and rechecks ownership/recovery holds. A new intentional resolution uses a new key and may return the currently existing seat under the existing resolve policy. Replayed historical seat IDs do not establish current ownership or native authority.

This corrects a missing protocol field and journal route under the existing recovery requirements; no new product feature or authority class is introduced. Managed-launch composition that internally resolves a seat must consume this same durable resolution path. Startup registration retains its existing CheckIn operation key and registration authority.

## Ownership and evidence required

Task43 owns ResolveSeat/IntentKind/shared fixture additions. Task24 owns parser/materialization action classification. Task26 owns the narrow service-allocation journal scope and recovery semantic conversion. Task6 owns operation replay around allocation; Task31 composes it. Require invalid-scope refusal, loss-after-allocation recovery to the same seat, changed-payload rejection, replay after retirement without revival, and fresh guard on uncommitted retry. Existing guarded allocation, operator whitelist and native ACK gates remain unchanged. All provisional and final review gates remain.
