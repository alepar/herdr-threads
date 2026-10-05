super-roast verdict: Should-fix (1 confirmed)
mode: pr        iteration: 1 of 3
profile (assumed): This is an internal, local coordination daemon and CLI for cooperative same-user use, with durable SQLite history. Production user data and canonical caller, seat, and receipt continuity matter. The main coordinator owns the combined-tree full-suite and release gates.
inputs: super-auto/human-message-intent@a972e578e528f0fd476ba15445b18164eb0748f3 vs main@5aa02d96c97a21b3eed9142b2b4f27390b82df74
coverage: scouts 11/11 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, data-migrations, api-contract, observability, testing, hygiene-docs) · raw 2 → deduped 1 → panel 1 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 1 · rr 0.00 · rg 0.00 · fg 1.00 · unanimous 0.00 · ground-loo n/a (n=0) · reproduce 0/1/0 · refute 1/0/0 · ground 1/0/0

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] src/store/summary.rs:1388 — Completed summary jobs retain cumulative fetched worker bundles indefinitely, causing quadratic persistent storage growth as live ledger entries accumulate across chunks.
  verdict: confirmed (reproduce ✗ / refute ✗-survived / ground ✓)
  evidence: The refute and ground seats trace cumulative active entries into each persisted fetched_bundle_json snapshot, which remains after submit and lease expiry; they cite 1+2+...+N retained entries across N jobs. The reproduce seat REJECTs on the approved spec's requirement for complete current-fetch/lease snapshots and clearing on lease acquisition/replacement. The confirming seats address the spec's current-fetch/lease scope and find no stated acceptance of indefinite retention after replay ceases; that evidence does not overturn the panel tally. The panel is same-family and seat-differentiated, so its agreement is not independent verification.
  fix-shape hint: Add bounded cleanup for completed snapshots after the valid replay window, preserving snapshot availability while lease and generation fences permit replay.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- none

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
---