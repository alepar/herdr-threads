super-roast verdict: Blocking (1 confirmed) [low coverage] [review readiness incomplete]
mode: design        iteration: 2 of 3
profile (assumed): Internal same-user cooperative agent messaging. Identity, receipt provenance and priority-correct wake are core contracts. No production implementation exists; source and fake-provider SDK probes are available, with TUI/live qualification explicitly deferred.
inputs: pi-harness/design.md revised 2026-11-14
delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 8 resolved · 1 regressed (1 Blocking) · 0 punch-listed (open)
coverage: scouts 9/9 (premortem, completeness, yagni, failure-mode, feasibility, regression, domain:distributed-systems, domain:agent-runtime-integration, domain:process-identity-and-lifecycle) · raw 2 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage bounded, provenance available — basis: "Primary-source extracts, supported research claims and real SDK nine-case boundary probe; TUI/live-provider qualification not yet performed."
independence: same-family (OpenAI GPT) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 1/0/0 · refute 1/0/0 · ground 1/0/0
lane-yield (found/confirmed/unique/refuted): premortem 0/0/0/0 · completeness 0/0/0/0 · yagni 0/0/0/0 · failure-mode 0/0/0/0 · feasibility 1/1/0/0 · regression 1/1/0/0 · domain:distributed-systems 0/0/0/0 · domain:agent-runtime-integration 0/0/0/0 · domain:process-identity-and-lifecycle 0/0/0/0

Prior dispositions below compare docs/superpowers/runs/2026-11-14-pi-harness/roast-design-1/design-input.md against docs/superpowers/runs/2026-11-14-pi-harness/roast-design-2/design-input.md. They establish repaired design text, not implementation or runtime qualification. Missing findings are not resolved from absence; TUI/live-provider readiness remains incomplete.
Prior role admission: resolved in revised D2 by an explicit extension-produced top-level assertion, inherited-owner and child refusal, and the documented marker-stripping limit.
Prior native queue survival: resolved in revised D3/D5 by removing bodies from native nextTurn/steer/follow-up queues and reconciling unacknowledged current-branch frames on reload.
Prior aggregate capacity: regressed; revised D4 adds bounds and urgent displacement but incompletely defines post-capacity replay, as confirmed below.
Prior failed wake observation: resolved in revised D5 by a 30-second unreceived transition, suppression for unchanged attention and explicit requalification triggers; bounded degraded progress is expressly accepted.
Prior first idle wake: resolved in revised D3/D5 by successful startup/reload admission plus fresh isIdle, without requiring an earlier agent_settled event.
Prior lifecycle guidance/provenance: resolved in revised D3 by ObserverOnly guidance output, watch-only mail and explicit boundary insertion with no guidance receipt.
Prior shutdown ownership: resolved in revised D3 by owner-scoped environment withdrawal, watch/timer cleanup and best-effort session-end liveness disconnection.
Prior same-session tree navigation: resolved in revised D3 by branch revision, evidence invalidation, current-branch reconciliation and guidance restoration without reversing valid receipts.
Prior cross-batch ordering: resolved in revised D4/D5 by sorting the retained pending union at each admission boundary and eliminating the native passive-body FIFO.

## Confirmed findings

- [Blocking] docs/superpowers/runs/2026-11-14-pi-harness/roast-design-2/design-input.md, D4 paragraphs 1–2 — Replay refill is conditional on resident displacement, so excess incoming bodies discarded without displacement have no recovery transition when capacity is released. [fix-regression] [lanes: feasibility, regression] (regressed: iteration 1 aggregate-capacity finding fixed incompletely)
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats distinguish “discard excess bodies without settlement” from the narrower “if displacement occurred” refill condition in D4. A full passive cache can discard later worse-ranked frames without replacing any resident, then admit and ACK retained pages after an ordinary user turn without triggering replacement.
  evidence: Reproduce and refute cite src/delivery/ledger.rs:21-24: three ordinary offers at 60-second spacing can all expire while capacity remains occupied. ACKs remove retained IDs only. All seats cite src/service/mod_channels.rs:743-838: replay captures one finite pass per lease, including a terminal empty capture, rather than refilling indefinitely after ACKs.
  evidence: D3 makes watch the sole automatic mail path. D4 requires eventual replay after space is released; the healthy-channel saturation case is outside the accepted failed-replay and runtime-wake degradation limits. Canonical retention avoids data loss but does not restore automatic in-context delivery.
  severity: Raised from the seats' Should-fix to Blocking under the core-purpose floor: normal supported saturation can indefinitely prevent the adapter's stated bounded in-context delivery after capacity is available. This is a missing healthy-path transition, not a demand for stronger recovery from an accepted channel failure.
  fix-shape hint: Track every discarded unacknowledged body as replay debt, including incoming excess, and consume that debt through the existing progress-triggered watch replacement.

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

- [FYI] D6. Setup and launch, interactive option grammar paragraph — D6 leaves the launch parser scope open-ended by requiring documented interactive options generally after declaring a restricted initial option set.
  refute spot vote: REJECT; unverified at panel strength. D6 already limits launch to a tested initial subset and refuses unknown options. Tightening adjacent general wording is editorial, not a requirement for full CLI parity.

## Escalations (need human)

- none
