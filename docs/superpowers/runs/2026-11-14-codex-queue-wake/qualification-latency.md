# Queue wake latency qualification

The first three-threshold live baseline failed all three six-second wake checks. The single authorized diagnostic repeat passed one and failed two. All six rows proved exact-attempt queue acceptance, hook body delivery, logical fulfillment and bounded cleanup. These transport results do not qualify the behavioral latency contract. The remaining 69 matrix rows remain unrun; no production change, merge or release follows from this evidence.

| Diagnostic threshold | Accepted commit (ms) | First exact-scope Started observation (ms) | First Herdr working observation (ms) | Six-second result |
|---|---:|---:|---:|---|
| 0 | 663 | 2492–2505 | 2563–2571 | PASS |
| -1 | 765 | 6550–6555 | 6604–6606 | FAIL |
| 3 | 769 | 6073–6078 | 6355–6357 | FAIL |

Offsets use the publication checkpoint's monotonic clock. Started and same-attempt fulfillment were first observed together in every row. The read intervals are observations, not exact native-event times; unchanged sampling ticks were not retained. Herdr first reported working shortly after the first observed Started, which supports receiver wait as the main observed delay rather than several seconds of status lag. Body delivery was checked after the turn; its exact first output time was not measured.

The [matching tagged source](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/ext/queue/src/service.rs#L82) checks external SQLite changes every ten seconds (lines 82–89). Enqueue's immediate wake uses its own manager (252–265, 447–456), and queue-change events use its own event sink (459–463). A separate helper therefore cannot promise a universal six-second receiver wake. This is historical source evidence, not a version gate or an exact timing measurement of these runs.

Among the already-inspected APIs, no supported notification into this embedded receiver was established. The helper's loaded-thread list was empty; add/list/delete address shared storage, and queue/changed is a notification rather than a receiver poke or delivery receipt. No new architecture was implemented to work around this limit. The design owner will present the latency tradeoff to the user; prior acceptance of cancellation races did not waive latency.

Both runs removed their owned workspace, daemon, scratch home and unchanged credential copy with no cleanup problems. The diagnostic's scoped process check passed; its temp-file checker reported no ledger. Evidence preserves that limit. The diagnostic used 9–11 distinct state samples, 94–108 existing host-status observations and 28–35 KiB per recipient, within the existing bounds and without extra host queries.

[Initial baseline](qualification-latency-baseline.json) retains all three failures. [Diagnostic evidence](qualification-latency-diagnostic.json) retains bounded canonical projections, host observations and raw-artifact hashes. [Instrumentation checks](qualification-latency-instrumentation.json) records focused 15+17 tests and required guards. The initial full suite and its timing failure remain separate and unresolved. Tracking: ht-d2jq.6.3 (latency disposition), ht-d2jq.6.2 (suite timing).
