### Per-task verdicts
ht-3bi.2.1 / Adapter-driven hook decode budgets and output — LEAF — One bounded hook orchestration/codec refactor with specified legacy bytes, admission, output and budget acceptance; no unresolved architectural choice or qualifying bottleneck split.
ht-3bi.2.2 / Durable qualified-turn routing and replay — LEAF — One durable journal selection/replay mechanism whose transition, watermark, deadline and canonical-authority rules are explicitly specified; Hermes worker qualification remains outside this task.
ht-3bi.2.3 / Exact runtime identity and domain contract model — LEAF — The deliberate pure-model split defines one shared adapter evidence contract, with exact grammar, hashing, legacy projection and delegation decisions supplied; it has only one listed child dependent.
ht-3bi.2.4 / Negotiated domain-scoped evidence recording — PROMOTE — The model split removes interface uncertainty, but this remaining delivery still spans client transport/gate files, protocol negotiation, daemon validation/recording, transactional store APIs and retention/holding policy (about five separable implementation workstreams), triggering the size test; it has no listed child dependents, so SPLIT does not apply.

### Decomposition verdict
COMPLETE

Applied: .2.1/.2.2/.2.3 leaves; .2.4 promoted for nested evidence recording design. AdmissionRequest signature clarified in root to bind Hermes actual callback runtime without PATH substitution.
