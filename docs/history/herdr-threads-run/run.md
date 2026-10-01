# super-auto run — 2026-09-26-herdr-native-mailbox-thread-plugin

flags: planOneShot=false skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: done

idea: Build a Herdr-native mailbox/thread plugin named herdr-threads.
spec: ./2026-09-27-herdr-threads-design.md
epic: ht-4is
branch: super-auto/herdr-native-mailbox-thread-plugin
base: main
skillSource: ~/code/superpowers/skills @ e8244b0 (6.4.2-alepar4.0)
migrated: cumulative original-roast audit rule (history-*/ directories, readproofs, root-adoption per NONZERO review) -> dropped for phase 3; code-roast fix rounds use super-auto §Phase 5 step-back + scope filter (stepBack-round-<N>, scopeFilter-round-<N>)
migrated: per-merge gate results / focused-test lines -> history; replaced by config.mergeCheck (cargo check --all-targets --all-features + clippy -D warnings, build-only) and the single phase-6 sweep (codeBuckets.sweep)
migrated: coordinator inline seam fixes at merge (e.g. 80f90cc, b0d24c5, bf00d19: editing code/tests to make the merged-tree check pass) -> history; from 2a360ee a failing config.mergeCheck aborts the merge and files a blocker/fix bead, and Merge: records end "check <pass|fail|none>"
migrated: per-lane review counters and multi-round fix chains -> history; super-code per-task loop is now one relaxed review + at most one fix pass (Critical/Important), declined findings merged as parked

approvals:
- top-split · ht-4is.1 LEAF, ht-4is.2 PROMOTE, ht-4is.3 PROMOTE, ht-4is.4 PROMOTE, ht-4is.5 PROMOTE, ht-4is.6 PROMOTE, ht-4is.7 PROMOTE, ht-4is.8 PROMOTE, ht-4is.9 LEAF, ht-4is.10 PROMOTE, ht-4is.11 PROMOTE · human advance approval: “auto approved upfront. you drive fully autonomous from here”

- coverage-round-1 · complete; inputs ./coverage-round-1/ and ./task-tree-coverage-round-1.json; three fresh input-bounded built-in reviewers planned per scope; native Claude batch rejected by automatic approval review before execution; built-in frontier-tier substitute with model provenance retained.
  canonical R-list:
  - R1: Real native Herdr plugin with manifest/build/runtime context and isolated clean installation.
  - R2: Persistent pane-bound seat survives native restart/resume/new/clear and label/layout changes.
  - R3: Observed moves preserve seat; close retires permanently; uncertain restore uses audited explicit rebind without ACK.
  - R4: Native top-level Codex and Claude evidence rejects child, unknown and stale receipt/accept/checkpoint mutations while allowing reads.
  - R5: Invite and ordinary addressed handoff persist before manual or managed native launch, including lost initial prompt.
  - R6: Thread topic/purpose, audit, peer controls, explicit archive/reopen and retained history.
  - R7: Transactional joined plus explicit-invited recipient snapshot excludes author and duplicates.
  - R8: Explicit atomic exact-ID ACK batch; duplicate ACK retains original provenance; no read/transport/accept auto-ACK.
  - R9: Accept subscribes future fanout; leave keeps prior obligations and permits reinvite; retirement marks pending obligations retired.
  - R10: Five-minute configurable invitation deadline starts at commit; receipt deadline starts eligible availability; no reset.
  - R11: One durable warning per missed deadline including downtime/late settlement; no warning after timely receipt.
  - R12: Unavailable joined-seat warning deduplicates by thread/episode; new receipt timers await eligibility.
  - R13: Info success events do not independently wake; actionable warnings coalesce native wake; system events require no ACK.
  - R14: Bounded directory/inbox/topic/stats/participants/history/search with stable pagination and explicit body continuation independent of ACK.
  - R15: Compact results and stdin/file input; optional cheap-subagent summaries only, no plugin model calls.
  - R16: SQLite single writer with immutable facts, atomic operation idempotency and durable client intent for response-loss retry.
  - R17: One recoverable daemon per host scope with exclusive ownership, bounded IPC, truthful health, safe ensure/stop and version mismatch handling.
  - R18: Event plus full-snapshot reconciliation survives loss/reconnect and never treats socket errors as closure.
  - R19: Native idle wake and supported active tool-boundary hints for both harnesses, respecting blocked/known-input states and documenting race.
  - R20: Crash-safe wake work, bounded retries and diagnostics distinguish commit, attempted/submitted, offered/read and explicit receipt.
  - R21: Scoped hook setup/removal and native launch preserve other hooks, permissions, sessions and original research.
  - R22: Compact native operator view and diagnostics expose overdue receipts, health, original provenance and unresolved identity.
  - R23: Deterministic crash/concurrency/deadline/storage-failure tests cover production component seams.
  - R24: Four native launch configurations plus parent/child, restart, blocked UI, pagination and isolated host restore/move have auditable receipt evidence before final sweep.
  - R25: Publishable repository documentation/CI/support matrix and marketplace follow-on, with reviewed SHA and no unsupported portability/token claims.

- coverage-round-1 disposition · auto · 10 deduplicated findings; applied C04 C05 C06 C09 C10; rejected C01 C02 C03 C07 C08. Ledger ./herdr-threads-coverage-ledger.md; changes ./coverage-round-1/applied-changes.json; verification ./coverage-round-1/builtin/verification.json. 30 seats / 35 attempts / 28 valid advisory reviews; attribution and store each 2/3, other scopes 3/3.
  requirements: 25 · mapped: 25 · unmapped: 0
- coverage-round-2 · complete; 15/15 valid advisory reviews; no retries; full evidence ./coverage-round-2/verification.json; changed scopes ht-4is.3, ht-4is.6, ht-4is.8, ht-4is.10 and full root ht-4is. Canonical R1–R25 retained unchanged.

- coverage-round-2 disposition · auto · 1 finding C11 applied to ht-4is.11.1, ht-4is.5.2, ht-4is.9 and scheduler/validation specs. Trajectory 10→1, novel 1/1; count shrank, not widening. All round-1 applied findings closed. Fixed two-round cap exhausted; no third coverage review.
  requirements: 25 · mapped: 25 · unmapped: 0
- integration-sweep · ht-4is.12 · depends on all 41 preceding leaves; one sweep only.

roast-design: ./2026-09-27-herdr-threads-roast-design-3.md
roastDesignRound: 3
roastDesignAssessment-1: ./roast-design-1-redesign-assessment.md
roastDesignDisposition-1: 7 Blocking and 6 Should-fix confirmed; focused architecture-contract redesign before fixes; F6 escalation retained; re-roast required after fixes.

parked:
- ./2026-09-27-herdr-threads-roast-design-3.md · degraded-verdict · "clean (0 nits) [low coverage]: complete empty late-round scout review; retained under autonomous continuation, not shipping clearance."
- ./coverage-round-1/ · degraded-verdict · "Coverage advisory: built-in reviewers load one immutable hashed input file in bounded chunks, then use no further tools; the literal prompt-only method could not be carried through the available tool interface at this input size. Autonomous continuation, with three independent reviewers per scope still required."

- ./coverage-round-1/builtin/verification.json · degraded-verdict · "Attribution and store coverage each returned two valid reviews after one seat exhausted its sole retry; coverage remains advisory. Mandatory human goal-vs-full-tree read-through is parked under autonomous continuation, not treated as completed."

- ./2026-09-27-herdr-threads-roast-design-1.md · escalation · "F6: restored-pane creation versus repair reservation remains unresolved; preserve material dissent and require explicit clarification plus fresh review before treating the recovery risk as cleared."

roastDesignFixes-1: F2=ht-4is.13, F3=ht-4is.14, F4=ht-4is.15, F7=ht-4is.16, F8=ht-4is.17, F9=ht-4is.18, F10=ht-4is.19, F11=ht-4is.20, F12=ht-4is.21, F13=ht-4is.22, F14=ht-4is.23, F15=ht-4is.24, F17=ht-4is.25

roastDesignApplied-1: ./roast-design-1-applied-disposition.md · 13 specification fix beads closed; 32 existing task descriptions amended; F6 remains unresolved pending fresh review; no code or native capability pass.

roastDesignStart-2: full eleven-artifact review frozen at 8794e1dcc77c7128c8a32156c139e2d1ce73dba3; late-round stance plus regression lens; prior round1 report supplied; same-family GPT fallback; no panel cap. Results pending, roast-design pointer remains last completed report.

roastDesignDisposition-2: Should-fix (1 confirmed) [converged] · 13 prior design findings resolved; R2-F1 bounded retirement remains ordinary punch-list work; F6 original dissent stays parked; no third design roast required by convergence.
roastDesignAssessment-2: ./roast-design-2-redesign-assessment.md

roastDesignFixes-2: R2-F1=ht-4is.26 · bounded retirement contract punch list before shared contracts/schema execution; no third design roast.

roastDesignApplied-2: ./roast-design-2/retirement-contract-check.md · ht-4is.26 closed for specification work; eight designs and 16 live task descriptions amended; graph audit 66 nodes/128 edges/131 citations, zero errors. Runtime validation remains future work.

codePlan: ../../../../.superpowers/sdd/ht-4is-plan/ht-4is-plan.md · 47 leaves: original42 plus five revision4 followups; original ordinals retained.
codePreflight: ./execution-preflight-scan.md · 42 task rows, 296 interface/shared-file pairs; P1–P7 rulings in ./execution-preflight-rulings.md; no unresolved execution-plan conflict.
codeLedger: ../../../../.superpowers/sdd/ht-4is-plan/progress.md

Resume 2026-09-27: User explicitly resumed and set max agent threads 10 / workflow concurrency 8. Implementers and judges/reviewers use gpt-6-sol medium, including fix implementers and final reviewers; all other delegated roles use gpt-6-astra low. This overrides prior skill model/effort defaults. Preserve all review counters and serial merge rules. Restart verified in the root non-login shell: HERDR_ENV=1, workspace w4, tab w4:t1, pane w4:p1. A read-only inherited-socket query confirmed Codex in the correct repository with scoped sandbox escalation. The previous missing-environment condition is resolved; ht-910 native attribution qualification remains open.

codeBuckets:
  completed: ht-4is.1, ht-4is.11.2, ht-4is.13, ht-4is.14, ht-4is.15, ht-4is.16, ht-4is.17, ht-4is.18, ht-4is.19, ht-4is.2.1, ht-4is.2.2, ht-4is.20, ht-4is.21, ht-4is.22, ht-4is.23, ht-4is.24, ht-4is.25, ht-4is.26, ht-4is.3.1, ht-4is.3.6, ht-4is.5.2, ht-4is.6.1, ht-4is.6.5, ht-4is.3.7, ht-4is.3.2, ht-4is.7.4, ht-4is.3.4, ht-4is.3.9, ht-4is.3.3, ht-4is.3.8, ht-4is.7.1, ht-4is.10.1, ht-4is.7.2, ht-4is.5.1, ht-4is.7.3, ht-4is.7, ht-4is.5.4
  escalated: ht-4is.2.3, ht-4is.3.5
  pendingRetry: ht-4is, ht-4is.10, ht-4is.10.2, ht-4is.10.3, ht-4is.10.4, ht-4is.11, ht-4is.11.1, ht-4is.11.3, ht-4is.11.4, ht-4is.11.5, ht-4is.11.6, ht-4is.12, ht-4is.2, ht-4is.3, ht-4is.4, ht-4is.4.1, ht-4is.4.2, ht-4is.4.3, ht-4is.4.4, ht-4is.5, ht-4is.5.3, ht-4is.6, ht-4is.6.2, ht-4is.6.3, ht-4is.6.4, ht-4is.6.6, ht-4is.8, ht-4is.8.1, ht-4is.8.2, ht-4is.8.3, ht-4is.8.4, ht-4is.9
  parked: []
  stalled: false
  review: not yet run

Code snapshot 2026-09-27: tracker refreshed with all statuses; completed includes fourteen specification-fix beads and nine implementation leaves. pendingRetry includes active work and open enclosing epics, not failed-only retries. Native Task 4 is quarantined through ht-910; independent implementation continues. Whole-epic review, code roast, final sweep and merge to main remain outstanding. Shared amendment revision 4 received scoped CLEAN review and has been adopted; new substrate source work awaits formal design roast3. P11 fix 1 received independent CLEAN rereview and merged as 27c6732c49b5a7d638600b392341126f3c978811 after a clean rebase and passing declared format/library/binary gate (50 library tests, 0 binary tests). This ports-only checkpoint closes no additional implementation leaf; Task 6 runtime ownership/recovery checks remain outstanding.

Execution update: Task36 ht-4is.11.2 merged at 4fb474738c9abfb16c1436a1c3028d0e00d11109 with independent fix1 CLEAN and passing merge gate. Task18 fix2 merged at 3a6d119 after independent CLEAN and passing 71-library-test merge gate. Task19 and Task20 fix1 are running; Task26 fix1 71f76a8 is in independent scoped rereview. Native receipt authenticity remains a later gate.

Shared amendment revision4: scoped rereview CLEAN; adopted ./shared-contract-amendment-adopted.md and ./shared-contract-amendment-adoption.md. New tasks ht-4is.3.7, ht-4is.3.8, ht-4is.3.9, ht-4is.5.4, ht-4is.7.4. Canonical spec reconciliation is complete and root inspected the eight changed diffs; formal design roast3 must run before substrate dispatch. Formal round3 has now started; its result remains pending.

roastDesignStart-3: full canonical spec set plus adopted revision4 and current 47-leaf tracker frozen at b5fc17027fcb7d3a64bf151796de18fd0f7d516f; ./roast-design-3/artifacts.json and task-tree.json. Late-round stance plus regression; both earlier reports and amendment review history retained. Triage started; no new substrate implementation until disposition. Last completed roast-design pointer remains round2.

Execution checkpoint during roast3: Task20 ht-4is.6.5 independently CLEAN fix1, clean disjoint rebase, fmt+77 library/0 binary merge gate passed; merged b9e9fd4 and closed. Task26 fix1 scoped CLEAN remains held for adopted shared encoder/typed-header/runner contracts. Task19 fix1 still active. Nine original implementation leaves complete; source acceptance and native gates remain open.

roastDesignDisposition-3: clean (0 nits) [low coverage] · nine of nine scouts complete, zero raw candidates; qualifier required by reporter rule, no judges needed. Autonomous caller-owned exception permits execution; qualifier retained, no convergence claim. ./roast-design-3-disposition.md. F6 parked and ht-910 unresolved.

Execution after design3: Task43 ht-4is.3.7 started on isolated11fba86 with verified local crate and clean77-library-test baseline. Task19 fix1 independent CLEAN at419b4a3 and Task26 fix1 CLEAN at71f76a8 remain held for shared API/encoder adaptation; neither leaf is closed. Workflow cap8 and user model split preserved.

Task6 originalfix2 assessment: ./structural-continuity-history-redesign-assessment.md · assessor freshly full-read all 138 finalized reports (555639 bytes); root read full substantive narrative and latest review, verified all manifest hashes, baseline134 unchanged, and no new W review arrivals. Adopt focused centralization of resolved structural continuity; no high-level product redesign. Originalfix2 remains NONZERO, next fix3; ordinary-resolution work remains separate. Native and final gates stay open.

Task43 integration: originalfix1 CLEAN93e54a8, rebased032b505c ontoe4160a82; mergeda0f9a0e. No source overlap/conflicts; declared fmt+locked library/bin gate173 passed, diffcheckpassed. Root verified exact reviewed source continuity,3rawhashes and tested=merged tree; ht-4is.3.7 closed. Consumed CLI/encoder/invitation-fixture ancestry retains separate ownership and acceptance. Full epic/native/final gates remain open. Ordinary-resolution producer+consumer scoped followup review is running at a9d34ba/d099d9d; Task6 originalfix3 remains running.

Ordinary-resolution initial NONZERO1Minor assessment: ./ordinary-resolution-history-redesign-assessment.md · Astra/low assessor freshly full-read all140 complete finalized reports (563904 bytes); root read entire substantive narrative and newest review, verified140 hashes/bytes, unchanged138 baseline, no new arrivals. Adopt bounded unusedmut test cleanup; no high-level redesign. Ordinaryfollowupfix1 and existing Task6originalfix3 counters retained; native/full gates open.

Integration acceptance: Task6 closed; Task8 source accepted at f813755f, tracker closure pending ht-4is.7.4;391 library tests and exact reviewed source conservation. See ./task6-task8-integration-acceptance.md and evidence JSON. Native/full gates retained.

Encoder prerequisite acceptance: imported seam CLEAN147; originalfix1 retained,14 encoder tests passed in exact391-library gate, no source drift. ht-4is.7.4 closed, followed by normal ht-4is.3.4 close; initial refusal retained. See ./encoder-import-integration-acceptance.md.

Transport source checkpoint integrated7572c328 after CLEAN148 and388-library gate; nativeTask11 remainsopen. Task45 closed after fixture CLEAN149. See ./transport-source-integration-acceptance.md and exact evidence JSON.

Store producer acceptance: Task7 latefollowup CLEAN150 and pristine38receipt test rows establish imported source acceptance; Task7closed normally, then Task44closed after its prerequisite and source/fixture CLEAN149. Source and gate conservation in ./receipt-materialization-integration-acceptance.md. Service/config/native gates retained.

Task10 merge quarantine: ht-3xy repeated aftersoleRESOLVEretry; thirdconflict replaying inheritedformat-only46cc443 at230/234. C-2 exhausted, nofurtherautoresolve orcounterreset. Original4b andpartialrebase preserved; independent workcontinues. See ./task10-integration-quarantine.md.

Pause checkpoint 2026-09-28: User requested a safe-boundary pause. The default Codex profile now sets `agents.max_concurrent_threads_per_session = 10`; the new limit takes effect in a new session. For this run, pass `config.concurrency = 8` to every `super-code` invocation and do not exceed eight concurrent workflow agents. The requested `super-auto` skill and its supporting Superpowers skills were installed in this profile from `alepar/superpowers`; restart before relying on skill discovery. C3 fix1 independent review finalized CLEAN (0 Blocking, 0 Should-fix, 0 Nit) for frozen commit `bdb9f34f46efad9d6b2068824c67c46649afd98d`, tree `3562a614f64df12d7deb95450d40f115943089e5`; review and evidence are `.superpowers/sdd/ht-4is-plan/graph-c3-fix1-review.md` and `graph-c3-fix1-review-evidence.json`. No C3 merge or new product work was started after that verdict. Resume at C3 serial integration, preserving all earlier review counters, source gates, graph task scope, and native validation requirements.

C3 fan-in checkpoint: rebased C3 candidate `c0f31d1` preserved reviewed C3/C2 source and passed 388 store, 799 library, 48 service, 55 contract and 11 view tests, but independent seam review is NONZERO (0 Blocking, 1 Should-fix, 0 Nit): v5 startup accepts missing recipient paging index and can sort an audience per page. Fresh cumulative audit of all 200 original reports/841991 bytes is complete in `./history-c3-fanin-nonzero/`; root verified hashes, intervals, prior-history conservation and adopted a bounded schema/index and incremental-recipient-count correction in `root-adoption.md` before any C3 fix/new judge. A root SQLite instruction-count probe also exposed growing per-quantum `COUNT(*)` work. The integration branch remains unmerged. Task19 composed transport and health retirement fixture source are separately frozen for later independent review; D2 and native/full gates remain open. Workflow cap8 and model split persist.

C3 fan-in fix1 checkpoint: isolated `2a7b3f4` passed 391 store, 802 library, 48 service, 55 contract and 11 view tests, corrected v5 semantic schema checks and incremental recipient count. Fresh independent Sol/medium seam review is NONZERO (0 Blocking, 2 Should-fix, 0 Nit): required-only ordinal paging sorts retained requirement history, and new preparation commits before the current service authority gate. Full cumulative audit of all 201 originals/845966 bytes is complete in `./history-c3-fanin-fix1-nonzero/`; root verified full reads and previous evidence conservation, then adopted a bounded, authorized preparation-quantum seam correction in `root-adoption.md` before any further source fix/new judge. C3 remains unmerged. Preserve prior counters, Task19/health isolated branches, D2, native and final product gates.

C3 accepted fan-in: fix2 frozen `88121e4`/tree `cf4f240`, independent Sol/medium review CLEAN (0 Blocking, 0 Should-fix, 0 Nit), merged serially at `98dc185` with exactly that tree and no conflicts. Merged-tree gate passed 805 library tests/9 ignored, 48 service, 55 contracts, 11 view, check/fmt/diff; candidate also passed 394 store and focused required-only/authority tests. Acceptance and remaining limits are in `./c3-integration-acceptance.md`. C3 store leaf can close; D2 routing, native harnesses, package/product sweep and Task19/health branches remain open.

Task19 composed transport fan-in checkpoint: isolated rebased `7cd2a31` passed 52 feature-enabled service, 50 ordinary service, 805 serial library/9 ignored, both all-target checks and fmt/diff. Independent Sol/medium seam review is NONZERO (0 Blocking, 1 Should-fix, 0 Nit): tests did not observe server worker completion before search permit-reuse or no-late-writer assertions. Fresh cumulative audit of all 202 originals/850302 bytes is complete in `./history-task19-composed-nonzero/`; root verified full reads/prior evidence conservation and adopted a narrow causal server-completion test seam in `root-adoption.md` before any source fix/new judge. No production defect demonstrated or Task19 merge/Beads closure. Preserve earlier parallel library timeout as a failed run. Health, D2 and native/product gates remain open.

Task19 accepted fan-in: corrected isolated `29a891c`/tree `bb98d80` received independent Sol/medium CLEAN review (0 Blocking, 0 Should-fix, 0 Nit), merged serially at `5a8ec74` with exactly that tree. Merged feature service 52, ordinary service 50, serial library 805/9 ignored, both all-target checks and fmt/diff passed. The prior parallel library timeout remains failed. Acceptance and limits are in `./task19-transport-acceptance.md`. Task19 transport leaf can close; health retirement, B1/B2 and D2/native/product gates remain open.

Health retirement fixture fan-in checkpoint: frozen test-only `34f3025` passed focused tests, serial library 807/9 ignored and fmt/diff, but independent Sol/medium review is NONZERO (0 Blocking, 1 Should-fix, 0 Nit): terminal seat summary and physically pending receipts were not independently observed. Fresh cumulative audit of all 203 original reports/854102 bytes is complete in `./history-health-retirement-nonzero/`; root verified full reads, earlier-history and candidate-artifact conservation, then adopted a narrow test-oracle correction in `root-adoption.md` before source fixes or a new judge. No fixture merge/Beads closure. Production Health/error provenance, D2, native and final product gates remain open.

Health retirement fixture accepted: corrected test-only `1a5233c`/tree `826d688` independently reviewed CLEAN (0 Blocking, 0 Should-fix, 0 Nit) and fast-forwarded with identical source tree. Merged serial library 807 passed/9 ignored; fmt/diff passed. The test independently checks terminal summary, physical pending rows and durable incomplete prefix before manual Stop/reopen. Scope and outstanding production Health/error/automatic boot/IPC/native gates are in `./health-retirement-fixture-acceptance.md`. B1 source was already merged and its transport prerequisite accepted; B1 may close. B2 source was already merged and its B1/transport/health prerequisites are now accepted; B2 may close. D2 and native/product gates remain open.

B1/B2 closure: ht-4is.30.1 and ht-4is.30.2 closed after their previously reviewed/merged source was matched to the accepted transport and health prerequisites. D2 routing slice `5bae39a`/tree `583fbf9` received independent scoped CLEAN (0 Blocking, 0 Should-fix, 0 Nit), was preserved and fast-forwarded to the identical reviewed tree after an initial direct-branch commit was corrected, and passed merged service 51, serial library 807/9 ignored, all-tests check, fmt/diff. See `./graph-d2-routing-acceptance.md`. Full D2 versioned native acceptance, recovery, CLI integration and product gates remain open.

Harness setup fan-in checkpoint: provisional `ht-4is.8.3` candidate `7eb8528` passed 16 focused tests and its isolated serial suite but independent Sol/medium review is NONZERO (0 Blocking, 2 Should-fix, 0 Minor): manifest-before-settings failure strands retry/removal, and inspection can report stale or empty ownership as configured. Fresh cumulative audit of all 208 original reports/875792 bytes is complete in `./history-harness-setup-nonzero/`; root verified full reads, 203-prior/661-candidate conservation, added two previously omitted finalized CLEAN reports, and adopted a narrow publication-recovery/inspection correction in `root-adoption.md` before any setup fix/new judge. The arbitrary noncooperating-writer rename window remains an explicit exclusive-access product gate. No setup merge/closure; adapter, native and final gates remain open. Separate cooperative CLI candidate `eed6e7e` is frozen for later review, not merged.

Cooperative CLI fan-in checkpoint: frozen isolated `44fed77`/tree `4009703` has independent Sol/medium NONZERO review (0 Blocking, 1 Should-fix, 0 Nit): local generation mismatch prevents exact historical CheckIn recovery after a successor. Candidate passed 72 focused CLI, 23 bridge and 811 serial library tests/9 ignored. Feature service initially failed 1/52 with `DatabaseBusy` in the unchanged resolution fixture; isolated rerun and full rerun then passed 52/52. Preserve both results. A fresh cumulative audit of all 209 originals/880042 bytes is complete in `./history-cli-cooperative-nonzero/`; root verified full reads, previous history/artifact conservation, and adopted a narrow committed-replay-first correction in `root-adoption.md` before any source fix or new judge. No CLI merge/closure; native, D2 and product gates remain open.

Setup fix1 and production Health review checkpoint: isolated setup `e15b918`/tree `2b32867` independent review NONZERO 0B/1S/0N; a prepared-manifest crash boundary can erase an unrelated settings edit on later removal and ambiguously adopt an identical hook. Isolated production Health `0d2862a`/tree `78928a5` independent review NONZERO 0B/1S/1N; two autocommit SELECTs can mix pending/degraded states and a query-error redaction test is absent. Fresh cumulative audit of all 211 originals/886581 bytes is complete in `./history-setup-health-reviews-nonzero/`; root verified exact full reads and conserved prior history/artifacts, then adopted separate narrow setup ownership and atomic Health-summary corrections in `root-adoption.md`. Neither candidate is merged or closed; exclusive-access setup limit, production last_error writer, native and final gates remain open.

CLI fix1 rereview checkpoint: isolated `454f5a7`/tree `f43dcae` passed focused CLI/bridge/elected-socket and 812 serial library tests/9 ignored but independent Sol/medium review is NONZERO 0B/2S/0N: successful-flush completion removes the `.intent` required by its older-generation exception, and omitting an original native session evades exact-payload checking. A fresh cumulative audit of all 212 originals/891978 bytes is complete in `./history-cli-fix1-nonzero/`; root verified full reads and prior conservation, then adopted a two-source pending/completed historical gate and symmetric optional-session check in `root-adoption.md`. No CLI fix2, merge or closure yet. Preserve the original 51/52 DatabaseBusy failure and later clean reruns, plus native/product gates.

Three-review checkpoint: isolated production Health fix1 `8ecb5f0` is NONZERO 0B/2S/0N (legacy migration expects v5 rather than v6; final short probe can outlive live budget). Isolated full D2 `7c018eb` is NONZERO 0B/1S/0N (four missing real-daemon acceptance crossings, no confirmed source defect). Isolated setup fix2 `007f325` is NONZERO 0B/2S/0N (manifest mode 0644 can expose backed-up settings; damaged baseline can remove unrelated content). Fresh cumulative audit of all 215 originals/907165 bytes is complete in `./history-three-reviews-nonzero/`; root checked five full-read proofs, prior 212 originals and 661 artifacts, and adopted three bounded corrections in `root-adoption.md` before any related fix/new judge. All three candidates remain isolated. CLI fix2 `492b29a` is frozen from the prior audit and awaits independent review. Native model receipts, package lifecycle, production error writer, integration/final gates remain open.

CLI source integration: isolated fix2 `492b29a`/tree `2ea98a6` independently CLEAN (0B/0S/0N), merged at `84c9563` with all nine reviewed source/test files byte-identical. Merged gate passed 812 serial library/9 ignored, 52 serial service, all-targets/all-features check, fmt/diff. Two independent-judge four-thread service runs exited101 without named failures and remain unclassified; earlier 51/52 DatabaseBusy remains failed history. See `./cli-cooperative-integration-acceptance.md`. `ht-4is.9` stays open for native setup, managed launch, daemon and model-receipt composition.

Native adapter source checkpoint: isolated Codex `d3e44ab` review NONZERO 0B/3S/1N (no native token rewrite, bypassable version gate, missing lifecycle/stale/hostile fixtures, partial child shape); isolated Claude `907f96f` review NONZERO 0B/2S/1N (bypassable version parser, missing fixed actionable continuation, partial child shape). Setup fix3 `db87c29` review CLEAN with 29 focused passes, broad 745/83 socket failures/9 ignored; invalid 743/85 pre-fix binary cache run retained separately. Fresh cumulative audit of all 219 originals/924623 bytes is complete in `./history-adapters-nonzero/`; root verified all five full reads and prior 215/661 conservation, then adopted fail-closed native transport proof and narrow version/presentation/source-fixture corrections in `root-adoption.md`. No adapter source fix/new judge preceded this adoption. Codex0.158 and Claude2.1.284 native captures remain UNSUPPORTED; both actual model receipt gates open. Health fix2 and D2 correction are isolated for later review.

Pause checkpoint 2026-09-29: The user requested a restart pause. In the current non-login shell `HERDR_ENV=1`, `HERDR_WORKSPACE_ID=w4`, `HERDR_TAB_ID=w4:t1`, `HERDR_PANE_ID=w4:p1`. The default Codex profile already has `agents.max_concurrent_threads_per_session = 10`; this run retains `config.concurrency = 8` for every super-code invocation and an eight-agent workflow cap. The `super-auto` and supporting Superpowers skills are already installed in the profile from `alepar/superpowers`. The setup fix3 CLEAN candidate `db87c29` was merged at `ad0b3a3`; the merged serial library gate passed 833/0/9, with fmt/diff checks passing. The full D2 fix1 frozen `ae8788e`/tree `31571a1` received corrected independent CLEAN 0B/0S/0N review and remains unmerged. Health fix2 frozen tree `ccf64df` received NONZERO 0B/1S/0N review: production Health creates a new cancellation token, so the current request cannot cancel retirement summary work; no related fix or new judge may start until a fresh cumulative full-history audit and high-level redesign assessment are recorded. Claude adapter fix1 is frozen at `e593904`/tree `92a985a` with focused checks passing; it is unreviewed and unmerged, and project setup still does not install its SessionStart hook. An isolated Codex 0.158 transport probe failed before model tool use with `workspace routing discovery failed`; its sanitized evidence is in `./native-codex-transport-probe/`, and native rewrite/approval/UDS/receipt support remains unproven. No native/product/release gate is complete. The integration branch remains at `ad0b3a3` before this checkpoint commit; resume from this exact state, preserving all counters, review history and failed/unsupported evidence. Do not merge into the base branch.

Resume checkpoint 2026-09-29 (Claude Code session): the run resumed under Claude Code (Opus 5.5) instead of Codex. The user lifted the eight-agent cap: concurrency is unrestricted, and dynamic workflows size themselves. Role mapping: Opus for implementers and judges, Sonnet for auditors and mechanical roles. Pre-flight: superpowers 6.3.0-alepar3.8 is current. The fresh cumulative audit of all 221 originals (932,417 bytes: the prior 219 plus the D2 fix1 CLEAN and Health fix2 NONZERO reviews) is complete in `./history-health-fix2-nonzero/`. It used eight readers; root verified hashes and coverage and that the prior 219 are unchanged. The redesign assessment concludes a bounded local correction: Health fix3 threads the request budget through a budget-taking provider, adds a production-seam cancellation test, and sweeps sibling fresh-budget sites report-only. No related fix or judge preceded that adoption. The D2 CLEAN candidate `ae8788e` is merged at `f4d1577`; its merged gate passed serial lib 837/0/9 and service 52/0 (`./graph-d2-integration-acceptance.md`). Claude adapter fix1 is still unreviewed, the Codex transport stays fail-closed, and native/product/final gates remain open.

Operator policy 2026-09-29 (user direction): optimistic dependency unblocking. A dependent bead may start as soon as its dependency's implementation is committed, while that dependency is still in review. Dependents build on the unreviewed candidate commit and record that base. If the review later changes the dependency, the dependent rebases, and its own review covers the delta. Merge order is unchanged: nothing merges into the integration branch before its own CLEAN review. Every audit-before-related-fix rule and all counters stay in force.

Four-review checkpoint: Health fix3 NONZERO 0/0/1, Claude adapter fix1 NONZERO 0/2/2, last_error writer NONZERO 0/2/2, SessionStart setup NONZERO 0/1/2. Fresh cumulative audit of all 225 originals/977571 bytes complete in `./history-four-reviews-nonzero/`; root-adoption rules all four bounded local corrections plus three narrow guards (mutation-named production-seam tests; typed redacted Health component status with a single failure-state owner; single hook declaration source with manifest-validated removal and per-field budgets). ht-4is.30 epic closed (tracker-only). Beads filed for the last_error writer (ht-4is.3 child) and identity/repair.rs:300 request budget (ht-4is.4 child). Codex fix1 review still pending.

Composition probe checkpoint: an end-to-end probe of integration 7c47f88 (`./composition-probe-1/`, no model calls) found that no seat, thread, message or receipt can be produced on a real Herdr host. The causes: native incarnation is always unknown; the seat-resolve epoch disagrees between store and adapter; there is no hook stdin entrypoint, and the installed hook exits 2, which would block Bash; operator mutations are Unsupported; and lifecycle UX is broken (ensure exits 2 on a degraded daemon, `--help` exits 2, errors print as Debug structs, doctor is a stub, long socket paths fail). Composition wave 1 is dispatched off `c0b6e8d` in four lanes (host-seats, hook-entrypoint, operator-cli, lifecycle-ux), each with an independent judge. Codex fix1 `b031899` is NONZERO 0/1/1: its removal of the context-only PreToolUse hook was wrong, and the version-observe timeout is incomplete. It waits for the next cumulative audit. Filed a Bead for the clippy `-D warnings` gate required by the 10.2 CI.

Wave-1 checkpoint: Health fix4 is merged (`7bf240d`, seam fix `80f90cc`) and last_error fix2 is merged (`77980f4`). Both are accepted and `ht-4is.3` is closed. Seven NONZERO reviews came from the corrections and composition wave 1. A fresh cumulative audit of all 234 originals (1,076,798 bytes) is in `./history-wave1-nonzero/`. The adoption rules six bounded local corrections plus a **bounded redesign of the hook check-in write model**: lifecycle hooks stay durable, tool-boundary hooks become non-durable and coalesced by attention frontier, and the context journal gets bounded retention and terminal abandon on definitive rejection. The redesign follows the harness design's "excluded from durable client intent" rule.

Wave-1 corrections: CLEAN and merged are Claude adapter fix3 (`58cfb87`), Codex adapter fix2 (`5aaedfa`, transport still fail-closed) and operator CLI fix1 (`c4d483e`). Every target of the merged gate passes (lib 880/0/9). NONZERO this round: hook redesign fix1 `ed2b4cc` 1B/1S/0N (a truncated inbox page lets later-thread offers be coalesced away, and receipts are read after CheckIn); setup fix2 `e210b11` 0/1/1; host-seats fix1 `6fb47ba` 0/0/1; lifecycle-ux fix1 `2317a78` 0/0/1; Health seam `80f90cc` review 0/0/1 (a test double ignores the budget). These wait for the next cumulative audit.

Wave-1 fix2 checkpoint: the Health test double is CLEAN and merged (`887b35d`). Lifecycle-UX fix2 is CLEAN; its merge with operator CLI conflicted semantically, was resolved at `f3d9116` on `merge/lifecycle-ux`, and awaits an independent seam review. Fresh audit of 248 originals in `./history-wave1-fix2-nonzero/`. Hook fix2 failed the completeness axis a second time, which triggers the pre-committed escalation to a server-side attention digest: a seat-scoped token from `LogicalAttentionFrontier` with a flat cost test. Setup, host-evidence and Health-pin Nits get bounded local corrections, including a monotone evidence rule.

Lifecycle-UX merge seam review (`merge-lifecycle-ux-seam-review.md`) of `f3d9116`: NONZERO 0/0/1. Nothing is lost or doubled, the ordering is correct, and the Retry reroute is guarded. The Nit: a caller that cannot be located gets inconsistent exit codes (4, 2 or 1). The fix is queued for the next audit round, and the merge is held until it is CLEAN.

Host-seats and Health pin landed. The Health field pin was CLEAN and merged at `98744e4`. Real-host seats (the `comp/host-seats` chain ending in fix3 `7f2a3b9`, CLEAN) were merged via semantic merge `0419513`: the host evidence derivation moved into the shared `elected_health_provider`. The seam review `merge-host-seats-seam-review.md` is CLEAN 0/0/0, and the merge was fast-forwarded. Serial gate at `0419513`: lib 888/0/9, contracts 55, host_adapter 24/0/1, local_endpoint 8, package 9, service 66, view 11. The merger saw one service-target race (63/1, then 64/0 on rerun), which is kept as unclassified. Still held: the setup merge `579998c` (seam NONZERO 0/1/0: stale docs say SessionStart is not installed) and the lifecycle-UX merge `f3d9116` (seam 0/0/1: inconsistent exit codes for an unlocatable caller). Attention digest `07975bb`: NONZERO 1/1/0 (the digest walks the seat's full receipt history at O(n²), so deadlines hit at 10k ACKed receipts; the service-notification warning source is untested). Noted for the native demonstration: the installed Claude is 2.1.284, while the adapter pin is 2.1.283.

Setup landed at `627234e`. This is the owned Claude setup with SessionStart composition (fix4 CLEAN; merge seam S1 fixed by docs `f6e2a74`, re-reviewed CLEAN). The merged tree equals the reviewed `6f65da0` plus the capture-evidence docs. Serial gate: lib 916/0/9, contracts 55, host_adapter 24/0/1, local_endpoint 8, package 9, service 66, view 11. The lane saw two host-seats service timing failures once (then 66/0 twice), which are unclassified. Claude 2.1.284 input capture recorded at `6e40455` (`./claude-284-hook-capture/`): all four payloads are accepted by the 2.1.283 parser unchanged, and output application is not yet verified. NONZERO this round:
- Digest fix2 `ec82fd8`: 1/0/2. The emitting check-in's `inbox_in_transaction`/`scan_effective_receipts(Thread)` manifest candidate is not restricted to pending rows, so at 100k manifest-backed ACKed receipts a new invitation fails with UnknownOutcome. Quiet digest calls are flat at 15–18 ms.
- Lifecycle exit fix `b1d5a48`: 0/1/1. An inherited host-seats resolution test flakes on timing, and one rerouted retry site is unasserted.

Claude 2.1.284 input pin landed at `d11451d` (`aa15b84` CLEAN, `claude-pin-284-review.md`): exact version set {2.1.283, 2.1.284}; the captured payloads are fixtures. Serial gate: lib 918/0/9, contracts 55, host_adapter 24/0/1, local_endpoint 8, package 9, service 66, view 11. Hook output application is still pending native verification.

Lifecycle-UX landed at `008ac2d`: lifecycle-UX fix2 CLEAN, then semantic merge with operator CLI `f3d9116`, exit-code fix `fa05239` and deterministic resolution test `3c61cd9`, re-reviewed CLEAN (`merge-lifecycle-ux-exit-fix2-review.md`). Compared with the reviewed `3c61cd9`, the merged tree adds only the separately reviewed setup and 2.1.284-pin changes (a conflict-free auto-merge). Serial gate: lib 922/0/9, contracts 55, host_adapter 24/0/1, lifecycle_ux 12, local_endpoint 8, package 9, service 66, view 12. Digest fix3 is still implementing.

User direction (2026-09-29): update the Codex adapter to the installed 0.158.0 as evidence allows, and support multiple adapter recipes, one per released version or compatible version interval. Parallelize more, with super-code rules as guidance rather than a hard limit. Parallel wave 2 is dispatched off `08c55ca`:
- Codex 0.158 native capture (normal auth, `-c` hook overrides only), then the evidence-backed recipe registry for both harnesses;
- package lifecycle (10.3);
- WorkerStatus redaction (5.5);
- invalidate budget (4.5);
- witness flake determinism (4.6);
- composed crash matrix (11.1).
Each lane gets an independent review. Digest fix3 is running in parallel.

Wave 2, first merges: the invalidate budget (4.5, `369608b`) and the witness-flake fix (4.6, `3683141`) were CLEAN and are merged. Serial gate at `3683141`: lib 923/0/9, contracts 55, host_adapter 24/0/1, lifecycle_ux 12, local_endpoint 8, package 9, service 70, view 12.
Codex 0.158 capture was PARTIAL. The embedded hook schemas are identical across 0.155.1, 0.157.1 and 0.158.0; the parser accepts schema-valid payloads; and the user's codex uses CODEX_HOME=aisw profile default. No live hook capture was made, because the permission classifier blocked `--dangerously-bypass-hook-trust` and the `hooks.state` trust write. This needs a user decision. Pre-existing gap: SessionStart source=fork is rejected.
NONZERO: WorkerStatus redaction 0/1/1; recipe registry 0/1/2.

Wave-2 reviews, all NONZERO:
- Recipe registry `508c812`: 0/1/2. A public ToolInvocation.recipe lets a caller forge a Supported Codex transport.
- WorkerStatus redaction `de1266a`: 0/1/1.
- Package lifecycle `5858003`: 1/1/0. The `[[startup]]` entry is never validated, and the docs wrongly claim Herdr 0.9.1 does not run startup hooks; the reviewer reproduced that it does. The lane also fixed a real install defect: `build.sh` installed a stale binary under an inherited CARGO_TARGET_DIR.
- Composed crash matrix: 1/3/2. It found a **product defect**: cooperative bindings are stored without terminal_id/incarnation (`src/store/seats.rs:3475`), so after one host socket denial Reconfirm can never match, and every cooperative seat stays unresolved permanently (CursorStale every turn). The test discarded that error.
The service target flaked under load (load average above 100) in several lanes, then passed on rerun. This is unclassified. Waiting on: the digest fix3 review and the live Codex capture (user approved the bypass flag for the scratch project only).

Live Codex 0.158 capture (`./codex-158-live-hook-capture/`, user-approved bypass flag in a scratch project only, real aisw default profile, gpt-6-luna low effort, about 119k input / 307 output tokens). Captured: SessionStart startup/resume, root PreToolUse Bash, SubagentStart, and child PreToolUse. **Context-only additionalContext from both SessionStart and PreToolUse reached the model** as developer messages, and the tool still ran. The earlier routing failure did not recur; `codex exec` needs stdin=/dev/null. `permission_mode` is always `bypassPermissions` and must not be read as the sandbox mode. Codex wrote one rollout plus cache/sqlite files to the profile; config, auth and history are unchanged, and no trust or hook entries were added.

Wave-2 corrections, all NONZERO:
- Digest fix4 `5e56510`: 1/0/0. Programmatic warn notices are pending forever, and the cap is applied after the full set, so the work is unbounded.
- Crash fix1 `dff7ebb`: 1/2/0. Structural reconfirmation is unreachable with the real adapter (occupancy/execution Unknown), and the CLI gate refuses a fresh lifecycle check-in after a generation bump. Design lines seat-identity 47 and root 169 describe structural reconnect recovery.
- Package fix1 `66fafc7`: 0/0/1.
- Recipes fix1 `8cf9544`: 0/0/1.
- Redaction fix1 `fabfe78`: 0/1/1.
Audit of 272 originals dispatched.

WorkerStatus redaction (5.5) landed at `63a2581` (fix2 CLEAN, `worker-status-redaction-fix2-review.md`; the first review attempt was interrupted and re-run in full). Serial gate: lib 930/0/9, contracts 55, host_adapter 24/0/1, lifecycle_ux 12, local_endpoint 8, package 9, service 70, view 12. Recipes fix2 `9e660af`: 0/0/1.

User process policy (2026-09-30):
1. A full cumulative audit is required only when a review has a Blocking or Should-fix finding. Nit-only findings are fixed directly and re-reviewed without an audit.
2. A lane whose review is 0 Blocking / 0 Should-fix may merge, with each Nit filed as a follow-up bead to fix before the final sweep (super-code PARK).
3. The digest/hook lane, now past the 5-round breaker: if fix6 is not CLEAN and only scale residuals beyond the tested 10^5–10^6 envelope remain, merge with the limits documented plus a follow-up bead. A correctness Blocking inside the envelope gets one more round.

User process policy update (2026-09-30), superseding the earlier entry: stay light on reviews during the super-code phase, because the full super-roast at phase 4 re-examines the whole branch.
- Each lane gets one light independent review, focused on correctness Blockers (wrong behaviour, crash, data loss), plus the merged-tree test gate.
- A lane merges when it has 0 Blocking. Should-fix and Nit findings are filed as follow-up beads for the final sweep and super-roast.
- Cumulative audits run only when a real Blocking finding recurs on the same axis.

Wave-3 landings under the user's lighter policy: composed crash matrix plus structural reconnect recovery (11.1, `8c4206c`, fix3 0/0/1); package lifecycle (10.3, fix2 `675cdf4`, fix3 `1b93360` with 0/1/2 parked); recipe registry covering Claude {2.1.283, 2.1.284} and Codex {0.157.1, 0.158.0}, with live capture fixtures (`d823d31`, fix3 CLEAN; capture-doc conflicts resolved to the lane's redacted copies). One seam fix: the worker-health wake fixture had to carry binding reconfirmation evidence (`b0d24c5`). Full serial gate at `d823d31`: lib 980/0/10, contracts 55, host_adapter 24/0/1, lifecycle_ux 12, local_endpoint 8, package 11/0/1, service 71, view 12. Parked findings are tracked in a sweep bead.

User test-scope policy (2026-09-30): per-lane and per-merge verification is narrowed to `cargo fmt --all --check`, `cargo check --all-targets --all-features`, and the tests covering the changed code (focused lib test modules plus integration targets whose files changed or which directly exercise the changed modules). One full serial suite runs as the gate before phase 4, the super-roast; that run is the super-code `config.sweep`.

Digest fix6 `ab5eb85` is CLEAN (`comp-attention-digest-fix6-review.md`). Occupant-frontier settlement is O(cap); at 3×10⁵ and 10⁶ notices each page of 16 comes back in 25–60 ms; every read axis is flat at 10⁵. Parked for the sweep: a replacement occupant is re-offered retained notices starting from the oldest (disclosed in docs). The hook+digest branch is being merged onto integration by a resolver, followed by a light seam review. Crash fix3's real-host acceptance was not run, because the permission classifier denied the private UDS proxy; it is covered by the production-shape fixture only and must be re-verified in the native demonstration.

Wave 3b results:
- Recipe merge seam: CLEAN.
- Flaky-probe (11.7): merged at `e606b34`, 0/0/1 parked. It includes a real CallBudget cancel-versus-deadline TOCTOU fix in store budget checks. Focused gate: lib store 416/0, service 71/0.
- Native demo driver `36c4ff9`: ready (unmerged). Its dry run on `a54008e` fails only on the missing hook entrypoint and the version pins.
- Docs/CI draft `d051bb7`: based on stale `a54008e`; to be reconciled after the hook merge.
New gaps: the installed Claude auto-updated to 2.1.285 (capture plus recipe extension dispatched), and no public setup CLI exists (dispatched). Codex UDS reachability under its sandbox (ht-910) is still unresolved.

User product decisions (2026-09-30):
1. Claude 2.1.285 permission-checks the rewritten Bash command. `herdr-threads setup claude` installs the owned, narrow allow rule `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` in the project's `.claude/settings.local.json`; `unsetup` removes it; this is documented.
2. Codex version churn (0.159.2 is now installed): admit an unlisted Codex version when the hook JSON schemas embedded in its binary hash-match a verified recipe's captured schemas. Report it as "schema-matched, live-unverified" in doctor and health. Unmatched schemas are refused.

**Hook path landed.** The hook entrypoint plus server-side attention digest (fix6 CLEAN) merged via the semantic merge `08612f8`, whose seam review was CLEAN 0/0/3 with Nits parked, then onto the current tip. Seam fix `bf00d19` moves the hook version-gate test to recipe [2.1.283, 2.1.285]. Focused gate: hook_entrypoint 15/0/7 (scale tests ignored), lifecycle_ux 12, service 71, contracts 56, lib harness 185. Schema is now user_version 8. Claude 2.1.285 recipe merged at `05a662c` (light review 0/2/3, parked). Next: setup CLI + allow rule, Codex fingerprint admission, demo driver, reconciled docs, then the native demonstration.

Native Claude demonstration 1 (`./native-claude-demo-1/`, Claude 2.1.285 print mode, $0.33): PARTIAL. The pipeline works: prelaunch invite and ACK-required handoff, hook context delivered, restart and resume keep receipts. With the specified prompt the model did not act (0/2), because the context never names the CLI (P1). With a one-line CLI hint the model accepted and ACKed its exact message in all 3 phases; provenance was cooperative_top_level. Product defects P1 (context lacks CLI/argv; the digest id form is not a CLI arg), P2 (production adapter's Unknown occupancy marks cooperative occupants unavailable on every snapshot), P3 (offer re-emitted, caused by P2 plus own-seat episodes counting as attention), P4 (Mail data always []), P5 (observation field naming). Driver defects D1 and D2.

Setup CLI landed (`setup`/`unsetup`/`setup-status` for Claude and Codex, owned Claude allow rule, and Codex hooks that coexist as a session-flags layer; review 0B/1S/4N parked). Codex schema-fingerprint admission with the private hook cache landed at `cb348bd` (0B, parked): installed Codex 0.159.2 is admitted as schema-matched, live-unverified; the hook's cold observation takes 0.9 s and warm 0.01 s. Focused tests: lib harness+cli 294/0/2, setup_cli 12, lifecycle_ux 13, hook_entrypoint 15/0/7 (serial). Pre-existing: the hook_entrypoint target exits 101 under the default parallel harness but passes serially, and needs a bead. Driver fix1 is 1B: the Codex child-ACK check cannot distinguish root from child under the cooperative model.

Demo-fix landings: the native demo driver merged at `0b6d994` (fix2 CLEAN; child-ACK absence is honestly UNVERIFIED whenever subagent activity occurs; it uses the public setup CLI). The occupancy fix merged at `e849013` (0/0/3). **Root confirmation:** Unknown occupancy or execution never unseats any binding, native or cooperative; only positive evidence does (empty shell, a different occupant, absence, an incarnation change). Snapshots cannot detect an occupant exiting while its pane stays open; the next lifecycle hook, pane close or incarnation change does. Model-context fix is 1B: the budget overflow drops the required startup overview. fix1 is dispatched.

Native Codex demonstration 1 (`./native-codex-demo-1/`): BLOCKED by the environment. The aisw Codex profile `default` is at its usage limit until 2026-10-04 22:31; the model was never reached. First live evidence: the setup-codex SessionStart hook fired under 0.159.2 and bound the prelaunch-invited seat with the real session id (cooperative_top_level). Driver defects D4 (a live Codex manifest can never PASS), D5 (failure reason hidden) and D6 (no token accounting). The user must decide whether to use another aisw profile.

Native Claude demonstration 2 (`./native-claude-demo-2/`, unhinted, $0.03): FAIL on P6. The model ran the exact ready command unaided, so P1 is fixed; there was no flapping and PreToolUse stayed quiet. But `setup claude` granted no permission for `herdr-threads` commands, so print mode denied them.
User decisions (2026-09-30):
- `setup claude` installs the owned allow rule `Bash(herdr-threads *)`, removed by `unsetup`, and drops the dead export-prefix rule (this supersedes the earlier allow-rule decision).
- The Codex demonstration uses another aisw Codex profile (the first one with quota), because the `default` profile is at its usage limit until 2026-10-04.

Clippy clean-up (10.5) landed at `267b300`: `-D warnings` passes with and without features, and CI now gates on it. There are 37 justified item-scoped allows and no behaviour change (review 0/0/2).

P6 (`setup claude` installs the owned `Bash(herdr-threads *)` rule and upgrades old installs; review 0/0/3) and driver fix3 (permission preflight; Codex delivery, tokens and failure reasons; `--codex-profile auto`; review B1 fixed by the coordinator in `c8b6f1c` with a killing test) landed at `83997aa`. The driver-script conflict was resolved to driver fix3's version, which subsumes P6's driver edits. Clippy `-D warnings` is clean; focused tests are green. Next: native Claude demo 3 (full phases) and native Codex demo 2 (profile auto).

Skill switch (2026-09-30, user-confirmed request from the superpowers session): from now on this run follows the NEW super-* definitions read directly from `~/code/superpowers` (branch `prompt-guide-audit`, `b5bf5d6`, v6.4.2-alepar3.9; skillsRoot `~/code/superpowers/skills`, read-only), not the plugin cache 6.3.0-alepar3.8. The recorded state maps forward as follows:
- The phase stays `code`.
- The per-lane "one light review + merge at 0 Blocking, park the rest" policy becomes super-code's per-task loop: one relaxed review, at most one fix pass for Critical/Important, merge without re-review, and declined-with-reason findings merged as `parked`.
- Per-merge focused tests are dropped ("nothing runs per merge"). The planned full serial suite before the super-roast becomes the phase-6 sweep (`deferSweep: true`). Merged-tree `cargo check`/`clippy` stays as a cheap build sanity check, since it caught 3 real cross-branch compile seams.
- The original run's "full cumulative audit of all original reports after every NONZERO review" rule is superseded by super-auto §Phase 5's step-back pass (super-design/step-back-prompt.md) plus scope filter, before each code-roast fix round. The existing `history-*` audit directories remain as record.
- Recorded lines with no counterpart in the new contract (audit counters, gate results) are kept as history; `codeBuckets` and `sweep` will be recorded per the new run-state.md at phase 3/6.

Superpowers owner decision (2026-09-30), relayed by the user: super-code keeps a cheap BUILD-ONLY check (no tests) on the merged tree at every serial merge (`config.mergeCheck`). For this run, mergeCheck = `cargo check --locked --all-targets --all-features`, plus `cargo clippy -D warnings` now that the tree is clippy-clean. A failure is a merge failure (blocker path).

**Native Claude demonstration 3** (`./native-claude-demo-3/`, $0.10): with no hint, the real Claude 2.1.285 model accepted the invitation and ACKed its exact message ID on its own. SQLite shows root calls with cooperative_top_level provenance; P6 is fixed; no flapping; PreToolUse stayed quiet. The manifest reported FAIL because of driver defects D7 (substring denial match) and D8 (the demo shim duplicates --state-dir, so ready commands exit 2). Restart and resume did not run. P7: the CLI should accept an identical repeated --state-dir; UX: add a send --body ready form.

Superpowers decision (pending commit): a failing `config.mergeCheck` routes first to one seam fix scoped to the compile errors (it may touch the files the errors name), one scoped seam review, and a rerun of the check. Only then does it fall back to the blocker path. Applied from now on in this run.

**Native Codex demonstration 2** (`./native-codex-demo-2/`, profile codex-1, 3 launches): the ht-910 transport unknown is resolved.
- Under the default workspace-write sandbox, AF_UNIX connects to the daemon get EPERM, so the model cannot act.
- With a narrow config verified default-deny (a unix_sockets allowlist of the exact daemon socket), the manifest PASSes 26/26: the model accepted and ACKed its exact messages unaided on initial and resume, provenance cooperative_top_level, no subagents.
- Product P1: `setup codex` installs no sandbox transport, and the socket path embeds the boot id, so an allowlist goes stale on restart.
- Environment E1: Codex wrote trust_level entries into the codex-1 config.toml (reported to the user).

skillSource updated to 44f9868. A failing mergeCheck now aborts the merge and gets one scoped merge-check fix (it may edit the files the errors name and never weakens tests), plus a read-only review and a re-merge with the check rerun. Otherwise it goes to the blocker path. Merge: check field values are pass | fail→fixed | fail | none.

**Native Claude demonstration 4: PASS** (`./native-claude-demo-4/`, 31/31, $0.19). With no hint and no permission denials, real Claude 2.1.285 accepted the prelaunch invitation, ACKed each exact message ID, and replied with send --body across initial, restart and resume. It ran the ready commands verbatim; provenance was cooperative_top_level and bindings chained cleanly; there was no flapping and PreToolUse stayed quiet. The Claude half of ht-910 and ht-4is.11.4 is demonstrated. New defect P8 (Medium): conflicting repeated global flags misroute to the hook path and exit 0 without doing anything.

Close-out map (read-only, scratch closeout-map.md): 37 open. Closed now: 4.1, 5.3, 6.3, 11.9. Root decisions: 4.3 reworded to the cooperative contract; 11.8 waived for this run (CI runs serially). Codex transport landed at `b0a772e` (stable socket path; `setup codex` emits the allowance only for the measured 0.159.2). User approved the TUI trust-dialog accept for scratch projects only (for /clear and interactive evidence). Remaining: managed-launch wiring (8.4 -> 9), the native evidence matrix (subagent child, mid-turn PreToolUse, warning wake, pagination burst, TUI /clear, Codex via setup transport, 32.3 required membership), 11.5 host identity/recovery on isolated Herdr, the 12.1 parked-findings backfill, the 11.6 evidence report, 10.4 claims, then the sweep and roast.

**Native Codex demonstration 3: PASS** (`./native-codex-demo-3/`, 26/26). It ran under the default workspace-write sandbox, with only the allowance `setup codex` printed. The model accepted, ACKed its exact messages and replied, unaided, on initial and resume. The default-deny negative control holds, the socket path is stable across restart, and denials surface as transport_denied. Both harnesses now demonstrate the core product natively with no hand-built configuration. ht-910 still needs the delegation (child) and /clear evidence from the scenario matrix. P4 (Low UX): the model guessed `participants` as a top-level subcommand. E1 recurs: Codex wrote trust entries into codex-1's config.toml.

Wave 4 landed: managed launch `385e10a` (88ca726, check pass), scenario matrix driver `fda042d` (3e71ace, check pass), host-recovery validation `ab42d5c` (48e7b0c, check pass; the first attempt was a no-op because an identical untracked evidence copy sat in the integration worktree — removed after diff -r showed it identical). 12.1 backfill `e2288e8`: parked-findings.md, 45 open / 9 fixed.

**11.5 host-recovery validation** (`./host-recovery-validation/`, isolated private Herdr 0.9.1, stand-ins, no models): 9 PASS / 3 FAIL. Two P1 product defects filed: ht-4is.11.10 (D1: sends conflict after any host outage — observed_targets epoch stale vs host_epoch) and ht-4is.4.7 (D2: registered seat never follows an observed move; the Stale outcome aborts reconciliation with CursorStale for every seat). Both dispatched as fix lanes; 11.5 closes on a rerun passing R04/R05/R08.

skillSource updated to 3d369ed. The merge now fails closed:
- The integration worktree must be clean, untracked files included. The only exception: byte-identical untracked copies of incoming files are removed and recorded as Merge-cleanup.
- Before mergeCheck runs, the merge must have exited 0 and MERGE_HEAD must exist.

Implementer and fix dispatches now carry a write fence: every write goes inside the task's own worktree. The previous merge, host-recovery `ab42d5c`, would count as Merge-cleanup: its evidence directory was removed after diff -r showed it identical. __pycache__/ is now gitignored, so the clean check needs no exclusions.

**Wave 5, native scenario matrix** (integration `ff81628`, fresh builds):
- **Claude print** (`./native-claude-matrix-1/`, $0.58):
  - PASS: child (ht-910 delegation: a complete sidechain with no child mutation, so PASS at the transcript level; DB-level child separation stays UNVERIFIED per B1), midturn, required (32.3: accept-required, leave refused), and managed launch across initial, restart and resume.
  - Warning: SW2 is UNVERIFIED. The warning was offered at check-in, and a print agent is never idle.
  - Burst: FAIL, caused by driver D1 (the scenario text sits past the 256 B preview). A patched diagnostic rerun passed.
  - Product P1 (Medium): the SessionStart ready commands list accepts for unrelated invitations, and the model ran them.
- **Codex exec** (`./native-codex-matrix-1/`):
  - PASS: midturn.
  - FAIL: child, where the model mistook the coordinator's joined row for its own seat (P2).
  - FAIL: burst, from P1 (the pending-ACK handoff was dropped from the ready commands), the model hand-mistyping the cursor, and P3 (the required-invitation paragraph is always shown).
  - Required: SR2 NOT_EXERCISED because of the preview (D5).
  - Managed: broken. P4 (High): launch appends `--no-daemon` after the prompt. P5 (High): the owned `-c hooks.*` pairs go before `exec`, where Codex 0.159.2 ignores them (verified by probe), so the agent gets no context while launch still reports success.
  - Driver D1–D7. E1: 6 more trust entries in codex-1's config.toml.
- **Claude TUI** (`./native-claude-tui-1/`): only the dry-run ran (23 PASS). The live TUI run was denied by the auto-mode classifier (`[Safety Bypass Flag]`) because the approval came via the workflow text, not the user. A TUI rerun needs the user's own approval.

**11.5 host identity/recovery on tip `fcb37c8`: PASS 12/12 (124 checks)** (`./host-recovery-validation-tip/`), with D1 and D2 fixed. Closed ht-4is.11.5.

**Live Claude TUI attempt** (the user explicitly approved it): it did not reach the model. Claude Code auto-updated to 2.1.286, which is outside recipe [2.1.283, 2.1.285], and the hook failed closed as designed (S02 FAIL, S14/S16R blocked, $0). A lane to capture and widen for 2.1.286 was dispatched; the TUI demo will rerun after it lands.

Wave 6 landed with check pass on every merge:
- managed Codex launch `6111024` (P4/P5: `--no-daemon` first; owned `-c` pairs after `exec`; unconfigurable subcommands refused);
- ready-command ranking `5e69dce` (require-ACK handoff pinned first; optional accepts capped at 2; required paragraph conditional; `self` participant marker; `participants` alias);
- demo driver fixes `d2f38cb` (scenario instructions within the preview; Codex child rollouts; per-step manifest reasons; managed outcome scoped per phase; TUI warning after idle).

Each lane had one review and one fix pass for its Important findings; Minors were appended to parked-findings.md.

Claude 2.1.286 support `20d6288` (lane 37ac5e6, check pass; focused harness, hook and demo tests pass after the auto-merge): the capture showed payloads byte-shape identical to 2.1.285, and the recipe was widened to [2.1.283, 2.1.286] (`./claude-286-hook-capture/`, $0.11).

**Native Codex matrix 2** (`./native-codex-matrix-2/`, tip `d635aca`):
- **PASS:** child (the child rollout was found; no child mutation; ht-910 delegation evidence for Codex), midturn, required (SR2 now exercised), and managed launch across initial, restart and resume (P4/P5 fixed).
- **Warning:** UNSUPPORTED `scenario_not_exercised`, as expected under exec.
- **Burst:** FAIL, from two causes. P6: the handoff invitation's accept line is missing, because the digest lists only the newest-N invitations. D8: the burst text says "accept no other invitation" before the handoff's accept step, which sits past the preview.
- **Confirmed fixed:** P2, P3 and D1–D7.
- **E1:** 6 more codex-1 trust entries.

**10.3 package lifecycle on `d635aca`: PASS** (`./package-validation-tip/`, 89 checks, private Herdr only).

**Native Claude matrix 2 on 2.1.286** (`./native-claude-matrix-2/`, e00a656, $0.48): burst, child, required, midturn and the unhinted default phases (initial/restart/resume) all PASS. The residual P1 (handoff accept missing from ready commands under a burst) is the same defect as ht-4is.8.7, which has a fix lane in flight.

**Live Claude TUI demo 2: PASS for initial and /clear** (`./native-claude-tui-2/`, Claude 2.1.286, the user explicitly approved it). Both phases accepted and ACKed with root calls; SessionStart was delivered after /clear; no subagents. This is the /clear evidence for ht-910. It took four coordinator driver fixes, each merged with a test:
1. the TUI gate records INFO where the launch step expects PASS;
2. `agent_not_found` needs a retry;
3. the 2.1.286 trust dialog opens on 'No';
4. the prompt was sent before the input box existed.

Warning SW0 was NOT_EXERCISED. Herdr reports a finished turn as `done`, not `idle`, and the product's `safe_wake_target` is hard-wired to None (TargetUnsafe). Idle wake therefore does not exist in production, although the design requires it (lines 44/126/128/168). Filed ht-4is.5.6 (P0).

Burst handoff accept fix `f24cac9` (ht-4is.8.7, lane 6ca0b65, CLEAN, check pass). A digest listing the receipt thread's own invitation first is paired with driver D8 text inside the preview.

Close-out audit 2 (`./closeout-audit-2.md`): 18 beads closed, including ht-4is.9, 10.3, 11.5, 32.3, 8.1, 8.2, 8.3, 8.4 and 4.x. Remaining:
- 5.6 safe wake;
- ht-910 and 11.3 (Codex /new clear, lost initial prompt, blocked UI, concurrent children, warning wake in interactive Codex);
- 11.4 (warning wake, lost prompt, blocked UI, concurrent children);
- 10.2 docs (README:55 stale), 10.4 claims, 11.6 report;
- 11.8 waiver, 12.1 disposition, 12 sweep.

**Live Claude TUI demo 3** (`./native-claude-tui-3/`, `979d25b`): the product woke an idle Claude TUI agent 3.1 s after new required mail (outcome submitted); the model woke, handled the mail and ACKed. SW1 PASS (one warning for the late ACK). initial and /clear PASS. SW2 was not separately owed; warning-only wake is covered by R13 on the isolated real host.

User decision (2026-09-30, AskUserQuestion): the driver may accept Codex's interactive folder-trust screen for /private/tmp scratch projects only, recording the codex-1 config.toml diff.

Wave 11 live matrix on `6110698` (`./native-matrix-w11/`):
- Claude TUI lostprompt: **PASS**. The agent was launched with no prompt; the product's idle recovery wake was submitted, and the model woke and accepted and ACKed.
- Claude TUI blockedui: **PASS**. Herdr reported blocked with an approval UI up; the warning wake was refused as `unsafe`, nothing was injected, and the receipt stayed pending.
- Claude children: no child write and the root ACKed, but concurrency was NOT_EXERCISED (the 2 subagents ran sequentially).
- Codex exec burst: **PASS** (8.7 fix confirmed).
- Codex interactive (3 runs): FAIL before launch, at the trust screen (now approved).

12.1 disposition (`./parked-disposition.md`): 100 entries; FIXED 10, FIX-NOW 9 (7 items, fixed in lane 84d2ce8, review CLEAN), ROAST 40, WAIVE 41 (including ht-4is.11.8).

**Native matrix w12–w14** (`./native-matrix-w12/`, interactive Codex with the user-approved scratch trust accept):
- Codex lostprompt: **PASS**. Idle recovery wake submitted; the model accepted and ACKed.
- Codex blockedui: **PASS**. The wake was refused as unsafe under the approval UI; nothing was injected; the receipt stayed pending.
- Codex TUI warning: the idle agent was woken and ACKed late; SW1 PASS. SW2 was not separately owed.
- Codex children: NOT_EXERCISED (the model spawned none).
- Codex /new: **PASS** (w14: SessionStart delivered in the new rollout; new-session ACK by a root call).
- Claude /clear: rerun **PASS** after the clear-before-handoff fix.

Coordinator driver fixes, each with a test:
- `258ad20`: clear before sending the clear-phase handoff, because the idle wake otherwise ACKs it in the old session;
- `de7895d`: input-box wait after /clear and /new.

The codex-1 config.toml grew by trust entries (2574 → 2993 B), as approved.

skillSource updated to 46cc007 (6.4.2-alepar3.10): a manifest-only release; the skill text is identical to 3d369ed.
skillSource renamed: e8244b0 (6.4.2-alepar4.0) replaces the deleted 3.10 tag; the skill text is unchanged.

sweep: `cargo test --locked --all-targets --all-features -- --test-threads=1` on `99ea74c`: PASS (exit 0; lib 1122 passed / 16 ignored; every integration target ok; 0 failed). The stamp is invalid if product source changes after 99ea74c.

User decision (2026-09-30, after a live trial of setup): `setup` installs at USER level only, following Herdr's own convention. Claude goes in ~/.claude/settings.json (hooks plus the `Bash(herdr-threads *)` allow rule). Codex goes in ~/.codex/hooks.json, plus the narrow sandbox allowance in ~/.codex/config.toml (the user chose user-level for it too). Project mode is removed, and the Herdr state dir and socket are auto-detected. Filed ht-4is.8.8.

User 2026-09-30: release targets are every platform Herdr and Rust both support (macOS arm64/x86_64, Linux x86_64/aarch64). Linux is labelled experimental; the user will confirm it on their own Linux box, so the installer and scripts must stay portable.

**Native revalidation on `e6ac1985`** (`./native-matrix-w23/`, after the user-level setup, operator mode, short IDs, skill and human output landed; scratch CLAUDE_CONFIG_DIR/CODEX_HOME): all four runs **PASS**:
- Claude print default phases (initial, restart, resume);
- Claude TUI with /clear and lostprompt;
- Codex exec default phases;
- Codex interactive with /new and lostprompt.

sweep: full serial suite on `5c864c2f` (post-UX tip): PASS (exit 0; lib 1158 passed / 16 ignored; every integration target ok). It will be re-run on the final tip before the roast.

User decision (2026-09-30): keep launch's Codex `--no-daemon` (it keeps Codex tool shells inside the pane, so they inherit Herdr identity; the rationale is assumed from the user's environment, not measured) and dedupe it against a shell wrapper that already passes it.

Phase 3 (code) complete: every implementation, validation, packaging and UX bead is closed. Since the last report, the user's live trial and the screencast work produced these additions:
- user-level setup with instance auto-detection and bare `setup`;
- operator `me init` and `--pane` names;
- short IDs;
- the agent skill;
- human and IRC output, plus `read --follow`;
- the release workflow and installer;
- the token diet: short cursors and compact agent output;
- bare ready commands;
- readable agent names;
- honest health (cooperative = healthy) and the scheduler-tick fix;
- Codex --no-daemon dedupe;
- Codex 0.159.3 measured;
- Codex sandbox writable roots (P0, with probe evidence);
- launch fixes (prompt limits, refusing multi-line arguments, confirming working agents);
- a follow clock bug;
- copied-hook adoption;
- the README with the mad tea party screencast (docs/media).

The full serial sweep on the tip `384a663c` is running. Phase 4 (super-roast, PR mode, iteration 1, new skill definitions) follows.

sweep: full serial suite (--all-targets --all-features --no-fail-fast, --test-threads=1) on `384a663c`: PASS (exit 0; 13 targets, 0 failed). This is the gate the user asked for before the super-roast. roastCodeRound: 1 (PR mode, new super-roast definitions, inputs super-auto@384a663c vs main@7c83543).

roast-code: ./2026-10-01-herdr-native-mailbox-thread-plugin-roast-pr-1.md (round 1 verdict: Should-fix, 24 confirmed: 0 Blocking, 10 Should-fix, 14 Nit). coverage: 13/13 scouts, 96 deduped, 44 panels, judge completion 100%, independence same-family (claude). roastCodeRound: 1. Phase 5 round 1: step-back, then scope filter, in progress.

stepBack-round-1: patch — round 1, so nothing recurs yet. Three loose clusters ((a) discovery not using the pending/indexed projections, (b) silent lane health, (c) fixed-interval polling), each already covered by a stated design rule; sweep each cluster consistently, no redesign. (./2026-10-01-herdr-native-mailbox-thread-plugin-roast-pr-1-step-back.md)
scopeFilter-round-1:
  [Should-fix] src/store/mod.rs:838 → punch-list — Wake-attention rebuild cost growing with history is a performance/scaling quality issue; results stay correct.
  [Should-fix] src/cli/hook.rs:716 → in-scope — Seat lookup silently returns no seat after ~800 seats, breaking goal-named seat identity and pane-bound roles.
  [Should-fix] src/store/queries.rs:2126 → in-scope — Body paging returns InvalidBudget or wrong pages though the body fits, a correctness defect in goal-named inspectable, context-efficient communication.
  [Should-fix] src/cli/input.rs:7 → in-scope — CLI accepts bodies the store always rejects, leaving unsendable durable intents in the message send path the goal names.
  [Should-fix] src/store/seats.rs:702 → punch-list — Unbounded snapshot retention growth is a resource/retention hardening concern the goal does not name.
  [Should-fix] src/identity/reconcile.rs:885 → punch-list — Missing backoff on observation retries is efficiency hardening, not incorrect goal-named behavior.
  [Should-fix] src/store/mod.rs:689 → punch-list — Work-job discovery scanning completed rows is a scaling inefficiency; discovery remains correct.
  [Should-fix] src/app.rs:446; src/app.rs:192 → in-scope — A panicked deadline/wake lane silently stops while Health reports Ready, so goal-named durable deadline warnings can die with a misleading health result.
  [Should-fix] src/main.rs:52 → punch-list — Startup stderr diagnostics are an operability improvement outside the goal-named behaviors.
  [Should-fix] src/ports.rs:1292 → punch-list — Unused port code and dead-code allowances are a code hygiene issue, not a correctness defect.
  [Nit] src/service/workers.rs:857 → punch-list — Materialization throughput and tick wiring is a performance improvement, not a goal-named correctness defect.
  [Nit] src/store/queries.rs:990 → punch-list — Quadratic page sizing is a performance quality issue with correct output.
  [Nit] src/daemon/transport.rs:67; src/daemon/transport.rs:68 → punch-list — Idle polling efficiency is not a goal-named behavior.
  [Nit] src/store/connection.rs:550 → punch-list — Error-code mapping precision is diagnostics polish outside the goal.
  [Nit] src/service/workers.rs:625 → punch-list — Diagnostic logging of worker failures is observability improvement not named by the goal.
  [Nit] src/service/workers.rs:599 → punch-list — Poisoned-mutex health edge case is observability hardening, unrelated to a goal-named behavior.
  [Nit] src/cli/journal.rs:467 → punch-list — O_NOFOLLOW hardening for a sandbox symlink case is security hardening outside the goal and the stated cooperative-identity contract.
  [Nit] src/daemon/lifecycle.rs:181 → punch-list — Version-skew error mapping in the ensure handshake is not goal-named behavior.
  [Nit] .github/workflows/ci.yml:101 → punch-list — CI test-harness hygiene, not a goal-named behavior.
  [Nit] tests/store/attention.rs:277 → punch-list — Test isolation of cost-flatness tests is test hygiene, not a failing test for a goal-named behavior.
  [Nit] Cargo.toml:1 → punch-list — Missing license is repository/release hygiene outside the goal's behaviors.
  [Nit] scripts/package-release.sh:46 → punch-list — Third-party notices in release archives are packaging compliance, not a goal-named behavior.
  [Nit] docs/install.md:67 → punch-list — Committed run evidence bloat is repo hygiene.
  [Nit] docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/task-tree-roast-design-1.json:10 → punch-list — Personal data in committed run evidence is repo hygiene unrelated to goal-named behavior.
scope-filter: 4 in-scope · 20 punch-listed
fixBeads-round-1: ht-4is.33 (06773f5a) · ht-4is.34 (a623ae86) · ht-4is.35 (f813875f) · ht-4is.36 (8030b5e2) — all merged via guarded-merge (check+clippy pass), closed
gate-round-1: full serial suite @06773f5a — 14 result lines, 0 failed
roastCodeRound: 2 (PR roast iteration 2 of 3 against 06773f5a, prior report roast-pr-1, regression lane added)
roast-code: ./2026-10-01-herdr-native-mailbox-thread-plugin-roast-pr-2.md (round 2 verdict: Should-fix (23 confirmed) [converged] · 0 Blocking · delta 3 new / 20 carried / 4 resolved / 0 regressed · coverage 14/14 scouts incl. regression lane, judge completion 100%, independence same-family (claude)). roastCodeRound: 2. Fix loop exits on [converged]; all 23 sub-Blocking findings parked to report.md Remaining (no round-2 scope filter: nothing is filed on converge).
fixLoopExit: converged at round 2 · new round-2 findings introduced by round-1 fixes: src/cli/retry.rs:119 (020055fa), src/cli/hook.rs:734 (eed544e7) — parked, flagged for the merge decision
sweep: full serial suite `cargo test --locked --all-features -- --test-threads=1` on `06773f5a`: PASS (exit 0; 14 result lines, lib 1214 passed / 16 ignored, 0 failed). Tip 06773f5a..HEAD changes only docs/superpowers/runs/ and docs/validation/integration-sweep.md, so the stamp holds for the reported tip.
integrationSweep: ht-4is.12 closed at 54bd8040 (docs/validation/integration-sweep.md @06773f5a: MET 13 / MET-WITH-GAP 12 / NOT-MET 0; open gaps B1 inert ports (ports.rs:1292), B2/G0 native evidence stamped at older SHAs, B3 owed native cells, B4 round-2 Should-fix mapped to R3/R16/R17/R18/R20)
codeBuckets (refreshed from tracker at phase 6):
  completed: all 125 descendants of ht-4is closed (80 task, 19 bug, 13 feature, 11 epic + ht-4is.10, ht-4is); fix-loop round 1: ht-4is.33, ht-4is.34, ht-4is.35, ht-4is.36
  escalated: [] (ht-4is.2.3, ht-4is.3.5 since resolved and closed; blockers ht-fy0, ht-3xy, ht-z7j, ht-910 closed)
  pendingRetry: []
  parked: [] (code-review minors parked in ./parked-findings.md, disposition ./parked-disposition.md)
  stalled: false
  review: whole-branch PR roast round 2 [converged] (super-roast replaces finalReview in this run)
  sweep: PASS @06773f5a
skillSource: ~/code/superpowers/skills @ 4cd542b (6.4.2-alepar4.2) — switched at the round-2 boundary on the human's relayed maintainer instruction; run-state.md and super-auto §Phase 5 re-read. super-roast now ships scripts/assemble-args (replaces the coordinator's build.js for any later roast).
migrated: round-2 report predates the engine's [fix-regression] tag; its two findings marked "new; introduced by commit <round-1 fix>" (retry.rs:119 ← 020055fa, hook.rs:734 ← eed544e7) are treated as [fix-regression]. The third new finding (seats.rs:1217) is not fix-introduced and stays on the punch list.
regressionPass note: report.md written at 3f34be44 was revised after the pass
regressionPass-round-2: 2 filed · [Should-fix] src/cli/retry.rs:119; src/cli/mod.rs:1262; tests/harness/bridge.rs:1291, [Should-fix] src/cli/hook.rs:734 · no re-roast — beads ht-4is.37, ht-4is.38
regressionPass-round-2 result: ht-4is.37 merged de0c4e0e (+ integration follow-up ad905554 scoping the allow-list to cooperative intents, caught by the phase-6 suite), ht-4is.38 merged 2be3b98c; both closed; ht-4is re-closed. No re-roast (per §Phase 5).
sweep: full serial suite `cargo test --locked --all-features -- --test-threads=1` on `ad905554`: PASS (exit 0; 14 result lines, lib 1217 passed / 16 ignored, 0 failed). An earlier run at 2be3b98c FAILED 1 (new_resolution_discards_only_definitively_rejected_first_submission) → fixed in ad905554. Commits after ad905554 change only docs.
codeBuckets.sweep: PASS @ad905554
integrationSweep update: docs/validation/integration-sweep.md @ad905554 — MET 14 / MET-WITH-GAP 11 / NOT-MET 0 (R16 back to MET)
feedback: parked draft ./upstream-feedback-draft.md (not filed; awaits the human at the phase-7 menu)
feedback: not filed — ./upstream-feedback-draft.md addressed upstream in v6.4.2-alepar4.3 (5c92bce) per the maintainer reply relayed by the human; Design Q B stays open with the owner. skillSource not switched again (run is at phase report; nothing left to execute under the new definitions).
finish: human chose (c) release hygiene first, then merge. Archive tag archive/herdr-threads-run-2026-09-26 (local) keeps full history + evidence. License MIT OR Apache-2.0 + third-party notices in release archives (7aca0829); run evidence trimmed to docs/design, docs/history, docs/evidence and scrubbed of owner email/home paths (28992889); README GIF shrunk 7.2→5.3 MB (984d9ddb).
sweep: full serial suite `cargo test --locked --all-features -- --test-threads=1` on `28992889`: PASS (exit 0; 14 result lines, lib 1217 passed / 16 ignored, 0 failed).
merged: squash-merged into main (local only, not pushed) on 2026-10-01 at the human's choice.
