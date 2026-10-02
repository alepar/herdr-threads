The design under review is a settled implementation design for epic ht-rzi in the Rust repo ~/AleCode/herdr-threads/.worktrees/trust-model-invariants (branch trust-model-invariants; read source there to ground claims). Read together:
- Root spec: ~/AleCode/herdr-threads/.worktrees/trust-model-invariants/docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/2026-10-01-b5-trust-policy-guards-design.md
- Normative policy it implements: ~/AleCode/herdr-threads/.worktrees/trust-model-invariants/TRUST-POLICY.md
- Task tree (beads with descriptions, acceptance and blocking deps): ~/AleCode/herdr-threads/.worktrees/trust-model-invariants/docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/tree-dump.json (ids ht-rzi.1 .. ht-rzi.9; ht-rzi.8 is the integration sweep, ht-rzi.9 a seam contract)
- Coverage ledger of decisions already applied: ~/AleCode/herdr-threads/.worktrees/trust-model-invariants/docs/superpowers/runs/2026-10-01-b5-trust-policy-guards/coverage-ledger.md
The trust model is cooperative and same-user by explicit owner decision; findings proposing adversarial verification are out of scope. The B4 deletions (C5/P10/W5-1) are a non-goal owned by another branch.
