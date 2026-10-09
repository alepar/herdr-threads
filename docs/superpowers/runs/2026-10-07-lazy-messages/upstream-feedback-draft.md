# lazy-messages: late-created tasks distort profiling and blocker-count termination hides progress

Plugin: 6.4.2-alepar4.20 (alepar/superpowers). Run: 14 completed leaf tasks, depth8, 2026-10-07/08. Draft only; not filed.

## Defects

### 1. Profiling counts waiting before a task existed

- **Evidence:** Profile-code-fix-r1 reports ht-big.11 as the bottleneck with 6h07m wait and 84% of the critical path. Its bead was created at12:17:20.741Z and dispatched at12:17:21Z; the profile starts at05:01Z. The reported prior wait predates creation. Raw profile and friction annotation are retained.
- **Premise to verify:** The profiler begins late-created task waiting at initial run launch rather than creation/readiness; verify using bead timestamps and dispatch history.
- **Suggested fix shape:** Bound waits by creation, use actual readiness where measured, and distinguish unknown intervals from scheduler waiting.

### 2. Profile concurrency compares all roles with a task-chain cap

- **Evidence:** Updated profile says runtime peak10 beside slot cap1, includes34 roast dispatches, while Detector round2 records a single active task chain with peak1. Roast fan-out and coordinator task execution use different scopes.
- **Premise to verify:** Profile peak includes roast roles but the displayed cap applies only to task-chain execution.
- **Suggested fix shape:** Report role-scoped concurrency/caps and separately label overall agent concurrency.

## Run metrics

### Judge panel


2026-10-07-lazy-messages-roast-design-1.md

mode: design        iteration: 1 of 3
independence: same-family (GPT) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 6 · rr 1.00 · rg 0.50 · fg 0.50 · unanimous 0.50 · ground-loo 0.50 (n=6) · reproduce 0/6/0 · refute 0/6/0 · ground 3/3/0

2026-10-07-lazy-messages-roast-pr-1.md

mode: pr        iteration: 1 of 3
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 6 · rr 0.50 · rg 0.67 · fg 0.83 · unanimous 0.50 · ground-loo 1.00 (n=3) · reproduce 3/3/0 · refute 6/0/0 · ground 5/1/0

2026-10-07-lazy-messages-roast-pr-2.md

mode: pr        iteration: 2 of 3
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 3 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=3) · reproduce 3/0/0 · refute 3/0/0 · ground 3/0/0

### Fix loop

Metrics: completions — 12 review clean · 2 after fix · 0 parked · 0 re-entry closes · 0 early dispatches (0 cancelled)
Metrics: fix passes — 2 entered · 2 FIXED · 0 BLOCKED

### Merge-back

Metrics: merge-back — 14 merges · 0 merge-failed · 0 rebase-conflicts · 0 seam-reviews (0 fixed) · 0 check-failures (0 fixed)
Metrics: ledger-check ok — append-failed 0 · append-retried 0; 14 success Merge lines and 14 completed checkpoint task second parents verified in actual first-parent git history.

14 success Merge lines; 15 first-parent merge commits, including one explicitly excluded tree-equivalent actual-main ancestry reconciliation6b57287a. All14 task second parents independently verified; no merge shortfall. No conflicts, seam reviews, check failures or blocker outcomes recorded.

### Coverage

Round1 requirements13/mapped13/unmapped0; round2 requirements13/mapped13/unmapped0. Fix round1 scope2in-scope/3punch-listed.

### Timing

All what-ifs below are schedule-model estimates, not measured savings. The updated late-bead wait and role/cap discrepancy are defective interpretations; preserve raw numbers for diagnosis.

```text
Profile: ht-big · 1 invocation · 2026-10-08T05:01Z → 2026-10-08T11:49Z · wall 6h49m (task graph 6h49m) · agents 36 (timed 36) · beads dispatched 13, landed 13, planned 13
Profile: shape — 13 beads · 30 edges · depth 8 · width 1.6 · beads per level 2/2/2/2/2/1/1/1 · widest fan-out ht-big.1 (5 direct, 11 transitive dependents)
Profile: bound — graph lower bound 4h32m along ht-big.1 → ht-big.2.1 → ht-big.2.2 → ht-big.3 → ht-big.5.1 → ht-big.5.2 → ht-big.7 → ht-big.10 (unlimited slots and merge lanes) · model of the run 5h29m (cap 1, one merge lane) vs actual 6h49m (80%) → slot-bound: 54m of the 57m above the graph's bound is the slot cap
Profile: critical path — 6h49m = implement 2h59m · review 18m · fix 11m · merge 19m · planning 5m · other 26m · dispatch 5m · slot 53s · wait 2h24m
Profile: critical chain — lazy other 26m → plan planning 5m → ht-big.9 implement+review+fix+merge 14m → ht-big.2.1 implement+review+merge 21m → ht-big.2.2 implement+review+merge 17m → ht-big.3 implement+review+fix+merge 43m → ht-big.5.1 implement+review+merge 25m → ht-big.5.2 implement+review+merge 31m → ht-big.6 implement+review+merge 16m → ht-big.7 implement+merge+review 34m → ht-big.10 implement+merge+review 26m
Profile: bottleneck — ht-big.3 — 59m on the critical path (15%) · implement 31m · review 2m · fix 8m · merge 2m · wait 16m · implement on the path 31m: test 34s · poll 36s · git 13s · read 3s · shell 5s · other 2s · model 30m (longest: "scripts/check-default-features > .tmp/ht-big.3/default-feat…" 12s, "git add src/protocol/capabilities.rs src/protocol/commands.…" 6s) · dependents 5 direct, 5 transitive
Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 5h52m over 12 of 13 landed beads — slot 2h23m (3) · retry 1h14m (3) · unexplained 2h15m (6)
Profile: rework — 27m of dispatch time in attempts that did not land — ht-big.2.1 17m (?; landed later) · ht-big.9 6m (?; landed later) · ht-big.2.2 4m (?; landed later)
Profile: tests — implementers and fixers, summed: 6m running tests and 8m polling background runs, 6% of their 4h04m · 0 Bash call(s) ran into the 10-minute tool limit
Profile: redo — on the critical path, up to retries 34m (ht-big.9, ht-big.2.1, ht-big.2.2) — from a bead's first attempt to its final one; the what-ifs model the real saving
Profile: merge lane — busy 27m of 5h18m (9%) · seam reviews and fixes 0s (0% of busy) · 15 merge dispatches, first try not merged for 2 of 13 · queue wait 10m total, peak 1 waiting · stack-parent wait 0s
Profile: concurrency — 36 dispatches over 6h47m · a dispatch running 4h59m (73%) · dispatches at once: 0 for 27% · 1 for 72% · 2 for 2% of the time · peak 2
Profile: dispatch time by kind, summed — impl 3h53m (16) · review 25m (13) · fix 11m (2) · plan 10m (2) · not named for a bead: lazy 26m (2) · dev 2m (1) · merges run by the coordinator 27m (15)
Profile: runtime — peak 2 agents at once · slot cap 1
Profile: what-if — unlimited slots → 4h35m (−54m, 16%) — no slot cap
Profile: what-if — faster ht-big.3 → 5h13m (−16m, 5%) — its implement time halved
Profile: what-if — faster ht-big.7 → 5h13m (−16m, 5%) — its implement time halved
Profile: unmeasured — 3 dispatch(es) not named for a bead of ht-big, in the run-level lines only (coordinator-subagents.md names dispatches <kind>_<bead id> on Codex, <kind>:<bead id> on Claude Code)
Profile: ht-big · 1 invocation · 2026-10-08T05:01Z → 2026-10-08T12:55Z · wall 7h54m (task graph 7h54m) · agents 76 (timed 76) · beads dispatched 14, landed 14, planned 14
Profile: shape — 14 beads · 30 edges · depth 8 · width 1.8 · beads per level 3/2/2/2/2/1/1/1 · widest fan-out ht-big.1 (5 direct, 11 transitive dependents)
Profile: bound — graph lower bound 4h32m along ht-big.1 → ht-big.2.1 → ht-big.2.2 → ht-big.3 → ht-big.5.1 → ht-big.5.2 → ht-big.7 → ht-big.10 (unlimited slots and merge lanes) · model of the run 5h59m (cap 1, one merge lane) vs actual 7h54m (76%) → slot-bound: 1h23m of the 1h27m above the graph's bound is the slot cap
Profile: critical path — 7h54m = implement 26m · review 2m · merge 3m · planning 5m · other 26m · wait 6h52m
Profile: critical chain — lazy other 26m → plan planning 5m → ht-big.11 implement+review+merge 31m
Profile: bottleneck — ht-big.11 — 6h38m on the critical path (84%) · implement 26m · review 2m · merge 3m · wait 6h07m · implement on the path 26m: test 43s · poll 6s · build 1s · git 9s · read 2s · shell 5s · other 1s · model 25m (longest: "git add src/cli/mod.rs src/cli/output.rs src/store/queries.…" 7s, "python3 - <<'PY'" 6s) · dependents 0
Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 11h58m over 13 of 14 landed beads — slot 2h23m (3) · retry 1h14m (3) · unexplained 8h21m (7)
Profile: rework — 27m of dispatch time in attempts that did not land — ht-big.2.1 17m (?; landed later) · ht-big.9 6m (?; landed later) · ht-big.2.2 4m (?; landed later)
Profile: tests — implementers and fixers, summed: 6m running tests and 8m polling background runs, 5% of their 4h30m · 0 Bash call(s) ran into the 10-minute tool limit
Profile: merge lane — busy 30m of 6h23m (8%) · seam reviews and fixes 0s (0% of busy) · 16 merge dispatches, first try not merged for 2 of 14 · queue wait 11m total, peak 1 waiting · stack-parent wait 0s
Profile: concurrency — 76 dispatches over 7h51m · a dispatch running 5h46m (73%) · dispatches at once: 0 for 27% · 1 for 71% · 2 for 2% · 3+ for 1% of the time · peak 10
Profile: dispatch time by kind, summed — impl 4h19m (17) · review 27m (14) · plan 14m (3) · fix 11m (2) · final-review 4m (1) · not named for a bead: roast 41m (34) · lazy 26m (2) · dev 2m (1) · stepback 40s (1) · scope 23s (1) · merges run by the coordinator 30m (16)
Profile: runtime — peak 10 agents at once · slot cap 1
Profile: what-if — unlimited slots → 4h36m (−1h23m, 23%) — no slot cap
Profile: what-if — faster ht-big.3 → 5h44m (−16m, 4%) — its implement time halved
Profile: what-if — faster ht-big.7 → 5h44m (−16m, 4%) — its implement time halved
Profile: what-if — cut ht-big.2.1 <- ht-big.9 → 5h53m (−6m, 2%) — payoff if this edge were cut — whether it can be is a design call
Profile: what-if — cut ht-big.10 <- ht-big.7 → 5h56m (−4m, 1%) — payoff if this edge were cut — whether it can be is a design call
Profile: unmeasured — 39 dispatch(es) not named for a bead of ht-big, in the run-level lines only (coordinator-subagents.md names dispatches <kind>_<bead id> on Codex, <kind>:<bead id> on Claude Code)
```

### Bead graph

| id | type | title | what |
| --- | --- | --- | --- |
| ht-big.11 | bug | Preserve invocation context through every inbox continuation | Covers: [Blocking] src/store/queries.rs:5959; [Should-fix] src/cli/mod.rs:2057; src/store/queries.rs:5959 |
| ht-big.10 | task | Integration sweep: passive lazy mail through complete explicit inbox | Verify lazy-send to explicit-inbox main flows end to end with focused tests, fix small wiring gaps and record final merge-readiness evidence. |
| ht-big.9 | task | Integrate frozen warning migration25 prerequisite on isolated branch | Integrate exact warning prerequisite8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab after independent actual-main97fb seam review, to provide real migration25 before lazy26. |
| ht-big.5.2 | task | Independent frozen ACK and lazy completion intent recovery | Persist both frozen intents before either submission; submit independently despite peer failures; retain exact retry refs on partial journal failure or lost reply and clear successful set only; lazy IDs never ACK. Preserve saved SemanticMutation+CallerClaim harness+IntentScope before replay/complete |
| ht-big.5.1 | task | V2 explicit inbox output and contiguous display journal | Render bounded v2 lazy chunks and continuation pages; produce durable occupant-bound fully-displayed candidates only after full selected page write/flush, contiguous offset zero through length. JSON/machine/explicit-seat remain read-only; skips/partial writes/flush failure/cancel/binding change leav |
| ht-big.4.2 | task | Read-only lazy markers on history body search pages | Annotate selected history/body/search pages using ≤100 exact-ID canonical mode queries; preserve old-daemon ordinary read-only fallback and complete-content summary inclusion with no renderer/chunker/cache/lease or model-work changes. |
| ht-big.4.1 | task | Lazy send CLI validation and compatibility refusal | Expose --lazy on native send with CLI validation and unsupported-daemon refusal before intent; preserve ordinary omission/digest and frozen actor classification, keep service-owner/compound handoffs ordinary. |
| ht-big.2.2 | task | Bounded lazy preparation publication and cleanup | Reject incompatible lazy ACK/deadline options canonically, then stage frozen audiences and publish manifest-visible deliveries without attention. |
| ht-big.2.1 | task | Audited lazy-delivery schema and store accessors | Introduce the additive mode/recipient schema, immutable identity triggers, pending indexes and bounded accessors with canonical trust bookkeeping documentation. |
| ht-big.8 | task | Canonical message delivery-mode metadata query | Implement bounded MessageDeliveryModes query and handler advertisement without changing v1 message shapes or summaries. |
| ht-big.7 | task | Configuration smoke: text JSON machine explicit-seat and legacy compatibility | Exercise each supported inbox mode and compatibility configuration early as runnable focused integration tests. |
| ht-big.6 | task | Prove lazy mail never creates attention across lifecycle | Prove no attention and addressed delivery retention across restart, postjoin, leaving, retirement and archival with ordinary positive controls. |
| ht-big.5 | epic | Journal and settle complete default text inbox delivery | Implement explicit CLI v2 inbox with durable contiguous display journal and independent frozen ACK/completion intent recovery. |
| ht-big.4 | epic | Expose lazy send and read-only metadata markers | Implement send --lazy validation and capability refusal plus canonical lazy markers on selected history/body/search pages while keeping summaries stable. |
| ht-big.3 | task | Implement daemon v2 inbox and delivery completion | Serve bounded separate lazy-source v2 inbox chunks and canonical exact-ID idempotent completion without ACK evidence. |
| ht-big.2 | epic | Persist and publish bounded lazy recipient deliveries | Implement additive audited DDL and bounded lazy recipient staging, publication and preparation cleanup with no receipts/warnings/send_attention. |
| ht-big.1 | task | Seam contract: lazy delivery types and compatibility dispatch | Expose compilable inert-by-default lazy mode, v2 batch, exact-ID mode metadata and CompleteInboxDelivery boundary types and route declarations while retaining ordinary digest/v1 compatibility. |
| ht-big | epic | Lazy messages: explicit discovery without attention | Implement the approved lazy delivery design. Goal: important nonurgent announcements reach frozen existing participants only at natural explicit inbox, with no attention or ACK obligation. Spec: docs/superpowers/specs/2026-10-07-lazy-messages-design.md. This root owns only merge-ready feature work,  |

| dependent | blocker | reason |
| --- | --- | --- |
| ht-big.10 | ht-big.4.2 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.8 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.3 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.1 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.7 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.4.1 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.2.2 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.9 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.5.2 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.5.1 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.2.1 | consumes all leaves (integration sweep) |
| ht-big.10 | ht-big.6 | consumes all leaves (integration sweep) |
| ht-big.5.2 | ht-big.3 | consumes canonical completion handler |
| ht-big.5.2 | ht-big.5.1 | consumes fully displayed candidate journal interface |
| ht-big.5.1 | ht-big.3 | consumes canonical v2 batch handler |
| ht-big.4.2 | ht-big.8 | consumes canonical message-mode handler |
| ht-big.4.1 | ht-big.1 | consumes boundary contract |
| ht-big.2.2 | ht-big.2.1 | consumes persisted lazy recipient schema/accessors |
| ht-big.2.1 | ht-big.9 | consumes real warning migration25 |
| ht-big.2.1 | ht-big.1 | consumes boundary contract |
| ht-big.8 | ht-big.1 | consumes boundary contract |
| ht-big.8 | ht-big.2.1 | consumes stored delivery-mode accessors |
| ht-big.7 | ht-big.4.1 | consumes lazy send CLI |
| ht-big.7 | ht-big.5.2 | consumes frozen text settlement/retry workflow |
| ht-big.7 | ht-big.4.2 | consumes canonical read markers |
| ht-big.7 | ht-big.3 | consumes canonical v2 batch and completion handlers |
| ht-big.6 | ht-big.2.2 | consumes published lazy recipient lifecycle |
| ht-big.6 | ht-big.3 | consumes canonical v2 batch and completion handlers |
| ht-big.5 | ht-big.3 | consumes canonical v2 batch and completion handlers |
| ht-big.5 | ht-big.1 | consumes boundary contract |
| ht-big.4 | ht-big.1 | consumes boundary contract |
| ht-big.3 | ht-big.1 | consumes boundary contract |
| ht-big.3 | ht-big.2.2 | consumes published recipient preparation/publication |
| ht-big.2 | ht-big.1 | consumes boundary contract |

## Design questions

### Blocker identity versus count in bounded-loop termination

Round1 had one Blocking continuation defect; its fix and Human continuation were resolved. Round2 found a different Blocking recovery-hook defect, with zero carried/regressed Blocking. The literal nonshrinking-count rule stops the loop at1→1; fresh step-back found independent causes and that the previous fix held. Counting only quantity gives a predictable cost bound and prevents indefinite discovery/fix cycles. Tracking identities would distinguish failed fixes from useful progress, but could permit more iterations; the round cap must remain independently enforced. Should equal counts with all prior Blocking resolved be classified as thrash, or continue within the existing round cap? If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

none retained — cleanup argument observation lacks a reproduced upstream contract diagnosis.

## Already fixed — do not re-litigate

No upstream fix established. This run annotated the defective timing interpretation without changing profiler source. Its initial rough cold-build estimate was corrected to actual34.87s Cargo time; do not report a two-minute incremental-build regression.

## Not established

- Independent analyst ran the inherited frontier OpenAI model, not opus; exact model ID was not exposed to this analysis.
- No measured speedup, historical idle duration/ready-queue peak, or Workflow throughput claim. The profile's what-ifs remain estimates.
- Same-family OpenAI roast panels do not establish cross-family independence.
- Full-suite performance was deliberately unmeasured in this worker: direct user scope delegates it to main.
- Historical design report may lack the standardized panel-agreement header; no metric is fabricated.
- Default append-path caveat: ledger-derived counts are lower bounds absent independent checks. This run's ledger-check verifies all14 task merges/second parents and records zero append failures/retries.

## Verification bar

Reproduce profiling with a bead created hours after initial launch, and with a one-chain coordinator plus independent roast fan-out. Verify no pre-creation wait is attributed and scope caps are comparable. Probe the loop with round1 A=Blocking, round2 A=resolved/B=new Blocking, and a separate genuinely carried blocker. Preserve independent hard round caps and honest termination labels.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
