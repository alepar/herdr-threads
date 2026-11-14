super-roast verdict: clean (0 nits) [low coverage] [review readiness incomplete]
mode: design        iteration: 3 of 3
profile (assumed): Internal same-user cooperative agent messaging. Identity, receipt provenance, bounded delivery and priority-correct wake are core contracts. No production implementation; primary-source and credential-free SDK probes exist. TUI/live qualification remains deferred explicitly.
inputs: pi-harness/design.md final revised design 2026-11-14
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 1 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
coverage: scouts 9/9 (premortem, completeness, yagni, failure-mode, feasibility, regression, domain:agent-runtime-integration, domain:distributed-systems, domain:identity-and-receipt-provenance) · raw 0 → deduped 0 → panel 0 · spot 0 · promoted 0 · judge completion n/a (no panels) · remainder-capped: 0 · review readiness: coverage bounded, provenance available — basis: "Primary sources, supported research claims and real SDK boundary probes; TUI/live provider not yet qualified."
independence: same-family (OpenAI GPT) — seat-differentiated panel · rung: manual fan-out
lane-yield (found/confirmed/unique/refuted): premortem 0/0/0/0 · completeness 0/0/0/0 · yagni 0/0/0/0 · failure-mode 0/0/0/0 · feasibility 0/0/0/0 · regression 0/0/0/0 · domain:agent-runtime-integration 0/0/0/0 · domain:distributed-systems 0/0/0/0 · domain:identity-and-receipt-provenance 0/0/0/0

Prior R2 replay-refill finding: resolved by explicit source comparison, not by scout silence. docs/superpowers/runs/2026-11-14-pi-harness/roast-design-2/design-input.md:45 requires refill only “if displacement occurred,” leaving discarded incoming overflow without a recovery transition. docs/superpowers/runs/2026-11-14-pi-harness/roast-design-3/design-input.md:45 sets replay-needed for every eligible unretained body, explicitly including incoming discards and resident displacement; capacity release consumes it through canonical replay without relying on old-watch re-offers. The same paragraph preserves the flag on failure, re-arms it on further drops, and requires the exact no-displacement/exhausted-re-offers regression. This closes the missing healthy-path transition at design level. docs/superpowers/runs/2026-11-14-pi-harness/roast-design-2-dispositions.md:3 independently records the same repair and explicitly disclaims an implemented or passing regression test at line 5.

All nine scouts returned no candidates; no panel or spot check ran. This does not establish source completeness or runtime qualification. Source readiness remains bounded, TUI/live-provider qualification remains incomplete, and no convergence is claimed. Prior rejections below remain standing without reconsideration; the prior spot-checked editorial FYI is not a current-round spot finding.

## Confirmed findings

- none

## Not verified (beyond panel cap)

- none

## Not verified (dedupe failed or judge lost)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- D5 active-run interval; Open decisions before implementation; D5 positive active-run interval; Open decisions before implementation — Busy delivery relies on an unverified active-run interval across retry, tool, abort and compaction transitions, risking passive mail starting a new run.
  reason: All seats reject. D5 expressly holds retry gaps, compaction and uncertainty, requires positive active-run evidence and fresh synchronous checks, and promises busy delivery where possible. D8 and Open decisions already require phase qualification before support is complete; no unsafe permitted transition is established.
- D4 first, third and fourth paragraphs; D4: sendMessage is fire-and-forget; deduplicate in-flight records; clear local state only on admitted completion or explicit generation teardown — The in-flight submission state lacks a recovery rule distinguishing a lost fire-and-forget injection from a still-queued frame, risking permanently suppressed replay or duplicate reinjection.
  reason: All seats reject. D4 deduplicates records, not every retry submission. D5 preserves unconsumed wake intent and permits local retries without upgrading passive priority. Pending receipts without evidence are accepted, and fire-and-forget alone does not establish silent loss before queuing. This does not refute the separately confirmed concrete reload-queue or capacity cases.
- D1 final paragraph; D3 final paragraph — Recovery from failed check-in or a dead watch depends on a future Pi callback, leaving an otherwise idle session without a specified autonomous path back to receiving wakeable mail.
  reason: All seats reject. D1 explicitly accepts no idle wake without a live channel; D3 retries at the next eligible callback, and D7 requires documenting no-channel behavior. Autonomous idle reconnection would strengthen the chosen contract.
- D5 final paragraph; D1 channel-only wake — Channel-only idle wake remains contingent on unqualified behavior of TUI selectors and extension dialogs.
  reason: All seats reject. D5 requires real-binary modal/draft qualification and keeps an interfering path disabled until a supported guard exists. D8 and Open decisions retain that gate. Rejection does not establish modal safety or completed qualification.
- D4 admission predicate — The design leaves bounded current-branch receipt lookup unspecified despite requiring it on the model-context path.
  reason: All seats reject. D4–D5 bound delivery batches and pending content, not native history length or lookup CPU time. Current-branch matching already forbids stale cache evidence. No workload failure or violated lookup budget is demonstrated.
- D4. Delivery, evidence and receipt timing, admission predicate and accepted-limit paragraphs — D4 makes current-branch storage matching an additional mandatory receipt dependency without stating what it proves beyond the already required exact-frame observation in the current session context.
  reason: All seats reject. D4 expressly selects context-plus-current-branch admission and pending receipts when either observation is missing; D7 repeats this policy. A context-only alternative may simplify it but no stated receipt-availability promise is violated.

## Unverified nits (spot-checked)

- none

## Escalations (need human)

- none
