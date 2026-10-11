# Handoff Delivery Redesign Finish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking. Planning completed and implementation resumed by the user on 2026-10-11: “ok lets split the work into independent parts and hand off to subagents for implementation”.

**Goal:** Finish existing epic ht-0lo and produce a reviewed, verified integration branch for Main's merge decision, retaining the worktrees and sending the authorized readiness notification only after every gate clears.

**Architecture:** Preserve the existing canonical send-time deadlines, independent invitation confirmation, frozen durable handoff payloads, stable hook entrypoints, and bounded V2 presentation design. Complete D on its owned branch, compose reviewed B/D with actual Main in the existing composition worktree, execute C, then promote and review the full integration branch. Capability activation is a separate reviewed change with actual advertised-path evidence.

**Tech Stack:** Rust, rusqlite/SQLite STRICT migrations, Tokio transport, serde wire protocols, Claude mod JavaScript/TypeScript tests, cargo-nextest, beads.

**Spec:** `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/2026-10-10-handoff-delivery-redesign-design.md`, its three subsystem designs, and the accepted refinements referenced by `SESSION-HANDOFF.md`. Latest explicitly accepted sections supersede chronological proposals.

## Global constraints

- Never run `git push` or `git stash`; keep actual Main unmerged and all warm worktrees retained.
- Never restart, stop or kill the shared Herdr server or close foreign workspaces/panes.
- Keep top-level membership in the uniquely identified active `psa-global`. Discovery and join are self-service and pre-approved under the latest user instructions; subagents never change membership. Membership was already verified in this planning session.
- Never write real `~/.claude`, `~/.codex`, `~/.aisw`, or `~/.gemini`; use isolated test configuration and owned children.
- Read actual AGENTS.md and normative TRUST-POLICY.md before protected changes. Canonical daemon state decides; no heuristic seat repair or adversarial caller verification.
- Messages are lazy by default; lazy delivery never wakes idle sessions unless `wake_on: all`. ACK-required messages are nudge-class; active sessions receive in-turn delivery.
- ACK means receipt only. Presentation confirmation, invitation acceptance, launch, registration, task adoption and completion are separate.
- Released/approved migrations 1..39 remain byte-identical. D's unlanded additions are 40/41; no new migration renumbering is justified by current Main026.
- Body acquisition remains exact UTF-8, nonempty, at most 65,536 bytes, frozen before effects. General RPC envelope remains 1 MiB; B's narrow matching modern AutomaticBootstrap response allowance remains 2 MiB.
- D: at most 32 blocks and 65,536 actual encoded native/Watch bytes; fresh Pull physical source bound 433, passive Refresh 160; relevance 13 sources/26 shared seeks plus declared keyed/witness work; publication 100 visits/200 keyed probes/3 seeks; sizing 352 candidate passes plus final RPC encode; Confirm does no discovery.
- Private carry cap 155731 and partial cap 49415 remain. Exhausted U+10FFFF sentinel is valid only in the two `after_thread` fields.
- Migration40 startup quick_check cost is explicitly accepted and grows with retained invitations. Healthy driver bounds apply after migration; do not reinstate the superseded constant-read claim.
- One writer and one Cargo command at a time in D's warm lane; `CARGO_BUILD_JOBS=4`, absolute `CARGO_TARGET_DIR`, `nice`, locked/all-features. Fresh quiescence confirmation and exact source mapping precede helper transfers; old observations expire.
- Per change: format, strict all-target/all-feature clippy and harness-neutral check. Before a merge: default-feature check also passes. Repeat final readiness gates on the final integration SHA after roast and reporting commits.
- No routine full suite while ht-zo4 remains unresolved. Focused integration sweeps use a fresh `HT_LEAK_RUN_ID`, then scoped leak checks and owned-process cleanup. Preserve nextest groups, including B's ALLSLOTS positives.
- Incremental build and per-change lint budgets are each under one minute. Distinguish multifile build history from a controlled single-source incremental measurement.
- Every new commit has a truthful `Outside-adapter: <repo-relative paths/globs or none>` trailer. Account for new model-visible strings; keep static explanations out of repeated deliveries.

## Assessment of the checkpoint

The handoff is a useful recovery map and correctly refuses to claim readiness. It is insufficient as a completion checklist: it mixes historical proposals, partially finished reviews, source evidence, selected test results and future gates. Preserve its facts, but require a current evidence matrix rather than treating its resume list as proof.

Observed on 2026-10-11 by read-only Git/source inspection:

| Component | Current source | Assessment |
| --- | --- | --- |
| Main | `026ad3ab444779216748cc5733e2241e2834ebaf` | Matches handoff; includes independent required-invitation reuse repair and v0.8.1. |
| Run root | `7ac930b6f658da9c3a30cb9b7450b9124a806bab` | Clean before this plan; changes after recorded `31ffd371` are handoff/progress documentation. Runtime is not fully composed. |
| A+B composition | `3ad07776ca21b5a8aea0cd624447142e536e0d40` | Matches handoff; Main026 and D still absent. |
| D | `4c4988d9ee937c333d602bac25dca25fc2c07d37`, tree `1589136ce7568bcd72b0050fa8d947e6f49503ab` | Clean; cumulative 74 files, 18,577 additions/249 deletions against `ee1e598c`. Whole/schema/interface final verdicts are absent. |
| B | `cdbf3c37e873a2cb571c098450602a53f81a6185` | Already reviewed and composed; preserve it and test the combined seams. |
| Run state | phase `code`, design roast round 2 | 19 completed, D/C/composition/root pending. No PR roast yet. Design low-coverage qualifier survives to final reporting. |

Confirmed open defects:

1. `src/store/invitation_content.rs::revisit_candidates` still reads three private invitation IDs as allocated `String`, then uses panic-on-invalid construction in dirty/revisit branches. `bounded_private_text` and `private_read_error` already exist and provide the narrow repair pattern. Type/UTF-8/length checks must occur on borrowed SQLite bytes, and ASCII/nonempty ID validation must be fallible. Cleanup must validate identity too, before deleting/updating anything.
2. `tests/service/delivery_pull.rs::presentation_v2_replays_one_canonical_invitation_until_emit_confirmation` still omits required `render` and checks obsolete scalar Confirm results. The protocol requires a render selector and returns `{status, acknowledged_controls}`. Compilation/clippy cannot catch raw-JSON semantic drift.

Important distinctions and planning corrections:

- The older interface-preparation finding about always including Watch `presentation_lease` is **source-fixed** in frozen D: actual transport now calls `advertised_presentation_lease(ADVERTISED.contains(...), ...)`. An actual transport regression exists. Verify its results and active behavior; do not file the old finding as a new defect.
- Earlier schema-preparation findings about state cursors, proof JSON and private item text have subsequent guards/tests. Audit frozen corrections and evidence rather than blindly reapplying the preparation report. The three revisit ID reads remain demonstrably open.
- The recorded 24/24 result is historical focused evidence, not all-D coverage. One newly added test visibly contradicts the current interface. Build an enumerated affected-test inventory with nonzero discovery and explicit exclusions; a count alone cannot establish completion.
- `task-15-report.md` contains stale “remaining” paragraphs about clippy/commit even though its later Frozen section records completion. Consolidate current status without deleting historical failure evidence.
- The Main026 third-required-attachment test change is a contract change backed by a released fix, not permission to weaken assertions. Preserve old invitation history and require a fresh identity, correct epochs, and actual native/cooperative acceptance behavior.
- Do not defer all default-feature checking to the end: current AGENTS.md also requires it before merges. Final parent checks remain necessary after the roast.
- Native agents interrupted in the prior session are not resumable dependencies. Recreate roles from exact artifacts, with one source owner; do not assume that reading a preparation report completed its audit.

This assessment is source-based. No Cargo/test command was run during planning, and this document is not a whole-branch review verdict or a PR roast.

## Workspace and ownership map

Let ROOT be `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`.

- Run state/reports: `ROOT/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign`.
- D source: `ROOT/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`, actual branch `task-ht-0lo.8.4`. The directory label is historical; never derive another directory from the task ID.
- Composition: `ROOT/.worktrees/upstream-composition-478ec4a9`, branch `super-auto/handoff-upstream-composition`.
- Parked B: `ROOT/.worktrees/parked--task-ht-0lo.8.2`.
- Coordinator owns run state, tracker, serial composition and root promotion. D author owns source changes. Fresh whole/schema/interface reviewers inspect frozen source without compiling or writing it. C owns actual combined seam tests after reviewed inputs exist. Parent owns PR roast/report/notification.
- A reviewer may request a Cargo reproduction; coordinator executes it through the sole lane. Independent read-only reviews can proceed in parallel.

## Task 1: Recover complete review coverage and a current evidence matrix

**Files:** Read `SESSION-HANDOFF.md`, `code-pause-handoff.md`, `run.md`, latest `progress.md`, `task-15-report.md`, final path/entrypoint inventories, all accepted refinement reviews, the two final-review preparation files, `task-15-attention-cleanup-restoration.diff/json`, and `task-14-main026-preflight.md`. Create run-local `finish-evidence.md`; later save the missing `task-15-whole-review.md`, `task-15-schema-final-review.md`, and `task-15-interface-final-review.md`.

**Interfaces:** Consumes exact D base/head/tree and accepted contracts. Produces a complete findings ledger and requirement-to-source/test mapping; preparation and final verdicts remain distinct.

- [ ] Recheck branch names, clean state, main SHA and tree hashes with `git status --short --branch`, `git rev-parse HEAD HEAD^{tree}`, and `git worktree list --porcelain`. Check current processes without stopping foreign work. Preserve counters and existing epic rather than creating a replacement run.
- [ ] Inventory every changed production/test path from `git diff --name-only ee1e598c 4c4988d9`. Map each accepted requirement to its source, actual registered test name, source SHA/log and remaining gap. Mark unsupported earlier generic-Attention assertions as explicitly superseded, not passing tests.
- [ ] Recreate fresh whole/schema/interface review roles on the full cumulative frozen package. Whole review also inspects actual restored attention test bodies; schema review covers genuine upgrades, private loaders, all writers and query/serializer bounds; interface review covers actual Hook/Watch/AGY/mod consumers, omission compatibility and post-flush confirmation. Finish all coverage before calling a final verdict.
- [ ] Record the two confirmed open defects, any additional supported findings, and source-fixed preparation findings separately. New evidence is required to reopen accepted design limits.

**Exit:** Every changed path is assigned review coverage; all discovered defects have dispositions; no preparation report is mislabeled CLEAN. This is the input to the single D repair batch.

## Task 2: Repair D's private ID loading and interface-test drift

**Files:** Modify D's `src/store/invitation_content.rs` and `tests/service/delivery_pull.rs`; add corruption regressions in that existing service test module. Update D's report/proof manifests in the run directory after evidence exists.

**Interfaces:** Existing `bounded_private_text(row, index, limit) -> rusqlite::Result<String>` checks borrowed SQLite TEXT bytes before allocation; `private_read_error` maps malformed private state to `StoreCorrupt`. `InvitationId::parse` validates nonempty ASCII-graphic IDs of at most 128 bytes. No public wire or schema changes.

- [ ] Add one parameterized real-store regression exercising cleanup, dirty sizing and revisit reads independently. Start from `fixture`, `pull_seat`, `canonical_lazy_invitation_backlog`, and a valid admitted Hook page so the private item exists. Isolate the selected branch by its consumer frontiers (`cleanup_after`, `dirty_after/dirty_high`, revisit byte/ordinal frontiers). For each branch corrupt only the private item's ID, using an explicitly scoped foreign-key-disabled repair connection where necessary, then restore/check connection settings.
- [ ] Cover empty, 129-byte, ASCII whitespace/control, non-ASCII, invalid UTF-8 TEXT and wrong-type values when the STRICT schema permits constructing them through the corruption fixture. Keep an exact 128-byte valid-ID control. Snapshot private consumer/item rows and canonical invitation/receipt/warning state after corruption but before the operation. Require `StoreCorrupt`, no panic and byte-identical snapshots after refusal. Do not infer preallocation safety from the returned error: inspect ordering as well.
- [ ] Run the new exact test and the existing stale replay test before implementation. Record actual failure and selected counts. A panic/deserialize error is meaningful RED; a fixture that never reaches the intended read is not.
- [ ] Replace all three reads with `bounded_private_text(r, 0, 128)` and use `.map_err(private_read_error)` for conversion failures. Validate IDs fallibly before cursor advancement or SQL effects; use the following conversion instead of `InvitationId::new`:

```rust
let id = InvitationId::parse(id)
    .map_err(|_| corrupt("invalid private invitation identity"))?;
```

Cleanup binds `id.as_str()` only after validation. Dirty/revisit candidates store the parsed ID. Keep visit quotas, index ordering and transaction rollback unchanged. Audit adjacent private readers for the same pattern without extending this into canonical caller verification.

- [ ] Correct the replay test's raw request by supplying the settled Hook selector:

```rust
let wire = json!({"kind":"delivery_pull_v2","args":{
    "claim":recipient,"consumer":"hook",
    "render":{"kind":"hook","confirmable_selector":0,"passive_selector":0}
}});
```

Compare `data.status` with `not_current_page`, `confirmed`, and `already_confirmed`; assert each expected `acknowledged_controls` value separately. Retain exact restart replay, token checks and the pending invitation/no receipt-ACK assertions. Derive this invitation-only page's expected empty controls from its actual ordered items.

- [ ] Audit every changed raw V2 JSON producer and Confirm assertion against `src/protocol/delivery.rs`. Build a consolidated affected service selection including all presentation tests, not only the prior 24. Run the new regressions, consolidated selection and affected private/schema/owner tests; record discovered/executed/ignored counts and exact source.
- [ ] Run fmt, strict clippy and harness-neutral checks. Freeze corrected D, regenerate complete cumulative/delta review packages, and obtain final whole/schema/interface verdicts naming the corrected SHA. Include truthful commit trailer paths and text-size accounting for any new model-visible error text.

**Exit:** No unresolved D correctness findings; all affected service tests have dispositions and executed evidence. The new capability remains unadvertised through this gate.

## Task 3: Review activation separately and prove advertised consumer behavior

**Files:** D `src/protocol/capabilities.rs`, actual activation-dependent tests in `tests/protocol/capabilities.rs`, `tests/daemon/watch_connection.rs`, `tests/cli/watch.rs`, `tests/harness/agy_delivery.rs`, `tests/service/delivery_pull.rs`, `integrations/claude/mod/tests/delivery.test.ts`; run-local activation proposal/evidence/review.

**Interfaces:** `DELIVERY_PRESENTATION_V2`, actual daemon capability response, optional Watch accepted lease, registered Hook codecs, ordered V2 pages, exact Confirm status/subset.

- [ ] Write a narrow activation proposal from the corrected frozen source: identify capability producer, every selecting consumer, V1 fallback and exact required evidence. Review it independently before changing ADVERTISED.
- [ ] Inspect actual capability negotiation/registration tests. Retain unadvertised old omission bytes and supported old-client behavior; add explicit non-active controls rather than deleting them when the build becomes active.
- [ ] Add `DELIVERY_PRESENTATION_V2` to the actual advertised set only after the source audit clears. Update capability handler-probe coverage to call the real route; a constant-membership assertion alone is insufficient.
- [ ] Exercise an elected isolated daemon's Capabilities → Watch registration → accepted 32-hex lease → actual CLI ordered output → successful flush → exact Confirm chain. Also exercise Hook System/User codecs and mod delivery through actual production consumers. Confirm write/flush failure, stale owner, replacement owner, lost response/replay and rejected subset do not consume unseen output or arm health incorrectly. Retain passive Lazy/no ACK/accept/wake and actual ordinary/Warn controls.
- [ ] Run affected Rust selections and the existing `scripts/test-claude-mod` workflow using isolated configuration. Native execution/configuration refusals remain truthful; fixture-only or V1 fallback results do not satisfy advertised-path completion. Record any unexecuted external native portion as a gap rather than model uptake evidence.
- [ ] Run per-change gates and default-feature check before merge; obtain exact active-tip source/schema/interface delta clearance plus the completed cumulative D verdict. Record active SHA, test names, timings and process cleanup.

**Exit:** Reviewed advertised production path works. Close D only after its required reviewed composition gate; retain its warm slot.

## Task 4: Compose Main026, B and reviewed active D serially

**Files:** Composition branch; primary overlaps `src/store/service_controls.rs`, `src/store/control.rs`, `src/store/seats.rs`, `tests/store/control.rs`, `TRUST-POLICY.md`, metadata and `.config/nextest.toml`. Follow `task-14-main026-preflight.md` and actual merge diff.

**Interfaces:** Main's pending-first `current_invitation_for_accept`, immutable invitation-to-requirement episode linkage; D's Lazy distinction, command/context epochs and V2 state; B's Ensure policy and body/recovery envelope.

- [ ] Re-observe Main. If it differs from026, inspect the exact new delta and refresh schema/capability/overlap inventory before merging. Do not import other unfinished branches or pause for hypothetical future Main changes.
- [ ] On the existing composition branch, merge the observed Main, then the reviewed D successor, preserving A/B/D ancestry. Keep this candidate out of run root until combined gates clear. Freeze source during Cargo. Use the sole warmed target through the coordinator's sequential source/target ownership protocol; no second cold target.
- [ ] Resolve `service_controls::invite` by keeping BOTH historical requirement-link exclusion and D's fourth Lazy-policy tuple/guard. Preserve all command/context writers. Verify native acceptance and cooperative permit preparation both use Main's shared pending-first helper. Inspect relevant clean automatic merges as well as conflicts.
- [ ] Change `same_invitation_requirement_attach_release_replacement_has_distinct_context_epochs` only on the composed source: third attachment succeeds with fresh invitation/requirement IDs; old invitation command epoch/history is unchanged, thread context advances for the actual new mutation, exact replay advances neither. Preserve `service_requirement_uses_an_independent_episode_without_mutating_native_lazy_invitation` and Main's expanded managed reattachment regression.
- [ ] Preserve upstream version0.8.1 consistently, released SQL1..31 plus approved32..39 byte-for-byte, D40/41, B's modern Some-only response allowance, legacy None behavior and canonical-summary/sidecar limits. Preserve B's ALLSLOTS nextest override for the two modern automatic maximum/boundary chains.
- [ ] Run exact producer regressions and public acceptance seams from the preflight, formatting/lint/neutral/default-feature gates. Save independent composed-source/seam review naming the candidate SHA and reviewed conflict/auto-merge paths.

**Exit:** One frozen, reviewed B+D+actual-Main candidate with no lost runtime hunks or historical SQL drift. It is the sole input to C.

## Task 5: Execute C's actual combined focused validation

**Files:** Existing C admission in Task14 of `ht-0lo-plan.md` and `plan-mapping.json`; existing CLI/handoff/direct/topology, schema/receipt, scheduler/notice/setup tests and skill/docs. Admit any additional producer-test path explicitly before editing; no new production scope or unregistered test target.

**Interfaces:** Reviewed active combined candidate, real public handoff/Ensure/body/recovery APIs, genuine released31 database, existing registered combined target.

- [ ] Refresh C's brief against actual merged B/D symbols and final reports. Replace historical proposed migration/version references with the real31→41 lineage. Select test names from actual registration, not directory names or old plan snippets.
- [ ] Cover the following combined acceptance matrix, reusing leaf evidence only where source is unchanged:

| Seam | Required assertion |
| --- | --- |
| Ensure Lazy → active D Hook/Watch | Actual full goal/context and usable per-item acceptance command, no auto accept/message ACK or ordinary idle wake. |
| Required attach/release/fresh reattach | Distinct IDs and current commands; old history/epochs retained; actual native and cooperative acceptance select pending ordinary correctly. |
| Body/source/retry | Automatic, explicit and existing routes; exact escaped65,536-byte positive, aggregate boundary success/next-size zero effects; retry does not reread bytes/options. |
| Summary/sidecar/recovery | B's bounded canonical summary survives missing sidecar; exact local diagnostic identity; NonStarted truthfulness and no possible-start relaunch. |
| Wire negotiation | Some-modern matching AutomaticBootstrap reply allowance; legacy None/generic/error/wrong-family/correlation controls remain bounded and refuse before effects. |
| Genuine upgrade | Released main31 populated DB → actual41, retained requests/keys/digests/rows, rollback/reopen, real39→40 and40→41 proofs, expanded H/D/I driver quotas/reset. |
| Warnings/publication | Every canonical departed sender/inviter; partial ACK remains open, final settlement clears passively; Lazy publication/rebind owner orders preserve both consumers' progress. |
| Recovery routing | All six existing `lazy_recovery_context` tests execute in `combined`; journal/ambient routing remains pinned, with the existing30s/20ms projection wait. |

- [ ] Export a fresh `HT_LEAK_RUN_ID` for the integration sweep and execute the registered six-case selection, checking the count is six:

```sh
nice cargo test --locked --all-features --test combined lazy_recovery_context::
```

Use cargo-nextest for the larger focused selection so configured ALLSLOTS and test groups apply. The initial concrete selection is:

```sh
nice cargo nextest list --locked --all-targets --all-features -E 'test(/presentation|canonical_v2|departed_receipt_origins|a_departed_inviter|managed_required_reattach|same_invitation_requirement|service_requirement_uses_an_independent|modern_automatic_|lazy_recovery_|send_time|send_projection|dormant|originator|original_inviter/)'
nice cargo nextest run --locked --all-targets --all-features -E 'test(/presentation|canonical_v2|departed_receipt_origins|a_departed_inviter|managed_required_reattach|same_invitation_requirement|service_requirement_uses_an_independent|modern_automatic_|lazy_recovery_|send_time|send_projection|dormant|originator|original_inviter/)'
```

Compare enumerated names to the Task1/Task5 acceptance matrix before running; append exact-name expressions for uncovered B summary/sidecar/response/legacy and genuine31-upgrade seams. Record that completed concrete filter and count in execution evidence. Pattern matches alone do not prove matrix coverage, and a historical count is never the selection oracle.

- [ ] Run `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"` even when a sweep fails. Stop only owned leaked processes and fix their owner paths. Save exact source SHA, selected/executed names, durations and scoped cleanup result.
- [ ] Run required per-change/default-feature gates, obtain fresh C review, and merge only the reviewed/gated C successor. Close C/composition tasks to fixpoint. Obtain unconditional final whole-epic review on the actual final candidate before root promotion and parent roast.

**Exit:** D and C closed with actual combined evidence, composition parent promotable, full-tree review clear. No readiness notification yet.

## Task 6: Promote and run the original PR roast without resetting history

**Files:** Run integration branch, `run.md`, tracker buckets, progress/review artifacts, run-local PR roast reports and bounded fix tasks.

**Interfaces:** Frozen combined SHA and observed base SHA, original epic/flags, design roast round2 low-coverage record; no existing code-roast round.

- [ ] Promote only the reviewed/gated composition candidate into the existing run branch. Reconcile terminal tracker buckets and four required metrics from actual artifacts; do not infer success from the number of closed original leaves.
- [ ] Read current super-auto/super-code/super-roast workflow and review prompts at execution time. Transition `run.md` to `roast-code` only after code completion. Invoke PR round1 with exact branch/base SHAs, autonomous=true and run-directory report override. Label same-family OpenAI review honestly; no fabricated cross-family independence.
- [ ] Run fresh triage, core/conditional scouts, dedupe, method-distinct reproduce/refute/ground verification and reporter. Coordinate Cargo reproductions with the sole lane. Preserve incomplete coverage, escalations and finding keys explicitly.
- [ ] Scope-filter before filing fix beads, preserve original counters/caps and dispositions, and execute the existing bounded fix loop. Blocking findings always remain in scope. No unresolved Blocking can be declared ready; re-review corrected frozen source and rerun affected checks rather than recycling the old SHA's verdict.

**Exit:** PR roast/fix loop has an explicit allowed exit and no unresolved Blocking. Nonblocking retained findings and design low coverage remain visible.

## Task 7: Report, verify the final SHA and notify Main

**Files:** Run `report.md`, `run.md`, friction/feedback draft and final readiness manifest; existing integration branch retained.

**Interfaces:** Exact final reviewed candidate and base, actual evidence matrix, five-section report format in current super-auto `report-prompt.md`.

- [ ] Write the prescribed five sourced report sections, initially with metrics pending. Include retained findings, actual timing/budget evidence and at least `[degraded: design low coverage]`; never report bare clean. Run outermost upstream-feedback once, park its draft locally under autonomous rules, and update only the prescribed metrics line afterward. No external issue submission is implied.
- [ ] Commit all final report/feedback/metadata before the final readiness stamp, with truthful path trailers. Capture `git rev-parse HEAD` and verify that it stays unchanged through the final gate.
- [ ] In the owned warm lane run fmt check, strict clippy, harness-neutral, default-feature check and the recorded affected combined selection against this final source. Use a fresh sweep UUID and scoped leak check. No routine full suite unless actual current ht-zo4/project policy permits and requires it.
- [ ] Measure a controlled warm single-source incremental build without committing a semantic probe: after baseline warm build, touch one existing source file and time `nice cargo build --locked --all-features`. Record actual elapsed time separately from tests/clippy and prior multifile rebuilds. A result above60s is a budget defect requiring disposition/fix, not evidence of passing the budget.
- [ ] Save exact final SHA/base, checks/test counts/log hashes, incremental/lint timings, review/roast references and zero owned-process result. If any source or report commit follows, invalidate the stamp and rerun the necessary final checks.
- [ ] Revalidate coordination thread topic/goal, membership and exact Main seat at send time. Expected routing is instance `51a03f2a-3c7e-46f5-b694-3aed374557d7`, state `/Users/alepar/.local/state/herdr/plugins/herdr-threads`, endpoint `/Users/alepar/.config/herdr/herdr.sock`. Last Main seat was `sRpHoWRVk`; labels and the design-thread sender do not prove identity. Only after readiness is established send the authorized concise project-thread notification with branch/SHA, report path, gates and retained qualifications. Never send project status to psa-global.

**Exit:** User receives the reviewable branch/report and exact final verification status; Main receives a truthful readiness notification. Actual Main merge, push and worktree deletion remain outside this finish boundary.

## Self-review and execution handoff

- Settled deadline/invitation/body/hook semantics are preserved through Tasks2–5 and the combined matrix; no new architecture is proposed.
- D private-loader and interface drift fixes have concrete test/fix shapes; source-fixed preparation findings are explicitly distinguished.
- Active feature proof precedes composition acceptance; real combined proof precedes root promotion; whole-epic review precedes PR roast; final readiness follows roast/report commits.
- Current project merge/default-feature/leak/process/trailer rules take precedence over older plan snapshots. No task repeats completed B work or resets the epic/roast history.
- Execution should retain a single coordinator and D writer, with fresh read-only review roles. This planning session makes no phase, tracker, source or capability change.
