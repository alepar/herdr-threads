# handoff-profile-and-skill-guidance: review collection ordering and unavailable archives

Plugin: 6.4.2-alepar6.2. Target: alepar/superpowers. Run:11 implementation tasks,10 review bookkeeping children,1 caller-owned root;2026-10-17/18.

**LOCAL DRAFT — not filed; final-body approval and optional scrubbing remain user-owned.**

## Defects

### 1. Committed current review intent makes collection self-referential
- **Evidence:** The installed instruction says to commit the pending review-collection intent before launch. A corrected expected-path, settled:false current intent committed into captured HEAD still caused incomplete claimed collection. Only the documented commit-after-settlement ordering exception allowed a fresh complete assembly.
- **Premise to verify:** Verify the corrected current record schema and distinguish its incomplete state from independently incomplete historical collections.
- **Suggested fix shape:** Permit the current invocation’s committed pending intent while validating completed historical records normally.

### 2. Unavailable assemblies require a digest their producer never wrote
- **Evidence:** A prior assembly failed before writing digest.md; the next assembler rejected its archive for lacking that digest. The run had to add explicitly labelled reconstructed diagnostic digests from immutable selection metadata. Original failed outputs/verdicts were retained.
- **Premise to verify:** Verify this archive is a supported failed-selection state and determine the immutable metadata required to identify its unavailable outputs.
- **Suggested fix shape:** Represent failed assemblies explicitly so later independent assembly can preserve the failure without demanding nonexistent producer output.

### 3. Unknown slot metadata becomes a precise one-slot model
- **Evidence:** The ordinary ledger used a JSON-shaped Launch: args line with concurrency4. The profile parser reads textual Launch: mechanism metadata and uses ledger?.concurrency ?? 1. Output reported cap1 beside task/all-agent peak5, model179% of actual, and an estimated47-minute slot benefit.
- **Premise to verify:** The recorded JSON launch may violate the ordinary coordinator protocol; do not claim valid native format rejection. Verify fallback/reporting and the missing-metadata path.
- **Suggested fix shape:** Keep unknown cap metadata unknown, diagnose its absence, and suppress quantitative slot-bound conclusions requiring that value.

Sources: friction.md; review-intent-recovery.md; collected original assembly/collection archives; code-profile-original.md/json; installed run-profile.mjs719–721,966. Independent fresh-context analysis used gpt-6-astra (high); parent retained all3 candidates after precision triage.

## Run metrics
### Judge panel
- 2026-10-17-handoff-profile-and-skill-guidance-roast-pr-1.md: mode: pr        iteration: 1 of 3
- 2026-10-17-handoff-profile-and-skill-guidance-roast-pr-1.md: independence: same-family (GPT-6) — seat-differentiated panel · rung: manual fan-out
- 2026-10-17-handoff-profile-and-skill-guidance-roast-pr-1.md: seat-agreement: panels 1 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=1) · reproduce 0/1/0 · refute 0/1/0 · ground 0/1/0
- 2026-10-17-handoff-profile-and-skill-guidance-roast-pr-2.md: mode: pr        iteration: 2 of 3
- 2026-10-17-handoff-profile-and-skill-guidance-roast-pr-2.md: delta vs prior: 0 new confirmed (0 Blocking) · 0 carried (0 Blocking) · 0 resolved · 0 regressed (0 Blocking) · 0 punch-listed (open)
- 2026-10-17-handoff-profile-and-skill-guidance-roast-pr-2.md: independence: same-family (GPT-6) — seat-differentiated panel · rung: manual fan-out
- Round1:12/12scouts,2raw,1fullpanel+1spot,all4votesREJECT;clean1nit. Round2:13/13scouts,0raw,no new panels;clean0nits[converged]. Same-family seats and n=1 constrain agreement interpretation.

### Fix loop
- Metrics: completions — review clean 10 · after fix pass 1 · parked 0 · re-entry closes 0 · dispatched early 8 · cancelled 0
- Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0 (legacy event lines); durable attempts 2 · historical FIXED 2; repaired findings unavailable (use scope-dispositions --project)
- One review fix and one raw-evidence recovery are distinct. The separately discovered native transport defect is one task landing, not an extra review-fix pass.

### Merge-back
- Metrics: merges 11 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 0 (fixed 0) · check-fails 0 (fixed 0)
- Metrics: ledger-check ok · ledger-only [] · completed-only [] · git-only [] · ledger-only-git [] · append-failed unknown · append-retried unknown · scope: observed ht-1urq ledger and authoritative tracker reconciliation; Git filtered to 11 implementation task IDs, old ancestry excluded · gaps: seven historical raw logs unavailable; concurrent topology attribution unestablished; finding population unknown; append retry/failure totals unknown
- Latest task landings11, latest implementation closures11;ledger-only[],Git-only[],completed-only[],ledger-only-Git[]. No baseAbsorbed landing. An earlier docs stack conflict was safely aborted and retried; latest success counts do not erase that attempt.

### Coverage
- Coverage rounds1and2:requirements12,mapped12,unmapped0 each. Scopefilter0in-scope,0punch-listed;no confirmed roast keys.
- Retained source projection: {"scope": "retained sources", "sources": 5, "sourceKinds": ["final-review", "roast"], "occurrences": 112, "occurrenceGroups": 112, "findings": 109, "otherSectionGroups": 3, "sections": ["Must-fix", "Untested scope", "Must fix before landing", "Deferred OK", "Rejected (with reason)", "Unverified nits (spot-checked)"], "repaired": 102, "rejected": 0, "outstanding": 7, "deferred": 0, "gaps": 0}.
- The102 supported resolutions primarily close coverage/measurement obligations, not102 code defects. Original task-review finding population remains unknown; projection covers retained final-review+roast sources only. All10 remaining groups have supported nonblocking relevance; root separately retains human finish plus2failure-attribution gaps.

### Timing
- Final quiet candidate:7653passed,48skipped,216.078s nextest summary(216.369s command);clippy0.309s;other required gates/scoped cleanup pass. First unprepared suite7653pass428.731s included390.552s release proof;middle prepared sweep7652pass1failure387.486s amid peer release compilation. Historical failures and causal uncertainty remain recorded.
- The following profile is the original producer output. Its cap1 input is not established,8beads readiness unavailable,model179%ofactual; schedule what-ifs are estimates and cannot establish speedups.
- Profile: ht-1urq · 1 invocation · 2026-10-18T02:41Z → 2026-10-18T04:41Z · wall 2h00m (task graph 1h58m, after the last landing 2m) · agents 64 (timed 64) · beads dispatched 11, landed 11, planned 11
- Profile: graph scope — observed invocation/residual graph; historical topology is unknown
- Profile: shape — 11 beads · 24 edges · depth 7 · width 1.6 · beads per level 2/3/2/1/1/1/1 · widest fan-out ht-1urq.1 (5 direct, 9 transitive dependents)
- Profile: bound — graph lower bound 2h42m along ht-1urq.1 → ht-1urq.3 → ht-1urq.4 → ht-1urq.6 → ht-1urq.19 → ht-1urq.9 → ht-1urq.10 (unlimited slots and merge lanes) · model of the run 3h31m (cap 1, one merge lane) vs actual 1h58m (179%) → slot-bound: 47m of the 49m above the graph's bound is the slot cap
- Profile: critical path — 1h58m = implement 53m · review 7m · merge 11m · planning 4m · other 13m · dispatch 3m · wait 24m · unmeasured-gap 3m · of which other beads' merge-lane turns 5m
- Profile: diagnosis — planning 4m (3% of first dispatch to last landing)
- Profile: critical chain — design other 13m → plan planning 2m → ht-1urq.3 implement 0s → ht-1urq.5 implement+review+merge 18m → merge lane ×2 5m → ht-1urq.6 implement+review+fix 19m → plan planning 2m → ht-1urq.19 implement+review+merge 18m → ht-1urq.10 implement+review+merge 12m
- Profile: bottleneck — ht-1urq.6 — 19m on the critical path (16%) · implement 17m · review 2m · dispatch 44s · implement on the path 17m: test 14s · poll 2m · build 1s · git 14s · read 4s · edit 19s · shell 27s · other 1s · model 14m (longest: "scripts/check-harness-neutral" 9s, "tail -8 target/task-evidence/topology-handoff.log" 8s) · dependents 3
- Profile: waits — ready→start, summed per bead (beads wait at the same time, so not wall time): 0s over 0 of 11 landed beads
- Profile: rework — 1m of dispatch time in attempts that did not land — ht-1urq.8 1m (?; landed later)
- Profile: tests — implementers and fixers, summed: 5m running tests and 29m polling background runs, 20% of their 2h48m · 0 Bash call(s) ran into the 10-minute tool limit
- Profile: merge lane — busy 27m of 1h31m (30%) · seam reviews and fixes 0s (0% of busy) · 15 merge dispatches, first try not merged for 4 of 11 · queue wait 6m total, peak 2 waiting · stack-parent wait 0s
- Profile: concurrency — 64 dispatches over 2h00m · a dispatch running 1h56m (97%) · dispatches at once: 0 for 3% · 1 for 25% · 2 for 36% · 3+ for 36% of the time · peak 5
- Profile: dispatch time by kind, summed — impl 2h43m (16) · merge 27m (15) · review 17m (14) · plan 7m (4) · fix 5m (2) · not named for a bead: design 13m (2) · investigate 9m (1) · recover 7m (1) · collect 5m (3) · repair 2m (4) · finish 1m (1) · assemble 28s (1)
- Profile: runtime — peak 5 agents at once · slot cap 1
- Profile: concurrency scope — all-agent concurrency peak 5 · task-slot concurrency peak 5, cap 1 (ordinary chains retain a slot through merge)
- Profile: what-if estimate — unlimited slots → 2h44m (−47m, 22%) — no slot cap; first dispatch to last landing, planning retained
- Profile: what-if estimate — faster ht-1urq.9 → 3h14m (−17m, 8%) — its implement time halved; first dispatch to last landing, planning retained
- Profile: what-if estimate — faster ht-1urq.6 → 3h23m (−8m, 4%) — its implement time halved; first dispatch to last landing, planning retained
- Profile: what-if estimate — cut ht-1urq.9 <- ht-1urq.19 → 3h28m (−3m, 1%) — payoff if this edge were cut — whether it can be is a design call; first dispatch to last landing, planning retained
- Profile: what-if estimate — cut ht-1urq.10 <- ht-1urq.9 → 3h28m (−3m, 1%) — payoff if this edge were cut — whether it can be is a design call; first dispatch to last landing, planning retained
- Profile: what-if estimate — cut ht-1urq.19 <- ht-1urq.6 → 3h29m (−2m, 1%) — payoff if this edge were cut — whether it can be is a design call; first dispatch to last landing, planning retained
- Profile: context — agents 56 (measured 56) · peak≥80% 0 · compacted 0 agents, 0 compactions · coordinator 258K/475K (54%), 0 compactions · max coordinator 54%
- Profile: context agent — coordinator peak 258K/475K (54%) · first 24K · compactions 0
- Profile: unmeasured — availability/readiness timing unknown for 8 bead(s): waits unmeasured; schedule estimates retain dispatch as the release boundary; 13 dispatch(es) not named for a bead of ht-1urq, in the run-level lines only (coordinator-subagents.md names dispatches <kind>_<bead id> on Codex, <kind>:<bead id> on Claude Code); the schedule model is 179% of actual, so the what-ifs are rough; coordinator rows counted from the session start (no invocation found)

### Bead graph
| id | type | title | what (first sentence) |
| --- | --- | --- | --- |
| ht-1urq | epic | Handoff profile directories and collaboration skill guidance | Deliver the committed handoff launch-directory and collaboration-guidance design to merge-ready code. |
| ht-1urq.1 | task | Seam contract: frozen launch directory context | Land the inert shared directory-context type, defaulted optional retained fields and adapter policy seam so later leaves compile against one contract. |
| ht-1urq.2 | task | Parse and validate new-tab profile directory inputs | Expose new-tab --profile-dir and validate explicit profile/cwd paths before any recipient resolution, durable publication or native effects. |
| ht-1urq.3 | task | Map explicit Codex and Claude profile directories | Make native config-root adapters select explicit profile directories ahead of inherited or shell hints for all launch inspection and child inputs. |
| ht-1urq.4 | task | Map explicit Hermes roots and retain unsupported adapter behavior | Let Hermes explicit directory selection seed HERMES_HOME without changing named-profile resolution; retain agy unsupported explicit selection. |
| ht-1urq.5 | task | Validate and retain context-bearing bootstrap identities | Admit additive context-bearing identities canonically while preserving exact historical context-free validation and replay. |
| ht-1urq.6 | task | Apply frozen profile environment through new-tab handoff | Wire selected directories end to end through preflight, tab creation, derived launch and exact retry for automatic and explicit-thread new-tab modes. |
| ht-1urq.7 | task | Teach names, immediate delivery and receipt recovery in shipped skill | Update the shipped collaboration guide with name-first prose, timely p0 dispatch and useful responses to missing receipt confirmations. |
| ht-1urq.8 | task | Document launch directory support and proposed minor version | Align help, user documentation and changelog with supported new-tab directories and deliberate pane/agy limitations. |
| ht-1urq.9 | task | Configuration smoke: selected profile and cwd in release matrix | Exercise public handoff configuration and real child env/cwd with isolated stand-ins across the existing Herdr release matrix. |
| ht-1urq.10 | task | Integration sweep: handoff profile directories and skill guidance | Verify the goal end to end and fix small unwired directory/context/guidance integration gaps; file blockers for larger gaps. |
| ht-1urq.11 | task | review: ht-1urq.7 | Review, fix and merge of ht-1urq.7, tracked separately so ht-1urq.7's dependents can start once it is implemented. |
| ht-1urq.12 | task | review: ht-1urq.1 | Review, fix and merge of ht-1urq.1, tracked separately so ht-1urq.1's dependents can start once it is implemented. |
| ht-1urq.13 | task | review: ht-1urq.5 | Review, fix and merge of ht-1urq.5, tracked separately so ht-1urq.5's dependents can start once it is implemented. |
| ht-1urq.14 | task | review: ht-1urq.3 | Review, fix and merge of ht-1urq.3, tracked separately so ht-1urq.3's dependents can start once it is implemented. |
| ht-1urq.15 | task | review: ht-1urq.2 | Review, fix and merge of ht-1urq.2, tracked separately so ht-1urq.2's dependents can start once it is implemented. |
| ht-1urq.16 | task | review: ht-1urq.4 | Review, fix and merge of ht-1urq.4, tracked separately so ht-1urq.4's dependents can start once it is implemented. |
| ht-1urq.17 | task | review: ht-1urq.8 | Review, fix and merge of ht-1urq.8, tracked separately so ht-1urq.8's dependents can start once it is implemented. |
| ht-1urq.18 | task | review: ht-1urq.6 | Review, fix and merge of ht-1urq.6, tracked separately so ht-1urq.6's dependents can start once it is implemented. |
| ht-1urq.19 | bug | Forward validated profile context through native tab creation | Covers: executable smoke RED in ht-1urq.9: NativeCli::create_tab rejects validated projected profile environment before transport. |
| ht-1urq.20 | task | review: ht-1urq.19 | Review, fix and merge of ht-1urq.19, tracked separately so ht-1urq.19's dependents can start once it is implemented. |
| ht-1urq.21 | task | review: ht-1urq.9 | Review, fix and merge of ht-1urq.9, tracked separately so ht-1urq.9's dependents can start once it is implemented. |

| dependent | blocker | reason |
| --- | --- | --- |
| ht-1urq.21 | ht-1urq.9 | unstated |
| ht-1urq.20 | ht-1urq.19 | unstated |
| ht-1urq.19 | ht-1urq.6 | unstated |
| ht-1urq.18 | ht-1urq.6 | unstated |
| ht-1urq.17 | ht-1urq.8 | unstated |
| ht-1urq.16 | ht-1urq.4 | unstated |
| ht-1urq.15 | ht-1urq.2 | unstated |
| ht-1urq.14 | ht-1urq.3 | unstated |
| ht-1urq.13 | ht-1urq.5 | unstated |
| ht-1urq.12 | ht-1urq.1 | unstated |
| ht-1urq.11 | ht-1urq.7 | unstated |
| ht-1urq.10 | ht-1urq.4 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.9 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.8 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.7 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.2 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.6 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.5 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.1 | consumes all leaves (integration sweep) |
| ht-1urq.10 | ht-1urq.19 | unstated |
| ht-1urq.10 | ht-1urq.3 | consumes all leaves (integration sweep) |
| ht-1urq.9 | ht-1urq.19 | unstated |
| ht-1urq.9 | ht-1urq.6 | consumes end-to-end frozen new-tab launch wiring |
| ht-1urq.8 | ht-1urq.2 | consumes public handoff directory grammar |
| ht-1urq.8 | ht-1urq.7 | unstated |
| ht-1urq.6 | ht-1urq.3 | consumes native config-root profile projection |
| ht-1urq.6 | ht-1urq.5 | consumes canonical launch-context admission |
| ht-1urq.6 | ht-1urq.4 | consumes Hermes profile projection |
| ht-1urq.6 | ht-1urq.2 | consumes normalized public directory inputs |
| ht-1urq.5 | ht-1urq.1 | consumes boundary contract |
| ht-1urq.4 | ht-1urq.3 | unstated |
| ht-1urq.4 | ht-1urq.1 | consumes boundary contract |
| ht-1urq.3 | ht-1urq.1 | consumes boundary contract |
| ht-1urq.2 | ht-1urq.1 | consumes boundary contract |

## Design questions
none

## Doc gaps
none

## Already fixed — do not re-litigate
none in the installed skill plugin; local orchestration workarounds did not patch it.

## Not established
- Analyst model was gpt-6-astra(high), not opus; fresh context, tools disabled.
- Whether JSON Launch formatting complied with the ordinary coordinator protocol; cap parsing is not proof of a valid-format rejection or an actual task-cap breach.
- A measured47-minute benefit, unnecessary serialization, or reliable schedule estimates from a179% model fit.
- Original7rawlogs and intermediate fixture failure; six exact-source NEW checks cannot recreate those bytes.
- Original task-review finding population;historical topology;8beads readiness;append-failed/retried totals.
- Actual interleavings or causal attribution of the historical concurrent topology failure and middle apply-lane failure; isolated/quiet passes do not exonerate source or prove load causation.
- Ledger append can lose lines; raw history totals are lower bounds unless specifically reconciled. The11latest landings have explicit ledger/Git/tracker reconciliation; this does not establish append retry/failure history.
- Parent errors are distinct:originalnullreviewpath,rawlogcopyloss,JSONlaunchformatuncertainty,projectCIprebuildomission. Humanpause and supported ordinary-execution selection add zero defect evidence.
- Stand-in release matrix proves declared child env/cwd, not authentication/model activation; this is product evidence scope, not a plugin defect.

## Verification bar
- Fixture a correctly committed current pending intent plus completed prior history; fresh assembly should succeed while an unrelated incomplete historical collection still fails honestly.
- Fixture a producer failure before digest output; preserve original absent output/failure, then run a fresh assembly and collection/replay without manual invented producer files.
- Exercise knowncap1,knowncap4,missingcap and malformed/native Launch formats. Missing metadata must not silently establish cap1 or numeric slot-bound claims; preserve original raw profile and evidence qualifications.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
