---
super-roast verdict: clean (0 nits)
mode: PR        iteration: 1 of 3
profile (assumed): Internal same-user cooperative CLI/daemon for agent collaboration. Durable SQLite membership, rejection history and occupant-specific delivery are real state; accidental delivery or wake changes matter, but hostile same-user identity attacks and multi-fleet deployments are out of scope under TRUST-POLICY.
inputs: super-auto/invitation-rejection-policy@55bc7cea8952a1f68f458c3c2d70a1c39884d711 vs main@f3431b31
coverage: correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, api-contract, observability, testing, hygiene-docs · 11 scouts dispatched, 0 dead · 2 raw → 1 deduped → 1 panel / 0 spot-checked / 0 promoted · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI GPT) — seat-differentiated panel
seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 0/1/0 · refute 0/1/0 · ground 0/1/0

## Confirmed findings
- none

## Not verified (beyond panel cap)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- src/store/attention.rs:1098 — A saturated backlog of 1,001 unoffered rejection notices becomes wake attention despite the new passive classification and no-wake guarantee.
  verdict: rejected (reproduce REJECT / refute REJECT / ground REJECT).
  reason: All three seats establish that the saturation mechanism exists, but TRUST-POLICY.md:409–414 explicitly permits conservative waking when bounded warning or notice narrowing cannot exclude older waking work, including moderation attention. The reproduce seat establishes that this exception already exists in base f3431b31 and that the design expressly adopts the same delivery limits. The ground seat confirms the unnarrowed-window fallback at attention.rs:1127 and the full-window wake predicate at attention.rs:695. The claim assumes an unconditional no-wake guarantee beyond the documented contract; the evidence supports an accepted limit rather than a regression.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
---
