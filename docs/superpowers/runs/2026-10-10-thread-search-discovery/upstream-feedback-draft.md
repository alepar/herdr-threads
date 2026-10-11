# thread-search-discovery: separate review coverage from finding yield and isolate mutable build outputs

Plugin: 6.4.2-alepar4.20 (cached and latest metadata; active local overrides lack independent version metadata). Run: 3 tasks plus epic, 2 dependency rounds, 2026-10-10; duration not measured.

## Defects

None established. The coverage behavior below follows explicit current policy; its treatment is a design question.

## Run metrics

### Judge panel

- Design, iteration 1, same-family OpenAI seat-differentiated panel: `seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 0/1/0 · refute 0/1/0 · ground 0/1/0`.
- PR, iteration 1, same-family OpenAI: 10 scouts completed, 0 dead, 0 raw/deduped findings, 0 panels; seat-agreement none (no required seats). `judgeCompletionPct=0` by engine convention, no failed judges.

### Fix loop

`Metrics: fix-loop round 1: 0 addressed / 0 entered · round 2: 0 addressed / 0 entered · round 3: 0 addressed / 0 entered · round 4: 0 addressed / 0 entered · round 5: 0 addressed / 0 entered`

`Metrics: fix-loop breaker-tripped: 0`

### Merge-back

`Metrics: merges 3 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 0 (fixed 0) · gate-fails 0`

`Metrics: ledger-check ok · append-failed 0 · append-retried 0`

Three successful Merge lines; zero task conflicts, seam reviews, failed gates or blocker merges. A separate main integration append conflict kept both independent INDEX rows; not a task merge failure.

### Coverage

- Round 1: requirements 7 · mapped 7 · unmapped 0; three actionable ownership/verification gaps assigned.
- Round 2: requirements 7 · mapped 7 · unmapped 0; all three fresh reviewers empty, all gaps closed.
- Scope filter: none; no confirmed code-roast findings existed, so fix-loop skipped.

### Bead graph

Gathered using `bd list --label sp:ht-akx --json --status all --limit 0`; all four records closed.

| id | type | title | what |
| --- | --- | --- | --- |
| ht-akx | epic | Make thread list search discover names and topics | Bounded literal name-or-topic discovery, cursor correctness and public contract. |
| ht-akx.1 | task | Implement bounded name-or-topic directory matching and rename-safe cursors | Production predicate, revision publication, cursor and focused behavior coverage. |
| ht-akx.2 | task | Document literal thread-name discovery in CLI help and contract | Help/docs and preserved wire/continuation compatibility. |
| ht-akx.3 | task | Integration sweep: bounded CLI thread-name discovery | Combined isolated production verification and design notes. |

| dependent | blocker | reason |
| --- | --- | --- |
| ht-akx.3 | ht-akx.1 | consumes all leaves (integration sweep) |
| ht-akx.3 | ht-akx.2 | consumes all leaves (integration sweep) |

## Design questions

### 1. Should zero finding yield alone force low-coverage status?

- **Evidence:** PR round 1 completed all ten scouts, dedupe and reporter with no raw findings, no candidates requiring judges and no failed agents. `reporter-prompt.md:174–180` nevertheless mandates `[low coverage]` for zero raw findings on a nontrivial artifact. `super-roast-workflow.md:292` reports judge completion 0 when total seats are zero. The outer report therefore correctly reads `status: clean [degraded: low coverage]` despite completed review obligations.
- **Case for current policy:** Unexpectedly empty output may indicate weak inspection or a shared blind spot; retaining a conservative signal avoids overstating confidence.
- **Case for changing it:** Finding yield does not measure completion, and requiring findings to earn unqualified coverage rewards invented candidates. Distinguishing low yield from missing review obligations would communicate the actual concern; judging can be not applicable when no seats are required.
- **Premise to verify:** Confirm this policy is intended for fully completed empty reviews as well as failed stages; decide how to represent confidence separately from operational coverage.
- **Suggested fix shape:** Separate review-obligation completion, finding yield and not-applicable judge completion, or state explicitly why zero yield intentionally remains degraded.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

### 2. Build provenance checks need mutable artifact isolation across worktrees

- **Evidence:** The initial ledger assumed `shared CARGO_TARGET_DIR is build artifacts only`. A docs-worktree gate then reused a behavior-worktree binary: 41 executed tests were absent from the docs source, compilation reported 0.05s. That evidence was invalidated. The timestamp-forcing workaround was rejected and both leaves/integration were revalidated with private targets, optionally seeded by an APFS clone followed by private-only package cleaning.
- **Premise to verify:** Investigate the actual Cargo reuse mechanism before claiming a Cargo defect or universal cross-worktree failure. Check whether current worktree/verification guidance elsewhere already covers mutable output isolation; the inspected coordinator/worktree/verification files did not.
- **Suggested fix shape:** Document private mutable build outputs for worktrees with divergent sources, safe cache seeding, and source/test-inventory checks when artifact provenance is uncertain. Never clean a shared target to recover one worker.

## Already fixed — do not re-litigate

No upstream fix established. The run recovered locally by rejecting contaminated evidence, using private targets and completing source-correct reruns. The current roast qualifier was preserved faithfully rather than suppressed.

## Not established

- This is one run; the underlying Cargo reuse cause is not reproduced independently. No general Cargo bug, measured speedup or warm incremental regression is claimed.
- Native manual fallback used; Workflow detector log unavailable, native ledger detector available: round 1 peak 2/cap 2, round 2 peak 1/cap 2 limited by dependencies.
- Analyst was gpt-6-astra, not opus; review roster same-family OpenAI, not cross-vendor independence.
- PR seat-agreement source absent because there were no required judge seats; agreement is not invented.
- Cached/latest metadata match, but local skill overrides have no independently established version.
- Ledger-derived counts are lower bounds because append can be fire-and-forget; the available cross-check reports `ledger-check ok`, append failures/retries zero.

## Verification bar

Exercise a nontrivial seeded artifact whose required scouts all complete with empty findings: assert chosen coverage/yield semantics and zero-seat judging representation. For artifact guidance, run two divergent Cargo worktrees with unique test inventories, alternate shared-output validation, capture source path/test identity, and compare private targets. Require source-correct evidence after any provenance uncertainty.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
