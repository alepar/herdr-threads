# automatic-one-to-one-threads: make review transport and final verification terminate predictably

Plugin: 6.4.2-alepar4.20. Target repository: alepar/superpowers. Run: 16 beads (3 epics,13 leaves), five code work rounds, two design roasts and two PR roasts,2026-10-10. Local draft only; no issue filed. One fresh gpt-6-astra analyst called no tools after report.md was committed. The controller retained two defects/doc gaps and reframed two policy conflicts as design questions.

## Defects

### 1. Installed SDD helpers fail under documented direct invocation

- **Evidence:** Friction records “shipped SDD helpers lack executable bits; direct invocation failed permission denied.” The workaround was `bash task-brief/review-package` with an explicit absolute OUTFILE. Controller read-only verification finds installed `subagent-driven-development/scripts/{task-brief,review-package,sdd-workspace}` all `-rw-r--r--`; both first helpers directly execute their sibling `sdd-workspace` when deriving output paths.
- **Premise to verify:** Check repository modes, installation preservation and documented invocation forms to locate the packaging/guidance boundary. This run establishes installed permissions, not which distribution step changed them.
- **Suggested fix shape:** Preserve executable modes or invoke bash consistently, including nested helper calls; test the installed path, with and without explicit OUTFILE.

## Run metrics

### Judge panel

- mode: design        iteration: 1 of 3 · independence: same-family (OpenAI) — seat-differentiated panel
  seat-agreement: panels 1 · rr 0.00 · rg 0.00 · fg 1.00 · unanimous 0.00 · ground-loo n/a (n=0) · reproduce 0/1/0 · refute 1/0/0 · ground 1/0/0
- mode: design        iteration: 2 of 3 · independence: same-family (OpenAI) — seat-differentiated panel (no judge panels applicable this round)
  seat-agreement: none (zero applicable panels)
- mode: PR        iteration: 1 of 3 · independence: same-family (OpenAI) — seat-differentiated panel
  seat-agreement: panels 3 · rr 0.67 · rg 0.67 · fg 1.00 · unanimous 0.67 · ground-loo 1.00 (n=2) · reproduce 0/3/0 · refute 1/2/0 · ground 1/2/0
- mode: PR        iteration: 2 of 3 · independence: same-family (OpenAI) — fresh scouts; no judge panel dispatched this round (zero new candidates)
  seat-agreement: none (zero applicable panels)

### Fix loop

Metrics: fix-loop round 1: 6 addressed / 5 entered · round 2: 1 addressed / 1 entered · round 3: 0 addressed / 0 entered · round 4: 0 addressed / 0 entered · round 5: 0 addressed / 0 entered
Metrics: fix-loop breaker-tripped: 0

### Merge-back

Metrics: merges 12 · merge-failed 0 · rebase-conflicts 2 · seam-reviews 3 (fixed 1) · gate-fails 0

Metrics: ledger-check M≠completed: 12 vs 13 · append-failed 0 · append-retried 0

Twelve literal canonical Merge rows: zero failed/`→ blocker`, two conflict records, three seam reviews (one fixed), zero serial merge-gate failures. The genuine first root full-suite failure is a separate gate class. `12 vs13` reflects design-only ht-e99.6, not a missing merge. Incoming-main merges inside a task candidate are not extra canonical implementation merges.

### Coverage

- Coverage round1: requirements22 · mapped21 · unmapped1(R22); initially adopted R22 as all-leaf verification acceptance.
- Coverage round2: requirements22 · mapped21 · unmapped1(R22); explicit terminal ht-e99.5 owns default-feature/leak/budget gate. Six→two candidates, novel1/2; no third coverage round. The literal unmapped count remains historical, rather than silently rewritten after disposition.
- Scope-filter round1: `scope-filter: 0 in-scope · 1 punch-listed`.
- Scope-filter round2: `scope-filter: 0 in-scope · 1 punch-listed`.
- Both filters retain `[Nit] src/store/archival.rs:222`: optimizing retained-parent archival scans is unstated scope, with no measured budget violation. Same finding carried, not two distinct Nits.

### Bead graph

| id | type | title | what (first sentence) |
| --- | --- | --- | --- |
| ht-e99.7 | bug | Reconcile completed automatic child through public historical retry after binding change | Required whole-epic review repair I1 at clean integration cd54f56310ddfc3f92c074713fc6187ba61e60ba. |
| ht-e99.10 | task | Refresh topology snapshot fixture staging from current main | Required current-main test-fixture refresh after reviewed Task 11 integration. |
| ht-e99.9 | task | Refresh terminal integration candidate with current main v0.6.4 Claude mod release | Required terminal current-main refresh for automatic one-to-one threads after reviewed Task 10 repair. |
| ht-e99.8 | task | Repair current-schema channel activation fixture and verify release proof preparation | Required post-review verification repair at integration base 83ea4ee3f90c3157012c55f231c96863c29f01da. |
| ht-e99.6 | task | Design repair: automatic handoff cannot reinvite after Ensure | DESIGN-ONLY roast1 repair, not a code implementation leaf. |
| ht-e99.5 | task | Integration sweep: automatic one-to-one threads | Final integration sweep of automatic target-seat send and all four threadless handoff destinations. |
| ht-e99.3.3 | task | New-tab automatic handoff producer | Wire threadless new-tab into shared automatic downstream plan after guarded recipient attachment, with unchanged topology/possible-start recovery and absorbing completion. |
| ht-e99.3.2 | task | Existing-seat and pane automatic handoff producer | Wire omitted channel in parser and existing-recipient/pane/native launch plans through shared automatic helper; freeze original target/caller/namespace/ensure keys and preserve override metadata rules. |
| ht-e99.3.1 | task | Automatic handoff plan and canonical attachment contract | Implement new-only automatic variant/version and strict compatibility validation, shared client helper signatures, registered Ensure child keys, exact canonical namespace/thread attachment and restricted phase validation. |
| ht-e99.1.2 | task | Atomic pair resolver and permanent audience guards | Implement accountable EnsureDirectThread atomic creation/reuse/replay and create/invite projections, canonical recipient checks, pair-only ordinary/service/operator enrollment and raw-send preparation/publication checks; activate capability only with full mechanism. |
| ht-e99.1.1 | task | Pair schema and compilable resolver protocol contract | Own the next additive pair migration, sorted unique pair/index/FKs, EnsureDirectThread command/result/permit/hash/capability, inert dispatch adapter and old-wire compatibility fixtures. |
| ht-e99.4 | task | Public guidance for automatic direct channels | Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. |
| ht-e99.3 | epic | Automatic channel handoff composition | Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. |
| ht-e99.2 | task | Durable target-seat-only send CLI | Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. |
| ht-e99.1 | epic | Canonical pair resolver and immutable membership boundary | Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. |
| ht-e99 | epic | Automatic one-to-one threads | Implement automatic target-seat-only send and threadless handoff. |

| dependent | blocker | reason |
| --- | --- | --- |
| ht-e99.5 | ht-e99.3.1 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.4 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.1.1 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.3.2 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.2 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.3.3 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.1.2 | consumes all leaves (integration sweep) |
| ht-e99.5 | ht-e99.6 | consumes all leaves (integration sweep) |
| ht-e99.3.3 | ht-e99.3.1 | consumes automatic handoff shared plan and canonical attachment contract |
| ht-e99.3.2 | ht-e99.3.1 | consumes automatic handoff shared plan and canonical attachment contract |
| ht-e99.3.1 | ht-e99.1.2 | consumes EnsureDirectThread protocol and canonical pair invariant |
| ht-e99.1.2 | ht-e99.1.1 | consumes pair schema and EnsureDirectThread interface |
| ht-e99.2 | ht-e99.1.2 | consumes EnsureDirectThread protocol and canonical pair invariant |

Dump obtained with `bd list --label sp:ht-e99 --json --status all --limit 0`; all16 nodes closed,13 leaves,15 parent edges,13 blocking edges. Remaining depth/terminal achievable width0, no division by zero. Historical longest blocking chain5; dependency-level widths10/1/2/2/1 include epics/design-only and later-created work. They are not actual rounds, concurrency or maximum antichain.

Detector round1: initialready2,topups6,totaldispatch8,cap3,peak2; query numerator/denominator and hot-file counters not persisted. Round2: one known repair dispatch,cap3,peak1; initial-ready and counters unavailable. Rounds3/4/5: ready1,topups0,cap3,peak1,logicalReadyphases1/40,deferrals0. Fresh report-only audits1/3 and2/3 at drains saw14/16 fully closed nodes respectively, no suspect edges, and reset the underfilled streak. Later review, verification and moving-main leaves arose sequentially; these terminal snapshots do not establish avoidable historical serialization.

## Design questions

### 1. Should complete empty late rounds force a degraded verdict?

**Evidence:** The log states “Late-round scout framing says no material findings is expected, but reporter literal zero-raw/nontrivial rule mandates [low coverage] and forbids [converged].” Design2 had9/9 valid scouts, zero candidates and prior Blocking resolved. PR2 had12/12 valid scouts, zero new candidates, one unchanged carried Nit, no stage loss or applicable judge seats. Both returned literal low coverage and no convergence. Autonomous proceed was parked honestly in run.md/report.md.

A completed empty review is consistent with expected late-round behavior, so distinguishing execution coverage from candidate yield could avoid needless degraded gates or pressure to manufacture marginal findings. Conversely, zero candidates on a substantial artifact provides no verification-panel evidence; conservatively retaining a qualifier may be deliberate. **Premise:** verify the installed scout/reporter/convergence rules together and decide what coverage means. **Suggested fix shape:** make that decision explicit while retaining zero raw/panel counts and unchanged carries. If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### 2. What is the terminating exact-tip verification protocol?

**Evidence:** “phase6 requires full-suite verification before report/upstream, while phase7 requires subsequent committed design-index/run-state/report edits. Those commits invalidate a literal exact-tip stamp despite source equivalence.” Exact979e4993 passed4,621 tests,44 skipped,wall194.052s with scoped cleanup after separate release/debug preparation. Report/upstream/design-record/finish commits follow; a second final exact-tip gate is planned with ignored evidence and no later commits. Its result was outstanding at analyst time.

Sealing all required commits before a final gate gives simple exact-SHA evidence but requires ordering the report/finish artifacts around a not-yet-known outcome, or repeating an expensive suite. Allowing narrowly verified source-equivalent documentation commits avoids that repetition but requires a precise equivalence rule; embedded product docs cannot be broadly excluded. **Premise:** verify phase ordering and whether literal commit identity is required. **Suggested fix shape:** document one finite protocol and the location of final evidence that does not invalidate its own stamp. If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

### 1. Review packets need loss-detecting native transport

- **Evidence:** “fork-all descendants lacked parent exec outputs”; “explicit inner/outer30000 still produced middle truncation of oversized packets”; “JSON-wrapped command outputs amplified packet size and hit hard truncation despite high token caps.” All18 coverage reviews completed only after explicit immutable acquisition and bounded range/suffix/tree repairs. Capacity7+2 was disclosed; no exploratory tracker/repository reads or spec drift occurred.
- **Premise to verify:** Check whether the native fallback already documents these transport limitations or offers an equivalent complete-input protocol. The analyst received selected diagnostic extracts, not every installed guidance file.
- **Suggested fix shape:** immutable file-backed packets or numbered bounded sections with completeness markers before review; do not rely on inherited tool outputs or increased token caps alone.

## Already fixed — do not re-litigate

No upstream fix/version was established during this run. The following are local workarounds or repaired project defects, not claims that the plugin was fixed: explicit packet acquisition, bash/OUTFILE helper invocation, root-owned tracked checkpoints and safe temp staging, owned isolated Cargo targets, registered nonzero test selections, reporter/key schema corrections, real lifecycle fixtures/public dispatch tests, scoped historical-retry repair, current-schema fixture correction and reviewed main imports. Do not refile those project bugs as skill defects.

The controller omitted CI-documented release preparation on the first full run; the failed325.384s result remains preserved. Separately preparing both configurations and correcting the fixture produced the successful194.052s gate. This acknowledged execution omission does not establish a missing skill rule. Reserved-zsh-variable mistakes and wrong test selectors were also execution drift, not proven upstream defects.

## Not established

- Analyst: fresh gpt-6-astra, no tools; input was a self-contained diagnostic selection and graph summary, not a fresh inspection of all skill source. Controller performed the installed helper-mode read afterward.
- No cross-family independence claim. OpenAI scouts/judges were fresh and method-differentiated; their agreement is panel agreement.
- Workflow-tool parallelism logs unavailable because native manual fanout was used. Round1/2 query and hot-file counters were not persisted; later exact counters cannot reconstruct them.
- Historical dependency levels/closed audits do not prove speed loss or concurrency availability. No speedup measurement, causal external-load explanation or shared-cache overwrite finding is asserted.
- Ledger-derived metrics are lower bounds under the default fire-and-forget append limitation. Latest recorded append-failed/retried0 and intentional12-vs13 ledger check are the available cross-check; no actual lost append was observed.
- No causal packaging step for executable modes, no complete existing-guidance audit for packet transport, and no upstream fixed version established.
- Final exact-tip second-gate result was unavailable at analysis/draft time; the successful194.052s measurement describes979e4993 only. Preparation costs109.691/86.460/19.931s are separate, not hidden in suite timing or incremental-source benchmarks.
- Project producer fixtures use controlled native/protocol seams; modeled producer and real canonical tests are not one external-socket/native end-to-end run. No adversarial-caller hardening is proposed for the cooperative same-user model.

## Verification bar

Test the helpers as installed, including nested default-output invocation. Exercise native handoffs where inherited tool outputs are absent and packets exceed outer output caps; require complete bounded input without changing the review artifact. Add reporter/convergence contract probes for valid empty initial versus late rounds, carried findings and actual stage loss; preserve explicit policy qualifiers. Walk report→feedback→design-index→finish with a literal exact-SHA full gate and show the protocol terminates with evidence matching final HEAD. Keep append/counter unknowns and historical failures visible.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
