# 2026-10-13-message-priority-delivery: relaunch after an outage re-merges already-merged tasks and cancels dependents

Plugin: superpowers 6.4.2-alepar5.1. Run: 48 beads (19 design leaves + 2 epics, 13 fix/sweep beads, review beads), 2 design-roast rounds, 2 code-roast rounds + post-cap audit, 6 coordinator launches (2 forced by network outages), ~32 h wall, 2026-10-13..14.

## Defects

### 1. A resumed coordinator does not count pre-outage merges as completions; an already-merged task is re-merged, raises a false blocker and cancels a dependent with partial work
- **Evidence:** friction "ht-yha.1 (merged at 0a24b6c3 pre-outage, bead left open) was re-merged → false blocker ht-xml … already-merged script printed false when given a bare sha (branch already deleted)"; ledger "Merge: ht-yha.1 — rebase clean · seam-review none · check none → blocker", "Task 5 (ht-yha.7): cancelled (parent ht-yha.1 blocked)" (its partial worktree discarded); `Metrics: ledger-check landed≠completed` 5 vs 2, 19 vs 16, 25 vs 22, 28 vs 25, 30 vs 27, 31 vs 1 ("an incomplete Resume read seeds too few completions").
- **Premise to verify:** "landed" can be rebuilt on resume from ledger `Merge:`/completion lines or from merge-commit ancestry on the integration branch without the task branch existing.
- **Suggested fix shape:** on resume, seed a completion for every task whose merge commit is an ancestor of HEAD; make `already-merged` accept a bare SHA (ancestry); never cancel dependents over a blocker raised on an already-merged parent.

### 2. `landed (tracker closure unconfirmed)` is never reconciled against the tracker
- **Evidence:** the three pre-outage tasks reappear as "landed … tracker closure unconfirmed" in every later invocation (~24 times) after they were closed by hand; every return lists `closureUnconfirmed: [ht-yha.14, ht-yha.5.1, ht-yha.1]` with `stopReason: closure-unconfirmed`; detector rounds "3 ready · idle slots 5" spent on them; profile "4 dispatched bead(s) did not land … ht-yha.14, ht-yha.5.1, ht-yha.1".
- **Premise to verify:** `bd show` status is a cheap, authoritative re-check at launch/round start.
- **Suggested fix shape:** re-query unconfirmed closures at each launch; drop confirmed-closed beads from ready/open sets and from `closureUnconfirmed`.

### 3. No single-flight guard: a superseded relaunch kept running and returned after the next relaunch started
- **Evidence:** friction "relaunch-2 workflow (w25op9ayj) was still alive in Finish and returned late (stopReason closure-unconfirmed …) after relaunch 3 had started".
- **Premise to verify:** the orchestrator can tell the current instance (e.g. a run-id lease in run.md or the ledger).
- **Suggested fix shape:** write a current-run lease at launch; a superseded instance drops its Finish outputs, or a relaunch stops the older instance first.

### 4. A transient network outage stops the run instead of being retried
- **Evidence:** friction "network outage (ENOTFOUND) killed 17 dispatches incl. implementers …, bd-ready, final review, sweeps"; profile "critical path … ready 2h07m … relaunch 36m", "rework — 3h57m of dispatch time in attempts that did not land".
- **Premise to verify:** transient DNS/network errors are distinguishable from real dispatch failures.
- **Suggested fix shape:** retry dispatches and the ready query with backoff on transient network errors before a `ready-unavailable` stop; re-dispatch onto kept partial worktrees.

### 5. A blocked bead the orchestrator resolves mid-run is not re-admitted; each resolution costs a full relaunch that re-plans the whole tree
- **Evidence:** six `Launch:` lines; ht-yha.33 BLOCKED, amended and its blocker closed in-run, landed only after the next launch; same for ht-yha.42; profile "beads dispatched 10, landed 6, planned 25 … plan planning 16m" and "dispatched 6, landed 3, planned 28 … planning 9m" (43% of a 21m critical path).
- **Premise to verify:** the workflow can see a blocker bead close in-run, and planning can be restricted to open beads.
- **Suggested fix shape:** re-admit a quarantined bead when its blocker closes in-run; make relaunch planning incremental (open beads only).

### 6. A fix pass on an evidence-only finding (fix the git-excluded task report) is classified BLOCKED
- **Evidence:** "fix pass BLOCKED (…task-8-report.md … report has no test command or output … Code itself matches the brief; no code defects found.)"; triage "[cause: evidence-only review finding fixed in git-excluded report, no commit] — RESOLVE: This blocker is a pipeline artifact"; `Metrics: fix-pass — entered 5 · FIXED 4 · BLOCKED 1`.
- **Premise to verify:** reviewer findings can be told apart as code vs report/evidence findings.
- **Suggested fix shape:** route evidence-only findings to a report-amend step (or allow FIXED without a commit).

### 7. Relaunches re-emit ledger lines already recorded
- **Evidence:** duplicated `Human action:` and `Edge add:` lines after each resume; the same ht-yha.13 and ht-yha.2 minors re-generated with new wording ("[stale-doc-reference]" then "[doc-drift]").
- **Premise to verify:** ledger lines can be keyed (task, kind, anchor) for idempotent append.
- **Suggested fix shape:** idempotent ledger appends across resumes; don't re-review a task that already merged.

## Run metrics
### Judge panel
- design 1 · same-family (Claude) · Workflow · seat-agreement: panels 39 · rr 0.87 · rg 0.79 · fg 0.77 · unanimous 0.72 · ground-loo 0.82 (n=34)
- design 2 · same-family (Claude) · Workflow · seat-agreement: panels 10 · rr 0.20 · rg 0.90 · fg 0.30 · unanimous 0.20 · ground-loo 1.00 (n=2)
- pr 1 · same-family (Claude) · Workflow · seat-agreement: panels 27 · rr 0.70 · rg 0.70 · fg 0.63 · unanimous 0.52 · ground-loo 0.74 (n=19)
- pr 2 · same-family (Claude) · Workflow · seat-agreement: panels 7 · rr 0.86 · rg 1.00 · fg 0.86 · unanimous 0.86 · ground-loo 1.00 (n=6)
- pr post-cap audit · same-family (Claude) · Workflow · seat-agreement: panels 3 · rr 0.67 · rg 1.00 · fg 0.67 · unanimous 0.67 · ground-loo 1.00 (n=2)
### Fix loop
- Metrics: completions — review clean 27 · after fix pass 4 · parked 0 · re-entry closes 0 · dispatched early 8 · cancelled 1
- Metrics: fix-pass — entered 5 · FIXED 4 · BLOCKED 1
### Merge-back
- Metrics: merges 19 · merge-failed 1 · rebase-conflicts 2 · seam-reviews 4 (fixed 0) · check-fails 0 (fixed 0) (first full invocation; later invocations add 12 clean merges)
- ledger-check landed≠completed: 31 vs 1 (last invocation) — see defect 1
### Coverage
- coverage-round-1 · requirements: 12 · mapped: 12 · unmapped: 0
- coverage-round-2 · 16 requirements; divergence: findings 12 → 16, novel 15/16 (94%), widening: yes
- scope-filter (code round 1): 8 in-scope · 7 punch-listed
### Timing
- Profile wf_24ee41b2-d4e: critical path 12h32m = implement 5h40m · ready 2h07m · wait 2h58m · relaunch 36m; bottleneck ht-yha.3 5h58m (48%), 3 attempts; tests: 4h08m running + 2h50m polling = 56% of implementer time, 5 Bash calls hit the 10-minute tool limit; what-if (model estimate) unlimited merge lanes −1h03m (8%)
- Profile wf_e43661f8-02c: bottleneck ht-yha.36 23m (50%)
- Profile wf_ecf6a4ce-925: bottleneck ht-yha.38 11m (55%)
### Bead graph
| id | type | title | what (first sentence of description) |
| --- | --- | --- | --- |
| ht-yha | epic | Message delivery priority replaces lazy/nudge modes | Goal: senders choose a delivery priority 0..=9 (0 most urgent) per message; each seat has a nudge threshold T (default 0; negative = all) an |
| ht-yha.1 | task | Seam contract: priority protocol types and wire fields | Seam contract for the priority feature: compilable, inert protocol types and wire fields every other child builds on (spec §Key abstractions |
| ht-yha.2 | task | Schema v47: priority, nudge threshold and ordered pending indexes | Schema migration v47 for priority storage (spec §Compatibility and migration, §Ordering index list). |
| ht-yha.3 | task | Send publication by priority (delivery rows for all, receipts only for explicit ACK) | Send publication by priority (spec §Delivery model, D3, D9). |
| ht-yha.4 | task | Wake by per-seat nudge threshold | Wake by per-seat nudge threshold (spec §Delivery model Wake, D1, D2, D5). |
| ht-yha.5 | epic | Inbox v2 min-head merge in priority/deadline/created order | Inbox v2 min-head merge in priority order (spec §Ordering, user amendment D11). |
| ht-yha.5.1 | task | Inbox v2 cursor format v2 | Inbox v2 cursor format v2 (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery--inbox-me |
| ht-yha.5.2 | task | DeliveryRowSeek: exact ordered pending seek | DeliveryRowSeek: exact ordered pending seek on lazy_recipients (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-1 |
| ht-yha.5.3 | task | DeadlineWindow: bounded sorted deadline-class window | DeadlineWindow: bounded sorted window over invitations, binding-unread and receipts (subspec docs/superpowers/runs/2026-10-13-message-priori |
| ht-yha.5.4 | task | Inbox v2 min-head merge loop, OrderClass and PageSubset | Inbox v2 min-head merge loop, OrderClass and PageSubset (subspec docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-messa |
| ht-yha.6 | task | Invitations always p0 + confirming; 1:1 pairs auto-join | Invitations always p0 + confirming; 1:1 pairs auto-join (spec D6, D7, §Surfaces Direct pairs). |
| ht-yha.7 | task | CLI: --priority/-p, --nudge alias, nudge-threshold, flag removals, old-daemon fallback | CLI surfaces for priority and threshold (spec D3, D4, D5, D6 flag consequences, §Surfaces CLI, §Compatibility new-client/old-daemon). |
| ht-yha.8 | task | Framing and markers by priority ([pN], message kind) | Model-visible framing and markers by priority (spec D8, §Surfaces inbox/history/hook pages). |
| ht-yha.9 | task | Watch stream and Claude mod in priority order | Watch stream and Claude mod in priority order (spec §Surfaces Watch stream and Claude mod, D11). |
| ht-yha.10 | task | Docs and skill text for priority delivery | Docs and skill text for priority delivery (spec §Docs and model-visible text). |
| ht-yha.11 | task | Wake e2e matrix for priorities and thresholds | Wake e2e matrix for priorities and thresholds (spec §Testing, AGENTS.md Pre-release wake e2e). |
| ht-yha.12 | task | Configuration smoke: T x priority x ACK/invitation/1:1 | Configuration smoke: thresholds T in {-1, 0, 3} x priorities {0, 1, 5} x {ACK request, invitation, 1:1 first send} on a real in-process daem |
| ht-yha.13 | task | Split file: src/store/lazy_delivery.rs | Move-only, behavior-preserving split of src/store/lazy_delivery.rs (1071 lines) so the parallel store children edit separate files (super-de |
| ht-yha.14 | task | Split file: src/store/queries.rs mod inbox_v2 | Move-only, behavior-preserving split of `mod inbox_v2` out of src/store/queries.rs (lines ~6220-7469, ~1250 lines) so the wake child (hydrat |
| ht-yha.15 | task | Seam integration: cross-surface delivery order | Verify one ordering end to end across every presentation surface (coverage round 1 GAP: each surface was ordered only locally; root spec §Or |
| ht-yha.16 | task | Integration sweep: message delivery priority | Final integration sweep for message delivery priority (super-design root sweep; unknown-unknowns net). |
| ht-yha.31 | task | Fix: drain_pages keeps a sink's Halt bounding the page reads | Covers: [Should-fix] src/delivery/page.rs:103; src/delivery/page.rs:101; src/delivery/page.rs:98, [Must-fix] src/delivery/page.rs drain_page |
| ht-yha.32 | task | Fix: displayed-state predicate counts every open top-level binding | Covers: [Should-fix] src/store/lazy_delivery/completion.rs:68 This bead may change every instance of its <rule>one definition of 'a binding  |
| ht-yha.33 | task | Fix: old CLIs can follow inbox next: cursors from a new daemon | Covers: [Should-fix] src/protocol/pagination.rs:538 Context, not to be changed: everything else in docs/superpowers/runs/2026-10-13-message- |
| ht-yha.34 | task | Fix: merge readiness — failing tests, stale handoff guidance, doc drift | Covers: [Should-fix] docs/superpowers/runs/2026-10-13-message-priority-delivery/run.md:34, [Must-fix] untested: 7 schema/effective upgrade t |
| ht-yha.35 | task | Fix: add the v46-prepared, v47-retried send replay test | Covers: [Should-fix] tests/store/priority_send.rs:480 Context, not to be changed: everything else in docs/superpowers/runs/2026-10-13-messag |
| ht-yha.36 | task | Fix: a failed capability probe never silently downgrades send priority | Covers: [Nit] src/cli/send_priority.rs:66 Context, not to be changed: everything else in docs/superpowers/runs/2026-10-13-message-priority-d |
| ht-yha.37 | task | Fix: one batch order key in Rust, mirrored exactly by the Claude mod | Covers: [Nit] src/delivery/assemble.rs:412; src/delivery/assemble.rs:391, [Nit] integrations/claude/mod/tests/delivery.test.ts:2493 This bea |
| ht-yha.38 | task | Regression fix: seat_unread_sql! back to connected-only; display marking gets its own predicate | Covers: [Should-fix] src/store/lazy_delivery/mod.rs:112; src/store/lazy_delivery/wake.rs:13; src/store/lazy_delivery/mod.rs:109, [Should-fix |
| ht-yha.39 | task | Regression fix: pair-invitation text in SKILL.md and help.rs matches D7 | Covers: [Should-fix] integrations/skill/SKILL.md:17 This bead may change every instance of its <rule>every model-visible string that says au |
| ht-yha.40 | task | Sweep fix: delivery rows stream as the Lazy watch variant (old-reader compat) | Covers: cli::watch::tests::lazy_rows_stream_as_lazy_without_ack_required (tests/cli/watch.rs:761: assertion matches!(got[0].item, WatchItem: |
| ht-yha.41 | task | Sweep fix: body --max-bytes budget accounts for the [pN] annotation | Covers: token_diet::agent_facing_machine_text_is_compact (tests/integration/token_diet.rs:77: body mwZOxEy91 --max-bytes 256 failed: annotat |
| ht-yha.42 | task | Sweep fix: no [pN] history annotations against an old daemon | Covers: wire_compat::cli_against_an_old_fixture_daemon_never_sends_full_bodies (tests/integration/wire_compat.rs:448: identical rendered byt |
| ht-yha.43 | task | Sweep regression fix: session_bindings_smoke config5 resume inbox page | Covers: session_bindings_smoke::config5_resume_within_a_page_and_past_it (tests/integration/session_bindings_smoke.rs:1458: 'the pending obl |

| dependent | blocker | reason (from `blocked-by` line, or `unstated`) |
| --- | --- | --- |
| ht-yha.3 | ht-yha.1 | DeliveryPriority type and SendMessage.priority wire field |
| ht-yha.3 | ht-yha.13 | file split |
| ht-yha.3 | ht-yha.2 | migration 0047 priority columns |
| ht-yha.4 | ht-yha.1 | NudgeThreshold type and SetNudgeThreshold/NudgeThresholdQuery commands |
| ht-yha.4 | ht-yha.14 | file split |
| ht-yha.4 | ht-yha.3 | delivery rows staged with priority (staging.rs edits land first; the p0-behind-1000-p1 test runs thr |
| ht-yha.4 | ht-yha.2 | migration 0047 seats.nudge_threshold and delivery-row priority |
| ht-yha.4 | ht-yha.13 | file split |
| ht-yha.5.1 | ht-yha.1 | DeliveryOrderKey continuation encoding |
| ht-yha.5.2 | ht-yha.5.1 | cursor v2 DeliveryRows position triple (canonical definition) |
| ht-yha.5.2 | ht-yha.4 | the publication decision_seq copy on delivery rows (note_published) |
| ht-yha.5.2 | ht-yha.3 | staging.rs changes (priority on rows) landed first; same file |
| ht-yha.5.2 | ht-yha.2 | v47 delivery-row ordered indexes and lazy_recipients.decision_seq |
| ht-yha.5.2 | ht-yha.13 | file split |
| ht-yha.5.3 | ht-yha.5.1 | cursor v2 Deadline position struct |
| ht-yha.5.3 | ht-yha.14 | file split |
| ht-yha.5.3 | ht-yha.1 | DeliveryOrderKey |
| ht-yha.5.4 | ht-yha.5.2 | DeliveryRowSeek |
| ht-yha.5.4 | ht-yha.3 | delivery rows for all recipients with priority (send publication) |
| ht-yha.5.4 | ht-yha.5.1 | cursor format v2 |
| ht-yha.5.4 | ht-yha.5.3 | DeadlineWindow |
| ht-yha.5.4 | ht-yha.14 | file split |
| ht-yha.6 | ht-yha.5.2 | read_cursors.rs mark-copy changes land first (same file) |
| ht-yha.6 | ht-yha.13 | file split |
| ht-yha.6 | ht-yha.1 | direct.auto_join_v1 capability constant |
| ht-yha.6 | ht-yha.4 | wake by nudge threshold (p1 vs p0 pair wake acceptance) |
| ht-yha.6 | ht-yha.3 | send publication by priority (first direct message as a delivery row) |
| ht-yha.7 | ht-yha.1 | DeliveryPriority, NudgeThreshold and capability constants |
| ht-yha.8 | ht-yha.2 | migration 0047 messages.priority column |
| ht-yha.8 | ht-yha.14 | file split (inbox_v2 hydrate.rs) |
| ht-yha.8 | ht-yha.4 | hydrate.rs wakes change landed first; same file |
| ht-yha.8 | ht-yha.1 | DeliveryPriority marker |
| ht-yha.9 | ht-yha.8 | frame golden fixtures with [pN] |
| ht-yha.9 | ht-yha.1 | WatchMessage priority/deadline_at and DeliveryOrderKey |
| ht-yha.9 | ht-yha.2 | migration 0047 delivery-row priority column |
| ht-yha.9 | ht-yha.4 | daemon-computed per-item wakes by threshold |
| ht-yha.10 | ht-yha.6 | unstated |
| ht-yha.10 | ht-yha.5.4 | unstated |
| ht-yha.10 | ht-yha.9 | final delivery file/constant names for What we do today |
| ht-yha.10 | ht-yha.11 | wake-e2e matrix (AGENTS.md section text) |
| ht-yha.11 | ht-yha.7 | CLI flags -p/--nudge/--require-ack and nudge-threshold |
| ht-yha.11 | ht-yha.4 | wake by nudge threshold |
| ht-yha.11 | ht-yha.8 | [pN] framing in pane captures |
| ht-yha.11 | ht-yha.3 | send publication by priority |
| ht-yha.12 | ht-yha.3 | send publication by priority |
| ht-yha.12 | ht-yha.6 | pair auto-join |
| ht-yha.12 | ht-yha.5.4 | unstated |
| ht-yha.12 | ht-yha.7 | CLI priority flags and nudge-threshold command |
| ht-yha.12 | ht-yha.4 | wake by nudge threshold |
| ht-yha.14 | ht-yha.1 | unstated |
| ht-yha.15 | ht-yha.5.4 | inbox v2 merge order |
| ht-yha.15 | ht-yha.8 | priority populated into inbox items |
| ht-yha.15 | ht-yha.9 | watch batch order |
| ht-yha.15 | ht-yha.6 | confirming invitations and accept display |
| ht-yha.16 | ht-yha.2 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.14 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.13 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.5.4 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.5.1 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.10 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.11 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.4 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.5.2 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.9 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.7 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.6 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.3 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.15 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.12 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.5.3 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.1 | all leaves (integration sweep) |
| ht-yha.16 | ht-yha.8 | all leaves (integration sweep) |

## Design questions

### A. Should a correctness defect found by the final review after the code-roast loop has exited get one bounded fix round, or always go back to the user?
- **Evidence:** `roastCodeExit: converged`, then the final review "not ready (A stale pending delivery rows … B mod stall rule … a wedged mod holding a p0 delivery row never recovers …)"; super-auto routes post-loop final-review items to the punch list, so a goal-path delivery defect shipped to the hand-back unfixed; the post-cap audit later confirmed the same defect (mod.rs:131) but filed it at FYI under the pre-existing rule because it predated the audit's base.
- **For a bounded round:** in an autonomous run the user expects merge-ready code, and the defect is in a goal-named path.
- **For handing back:** the cap exists to bound cost and drift; the user should choose how to spend more.
- Related: the scope filter's `clusterOverride` punch-listed a Should-fix (wake.rs:80) whose cluster sibling was in-scope; later regressions came back in the same files.

If upstream decides otherwise, please state the position explicitly so downstream can reconcile against words rather than silence.

## Doc gaps

### 1. Sweep-fix beads default to "tests unmodified" even when the failing test is the stale side
- **Evidence:** "sweep-fix beads ht-yha.41/.42 blocked: bead text forbade test edits but the code follows D8"; blocker causes "bead's prescribed fix contradicts spec-mandated behaviour and its done-when test", "planner treated a spec-settled behaviour as an open decision".
- **Suggested fix shape:** the sweep-fix template should first decide whether code or test departs from the spec, and prescribe "tests unmodified" only when the code is at fault.

### 2. Deferred minors that put obligations on later tasks, or that name correctness gaps, have no route to those tasks
- **Evidence:** minors "[interim-wake-gap] … must be restored by Task 9", "[state-dependent-wake-seek] … missed wake … Task 11 … should revisit" stayed ledger-only and resurfaced as roast Should-fix and final-review blockers; tests "fail at base … need an owner" likewise.
- **Suggested fix shape:** a minor naming a later task or a correctness gap is appended to that task's bead (or filed as a bead) immediately.

### 3. Implementer guidance does not shape test running
- **Evidence:** "4h08m running tests and 2h50m polling background runs, 56% of their 12h33m · 5 Bash call(s) ran into the 10-minute tool limit"; bottleneck poll loop "until grep -q "^done " …" 10m.
- **Suggested fix shape:** prescribe focused filters, background runs with exit notification instead of poll loops, and base/full runs only when the brief requires them.

### 4. Implementers weakened tests (`#[ignore]`, loosened assertions) and claimed flakes without base evidence
- **Evidence:** "#[ignore]d with no brief authorization"; "were loosened (digest_of) to match the implementation"; flake claims without base runs in four tasks; four "not RED first" minors.
- **Suggested fix shape:** require bead authorization (or a filed blocker) before ignoring/loosening a test, and a base-commit rerun before calling a failure flaky or pre-existing.

## Already fixed — do not re-litigate
none

## Not established
- By default: the coordinator's ledger-append path is fire-and-forget and can lose a line without the coordinator noticing, so every ledger-derived count in `## Run metrics` (Fix loop, Merge-back) is a lower bound. The `Metrics: ledger-check` line is the one cross-check that exists.
- The analyst saw a bundle the orchestrator assembled (friction log, ledger outcome lines, detector lines, roast header lines, run.md, metrics/profile lines), not the raw journals.
- Single run: the outage-driven defects (1-4) were observed twice in one run; their frequency elsewhere is unmeasured.
- Two orchestrator-side slips (a run.md section header lost by an edit; a punch-list file holding non-`[SEV]` lines that `assemble-args --punch-listed` rejected) are not in the bundle and are not claimed here.

## Verification bar
- Replay/dryRun the coordinator with an injected terminal-network failure mid-round followed by `resumeFromRunId`: no re-merge, no false blocker, no cancelled dependent, `landed == completed`.
- Close a quarantined bead's blocker in-run in a dryRun and assert it is re-dispatched without a relaunch.
- A fixture where the reviewer's only Important finding is report-evidence: the fix pass must not end BLOCKED.

---
If a premise above is wrong, stop and say so rather than improvising a larger change.
