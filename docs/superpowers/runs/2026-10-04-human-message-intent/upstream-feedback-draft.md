# human-message-intent: distinguish deferred findings from successful repairs

Plugin: 6.4.2-alepar4.15. Run: one epic, nine leaves, nine serial task rounds, 2026-10-04. Target: alepar/superpowers. Local parked proposal; no issue filed.

## Defects

### 1. Fix-pass accounting calls an ownership-deferred unresolved finding FIXED

- **Evidence:** Task4 ledger: `fix pass FIXED (valid Important guide mismatch declined: Task5 explicitly owns prose reconciliation)`; its completion remains `parked: Important guide mismatch remains Task5-owned; no clean verdict claimed`. Metrics count `entered 1 · FIXED 1 · BLOCKED 0`. Task5 later resolved the guide seam independently.
- **Premise to verify:** The skill currently permits a grounded decline to return FIXED and carries the unresolved finding as parked. Determine whether FIXED means successful handling or actual repair; overall parked tracking stayed honest, but repair counters lose this distinction.
- **Suggested fix shape:** Record DEFERRED/PARKED separately from verified FIXED, preserve the owning task and resolution evidence, and extend metrics/parser probes accordingly.

## Run metrics

### Judge panel


design round 1: independence: same-family (OpenAI GPT) — seat-differentiated panel · rung: manual fan-out; seat-agreement: panels 5 · rr 0.80 · rg 0.60 · fg 0.80 · unanimous 0.60 · ground-loo 0.75 (n=4) · reproduce 1/4/0 · refute 2/3/0 · ground 3/2/0

design round 2: independence: same-family (OpenAI GPT) — isolated scouts; no judge seats required this round · rung: manual fan-out

pr round 1: independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out; seat-agreement: panels 1 · rr 0.00 · rg 0.00 · fg 1.00 · unanimous 0.00 · ground-loo n/a (n=0) · reproduce 0/1/0 · refute 1/0/0 · ground 1/0/0

pr round 2: independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out

### Fix loop

Metrics: completions — review clean 8 · after fix pass 0 · parked 1 · re-entry closes 0 · dispatched early 0 · cancelled 0
Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0

### Merge-back

Metrics: merges 9 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 0 (fixed 0) · check-fails 0 (fixed 0)
Metrics: ledger-check ok · append-failed 0 · append-retried 0

Git shows nine distinct first-parent task merges and nine successful Merge lines; conflicts/seam reviews/failed checks/blocker merges: zero.

### Coverage

Design coverage rounds1/2: requirements11 mapped11 unmapped0. Code scope-filter round1: 0 in-scope · 1 punch-listed; round2 converged with12/12scouts, no new confirmations, retention finding remains open.

### Bead graph

| id | type | title | what |
| --- | --- | --- | --- |
| ht-nmp.9 | task | Freeze first-fetched summary worker input | Persist and reuse the first-fetched JobBundle per valid fetch/lease so intervening equal-timestamp or clock-rollback commits cannot change validation input. |
| ht-nmp.8 | task | Withdrawn or replaced query closure regression | Exercise explicit human withdrawal/replacement of unanswered queries through the worker contract and final summary fold, with the replacement separately classified and no daemon semantic inference. |
| ht-nmp.7 | task | Integration sweep: human message intent | Verify main flows end to end, add uncovered seam tests and fix small gaps, then run only focused tests and required fmt/clippy/default-feature checks. |
| ht-nmp.6 | task | Configuration smoke: direct human, relaying agent, legacy and refused sources | Exercise direct human, relaying agent, unclassified legacy, unrelayed-agent and service/system refusal configurations through send/storage/JSON and summary entry points using focused integration tests. |
| ht-nmp.5 | task | Human intent transcript and agent guidance | Render independent attribution/intent markers and actual summary ledger tokens and update ht skill, CLI help, agent usage and trust policy for explicit forwarding classification, source claims and summary-time lifetimes. |
| ht-nmp.4 | task | Cumulative summary jobs and compatible generations | Wire intent-aware records through cumulative level0 bundles, local-block persistence/fallback, fetched_at replay, Ready/rollup pinning and renderer2/submission2/promptv2 generation isolation. |
| ht-nmp.3 | task | Intent-aware deterministic ledger lifetimes | Derive one stable i.seq record per priority source, preserve unclassified legacy behavior, apply query/request resolution and active-rule withdrawal/replacement guards, and reject duplicate classified open items. |
| ht-nmp.2 | task | Classified human sends and attribution validation | Expose --user-intent query/request/rule and validate canonical ordinary-human or relaying-agent eligibility without changing human attention priority or receipt semantics. |
| ht-nmp.1 | task | Seam contract: recorded intent and rule-change storage | Land compilable optional UserIntent and RuleChange types with message/result/journal/bundle/item/transition plumbing, one schema22 migration and wire5 boundary, preserving absent-field decoding and all fixtures. |
| ht-nmp | epic | Recorded human message intent and summary lifetimes | Implement the approved human query/request/rule intent design independently of relay attribution, preserving legacy messages and attention priority. Spec: docs/superpowers/specs/2026-10-04-user-message-intent-design.md. Scope stops at reviewed merge-ready code; coordinator owns main merge, full suite and release. |

| dependent | blocker | reason |
| --- | --- | --- |
| ht-nmp.9 | ht-nmp.4 | consumes cumulative job runtime |
| ht-nmp.9 | ht-nmp.1 | consumes boundary contract |
| ht-nmp.8 | ht-nmp.5 | consumes worker guidance |
| ht-nmp.8 | ht-nmp.4 | consumes cumulative summary runtime |
| ht-nmp.7 | ht-nmp.6 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.2 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.8 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.4 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.9 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.5 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.3 | consumes all leaves (integration sweep) |
| ht-nmp.7 | ht-nmp.1 | consumes all leaves (integration sweep) |
| ht-nmp.6 | ht-nmp.5 | consumes Human intent transcript and agent guidance |
| ht-nmp.6 | ht-nmp.2 | consumes Classified human sends and attribution validation |
| ht-nmp.6 | ht-nmp.4 | consumes Cumulative summary jobs and compatible generations |
| ht-nmp.5 | ht-nmp.4 | consumes SUMMARY_PROMPT_VERSION=thread-summary-v2 and SUBMISSION_SCHEMA=2, asserted by embedded guidance tests. |
| ht-nmp.5 | ht-nmp.1 | consumes boundary contract |
| ht-nmp.4 | ht-nmp.3 | consumes intent-aware prefill/fold/validator |
| ht-nmp.4 | ht-nmp.1 | consumes boundary contract |
| ht-nmp.3 | ht-nmp.1 | consumes boundary contract |
| ht-nmp.2 | ht-nmp.1 | consumes boundary contract |

## Design questions

None beyond clarifying FIXED semantics above.

## Doc gaps

None surviving triage.

## Already fixed — do not re-litigate

The project guide/version ownership seam was resolved by Task5 and independently verified in final review. No unresolved product defect is attributed to that historical intermediate state.

## Not established

- This is one run; no measured throughput improvement or repeated operational failure is established.
- Analyst: gpt-6-astra/high, fresh context and supplied evidence only; no tools.
- No branch full-suite measurement exists by explicit user instruction; main coordinator owns combined-tree validation.
- Ledger counts are cross-checked here against actual nine task merges and closed beads. They do not establish generic fire-and-forget append reliability.
- Same-family panel agreement is not cross-family independent verification.

## Verification bar

Exercise a fresh bounded fixer declining a plan-mandated ownership seam; verify unresolved status/owner persists, deferred count is distinct from repaired count, resume keeps its spent allowance, and later owning-task resolution updates current readiness without rewriting historical outcomes.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
