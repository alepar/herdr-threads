### Per-task verdicts
ht-3bi.2.4.1 / Transactional v2 evidence store — LEAF — One storage artifact over externally owned model/schema: exact-key transactions, milestones, descriptor integrity, bounded reads/retention and their acceptance tests; no unresolved policy or separable downstream migration work.
ht-3bi.2.4.2 / Negotiated v2 daemon evidence handler — LEAF — Strict wire/capability and its implemented descriptor-driven handler form one reviewable server operation; recording, bounded holding, legacy suppression and advisory fetch behavior are specified acceptance obligations rather than independent subsystem redesigns.
ht-3bi.2.4.3 / Adapter evidence submission and durable gates — LEAF — One client evidence-send path with its durable retry/dedup state; capability fallback, adapter attribution, observer limits and legacy gates are specified, with no new model/schema/native callback ownership.

### Decomposition verdict
COMPLETE
- Coverage: nested Store API and transaction maps to .1; Negotiated wire and daemon maps to .2; Client evidence and gates maps to .3, including their focused acceptance tests and compatibility obligations.
- Correctness: the actual chain is .1 → .2 → .3, with .1 consuming external .2.3 model and .4 SQL schema; no epic blocking edge is treated as an implementation dependency. Each child has at most one direct sibling dependent, so no task satisfies the SPLIT bottleneck test.
- Boundaries: .2.1 codec/output, .2.2 durable qualified-turn routing and .2.3 runtime/domain model remain sibling responsibilities; schema allocation belongs to .4 (CLI 19/20, adapter 21 after main absorption; prototype provisional). No child claims Python/native Hermes acceptance, health rendering, publication, or coordinator-owned final full sweep/cleanup.
- Reviewability: the three children divide the vertical recording flow at its store, negotiated server operation and client submission interfaces; the exact behavior is resolved by the parent/nested specs and does not force another design epic.
