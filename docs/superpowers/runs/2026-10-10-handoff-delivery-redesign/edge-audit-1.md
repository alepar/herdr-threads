# Edge audit 1 — report only

Observed at 2026-10-11T02:01:00.590319+00:00. One bounded canonical `bd show` query covered root, four subepics and the 11 mapped implementation/sweep leaves. Dependency analysis uses only actual `blocks` edges; `parent-child` edges are ownership, not predecessor work. No edge/status/source/worktree mutation performed.

## Frontier and critical path

- Nonterminal mapped leaves: **8** (in-progress leaves included).
- Remaining critical-path depth: **4 leaf completions**.
- Average achievable width requested by detector: **8/4 = 2**, below configured cap **4**.
- Maximum graph antichain/frontier width: **3**, also below cap4; example `ht-0lo.4.1, ht-0lo.1.3, ht-0lo.2.2`. This is a theoretical structural ceiling, not a claim all those leaves are currently ready.
- Canonical open-ready leaves: **0**. The three remaining source leaves are already in progress: `ht-0lo.4.1, ht-0lo.1.3, ht-0lo.2.2`. Parent reports their actual stages as Task3 integration, Task5 GREEN/review, Task6 fix.
- Longest remaining chains: Task5 `.1.3` → Task7 `.1.2` → Task10 `.3.2` → Task11 `.5`; and Task6 `.2.2` → Task7 `.1.2` → Task10 `.3.2` → Task11 `.5`. Task3 `.4.1` → Task9 `.4.2` → Task11 is three completions; Task6 → Task8 `.2.3` → Task11 is three.

## Actual remaining graph and acceptance consumption

| Task | Canonical state | Nonterminal blockers | Acceptance consumed / edge disposition |
| --- | --- | --- | --- |
| 3 (ht-0lo.4.1) | in_progress | none | Stable event grammar/API producer; no remaining predecessor. Review/integration is work remaining, not a suspicious dependency. |
| 5 (ht-0lo.1.3) | in_progress | none | Historical repair producer consumes closed send-time projection. Its own local repair accounting/completion tests remain prerequisite for downstream global bound. |
| 6 (ht-0lo.2.2) | in_progress | none | Invitation lifecycle/class-cursor producer consumes closed wire/schema contract. Its own isolated behavior is required before consumer execution. |
| 7 (ht-0lo.1.2) | open | ht-0lo.1.3, ht-0lo.2.2 | REAL producer-consumer edges to historical repair (.1.3) and invitation class cursor (.2.2). H/D/I global fairness and coalesced sender routing acceptance consume those landed contracts. |
| 8 (ht-0lo.2.3) | open | ht-0lo.2.2 | REAL invitation store/query/result metadata consumer (.2.2); CLI lazy default, presentation and watch attention cannot precede active lifecycle. |
| 9 (ht-0lo.4.2) | open | ht-0lo.4.1 | REAL stable grammar/API consumer (.4.1), specifically installed_event_argv/native_event_command. Setup ownership migration cannot substitute a guessed interface. |
| 10 (ht-0lo.3.2) | open | ht-0lo.2.2, ht-0lo.1.2 | REAL overdue-warning lifecycle (.1.2) and invitation lifecycle (.2.2) consumers. Direct .2.2 edge is transitively redundant through .1.2, but explicitly documents independent policy consumption and imposes no extra wait. |
| 11 (ht-0lo.5) | open | ht-0lo.2.3, ht-0lo.2.2, ht-0lo.1.3, ht-0lo.3.2, ht-0lo.4.1, ht-0lo.4.2, ht-0lo.1.2 | TERMINAL integration sweep consumes all implementation leaves. Several direct edges are transitively redundant by design but impose no extra critical-path work; full integration acceptance justifies them. |

## Suspect narrative / epic edges

- **No open blocking epic edges, cycles, narrative-only predecessor edges, or unjustified implementation ordering found.** Root and four subepics are open ownership containers with parent-child links only. None gates a mapped leaf as a `blocks` predecessor.
- Task7→Task6 is the explicitly settled class-scanner consumer edge, not an invitation for Task6 to depend on its future combined fairness tests. Task6’s older broad shared-fairness acceptance text must be read with the recorded producer/consumer ruling; it does not establish a reverse edge or require duplicate shared-driver implementation.
- Task11 retains closed design-only `.6/.7` blockers and closed implementation blockers. These historical edges are already satisfied and do not narrow the current frontier. No removal is proposed.
- Task10 retains direct lifecycle and closed wire/projection/body-source edges, some transitively redundant. Each names a real contract, and redundant satisfied/transitive edges do not add work to the remaining DAG.
- Task9 and Task10 may touch shared launch/setup-consumer surfaces; existing exact API preservation, isolated worktrees and serial seam review govern that overlap. File overlap alone does not justify another dependency.

## Conclusion and ledger line

The low ready frontier is explained by three already-active producers plus real downstream chains and the final join, not excessive narrative/epic dependencies. Cap4 cannot be fully utilized by this remaining DAG; average width2 and maximal frontier3 are structurally expected. Do not prune meaningful producer edges merely to inflate readiness. Parent owns finish/final readiness; this audit requests no changes.

Edge-audit 1: report-only; openLeaves=8 depth=4 achievableWidth=2/cap4 maxFrontier=3 ready=0 activeSources=3; no suspect narrative/epic blocks; Task7←Task6 and Task9←Task3 are real API consumers; no changes.
