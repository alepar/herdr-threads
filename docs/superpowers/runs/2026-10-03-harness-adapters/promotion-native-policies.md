### Per-task verdicts
ht-3bi.5.1 / Local adapter status and setup dispatch — LEAF — The spec fixes the shared request/status/repair and scope/options behavior; generic setup dispatch with temporary legacy delegates is one bounded producer operation, and only .5.2 directly lists it among these children's dependencies.
ht-3bi.5.2 / Claude Codex owned setup backend migration — LEAF — Relocating the two existing concrete backends preserves specified transactions, ownership and consent; shared helpers and regression checks support one setup migration without unresolved design or a second subsystem.
ht-3bi.5.3 / Adapter managed launch composition — LEAF — LaunchPolicy, shared guards and concrete argv/config/wrapper providers implement one specified managed-launch operation; its supporting provider and probe surfaces do not independently require promotion, and only .5.4 depends on it.
ht-3bi.5.4 / Registry host wake and composer providers — LEAF — Registry recognition and optional composer dispatch are bounded host-interaction policy migration with explicit fencing, recipe capability checks and no-provider behavior; no child depends on this task.

### Decomposition verdict
COMPLETE
