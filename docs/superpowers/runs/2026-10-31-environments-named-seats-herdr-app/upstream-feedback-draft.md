# 2026-10-31-environments-named-seats-herdr-app: a malformed archive in an unrelated older run kept every whole-range review from running

Plugin: 6.4.2-alepar6.6. Run: 222 beads (1 epic, 136 work, 85 `review:`), 128 tasks landed, 8 coordinator invocations, about 27h of profiled coordinator wall time and 1304 agents, 2026-10-31 to 2026-11-02 (UTC, profile timestamps).

Paths: `RD/` is the run directory, `SDD/` is `.superpowers/sdd/ht-kqrt-plan/`, `skills/` is the plugin's skills directory.

## Defects

### 1. A malformed failed-assembly archive in an unrelated, unchanged older run makes review-input selection unavailable for the whole branch, so no whole-range review ran
- **Evidence:** `SDD/assembled/final-review-inv-8-final/review-inputs.json`: `status: unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, `changed` 661, `slices` 0, `archives` 16, `omitted` 0, `gaps` 4. All four gaps have this form: `docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md:51 inv-8759cc82-…: invalid failed assembly archive: failed bounded archive contains successful assembly or split claims` (lines 51 and 58, at base and at head). `git diff de5af6736 573c76b5e -- <that run dir>` is empty. Friction: `assembler: unavailable — review input selection: provenance unavailable; … code roast r1 runs as a bounded direct review, verdict incomplete for acceptance`. All 4 code roasts report `[low coverage] [review readiness incomplete]`. All 8 final reviews report `not ready (unreviewed: whole-epic coverage and omission provenance remain incomplete)`. Report coverage: "The union of all eight is 145 of 661 paths, 141 of 176 product paths." inv-8 found a new Must-fix in `src/service/delivery_presentation.rs:85`, "a file this epic did not change, which is why no task review saw it".
- **Premise to verify:** that a gap in a run directory unchanged in base..head cannot affect omission provenance for this range; and why the 2026-10-17 archive is invalid (possibly written by an older plugin version or on another host, since its workspace is under a different home).
- **Suggested fix shape:** scope the archive scan to run directories changed in base..head, or confine a gap to the paths that archive governs, so one bad record degrades only those paths; name a repair or waiver path for a historical bad record.

### 2. The ledger read relays the digest through a model-authored structured result, which fails at this ledger size, so run-wide metrics were unavailable in 6 of 8 invocations
- **Evidence:** the digest was 133,798 bytes (`# ledger-digest: 765 of 1671 lines`), against the comment at `coordinator.js:2959`: "so the echo stays small however long the ledger grows". `Metrics: UNAVAILABLE (merges) — the Finish ledger re-read returned null or unavailable transport (malformed)` appears for inv-3 through inv-8. Transcripts of 31 read-ledger dispatches found: 5 ended in `refusal` ("safeguards flagged this message"). Many of the rest returned prose in `text`: "Ledger digest not relayed in full", "my transcription of the 721-line out…", "[TRUNCATED BY SUBAGENT: …", which `checkDigest` classifies as malformed. Friction: "one retry was flagged [Auto-Mode Bypass]; its transcript shows read-only commands only". Downstream, final review inv-8 says "ledger absence proves nothing".
- **Premise to verify:** that Finish needs only aggregates from the digest, not the full lines. The Resume read and Recovery handling may need more.
- **Suggested fix shape:** have a script compute the metrics and print a small bounded result for the agent to echo, keeping the full digest on disk; never route ledger-sized text through a model.

### 3. One process with an unreadable `/proc` entry poisons every cleanup root, so merged worktrees are never removed for the whole run
- **Evidence:** 125 of 128 `Cleanup:` ledger lines record exit 4 or `process cleanup incomplete`, for example `unknown: inspection failed for processes 1709587, 1824140, 1891001, 1901, 1938, 3080385`. `Slowness: merged task worktrees never removed (remove-task-worktree exit 4 …; disk 97 GB free)`. It was followed by `disk fell 100 -> 55 GB free in ~40 min … unplanned stop of the coordinator (in-flight ht-kqrt.10/.14/.5/.53/.7/.73 interrupted…)`. The same exit reached task setup: `Task 60 (ht-kqrt.57): pending retry [cause: stack conflict surfaced as SETUP_FAILED because discard helper exited 4]`, and the same for Task 66. The orchestrator's substitute reaper then misused a helper and removed three unmerged clean worktrees (branches kept). `Merge: … → held: … task worktree … does not exist`.
- **Premise to verify:** the friction log calls the blocker a root-owned sshd-session, but `stop-run-processes` skips any process whose `/proc/PID` owner differs from the caller. So the failing processes passed that test and then failed `readlink cwd`. The likely case is same-uid non-dumpable processes. Check this on a host with an sshd-session before choosing a fix.
- **Suggested fix shape:** classify a permission-denied process as not inspectable and exclude it with a per-process note, leaving exit 4 for real enumeration failures and per-process races; keep discard failures from masking the original setup outcome.

### 4. Stacked tasks built on a parent's pre-rebase commit fail at setup after the parent lands rebased, and the profile records no stack bounce
- **Evidence:** `Task 50 (ht-kqrt.70): waiting on ht-kqrt.108`: "stacked … on an older ht-kqrt.15 commit, 2254d58b8. The integration branch instead merged a different, rebased ht-kqrt.15 commit". Task 46: "Merging both into the .46 branch fails because task-ht-kqrt.18 was built on .16's first commit". Task 60: "the setup merged the pre-rebase ht-kqrt.134 tip". `Profile: rework — 3h56m of dispatch time in attempts that did not land — ht-kqrt.46 1h31m (SETUP_FAILED, ?…) · ht-kqrt.70 1h09m (SETUP_FAILED, ?…)`. inv-3: `first try not merged for 29 of 43`. Yet every invocation reports `bounced on a stack conflict then waited for the merge 0 (0s)` and `stack conflicts 0`.
- **Premise to verify:** the shares of those attempts spent in setup versus the following attempt are not separated in the profile.
- **Suggested fix shape:** resolve a stack parent to its landed commit when one exists before merging it in, and classify the setup failure as a stack conflict so the profile counts it.

### 5. Roast scouts in one run share scratchpad pointer files, so a mutable step can reach a sibling's lane
- **Evidence:** friction: "r3 regression scout force-removed and recreated a sibling review worktree after reading a shared scratchpad pointer; left 126G target behind". Also: "post-cap audit scout ran check-no-leaked-processes --kill with a run id from a shared scratchpad pointer and killed a sibling's stress loop". The audit's run notes say a FAILED from that loop "around 22:39-22:44" is not evidence of a flake. `scout-prompts-pr.md:27-28` require a unique worktree and a separate output/temp directory, but say nothing about pointer files or kill-class cleanup.
- **Premise to verify:** that the shared location is the harness session scratchpad, which all seats in a Workflow inherit, not a path the skill names.
- **Suggested fix shape:** give each scout a seat-keyed scratch directory in its prompt, require pointers there, and forbid destructive cleanup of anything the seat did not create in that lane.

### 6. Concurrency caps bound agent slots while the binding resource in this run was disk and tmpfs
- **Evidence:** `disk fell 100 -> 55 GB free in ~40 min (≈49 GB written into 6 task targets)`; `relaunch at concurrency 3 with CARGO_INCREMENTAL=0`; `Human action: ht-kqrt.15 — Free disk space … it is at 0 MB free`. Friction: "/tmp tmpfs filled to 32 GB at least three times … a task implementer (ht-kqrt.16) seeded a target/ under its TMPDIR …; super-code's per-task TMPDIR under /tmp invites this". The cap stayed at 4 afterwards, with peak all-agent concurrency 6-8 of `16 runtime slots`. Slot waits (summed per bead, not wall time): 6h00m at inv-3, 2h29m at inv-4, 13h32m at inv-5, 3h05m at inv-6. Modelled: `unlimited slots → 7h05m (−54m, 11%)` at inv-3 against model-vs-actual 99%.
- **Premise to verify:** that free-space checks at admission are cheap and portable across hosts, and that build-output size per task is predictable enough to set a floor.
- **Suggested fix shape:** add an optional free-space floor for the workspace and temp volumes, consulted at dispatch, which holds new work instead of forcing a stop; state in implementer guidance that build output never goes under the per-task temp root.

### 7. Root-acceptance guidance requires a snapshot before code that the validator cannot accept
- **Evidence:** `run-state.md:265` says `sourceTree` is "the full Git tree of the reviewed/measured code", and the next line says "Write the initial snapshot before code". `report-status:90` throws `invalid source/owner/version` unless `sourceTree` is a hash. The run's two snapshots carry `"sourceTree": "pending"`. The report's status line: `root acceptance pending: root acceptance record: invalid source/owner/version; caller acceptance requirements unavailable`.
- **Premise to verify:** whether a later snapshot was meant to replace the initial one, and whether the checks fail open or closed before that point.
- **Suggested fix shape:** define an explicit pending form the validator accepts, or defer `sourceTree` to the first bound snapshot; have the error name the failing field.

### 8. The report's status line is unbounded, headlines "0 unresolved Blocking" over open Must-fix items, and its outstanding count ignores resolved findings
- **Evidence:** the `status:` line is 47,306 bytes, with 384 `finding acceptance relevance unproven` entries and 12 `source coverage` entries. It begins `status: completed with 0 unresolved Blocking`, while the report body lists three open Must-fix items from final review inv-8. `scope-dispositions --project`: `findings 313 … repaired 0 … outstanding 313`, while the report notes `RD/findings-ledger.md` has 34 rows `resolved@r3` and "this counts findings since fixed as well … Treat it as an upper bound."
- **Premise to verify:** `SKILL.md` says "Neither a later ready verdict nor a merged fix bead resolves an earlier finding", so `repaired 0` may be intended when no dispositions are recorded. The question is then who records them in a run that ends past the cap.
- **Suggested fix shape:** render counts with a pointer in the status line, include final-review Must-fix count in the headline, and record dispositions from the findings ledger where its resolved rows are source-bound.

### 9. The report drafter's required file write is refused for subagents in this harness
- **Evidence:** `report-prompt.md:7` requires the draft at `<run-dir>/report.draft.md`, and the orchestrator splices it "without reading the body". Drafter's final message: `drafted, NOT written — the file does not exist: the Write tool refused report.draft.md ("Subagents should return findings as text, not write report files"). I did not route around that refusal through Bash, so the full draft is inline below`. So the body passed through the orchestrator's context, which the contract meant to avoid.
- **Premise to verify:** what triggers the refusal (name, extension or a policy on all subagents), and whether it applies on all harness versions.
- **Suggested fix shape:** make the contract accept an inline draft, or have the orchestrator persist the drafter's returned text without a model step.

### 10. `already-merged --landed` walks the whole first-parent history, so metrics count other epics' tasks
- **Evidence:** on this branch it printed 312 landings: 128 `ht-kqrt` and 184 others (`ht-54r` 59, `ht-yha` 57, `ht-j16` 26, `ht-j87z` 20, `ht-76u6` 14, `ht-4ne` 8). The `Metrics: ledger-check` line for inv-2 reads `ok` and carries all 184 as `git-only`, and `coordinator.js:2336` pushes `git-only` into metrics gaps. The script calls `git log --first-parent --merges` with no base limit.
- **Premise to verify:** whether a run ever needs history below the base (a re-entry with a moved base, for example).
- **Suggested fix shape:** add a base argument or filter by epic prefix.

### 11. The sliding-window coordinator offers no way to quiesce before a planned stop
- **Evidence:** friction: "unplanned coordinator stop for disk (per coordinator-agreed 50 GB guard; no Detector/round boundary occurred within the first ~2 h of the sliding window, so the planned quiesce point never arrived); in-flight work of 6 tasks interrupted". Also the user's "lets pause once the workflow finishes", then a mid-run stop recorded in `resumeChange`.
- **Premise to verify:** that a running Workflow script can observe an external signal at admission time through a dispatched check without a meaningful per-dispatch cost.
- **Suggested fix shape:** an admission-hold marker checked at top-up: stop dispatching, drain in-flight work, return.

### 12. `context-preflight` warns of a 200K window that the run measured to be wrong
- **Evidence:** `run.md`: `planner=200K/167K … warn: planner, finalReview on a 200K model`. `context-preflight:148` assigns 200000 to any model id without `[1m]`. Profile: `plan peak 525K/1000K`, `plan-rest peak 505K/1000K`, `final-review peak 521K/1000K`, and `Claude context windows from the model table: claude-opus-5-5 1000K`.
- **Premise to verify:** the Profile's 1000K is itself from a model table; only the >200K peaks are measured.
- **Suggested fix shape:** take the window from the same model table the profiler uses.

## Run metrics

### Judge panel
- design, iteration 1 of 3, same-family (Claude) seat-differentiated panel, rung Workflow: `panels 36 · rr 0.81 · rg 0.75 · fg 0.67 · unanimous 0.61 · ground-loo 0.76 (n=29) · reproduce 13/22/1 · refute 6/29/1 · ground 17/18/1`
- design, iteration 2 of 4, same independence: `panels 11 · rr 0.91 · rg 0.64 · fg 0.73 · unanimous 0.64 · ground-loo 0.70 (n=10) · reproduce 6/5/0 · refute 5/6/0 · ground 8/3/0`; delta `5 new confirmed (0 Blocking) · 0 carried · 11 resolved · 0 regressed · 0 punch-listed`
- pr, iteration 1 of 3, same independence: `panels 62 · rr 0.84 · rg 0.79 · fg 0.73 · unanimous 0.68 · ground-loo 0.81 (n=52) · reproduce 39/23/0 · refute 35/26/1 · ground 50/12/0`
- pr, iteration 2 of 3, same independence: `panels 24 · rr 0.92 · rg 0.92 · fg 0.83 · unanimous 0.83 · ground-loo 0.91 (n=22) · reproduce 22/2/0 · refute 20/4/0 · ground 24/0/0`; delta `19 new confirmed (0 Blocking) · 0 carried · 26 resolved · 4 regressed · 9 punch-listed`
- pr, iteration 3 of 3, same independence: `panels 16 · rr 0.94 · rg 0.88 · fg 0.94 · unanimous 0.88 · ground-loo 0.93 (n=15) · reproduce 14/2/0 · refute 15/1/0 · ground 16/0/0`; delta `14 new confirmed (0 Blocking) · 0 carried · 18 resolved · 1 regressed · 3 punch-listed`
- pr, post-cap audit, same independence: `panels 11 · rr 1.00 · rg 1.00 · fg 1.00 · unanimous 1.00 · ground-loo 1.00 (n=11) · reproduce 11/0/0 · refute 11/0/0 · ground 11/0/0`; delta `8 new confirmed (0 Blocking) · 0 carried · 1 resolved · 3 regressed · 11 punch-listed`

### Fix loop
- inv-1: `Metrics: completions — review clean 13 · after fix pass 0 · parked 0 · re-entry closes 0 · dispatched early 0 · cancelled 0`; `Metrics: fix-pass — entered 1 · FIXED 1 · BLOCKED 0 (legacy event lines); durable attempts 1 · historical FIXED 1; repaired findings unavailable (use scope-dispositions --project)`. inv-2 repeats the same counts.
- inv-3 through inv-8: `Metrics: UNAVAILABLE (completions)` and `UNAVAILABLE (fix-pass)`. Per-invocation memory counts in those lines: completed 19 (parked 2) · escalated 2 · FIXED 6; 27 (1) · 0 · 4; 25 (1) · 0 · 3; 11 (0) · 0 · 0; 7 (0) · 0 · 1; 2 (0) · 0 · 0. Labelled "this invocation only" and "known historical", so not summed here.

### Merge-back
- inv-1: `Metrics: merges 13 · merge-failed 0 · rebase-conflicts 0 · seam-reviews 2 (fixed 0) · check-fails 0 (fixed 0)`; `Metrics: ledger-check landed≠completed: 13 vs 8 — ledger-only [ht-kqrt.1, .12, .2, .3, .37]`, plus 184 foreign `git-only` ids (Defect 10).
- inv-2: `Metrics: ledger-check ok`, same foreign `git-only` ids. inv-3 through inv-8: `Metrics: UNAVAILABLE (merges)`, `UNAVAILABLE (ledger-check)`.
- `Merge:` lines counted directly from the ledger: 129. Conflict 6, seam-review fired 49 (cleared 45, fixed 4), check fail→fixed 1, check fail 0, held 1 (ht-kqrt.14, later landed), `→ blocker` 0.
- Reconciliation: 128 distinct IDs in `already-merged --landed` (ht-kqrt only), 128 in success-path `Merge:` lines, 128 in `complete` records. Ledger-only none, Git-only none within the epic, completed-only none. The 184 foreign landings are out of scope. Profile landings per invocation also sum to 128: 13, 0, 43, 27, 25, 11, 7, 2. `baseAbsorbed` contributes no landing. `codeBuckets.completed` still reads 83, its value at inv-4.

### Coverage
- Requirements: design coverage rounds record `requirements: 45` (rounds 1 and 2). The amend-design round's count is in `coverage-round-amend-design-1-requirements.md`, not read for this draft.
- Scope filter: `scope-filter: 28 in-scope · 11 punch-listed`, `scope-filter: 19 in-scope · 3 punch-listed`.
- `cluster dropped: locate-verdict-cache`. `graph-pass: depth 18→15 · width 3.8→4.6 · applied 2 · parked 2`.

### Timing
Lightly trimmed from the ledger; the remainder is in `RD/profile-inv-{1..8}.md`. What-if lines are schedule-model estimates, each to be read with its `bound` model-vs-actual. Compaction causes are classified, not logged. Detector: 13 lines, cap 3-4 after the disk stop, one round with `top-up queries 40/40`, peak in-flight at cap in most rounds.

inv-1
- `Profile: ht-kqrt · 2026-10-31T23:14Z → 2026-11-01T02:40Z · wall 3h26m (task graph 2h27m, after the last landing 59m) · agents 200 · beads dispatched 19, landed 13, planned 71`
- bound: `graph lower bound 2h04m along ht-kqrt.5 → ht-kqrt.8 · model of the run 2h05m (cap 6, one merge lane) vs actual 2h27m (85%) → graph-bound`
- critical path: `2h27m = implement 1h19m · review 2m · merge 5m · planning 33m · ready 1m · startup 4m · dispatch 28s · wait 22m`
- bottleneck: `ht-kqrt.5 — 42m (29%) · implement 38m · review 41s · merge 3m · 2 attempts · dependents 7 direct, 47 transitive`
- waits: `2h05m over 9 of 13 — planning 17m (4) · retry 21m (2) · unexplained 1h27m (3)`
- merge lane: `busy 49m of 2h15m (36%) · seam 1m · 17 merge dispatches, first try not merged for 2 of 13 · queue wait 3m, peak 1`
- what-if: `faster ht-kqrt.5 → 2h03m (−1m, 1%)`
- context: `agents 201 · peak≥80% 0 · tasks>1 7 · compacted 0 · coordinator 680K/1000K (68%)`; agent: `coordinator peak 680K/1000K · first 261K · compactions 0`

inv-2
- `Profile: unavailable — no timed task dispatch in the runs given (transcripts missing)`
- context: `agents 16 · peak≥80% 0 · compacted 0 · coordinator 702K/1000K (70%)`; agent: `coordinator peak 702K/1000K · first 261K · compactions 0`

inv-3 (final block; an earlier partial block is at ledger line 624)
- `Profile: ht-kqrt · 2026-11-01T03:06Z → 11:27Z · wall 8h21m (task graph 8h02m, after the last landing 20m) · agents 425 · beads dispatched 46, landed 43, planned 72`
- bound: `graph lower bound 7h00m along ht-kqrt.26 → ht-kqrt.48 · model 7h59m (cap 4, one merge lane) vs actual 8h02m (99%) → slot-bound: 54m of the 59m above the graph's bound is the slot cap`
- critical path: `8h02m = implement 6h57m · review 5m · fix 16m · merge 4m · seam 3m · blocker 5m · cancel 9s · close 10s · planning 14m · ready 1m · startup 6m · dispatch 1m · slot 9s · wait 10m`
- bottleneck: `ht-kqrt.15 — 2h12m (27%) · implement 2h11m · wait 59s · poll 1h55m · shell 4m · model 11m · dependents 6 direct, 38 transitive`
- waits: `9h10m over 28 of 43 — slot 6h00m (19) · retry 3h08m (6) · unexplained 3m (3)`
- merge lane: `busy 3h13m of 7h51m (41%) · seam 48m (25% of busy) · 72 merge dispatches, first try not merged for 29 of 43 · queue wait 3h26m, peak 4`
- what-if: `unlimited slots → 7h05m (−54m, 11%)`; `split ht-kqrt.72 → 7h42m (−18m, 4%)`; `cut ht-kqrt.28 <- ht-kqrt.72 → 7h45m (−15m, 3%)`; `faster ht-kqrt.72 → 7h46m (−14m, 3%)`; `faster ht-kqrt.38 → 7h47m (−12m, 3%)`; `cut ht-kqrt.38 <- ht-kqrt.28 → 7h48m (−11m, 2%)`
- context: `agents 426 · peak≥80% 1 · tasks>1 4 · compacted 0 · coordinator 797K/1000K (80%)`; agent: `coordinator peak 797K/1000K · first 261K · compactions 0`

inv-4
- `Profile: ht-kqrt · 2026-11-01T11:52Z → 18:18Z · wall 6h26m (task graph 6h12m, after the last landing 14m) · agents 326 · beads dispatched 27, landed 27, planned 84`
- bound: `graph lower bound 6h12m along ht-kqrt.164 → ht-kqrt.64 → ht-kqrt.66 · model 6h12m (cap 4) vs actual 6h12m (100%) → graph-bound`
- critical path: `6h12m = implement 4h55m · review 8m · fix 3m · merge 4m · seam 56s · split 29s · blocker 9m · close 16s · planning 43m · ready 3m · startup 2m · dispatch 2m · slot 12s · wait 32s`
- bottleneck: `ht-kqrt.62 — 1h37m (26%) · implement 1h23m · test 21m · poll 37m · model 19m · 2 attempts (BLOCKED, IMPLEMENTED)`
- waits: `3h05m over 14 of 27 — slot 2h29m (10) · retry 36m (4)`
- merge lane: `busy 1h01m of 5h36m (18%) · seam 5m · 35 merge dispatches, first try not merged for 8 of 27 · queue wait 1m, peak 1`
- what-if: `cut ht-kqrt.66 <- ht-kqrt.64 → 5h58m (−14m, 4%)`; `split ht-kqrt.64 → 6h01m (−10m, 3%)`; `faster ht-kqrt.66 → 6h02m (−10m, 3%)`; `faster ht-kqrt.64 → 6h04m (−7m, 2%)`
- context: `agents 327 · peak≥80% 1 · compacted 0 · coordinator 868K/1000K (87%)`; agents: `coordinator peak 868K/1000K · first 261K · compactions 0`; `plan-rest peak 505K/1000K (51%) · first 41K · compactions 0`

inv-5
- `Profile: ht-kqrt · 2026-11-01T19:26Z → 23:35Z · wall 4h09m (task graph 3h53m, after the last landing 16m) · agents 181 · beads dispatched 25, landed 25, planned 110`
- bound: `graph lower bound 3h53m along ht-kqrt.193 → ht-kqrt.194 · model 3h53m (cap 4) vs actual 3h53m (100%) → graph-bound`
- critical path: `3h53m = implement 2h56m · review 4m · fix 6m · merge 3m · seam 32s · close 12s · planning 40m · ready 33s · startup 1m · dispatch 1m · slot 8s`
- bottleneck: `ht-kqrt.183 — 47m (20%) · implement 47m · shell 44m · model 2m · dependents 0`
- waits: `14h33m over 21 of 25 — slot 13h32m (17) · planning 2m (3) · retry 1h00m (1)`
- merge lane: `busy 1h14m of 3h15m (38%) · seam 4m · 28 merge dispatches, first try not merged for 3 of 25 · queue wait 18m, peak 2`
- what-if: `cut ht-kqrt.194 <- ht-kqrt.193 → 3h39m (−14m, 6%)`; `faster ht-kqrt.194 → 3h39m (−14m, 6%)`; `split ht-kqrt.193 → 3h42m (−11m, 5%)`; `faster ht-kqrt.193 → 3h45m (−8m, 3%)`
- context: `agents 182 · peak≥80% 1 · compacted 0 · coordinator 958K/1000K (96%)`; agents: `coordinator peak 958K/1000K · first 261K · compactions 0`; `final-review peak 521K/1000K (52%) · first 44K · compactions 0`; `plan peak 525K/1000K (52%) · first 41K · compactions 0`

inv-6
- `Profile: ht-kqrt · 2026-11-02T00:28Z → 02:52Z · wall 2h24m (task graph 2h10m, after the last landing 15m) · agents 80 · beads dispatched 11, landed 11, planned 121`
- bound: `graph lower bound 1h58m along ht-kqrt.203 → ht-kqrt.205 · model 2h15m (cap 4) vs actual 2h10m (104%) → slot-bound: 17m of the 17m above the graph's bound is the slot cap`
- critical path: `2h10m = implement 1h45m · review 2m · merge 1m · planning 19m · ready 13s · startup 1m · dispatch 16s · slot 7s`
- bottleneck: `ht-kqrt.204 — 42m (33%) · implement 41m · poll 28m · shell 7m · model 5m · dependents 0`
- waits: `3h05m over 7 of 11 — slot 3h05m (7)`
- merge lane: `busy 38m of 1h38m (39%) · seam 1m · 14 merge dispatches, first try not merged for 3 of 11 · queue wait 10m, peak 2`
- what-if: `unlimited slots → 1h58m (−17m, 13%)`; `faster ht-kqrt.199 → 2h09m (−6m, 4%)`; `faster ht-kqrt.200 → 2h09m (−6m, 4%)`
- context: `agents 81 · peak≥80% 1 · compacted 1 agents, 1 compactions (cap 1) · coordinator 968K/1000K (97%), 1 compactions`; agent: `coordinator peak 968K/1000K · first 261K · compactions 1 (cap 1)`

inv-7
- `Profile: ht-kqrt · 2026-11-02T03:50Z → 05:16Z · wall 1h26m (task graph 1h09m, after the last landing 17m) · agents 64 · beads dispatched 7, landed 7, planned 128`
- bound: `graph lower bound 56m · model 1h05m (cap 4) vs actual 1h09m (95%) → slot-bound: 5m of the 9m above the graph's bound is the slot cap`
- critical path: `1h09m = implement 38m · review 1m · merge 2m · seam 3m · planning 22m · ready 13s · startup 2m · dispatch 24s · slot 2s`
- bottleneck: `ht-kqrt.215 — 27m (40%) · implement 21m · review 48s · merge 2m · seam 3m · poll 10m · shell 8m · model 3m · dependents 0`
- waits: `35m over 3 of 7 — slot 35m (3)`
- merge lane: `busy 23m of 36m (64%) · seam 4m (18% of busy) · 12 merge dispatches, first try not merged for 4 of 7 · queue wait 3m, peak 1`
- what-if: `unlimited slots → 1h01m (−5m, 7%)`; `faster ht-kqrt.212 → 1h03m (−3m, 4%)`; `faster ht-kqrt.213 → 1h04m (−2m, 3%)`; `unlimited merge lanes → 1h04m (−2m, 3%)`; `seam work off the merge lane → 1h04m (−1m, 2%)`
- context: `agents 65 · peak≥80% 1 · compacted 1 agents, 1 compactions (cap 1) · coordinator 968K/1000K (97%), 1 compactions`; agent: `coordinator peak 968K/1000K · first 261K · compactions 1 (cap 1)`

inv-8
- `Profile: ht-kqrt · 2026-11-02T05:25Z → 06:19Z · wall 54m (task graph 37m, after the last landing 16m) · agents 28 · beads dispatched 2, landed 2, planned 130`
- bound: `graph lower bound 37m · model 37m (cap 4) vs actual 37m (100%) → bead-bound: ht-kqrt.219 alone set the pace`
- critical path: `37m = implement 21m · review 29s · merge 1m · planning 12m · ready 12s · startup 2m · dispatch 7s`
- bottleneck: `ht-kqrt.219 — 23m (62%) · implement 21m · shell 19m · model 2m · dependents 0`
- waits: `0s over 0 of 2 landed beads`
- merge lane: `busy 2m of 16m (14%) · 2 merge dispatches, first try not merged for 0 of 2 · queue wait 3s, peak 1`
- what-if: `faster ht-kqrt.219 → 27m (−10m, 28%)`
- context: `agents 29 · peak≥80% 1 · compacted 1 agents, 1 compactions (cap 1) · coordinator 968K/1000K (97%), 1 compactions`; agent: `coordinator peak 968K/1000K · first 261K · compactions 1 (cap 1)`

### Bead graph
Status: all closed except the epic. The 85 `review:` beads (all closed, reason `Closed`) are collapsed out of both tables. Title clipped at 60 characters, "what" at 72.

| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-kqrt | epic | Environments: named seats, tmux wake and herdr-core terminal | Root epic of super-auto run 2026-10-31-environments-named-seats-herdr-ap |
| .1 | task | Bring main into the run branch | Merge local `main` (contains ht-tod8 5e19e9bce: `lifecycle::scrub_env`, |
| .2 | task | K5 env scrub: child_env_scrub_list() and spawn_detached | Make `scrub_env` `pub(crate)`, add `host::child_env_scrub_list()` (union |
| .3 | task | HostPort capability answers: HostKind{Herdr,Unknown}, six de | In src/ports.rs add `HostKind{Herdr, Unknown}`, `WakeCapability{None, Co |
| .4 | task | NodeDelta 0012_host_environments.sql, Herdr kind writes, unk | Add `migrations/node/0012_host_environments.sql` (next free slot after F |
| .5 | task | HostSessions builtin slot, attach_builtin, all_for_reads, Op | Extend src/service/host_sessions.rs: `builtin: OnceLock<Arc<HostSession> |
| .6 | task | 07 §5.3 attached()/all_for_reads consumer audit (G1) and ope | Apply the verified consumer table in 07 §5.3 at each call site: idle exi |
| .7 | task | HarnessRegistry::idle_wake_on and its table test | Add `HarnessRegistry::idle_wake_on(harness, WakeCapability, channel_live |
| .8 | task | wake_status per-row host through all_for_reads | `wake_status` (src/service/dispatch.rs:252) takes each row's own host (j |
| .9 | task | HostTopologies (+capability), SeatDirectory.name, K4 label k | Fix K4: `Command::HostTopologies` gated on local capability `topology.ho |
| .10 | task | caller_location.rs: CallerEnv/Located/CallerSite (Pane), env | Create src/cli/caller_location.rs, the CLI's only reader of caller-locat |
| .11 | task | LocateCaller (Herdr candidate), transport peer pid, TUI leak | Add `Command::LocateCaller{candidates, subject}` → `LocateReply::{Verifi |
| .12 | task | SeatsQuery.host and ResolveThreadQuery caller host (unsent); | Add `SeatsQuery.host: Option<HostLocator>` and the caller host on `Resol |
| .13 | task | F behavioural gate: herdr-release-smoke and wake-e2e self-te | Before N and H1 build on F, run `nice scripts/herdr-release-smoke` (hook |
| .14 | task | Split file: src/store/seats.rs | Move-only, behaviour-preserving split of src/store/seats.rs (6.5k lines) |
| .15 | task | NamedHost, NamedSeatSource, SeatName, HostKind::Named, attac | Add `SeatName` (src/protocol/ids.rs: 1-64 bytes of `[A-Za-z0-9._-]`, no |
| .16 | task | Named caller site: CallerLocator::Named site answers, named | Add `CallerLocator::Named` to caller_location.rs (precedence rule 1: a v |
| .17 | task | Named site remainder: M14/M15/M16 refusals, watch S3, Hermes | Refusals on the Named site: invalid name → M16; explicit cooperative fla |
| .18 | task | Instance discovery for named sites (SA1), default state dir, | Implement SA1's discovery order for non-Pane sites in src/cli/instance.r |
| .19 | task | Standalone (Herdr-less) install: install.sh setup, install.j | scripts/install.sh runs `setup` whenever it can (today it skips unless t |
| .20 | task | Store: named allocation on the named host with the M8 creati | In src/store/seats/ (after the split): allocate a named seat with `seats |
| .21 | task | Store: named continuity (C1), cross-kind rebind refusal, Clo | Add the fourth C1 path "named continuity" (an ordinary resolve whose nam |
| .22 | task | Store: restore rules for named and unnamed seats (SA4, D12) | Change the local-origin restore in `materialize_seat_created`: a row wit |
| .23 | task | Store: live-holder guard (SA16, D19) and M7 | In the enroll transaction, when a named seat would gain or reconnect a t |
| .24 | task | Read-only bindings (D3): HERDR_THREADS_SEAT_READONLY | `readonly=1` is recorded on the binding when the caller env has `HERDR_T |
| .25 | task | Named presence (SA17): no baseline/recovery holds, structura | Named presence follows the pane precedent: a named seat goes offline onl |
| .26 | task | Self-retire: retire_seat, Command::SelfRetire, seat retire - | Factor `retire_seat(tx, seat, wire, label)` out of `operator_retire`. |
| .27 | task | Inherited-name refusal (SA2, D1): server environ reader, res | When a host candidate sits beside the name, the resolve command carries |
| .28 | task | Persisted locator_hint, M9 second-locator notice, seat inspe | Each named binding records `occupant_bindings.locator_hint`; a binding w |
| .29 | task | Pane-form allocation refusal M10 (SA9) | Pane-form allocation of a pane whose current named binding hinted it is |
| .30 | task | Named display and selectors (SA14, D4), handoff out of a nam | Fill `SeatDirectory.name`; `thread show`, delivery `from` headers, `seat |
| .31 | task | No-wake reports: persisted no_wake_reports, WakeBlock::NoWak | `wake_status` maps an effective capability `None` to `WakeBlock::NoWake` |
| .32 | task | No-wake remainder: inert threshold M12, M19 offline with pok | ` offline` suffix (M19) on `wake=no_wake` for named/hc seats whose bindi |
| .33 | task | Sync: display_name publish and remote materialize; name@node | `ensure_published` sets `seat.created.display_name` from `seats.name`; o |
| .34 | task | Named seats end to end through one daemon | Integration suite (own `[[test]]` target with `required-features = ["tes |
| .35 | task | HostPort::target_wake_capability and per-target no-wake repo | Add defaulted `HostPort::target_wake_capability(&HostTargetId) -> WakeCa |
| .36 | task | turn_state_edge adapter method, Claude report-only Stop, dae | Add neutral defaulted `HarnessAdapter::turn_state_edge(event) -> Option< |
| .37 | task | ComposerPolicy::screen_blocked default and harness recognize | Add defaulted `ComposerPolicy::screen_blocked(&str) -> Option<bool>` (de |
| .38 | task | tmux hint: TMUX/TMUX_PANE read, resolve-command hint, verifi | caller_location.rs reads `TMUX` (socket, server pid) and `TMUX_PANE` int |
| .39 | task | tmux composer wake: built-in wake lane, capture-pane read, s | `NamedHost::wake_capability()` becomes `Composer(ComposerTraits::TMUX{su |
| .40 | task | hc JSON-RPC client and wire types, fake_hc (in-thread + fake | Create src/host/hc/rpc.rs (JSON-RPC 2.0 line client, one connection per |
| .41 | task | HcHost observation, HcBoot, HostKind::Hc, target_root_proces | Create `HcHost` in src/host/hc/mod.rs: `HostKind::Hc` plus implemented-s |
| .42 | task | HcHost finish-reason closure, C2 holds on core-id rotation, | Finish-reason closure (D10) through the store publication-absence path o |
| .43 | task | hc labels <session>:tN, herdr-app doc names, ambiguous bare | `HcHost::seat_labels`/`live_topology` take workspace from `herdr.space`, |
| .44 | task | HostFactory {exists, probe, build(kind, locator)}; probe bef | `HostFactory` (host_sessions.rs:61-77, closures in src/main.rs:67-80) be |
| .45 | task | HcDiscovery: core.json scan, socket derivation, hello id mat | `HcDiscovery` in src/host/hc/ reads core state and never runs `hc`: list |
| .46 | task | CallerLocator::Terminal, full LocateCaller, M4 and M17; remo | Add `CallerLocator::Terminal{socket,id}` (`Terminal` without `HC_TERMINA |
| .47 | task | HcHost::server_process and the hc inherited-name check (E1 h | `HcHost::server_process` = the core socket's peer pid from the `LocalEnd |
| .48 | task | seat.terminal_sites_v1 and hc messaging end to end with wake | Advertise `seat.terminal_sites_v1` and gate Terminal sites on it in the |
| .49 | task | hc-only setup (no TUI --host-endpoint) and Hermes PANE_ENV t | On an hc-only machine (a core state dir present, no `herdr` on PATH) `se |
| .50 | task | Two-host dialled-only consumer tests (TUI fake + fake hc), t | Each dialled-only consumer gets a two-host test where hc holds the lowes |
| .51 | task | hc schema-conformance test (always shipped) | `tests/host/hc_schema_conformance.rs` in tests/combined.rs validates eve |
| .52 | task | scripts/hc-smoke real-core smoke (opt-in, only with a verifi | Ship `scripts/hc-smoke` plus `tests/herdr-core-releases.tsv` only if the |
| .53 | task | Seam contract: CreateTargetPort, Placement, HostWitness, Cre | Add `CreateTargetPort` beside `CreateTabPort` (src/ports.rs:5536) with ` |
| .54 | task | Launch-gate shim: internal launch-gate, env blanking, child_ | src/cli/launch_gate.rs (new): `herdr-threads internal launch-gate --boot |
| .55 | task | HcHost CreateTargetPort via terminal.run; fake_hc terminal.r | Add `terminal.run` to the hc client and fake_hc. |
| .56 | task | Herdr launch and handoff through CreateTargetPort (generic r | Make topology_handoff*, topology_runtime, handoff and launch code generi |
| .57 | task | hc handoff/launch CLI: --space/--new-tab/--new-terminal, in- | The user-facing layer on top of the V3 pipeline (narrowed by ht-ja24: ht |
| .58 | task | Peer launch: NewTab onto hc spaces, cap-gated LaunchTarget:: | Make peer_launch and peer_launcher generic over `CreateTargetPort` (src/ |
| .59 | task | Configuration smoke: Herdr pane, named, named+tmux and hc te | One integration target that exercises each enumerated configuration once |
| .60 | task | hc seats across nodes: remote publication and addressing of | Publish hc terminal seats' labels to peers through the existing `SeatSta |
| .61 | task | Opt-in real-agent wake-e2e case: named seat in a private tmu | Add an opt-in case to scripts/wake-e2e.py (`--cases tmux-named/send-p0`, |
| .62 | task | N2 phase gate: unfiltered suite, leak checks, Herdr-pane reg | Before phase N2 merges into the run branch, run the per-phase gate on th |
| .63 | task | H1 phase gate: unfiltered suite, leak checks, D1/E1/S5/H-C1 | Before phase H1 merges into the run branch, run the per-phase gate on th |
| .64 | task | Final gate (whole merged run branch, also H3 phase gate): un | This is the final gate. |
| .65 | task | Host-neutrality source scan: no HostKind::Named/Hc or host e | Add a source-scan test (a `tests/combined.rs` `#[path]` module, no proce |
| .66 | task | Integration sweep: three environments behind one host seam | Scope-bounded root integration sweep, run after every leaf has merged in |
| .67 | task | HcHost D10 finality closure over a capture with finished rec | Owns: `enumerate_targets` in full (sole owner: `hello` + `events.subscri |
| .68 | task | Launched hc worker retires on exit (H3 keep:false finality) | Owns: the H3 acceptance that a gate-shim-launched worker (`keep: false`, |
| .69 | task | tmux composer wake: built-in lane, idle-exit keep-alive, pan | Owns: `ComposerTraits::TMUX`; the built-in session wake lane (pacer, rea |
| .70 | task | tmux turn state: turn_state_edge with every turn-ending edge | Owns: `HarnessAdapter::turn_state_edge` and `TurnEdge` (including `Scree |
| .71 | task | Store: restore rules with nameless hold on a name collision | Owns: the local-origin restore in `materialize_seat_created` (named row |
| .72 | task | Inherited-name refusal (SA2, D1) with the same-name limit an | Owns: server environ reader (Linux `/proc`, macOS `KERN_PROCARGS2` behin |
| .73 | task | spawn_detached reparents the daemon (double fork through int | Owns: `herdr-threads internal daemon-detach` (setsid, spawn the daemon w |
| .74 | task | LocateCaller terminal-root wall and start-stamped cache key | Owns: `host::foreign_terminal_process_names()` (F: a const list in src/h |
| .75 | task | hc JSON-RPC client, wire types, fake_hc (in-thread + fake-hc | Split from ht-kqrt.40: Two independent groups in 17 files: the hc JSON-R |
| .76 | task | Isolated-core helper, herdr-core serve/hold leak recognition | Split from ht-kqrt.40: Two independent groups in 17 files: the hc JSON-R |
| .99 | task | Wire hc <session>:tN into CLI selectors and displays (hc_cor | Gap from ht-kqrt.43 review: src/cli/seat_resolution.rs HcResolver::hc_co |
| .133 | task | Automatic bootstrap V3 records: daemon persistence of Create | The default handoff path (automatic bootstrap: `automatic: !new_thread & |
| .134 | task | Automatic handoff pipeline runs a V3 plan through CreateTarg | Make the automatic handoff pipeline in the CLI run a V3 plan: it creates |
| .135 | task | Named-site CLI caller: seat-acting selection, caller-default | Wire the Named site through the ordinary CLI caller path. |
| .136 | task | Startup hint attaches again when its probe fails (ht-kqrt.44 | Regression at ee7c7aa46, real production behaviour, not a fixture proble |
| .137 | task | Fake private Herdr hosts answer the probe's hello as real He | Test-fixture gap left by ht-kqrt.44 (1e0910918, H1b), which added the da |
| .138 | task | Regenerate doctor JSON goldens for doctor.hosts and doctor.o | golden_surfaces::doctor_reports fails on tests/fixtures/golden/doctor/al |
| .139 | task | Clear host_neutral_scan: 28 offenders, one code move, the re | host_neutral_scan::src_names_no_host_kind_or_host_env_outside_the_allowl |
| .140 | task | Fix order-dependent assertion in unresolved_named_row_does_n | Flake, reproduced at ee7c7aa46: store::seats::observation::named_presenc |
| .141 | task | install_sh golden: Stop hooks and temp-root-dependent trunca | installer_integrations::install_sh_with_stub_herdr_byte_identical fails |
| .142 | task | Account for five new timed waits in the daemon loop inventor | service::loop_inventory::every_timed_wait_is_on_a_pacer_or_allowlisted_w |
| .143 | task | Peer launch onto hc: real bootstrap and parseable gate argv | Peer launch onto an hc terminal authorizes through a real bootstrap. |
| .164 | task | Delivery from headers render a named seat name | Delivery `from` headers render a named seat's name, per the approved nam |
| .170 | task | Terminal identity: a terminal location is (core socket, tN) | - [Should-fix] src/cli/mod.rs:3940 |
| .171 | task | Parked readiness mirrors what the parked materializer waits | - [Should-fix] src/store/apply.rs:985 |
| .172 | task | Turn state: per-binding key, atomic transitions, wake gate r | - [Should-fix] src/service/turn_state.rs:35 |
| .173 | task | Wake answers derive from the wake gate: capability and wake_ | - [Must-fix] tmux Codex seats are never typed and never reported: `scree |
| .174 | task | tmux hint belongs to the binding: harness anchor, refreshed | - [Should-fix] src/host/named/tmux.rs:221 |
| .175 | task | hc capture has three outcomes: failure is Err, unverified is | - [Should-fix] src/host/hc/mod.rs:877; src/host/hc/mod.rs:878 |
| .176 | task | hc observation sequence owned through HostPort | - [Should-fix] src/host/hc/mod.rs:320 |
| .177 | task | hc rpc budget and launch gate: HostUnavailable retries to th | - [Nit] src/service/launch_gate.rs:100; src/service/launch_gate.rs:48 |
| .178 | task | Test scrub covers every caller-location variable in one plac | - [Should-fix] src/cli/caller_location.rs:143 |
| .179 | task | Pin new guards: one test per guard in which it alone decides | - [Should-fix] src/store/local_seat_status.rs:92 |
| .180 | task | Acceptance features tested through their production entry: b | - [Should-fix] src/service/host_sessions.rs:627 |
| .181 | task | Readonly caller honoured on every enrollment path | - [Must-fix] Parked `.24`, needs a human decision: a session enrolled on |
| .182 | task | Own-log restore keeps a retired named seat retired | - [Should-fix] src/store/seats/publication.rs:245 |
| .183 | task | Backfill seats.name for remote rows materialized before the | - [Nit] src/store/seats/publication.rs:197 |
| .184 | task | tmux-named wake e2e counts wake lines per case | - [Should-fix] scripts/wake-e2e.py:2767 |
| .185 | task | Skill names the three seat sources | - [Nit] integrations/skill/SKILL.md:3 |
| .186 | task | launch gains --space, --new-tab and --new-terminal | - [Must-fix] `launch` has no `--space`, `--new-tab` or `--new-terminal` |
| .187 | task | handoff --existing --seat from a named seat dials the reserv | - [Must-fix] Ledger 1137 `[known-unfixed-defect]`: `handoff --existing - |
| .188 | task | HostBootId::new panic on a long core id | - [Must-fix] Ledger 1050 `[latent-panic]`: `HostBootId::new` at src/host |
| .189 | task | CI runs the tmux wake and beside-Herdr tests instead of skip | - [Must-fix] Ledger 1202 and 1136 `[silent-skip]`: `named_tmux_idle_reci |
| .190 | task | Record a shellcheck run of isolated-core.sh | - [Must-fix] Ledger 213 `[unverified-live-path]`: the shellcheck test of |
| .191 | task | guard_live_holder admits a readonly caller beside a live hol | - [Must-fix] `guard_live_holder` has no caller-side readonly (src/store/ |
| .192 | task | Gate the readonly wire field on a daemon capability | - [Nit] src/cli/commands.rs:929 |
| .193 | task | Daemon selects the core by socket: LocateCaller and the inhe | Split from ht-kqrt.170: The bead covers ten findings across CLI, hook, d |
| .194 | task | The caller acts as its verified terminal; every CLI lookup a | Split from ht-kqrt.170: The bead covers ten findings across CLI, hook, d |
| .195 | task | Delete the unused CallerSite answers (ht-tyil) | Split from ht-kqrt.170: The bead covers ten findings across CLI, hook, d |
| .199 | task | The claim host is never dialled: launch, invite and seat res | - [Should-fix] src/cli/launch_new.rs:166; src/cli/launch_new.rs:167 |
| .200 | task | Guarded capabilities are decided for the host they act on, n | - [Must-fix] `handoff` onto a herdr-core fails on a core-only daemon: th |
| .201 | task | A replayed own seat.retired takes the one retirement transit | - [Should-fix] tests/sync/facts_seat.rs:1203 |
| .202 | task | Every consumer states gate_never_types: a recognizer-less tm | - [Should-fix] tests/config_smoke.rs:657; tests/config_smoke.rs:501 |
| .203 | task | A terminal is (core socket, tN) at every hop: one entry-poin | - [Should-fix] src/cli/me.rs:203 |
| .204 | task | Stated time bounds hold from the production entry point: one | - [Should-fix] src/main.rs:73 |
| .205 | task | Every test child gets one scrub; in-process hook tests read | - [Should-fix] tests/integration/trust_policy.rs:347 |
| .206 | task | Each round-1 fix guard gets one test through its production | - [Should-fix] src/service/dispatch.rs:822 |
| .207 | task | An unverified hc capture matches its contract: it closes not | - [Should-fix] src/host/hc/mod.rs:941; TRUST-POLICY.md:315 |
| .208 | task | assert_gate_launch waits for the task to replicate instead o | - [Should-fix] tests/multinode/peer_launch_terminal.rs:201 |
| .209 | task | tmux wake applies the focused-composer hold | - [Must-fix] tmux wake has no focused-composer hold (design line 573, TR |
| .212 | task | A two-family terminal's core socket is taken for Herdr: deci | - [Should-fix] src/cli/topology_runtime.rs:79; src/cli/mod.rs:457 |
| .213 | task | A journaled V3 bootstrap on a core-only daemon gates on the | - [Must-fix] Operator recovery of a V3 bootstrap gates on the node-wide |
| .214 | task | The tmux wake gate copies the native gate piecemeal: match i | - [Must-fix] tmux wake reads the composer as plain text only (src/host/n |
| .215 | task | The named lane publishes every 5 s from its own store: publi | - [Should-fix] src/host/named.rs:631 |
| .216 | task | Two new tests assert on timing, not on an observed edge: com | - [Should-fix] tests/service/composition.rs:1603 |
| .217 | task | Two source scans fail at head: allowlist the probe connect p | - [Should-fix] src/host/probe.rs:135; src/app.rs:48; tests/host_neutral_ |
| .218 | task | A process-exited named or hc seat becomes a dormant wake tar | - merge residue: main's `dormant_top_level` is not host-aware |
| .219 | task | A lane failure reports recovery and stays within the health | - sweep failure: sweep_remaining::lane_failure_surfaces_through_remedy_t |
| .220 | task | two_family_pane_forms_never_dial_the_core is independent of | - sweep failure: two_family_pane_forms_never_dial_the_core (tests/hc_mes |
| .221 | task | Spawn-site scan: the store-only lane rig's thread worker car | - sweep failure: no_leaks::every_test_spawn_site_is_owned (tests/service |

Edges: 452 `blocks` in total, including 6 from work beads into `review:` beads (not shown) and edges among `review:` beads. Grouped per dependent. Per-edge reasons are not extracted here; 140 descriptions carry `blocked-by` lines, for example `blocked-by ht-kqrt.3: consumes HostKind and the implemented-kinds set`. Read the reason column as "in bead description, or unstated". `supersedes`: 7 edges, not shown.

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| .2, .3, .12, .14, .37, .40, .53, .75, .76 | .1 | see note |
| .4, .5, .7 | .3 | see note |
| .6, .9 | .5 | see note |
| .8 | .5 .7 | see note |
| .10, .73 | .2 | see note |
| .11 | .3 .5 .10 | see note |
| .13 | .2 .4 .6 .8 .9 .10 .11 .12 .73 .74 | see note |
| .15 | .2 .4 .5 | see note |
| .16 | .10 .11 .12 .15 | see note |
| .17, .18 | .16 | see note |
| .19 | .1 .18 | see note |
| .20 | .4 .14 .15 .16 | see note |
| .21, .22, .23, .25 | .14 .20 | see note |
| .24 | .16 .20 | see note |
| .26 | .14 .16 .20 | see note |
| .27 | .11 .16 .20 | see note |
| .28 | .14 .20 .72 | see note |
| .29 | .14 .20 .28 | see note |
| .30 | .9 .16 .18 .20 | see note |
| .31 | .4 .8 .15 .20 | see note |
| .32 | .15 .20 .31 | see note |
| .33 | .14 .20 .30 | see note |
| .34 | .6 .13 .16 .17 .18 .19 .20 .21 .23 .24 .25 .26 .28 .29 .30 .31 .32 .33 .65 .71 .72 .135 .140 | see note |
| .35 | .31 .32 | see note |
| .36, .70 | .1 .10 .11 .16 | see note |
| .38 | .15 .16 .28 .35 | see note |
| .39 | .35 .36 .37 .38 | see note |
| .41 | .2 .3 .14 .40 .75 .76 | see note |
| .42 | .14 .41 | see note |
| .43 | .9 .41 | see note |
| .44 | .4 .40 .41 .75 .76 | see note |
| .45 | .40 .41 .75 .76 | see note |
| .46 | .11 .16 .18 .41 .44 .74 | see note |
| .47 | .40 .41 .46 .72 .75 .76 | see note |
| .48 | .12 .16 .17 .26 .31 .44 .46 | see note |
| .49 | .17 .18 .19 .45 | see note |
| .50 | .6 .41 .44 .69 .71 | see note |
| .51 | .40 .75 .76 | see note |
| .52 | .2 .40 .43 .44 .67 .75 .76 | see note |
| .54 | .2 .10 .41 .46 .53 | see note |
| .55 | .40 .41 .53 .75 .76 | see note |
| .56 | .53 | see note |
| .57 | .45 .46 .54 .55 .56 .133 .134 | see note |
| .58 | .53 .55 | see note |
| .59 | .16 .34 .43 .48 .69 | see note |
| .60 | .33 .43 .48 | see note |
| .61 | .69 .70 .135 | see note |
| .62 | .61 .65 .69 .141 .142 | see note |
| .63 | .13 .45 .47 .49 .50 .51 .52 .60 .65 .67 .136 .137 .138 .139 | see note |
| .64 | .57 .58 .59 .62 .63 .65 .68 .133 .134 .143 .164 | see note |
| .65 | .3 .10 | see note |
| .66 | all of .1-.76 and .133-.143, .164, except .22 .27 .36 .39 .42 and itself | see note |
| .67 | .14 .40 .41 .75 .76 | see note |
| .68 | .54 .55 .57 .67 | see note |
| .69 | .5 .6 .35 .37 .38 .70 | see note |
| .71 | .3 .14 .20 .26 .33 | see note |
| .72 | .3 .11 .16 .20 | see note |
| .74 | .11 | see note |
| .134 | .133 | see note |
| .135 | .30 | see note |
| .143 | .57 .133 | see note |
| .164 | .33 | see note |
| .173 | .172 | see note |
| .191, .192 | .181 | see note |
| .194 | .193 | see note |
| .205 | .203 | see note |
| .209 | .202 | see note |

## Design questions

### 1. Should fix tasks be scoped by the rule they restore, or by the files they declare?
Round 2 step-back: "14 of 19 round-2 findings are fix residue from rule fixes scoped to file lists". Commit f99941014's body says me.rs was "outside this task's file list", which produced a Should-fix there. The step-back's approach change was that fix tasks "must now be scoped by rule and run the unfiltered suite", yet round 3 attributes "at least seven of 15" to round-2 fix commits, and the post-cap audit lists 3 regressed from the last fix round.

For rule scope: the defect class is a contract change whose tests and normative docs were not swept, and only a consumer sweep reaches them. Against: the scheduler relies on declared surfaces (`TRUST-POLICY.md` was declared by 43 beads at inv-7, and hot-file deferrals fired), so an open-ended scope brings merge conflicts, slower lanes and less disjoint parallelism, and it blurs what a reviewer checks. A middle form is to keep the declared surface, but add a required step that greps the changed contract's consumers and returns any outside the surface as a new surface or a follow-up bead.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### 2. Should `run.md` carry full review records inline?
`run.md` reached 940,564 bytes, with lines up to 125,371 bytes, because `review-collection`, `finding-source` and `root-acceptance` JSON sit in the approvals block. It also appeared as a merge overlap: `docs/superpowers/runs/…/run.md (stack conflicts 0, seam overlaps 3, declared by 12)`. Coordinator context peaked at 968K/1000K with one cap compaction at inv-6, though this run does not establish what share came from `run.md`.

For inline: one file, committed, greppable, and the provenance validators read it at captured revisions. Against: a state file meant for quick resume becomes too large to read, and it is a shared write target for tasks. The alternative is side files with hashes in `run.md`.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

### 3. Should a small relaunch skip the full planning pass?
Planning on the critical path: 33m of 2h27m (inv-1), 43m of 6h12m (inv-4), 40m of 3h53m (inv-5), 19m of 2h10m (inv-6), 22m of 1h09m (inv-7), 12m of 37m (inv-8, 2 beads). `Profile: diagnosis — planning 33% of first dispatch to last landing` at inv-8.

For skipping: fix-round beads are already scoped with findings, so plan text repeats them, and it dominates short invocations. Against: planning catches errors; in this run "ht-kqrt.143 … my earlier V3 wording violated TRUST-POLICY N15; caught by the planner and triage".

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

- **Long-lived scratch on tmpfs.** `super-code/SKILL.md:39` relaunches from "the scratchpad copy you launched". Friction: "session scratchpad under /tmp was swept mid-run (coordinator.js copy, reaper scripts lost); moved them to the git-ignored run workspace on disk". Who swept it is not recorded. The guidance could say that a multi-day run keeps its launch copy in the run workspace.
- **Shell-safe bead writes.** Friction: "coverage applier (subagent) wrote a bead description through an unquoted heredoc; backticked command names executed and a `cargo nextest` run started in the run worktree". The skills' markdown has no heredoc guidance. A line requiring a quoted heredoc or body file would cover it.
- **Workflow script size.** Friction: "first assembly exceeded the Workflow 512 KB script cap (context embedded per scout); context trimmed to file pointers", and "Workflow scripts cannot read files" led to file-based coverage inputs, unlike the template. The roast and coverage docs could state the limit and the pointer form as the default for large contexts.

## Already fixed — do not re-litigate

none

## Not established

- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists. In this run it exists only for inv-1 and inv-2. The direct reconciliation above counted the ledger file itself.
- The non-dumpable process mechanism in Defect 3 is inferred from code and `Cleanup:` lines, not observed.
- The transcript count of read-ledger dispatches (31, 5 refusals) comes from a text match on prompts and may miss some.
- The first-try-not-merged figures (29 of 43 at inv-3) were not traced to causes. Rebase conflicts were only 6 of 129 merges.
- `poll 1h55m` on ht-kqrt.15 is unexplained; it overlaps the disk and tmpfs incidents. Implementer polling (37% of 23h24m at inv-3) is mostly test wall time, not established as waste.
- `unexplained 1h27m (3)` at inv-1 is unattributed.
- What-if lines are modelled estimates; none was tested.
- Thirteen `Detector:` lines were read after the concurrency cap had already been lowered; peak-versus-cap says little about parallelism limits at a healthy cap.
- Reviewed as a candidate and dropped: `already-merged` printing `false` with exit 0 is documented in its header. The misuse was in a reaper written to work around Defect 3.
- A bad finding-count baseline: the 384 groups in `scope-dispositions` include fixed findings, so they are not a defect count.

## Verification bar

- Defect 1: a fixture repo with an invalid archive in an untouched run dir must give `wholeRangeReady: true` for a range that does not touch it, and a gap for one that does.
- Defect 2: run Finish against a 1,600-line ledger through a real dispatch, end to end, and check that all four `Metrics:` lines are present.
- Defect 3: on Linux with an sshd-session or a `PR_SET_DUMPABLE=0` same-uid process, `remove-task-worktree` must remove a clean merged worktree, while a process with cwd in the root still stops it. Run the same on macOS.
- Defect 4: a live round with a stack whose parent is rebased before landing, then a dependent with two stacked parents.
- Defect 5: two scouts in one Workflow, each writing a same-named pointer, must not see each other's.
- Defect 7: run the documented initial snapshot through `report-status` as written.
- Defect 9: a real subagent run of the report drafter on the current harness.
- Defect 10: `already-merged --landed` on a branch whose base carries other epics' merges.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
