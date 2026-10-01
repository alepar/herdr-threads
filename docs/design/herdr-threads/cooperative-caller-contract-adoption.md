# Adopted cooperative caller contract

> The draft and review files named below are archived in git tag `archive/herdr-threads-run-2026-09-26` under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/` (path unchanged there).

Root adopts revision 2 after the independent scoped Sol/medium review: CLEAN, zero findings. Root read the complete report and independently verified all 24 inspected source/input hashes and the report hash. This is finalized cumulative review159. Original formal design3/3 and all previous counters remain unchanged.

The amendment is normative where earlier root, seat, caller, harness or shared contracts require adversarial proof of top-level receipt authority. It implements the explicit user direction at a64e7c9: cooperative lifecycle/current CheckIn, durable generation/instance/context fences, top-level model-owned acceptance and ACK, and child read/summarize instructions. Claims and plugin context are identified honestly; native attestation is not a receipt prerequisite. Other mapping/retirement/prompt/launch and operator boundaries remain required.

Authorize scoped TDD implementation by the named contract, identity, store, harness/client and runtime owners. The reviewer explicitly checked initial-generation CAS, historical replay, availability/timers, exact durable client request persistence, single-use permit and independent product gates. Implementation review and both installed native harness demonstrations with model calls and SQLite evidence are still required. No code/test/native acceptance is inferred here.

No nonzero finding resulted, so no fix/history-redesign loop is triggered by this review. The last full cumulative-history assessment158 remains intact; the next nonzero must include this new report in its fresh complete read.
