# Session handoff — handoff delivery redesign

Paused at the user's explicit request on 2026-10-11: “can you pause and write a handoff to another session pls?” Resume only when the next session is instructed to resume. This is a pause checkpoint, not completion or merge readiness.

## Objective and authorization

Continue the existing `$super-auto` handoff redesign, fully autonomous with both design and PR roasts enabled. Prepare the integration branch for merge and then notify the main tab through `$herdr-threads`. Keep the integration branch unmerged into main; never push or stash. The user has already authorized implementation, fixes, task merges and reviews: do not re-ask flags or routine design/fix permissions.

The user's settled semantics: message ACK deadlines start at canonical committed send, independent of registration/join/availability. Invitations default Lazy; an explicitly timed invitation has one deadline spanning registration and explicit acceptance, and warns its origin if missed. Registration is not a separate ACK. Handoff instructions always request message ACK, independently of invitation mode. Read the recorded design and accepted refinements rather than reconstructing them from conversation.

## Durable state and worktrees

- Run worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`
- Run branch: `super-auto/handoff-delivery-redesign`
- Run directory: `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign`
- `run.md` is authoritative: phase `code`; epic `ht-0lo`; flags `planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true`. Do not reset phase/counters/approvals on resume. Pause is recorded here, without inventing a phase token.
- Root checkpoint before this handoff: `31ffd371fff7e02d3c1b1b0e2be6b8ef835abd88`.
- Composition worktree: `.worktrees/upstream-composition-478ec4a9` under the run worktree; branch `super-auto/handoff-upstream-composition`, last known tip `3ad07776ca21b5a8aea0cd624447142e536e0d40` (A and B composed, D/C pending).
- **D's current actual worktree**: `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2` under the run worktree. Its stale `.3.2` directory label is intentional. Actual branch is `task-ht-0lo.8.4`. Do not derive a different path from the task ID.
- D frozen checkpoint: `4c4988d9ee937c333d602bac25dca25fc2c07d37`; tree `1589136ce7568bcd72b0050fa8d947e6f49503ab`; base `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`. At freeze its worktree was clean. Cumulative diff is 74 paths, about 18,577 additions.
- Main advanced during this turn: `026ad3ab444779216748cc5733e2241e2834ebaf`, via `58ad5634` required-invitation reattachment fix, then v0.8.1 release. Recheck main before composition/final verification.

Read `code-pause-handoff.md` for the code coordinator's exact descendant/process/review checkpoint. All four active code descendants were interrupted on pause, D's worktree was clean, and the owned-process check found no remaining task15 Cargo/test/helper. Unrelated worktree processes were left untouched. The three final review reports had not yet been saved: their communicated findings are partial coverage, not completed review verdicts. Native agents from this session are not a durable dependency; restart roles from these artifacts in a new session.

## Completed versus remaining

Original 11 implementation leaves, their parents, design fixes `.6/.7`, A `.8.1`, and B `.8.2` are closed/merged within this run. `run.md` records 19 completed and four pending: D `ht-0lo.8.4`, C `ht-0lo.8.3`, composition parent `.8`, root `ht-0lo`.

A preserved released SQL1..31 and original32..38, added39 and genuine old upgrade/rollback evidence. B completed large-body admission, summary/sidecar, legacy limits and frozen identity fixes, with whole/interface and main43 seam reviews CLEAN. B final source `cdbf3c37e873a2cb571c098450602a53f81a6185`, merged into composition at `3ad07776`.

D implements bounded canonical in-turn presentation (Hook/Watch/mod), passive invitation context, exact explicit post-flush confirmation and control epochs, relevance discovery, ordinary Lazy publication signal, and advisory readiness. **Capability `delivery.presentation_v2` is intentionally unadvertised. D is incomplete until its final findings are fixed/reviewed, then a narrow activation delta is reviewed and real advertised transport/native paths pass. Fallback-only is not completion.**

C combined validation, D merge, root promotion, final whole-epic review, PR roast, final report/upstream feedback and exact-tip readiness checks have NOT completed. Main has NOT received a ready notification.

## Current review findings — resume here

Three final D reviews were dispatched against exact frozen4c4988: whole-task, schema, interface. These are cumulative reviews, not approvals inherited from earlier focused refinement reviews. They were still in progress when the user paused; use partial review artifacts and exact coverage rather than claiming CLEAN.

Confirmed findings communicated before pause:

1. Schema and whole reviewers independently found **three `revisit_candidates` invitation_id reads in `src/store/invitation_content.rs` that allocate `String` before validating type/UTF-8/128-byte ID bounds**. A malformed private ID can reach a typed constructor and panic. Other private readers had already received borrowed SQLite `ValueRef` validation. Prepare the same narrow integrity fix for these three reads, with actual corruption/no-effects regression tests. Do not add adversarial caller verification.
2. Whole reviewer found stale newly added test `presentation_v2_replays_one_canonical_invitation_until_emit_confirmation`: raw PullV2 JSON lacks required `render`; Confirm expectation still uses an obsolete scalar rather than current `status`/`acknowledged` shape. The last 24-test focused selection did not cover it. Audit remaining changed tests for this interface drift, update to settled interface and run a consolidated affected service selection.

Author was told to prepare fixes/tests **read-only**, pending complete findings; no source correction or capability activation had been authorized as already applied. New session should verify exact worktree state, read every final/partial review and resume the existing autonomous fix authority. Re-freeze/review the corrected source.

## Latest actual verification

At frozen D source:

- Strict `nice cargo clippy --locked --all-targets --all-features -- -D warnings` PASS, 46.96s.
- `cargo fmt --check` and harness-neutral check PASS (0 violations).
- Final affected selection: **24/24 PASS**, 2.340s runtime, actual multifile crate build59.70s. Includes all14 canonical relevance/private tests plus reset, native System/User, control slots, origin/goal, healthy warning/ordinary and Watch seams.
- Actual log `/private/tmp/ht-task15-final-affected.log`; strict lint `/private/tmp/ht-task15-final-clippy-successor.log`. Durable hashes/results: `task-15-final-affected-sourcepoint.json`. Its initial parser included spurious `v0.8.0` among test names; coordinator requested metadata-only correction preserving actual log/hash, no retest solely for this correction.
- These are **focused tests, not all D tests, final combined sweep, default-feature check, full suite or merge-readiness proof**. Newly discovered stale test demonstrates that limitation.

Lint cleanup incident: an overbroad test edit was caught before compile. Original immediate pre-cleanup bytes were not saved (only hash), so no byte-identical restoration claim is possible. Full reviewed367→current source diff and bodies, two known setup corrections, mechanical Option changes and appended private proof tests are auditable in `task-15-attention-cleanup-restoration.diff/json`; all14 actual affected tests passed. Final reviewer must inspect current bodies. `task-15-report.md` and appended friction record preserve this limitation.

## Sole Cargo lane and project rules

Only D's warm `.3.2` source/cache may run Cargo while it owns the lane. Use explicit absolute `CARGO_TARGET_DIR=<D-worktree>/target`, `CARGO_BUILD_JOBS=4`, `nice`, locked/all-features. Source helpers/reviewers do not compile or mutate this lane. Before any helper transfer, obtain a NEW immediate quiescent ACK, verify baseline and full path/hash mapping, then apply sequentially. Do not rely on an old drained observation.

Read AGENTS.md and normative TRUST-POLICY before protected code. Per-change lint as above; format and harness-neutral checks; before merge `nice scripts/check-default-features`. Relevant tests only; no routine full suite until ht-zo4 lands. Suite/integration sweeps require fresh `HT_LEAK_RUN_ID` and scoped `scripts/check-no-leaked-processes --run-id ...`; clean up only owned processes. Never stop shared Herdr, foreign panes/servers, or write real user config. New tests follow explicit-target/test-support requirements. Every commit has truthful Outside-adapter metadata and model-visible string accounting.

## Main026 composition seam

`task-14-main026-preflight.md` SHA256 `86ac61c098363670ec12ce9c827c12ac1eb6ba26f106b722b68e95d050ebb6ba` describes exact43→026 delta: ten paths, no schema/wire/capability movement. Main's historical requirement-linkage guard must coexist with modern Lazy policy and D context epoch writers; pending-first acceptance shared helper overlaps D service_controls. The old third-required-reattachment Conflict oracle is stale and must become a fresh-invitation-ID oracle. Independent ht-clj fix is now on main; do not duplicate or discard it. Compose after frozen D review stage, review conflicts AND relevant clean auto-merges, run actual combined tests.

## Accepted D refinements — don't reopen without evidence

Authoritative sections of `task-15-report.md`, `task-15-bounded-design-dispositions.md`, final path inventory/entrypoint map and exact refinement reviews govern. Key limits:

- Fresh Pull physical source bound433; passive Refresh160. Relevance13 sources, bounded26 seeks plus explicit keyed/witness work. Publication≤100 physical/200 keyed/3 seeks. Confirm no discovery.
- ≤32 ordered blocks, actual native/Watch encoded page≤65,536 bytes, actual framed RPC within envelope; sizing≤352 passes plus final encode. Registered finite codec descriptors, no neutral harness branches.
- One bounded nonordinary outbox; original fixed control slots, explicit emitted subset, one previous token; replay/AlreadyConfirmed do not re-arm health or rotate output.
- Exact Live Watch owner tuple under same entry lock; per-seat permit survives replacement. Hook post-flush completion and Watch post-flush Confirm; no automatic message ACK, invitation acceptance or model-uptake claim.
- Private ordinary carry cap155731 and partial49415 have derived accounting; legacy ordinary8KiB display remains. Exhausted thread cursor sentinel U+10FFFF is canonical only in two after_thread fields; don't restore ASCII-only advice.
- Refresh uses raw presentation load and independent headers; preserve presentation state and four publication fields across both rebind owner orders.
- **No new pre-materializer generic warning/ordinary Attention guarantee.** Selected existing bounded materializer→native bump→registry path is intended; two distinct Warn Notice and ordinary exact-body/ack_required tests passed. Unsupported earlier expectations stay excluded, not called repaired production bugs.
- Unlanded40/41 additive changes only; released1..39 immutable. Migration40 no-CHECK ALTER still invokes SQLite STRICT named-table quick_check. Corrected accepted cost wording/proof `task-15-epoch-startup-cost-correction.md` exact2a845cc5 and CLEAN focused review supersede the misleading48 outer-VM/constant-read conclusion. Healthy driver bounds apply after migration, not during startup/outage. No workaround or deadline reset.

## Resume sequence

1. Read run/ledger/code pause handoff and exact partial reviews; verify all worktree/branch/source state. Re-establish coordinator, author and final reviewer roles without resetting the run.
2. Finish D whole/schema/interface audits, fix confirmed bounds/interface defects, batch meaningful affected tests and gates, freeze and re-review. Prepare activation proposal independently while source review runs, but advertise only after clearance; verify real advertised transport/native consumers.
3. Compose Main026 (or freshly observed newer main), B and reviewed D serially. C actual combined tests include Ensure Lazy→D FULL/no ACK/accept, true old31→final41 migration and driver bounds, six lazy_recovery_context cases, B max-body/summary/sidecar/NonStarted/Some-wire seams. Heavy B positive chains require their nextest ALLSLOTS group. Focused sweep with scoped leaks.
4. Code coordinator completes final whole-epic CLEAN, terminal tracker buckets and root promotion. Only then parent transitions `run.md` to roast-code and invokes fresh `$super-roast` PR round1 against exact combined SHA and observed base SHA.
5. PR roast is **not run yet**. A prep-only parent agent read current skill/workflow/prompts. In a new session read them anew: manual native fanout because no Workflow tool; honest same-family OpenAI seat labeling; triage→six core+conditional scouts→fresh dedupe→distinct judge methods→fresh reporter/coverage. Follow actual skill model tiers. Coordinate any Cargo repro with sole lane. All inputs exact/frozen; iteration from run.md, autonomous=true, run-directory override.
6. Scope-filter confirmed findings BEFORE creating fix beads, preserve exact finding keys/dispositions; Blocking always in scope; run prescribed bounded fix loop. Do not claim ready with unresolved Blocking.
7. Write final report exactly five sourced sections, first metrics pending; outermost upstream-feedback fresh strongest analyst ONCE, park/commit draft autonomously (no external issue approval); rewrite only metrics line. Design roast round2 is parked `clean [low coverage]`, so final status must retain at least `[degraded: design low coverage]`, never bare clean.
8. All report/feedback/metadata commits before final exact-tip format/lint/neutral/default-feature/affected combined checks/leaks. No stale SHA readiness stamp. Retain worktrees/branch and notify exact main seat only when actually ready; no main merge/push.

## Durable Herdr routing and eventual notification

Read `$herdr-threads` skill. Expected routing is exact:

```json
{"instance":"51a03f2a-3c7e-46f5-b694-3aed374557d7","state_dir":"/Users/alepar/.local/state/herdr/plugins/herdr-threads","host_endpoint":"/Users/alepar/.config/herdr/herdr.sock"}
```

Use a trusted hook command group only if ALL canonical fields match; missing/null/different/ambiguous cannot supersede this target. Otherwise exact fallback:

```sh
herdr-threads --state-dir /Users/alepar/.local/state/herdr/plugins/herdr-threads --host-endpoint /Users/alepar/.config/herdr/herdr.sock inbox
```

Use CLI-only escalated rule where needed. Inbox once on new attention, exact printed `next:` continuations; never read/body already displayed handoff task. Default text inbox ACKs fully displayed messages, accept invitations separately. Launch never accepts/ACKs. No polling/follow while waiting; end turn and let hooks notify.

Old session own seat s2wUWZ7n3 (w4:t8Z/pET); joined active psa-global teW1XBj6v. New session must resolve its own seat and follow mandatory psa-global membership by active name, not this old ID. Project coordination thread taQXwBH7X dev-herdr-threads was compatible/joined; main exact seat sRpHoWRVk (w4:t1/p1). Revalidate topic/goal/membership/exact target at send time. Original teAA2NJVh design seat is NOT main. Last main update msgJsFHrb said NOT ready; no ready notification since. Don't broadcast project status to psa-global.
