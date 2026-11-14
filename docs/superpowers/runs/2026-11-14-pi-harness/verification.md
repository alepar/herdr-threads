# Research and design verification

This checkpoint contains research, probe code and design artifacts only. Implementation has not started. The checks below ran against the initial `c4c5ecb9e` baseline; the documentation branch was subsequently rebased onto landed opencode core at `cf93b1bef`. They are historical baseline results, not checks of the new integrated base.

| Check | Result |
|---|---|
| Deep-research coverage and stop consistency | Passed, two retrieval rounds; bounded stop, not saturation |
| Independent claim support | 38/38 supported; judges configured as `gpt-6.1-terra`; no lexical fallback |
| Report/dossier package consistency | Passed |
| Report bibliography accessibility | 14/14 accessible; sandbox DNS failure recovered through approved network check |
| Model-free real pi SDK/RPC experiment | Queue/context ordering and isolated RPC boot observed; no live credential used |
| Boundary follow-up | Nine asserted real-SDK cases; completed-outcome guard required |
| All-target/all-feature locked clippy, warnings denied | Passed; first crate check from seeded target took 63 seconds (three seconds beyond the per-change target; no production delta to attribute a regression to) |
| Harness-neutral guard | Passed, zero violations |
| Wire compatibility guard | Passed, zero violations/stale entries |
| Rust formatting check | Passed |

Build/check commands ran through the required shared heavy/toolbox wrapper. Transient full upstream captures were removed after extracting the cited evidence; selected verbatim passages, source locators and original capture hashes remain in the research ledgers. Probe dependencies and scratch were removed; owned child processes were stopped and awaited. The pi worktree and seeded target remain owned by the ongoing implementation task.

Retrieval health is **DEGRADED** because earlier sandbox searches failed and a guessed singular documentation route was unavailable. All claim evidence uses successful raw extracts, and the unavailable route is not relied on. The research report records exact failures, partial accesses and recovery recommendations.

No real-TUI, credentialed provider, public integrated adapter, full nextest or release qualification is claimed. Those are implementation/integration gates in `implementation-plan.md`. Review verdicts and source-readiness limitations are in the corresponding `roast-design-*` reports.

Final design review: R3 reports clean (0 nits), with low-coverage and incomplete-runtime-readiness qualifiers. Nine scouts returned no material candidates; the reporter explicitly resolved R2-F1 against the frozen designs. No production or live-TUI qualification is implied. The R1 reporter-format failure remains recorded in its friction artifact; R2 and R3 format validation passed.
