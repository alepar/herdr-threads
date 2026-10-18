status: clean [degraded: plan roast skipped, outstanding findings: 10, root acceptance pending: finish: human decision pending; owner: main release coordinator; ht-1urq.6:concurrent-topology: gap open; owner: super-auto / main qualification; final-sweep:apply-lane: gap open; owner: super-auto / main qualification]
metrics: pending (upstream-feedback not yet run)

## Implemented

- Eleven implementation leaves closed and landed; ten review bookkeeping beads closed; root `ht-1urq` remains open. Latest outcomes: ten review-clean, one completed after a review fix; two historical durable FIXED records comprise that code fix and separate raw-log evidence recovery. Ledger-only, completed-only, Git-only and ledger-only-Git ID sets are all empty; append failure/retry totals remain unknown. Source: `.superpowers/sdd/ht-1urq-plan/metrics-evidence.json`, latest `Metrics:` block in its `progress.md` source.

| Task | Delivered | Commit range |
|---|---|---|
| `ht-1urq.1` | Seam contract: frozen launch directory context | `92d5860..078ce1b` |
| `ht-1urq.2` | Parse and validate new-tab profile directory inputs | `27d2670..fc57b6c` |
| `ht-1urq.3` | Map explicit Codex and Claude profile directories | `92e2f1e..806f2c0` |
| `ht-1urq.4` | Map explicit Hermes roots and retain unsupported adapter behavior | `2647440..9afe89a` |
| `ht-1urq.5` | Validate and retain context-bearing bootstrap identities | `534e9da..cbd2407` |
| `ht-1urq.6` | Apply frozen profile environment through new-tab handoff | `9a5f025..010094b` |
| `ht-1urq.19` | Forward validated profile context through native tab creation | `e5ded0b..6f5b10c` |
| `ht-1urq.7` | Teach names, immediate delivery and receipt recovery in shipped skill | `ece39c4..0e5fb48` |
| `ht-1urq.8` | Document launch directory support and proposed minor version | `c509008..18f4d92` |
| `ht-1urq.9` | Configuration smoke: selected profile and cwd in release matrix | `9cc58dd..71b8d5c` |
| `ht-1urq.10` | Integration sweep: handoff profile directories and skill guidance | `b98f3ec..8a56f28` |

- Task table source: closed beads and ledger completion lines reconciled in `.superpowers/sdd/ht-1urq-plan/metrics-evidence.json`; task reports under that workspace retain implementation evidence.

- New-tab handoffs retain validated existing absolute profile/cwd selections across retry, with adapter-owned Codex, Claude and Hermes mappings; unsupported pane/existing-recipient combinations refuse before effects. Shipped guidance uses names in prose, urgent delivery when reaction is needed now, and receipt-dependent ACK/recovery without changing receipt semantics. Source: `run.md` requirements R1–R12 and completed task records; branch diff against `b4d5f9628d17f869ab5b139b7f4ae3c933d02841`.

- Final candidate `7546897944cd4701674a640e0a7f07c83dfa7266`: 7,653 passed, 48 skipped, zero failed/errors; nextest summary 216.078s, command elapsed 216.369s. Scoped process/temp leak check, all-feature clippy (0.309s), neutral-core/default-feature checks, format and diff checks passed; prepared release-proof fixture and quiet compiler/test observations qualify this timing. Source: `run.md` root acceptance/sweep and `evidence/final-verification.{md,json}`.

- Complete independent review covered 98 selected slices and 12 hash-verified generated omissions; caller assessment is ready after the final sweep. Two code-roast rounds completed, second `clean (0 nits) [converged]`; no code redesign or confirmed roast defect. Source: `run.md` review collection, codeBuckets, roastCodeExit; `evidence/review-coverage-verification.json`; round-2 roast report.

## Remaining

- Main release coordinator must decide merge/release; proposed minor version is 2.1.0. Branch `integration/handoff-profile-and-skill-guidance` is retained; no merge/tag is authorized here. Design roast was explicitly skipped. Source: `run.md` idea/flags/root acceptance and `.superpowers/sdd/ht-1urq-plan/report-status.txt`.

- Attribution gap `ht-1urq.6:concurrent-topology`: historical concurrent libtest retry test failed once; isolated/serial and bounded exact-base/final controls passed, but source/runner causation is unestablished and nextest passes do not exonerate it. Owner: super-auto / main qualification; no supported task-base exoneration. Source: `run.md` root acceptance Recovery/runFailures; `.superpowers/sdd/ht-1urq-plan/topology-failure-investigation.md`, `task-7-report.md`.

- Attribution gap `final-sweep:apply-lane`: prepared sweep had 7,652 passes and one retry-backlog assertion failure in 387.486s during observed external release compilation. Quiet confirmation passed without product/test edits; failing interleaving and causation remain unknown. Owner: super-auto / main qualification; no supported task-base exoneration. Source: `run.md` root acceptance Recovery/runFailures; `evidence/final-verification-contention.json`, `evidence/apply-lane-investigation.md`.

- Retained-source projection has five final-review/roast sources, 112 raw occurrences, 109 finding groups and three other-section groups: 102 supported resolved groups (chiefly coverage closure, not 102 code fixes), seven outstanding finding groups plus three other-section groups, zero deferred and zero projection gaps. Original task-review finding population remains unestablished; these counts cover retained final-review/roast extractions only. Source: `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `metrics-evidence.json` supplies the older, explicitly partial population.

- Final review: Historical bounded reviewer ran no tests/lint/live reproductions; later caller checks do not rewrite that execution scope. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `inv-8759cc82-264d-4506-beaa-307c9600bb8d/final` / `Untested scope` #2, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Final review: Original failed assembly lacked digest.md; reconstructed diagnostic digest cannot supply original provenance. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `inv-b6e4e8fd-19d3-4193-81b5-f0887ab5e66f/final` / `Untested scope` #3, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Final review: Five-release stand-ins prove declared child env/cwd consumption; shell preservation, account/model selection and live authentication remain unmeasured. Releasing agent owns authenticated wake e2e before release. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: main release coordinator for release-only validation; super-auto for retained evidence. Source: `inv-7506af49-ec8a-4bf3-b2e8-65c91547bf14/final` / `Untested scope` #2, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Final review: Seven historical raw logs remain unavailable; six exact-source replacement checks cannot restore their bytes or validate the unreproduced intermediate counter-fixture failure. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `inv-7506af49-ec8a-4bf3-b2e8-65c91547bf14/final` / `Untested scope` #3, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Final review: Topology attribution, historical topology, original finding population and append failure/retry totals remain unknown. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `inv-7506af49-ec8a-4bf3-b2e8-65c91547bf14/final` / `Untested scope` #4, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Final review: Historical nice/setpriority warnings did not prevent successful task checks. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `inv-7506af49-ec8a-4bf3-b2e8-65c91547bf14/final` / `Deferred OK` #1, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Final review: Earlier unavailable/bounded reports, original sidecar gaps and reconstructed-digest provenance remain historical limitations. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `inv-7506af49-ec8a-4bf3-b2e8-65c91547bf14/final` / `Deferred OK` #3, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Other extraction section: Old-binary rollback of newly opted-in pending handoffs remains outside the demonstrated compatibility guarantee; panel rejected a contract violation unanimously. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `roast-code/round-1` / `Rejected (with reason)` #1, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Other extraction section: Generic retryable final-boundary diagnostic loses the specific directory cause; spot check rejected a material contract violation, with no panel-strength confirmation. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `roast-code/round-1` / `Unverified nits (spot-checked)` #1, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.
- Other extraction section: Prior rollback candidate remains rejected; round 2 supplied no materially different evidence. Acceptance relevance: supported nonblocking judgment; occurrence remains outstanding. Owner: super-auto / main qualification. Source: `roast-code/round-2` / `Rejected (with reason)` #1, `.superpowers/sdd/ht-1urq-plan/final-projection.json`; `run.md` findingAcceptance.

- Administrative `docs/superpowers/specs/INDEX.md` status change (`draft` → `implemented`) was not roast-reviewed; product source is unchanged since complete review `89b60fcd` and converged roast `c9f8a4c0`. Source: `run.md` final-source-assessment; `evidence/post-roast-source-identity.json`.

- Integration worktree `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-profile-and-skill-guidance` and its ignored `.superpowers/sdd/ht-1urq-plan` evidence remain deliberately retained for main; do not remove their parent before archival/handoff. Source: `run.md` codeBuckets.worktreesKept.

- [cleanup:parent] Final parent process sweep is still pending at draft time; task cleanup and scoped suite leak checks do not establish parent cleanup. Next safe action: verify parent run processes from outside the worktree and persist the result before finish. Source: phase-7 cleanup obligation; `run.md` codeBuckets.processSweep is not yet a final parent verification.

## Gotchas & surprises

- Real native transport rejected nonempty profile environments despite higher-layer preparation, requiring separate defect leaf `ht-1urq.19`; cwd-only automatic handoff also needed a final-boundary recheck during its one review fix. Source: task `.19` and `.6` ledger/Recovery records in `.superpowers/sdd/ht-1urq-plan/metrics-evidence.json`.

- Review collection needed an explicit ordering exception and fresh complete-source invocation after two unavailable/bounded attempts; preserved diagnostic reconstructions do not become original producer artifacts. Source: `run.md` review-order-exception/collection history and `review-intent-recovery.md`.

- Initial unprepared full sweep passed but took 428.731s, including 390.552s release proof; prepared intermediate sweep failed once and exceeded budget; final quiet prepared sweep met five minutes. Initial seeded task builds took 2m39–3m40 and some lint exceeded one minute; subsequent recorded incremental build was 55.16s. No causal production regression or exoneration follows from contention observations alone. Source: `evidence/final-verification.md`; `run.md` slowness; ledger `Slowness:` record.

- Original profile covers one 02:41–04:41Z invocation (2h00m; task graph 1h58m), 11 landings and 64 timed dispatches; observed residual graph has 24 edges, depth seven, width 1.6, historical topology unknown. Planning diagnosis was 4m/3%; task `.6` bottleneck 19m/16%. Largest modeled what-if was unlimited slots saving 47m/22% over first-dispatch-to-last-landing with planning retained. All-agent and task-slot peak were five, while parsed cap was one; model was 179% of actual, readiness unknown for eight beads, so slot-bound and what-if estimates are qualified, not measured savings. Source: `code-profile-original.{md,json}`, ledger `Profile:` lines.

- Round-1 step-back chose local work with no triggers/clusters; graph pass kept all five candidates and parked none; code roast converged in round two. Account-switch resume adopted the existing design without scope change. Source: `run.md` graph-pass/stepBackCode-round-1/resumeChange and round-1 step-back file.

## Entrypoints

1. Read `src/protocol/handoff/launch_directories.rs`, then `src/protocol/handoff.rs` and `src/protocol/capabilities.rs` for the retained seam and admission contract. Source: task `.1`/`.5` dependency order and base-to-branch diff.
2. Read `src/harness/adapter.rs`, `src/harness/{codex,claude,hermes}.rs` and `src/harness/hermes/launch.rs` for profile ownership; then `src/cli/commands.rs` and `src/cli/topology_handoff.rs` and `src/cli/topology_handoff/automatic.rs` for public selection and retries. Source: task `.2`–`.6` dependency order and base-to-branch diff.
3. Read `src/host/{native,transport}.rs` for actual tab forwarding, then `scripts/herdr-release-smoke` and `evidence/profile-directories.md` for executable matrix/public-retry proof. Source: task `.19`/`.9`/`.10` dependency order and base-to-branch diff.
4. Read `integrations/skill/SKILL.md`, `docs/agent-usage.md`, `README.md` and `CHANGELOG.md` for the shipped guidance and release proposal. Source: task `.7`/`.8` and base-to-branch diff.

## Smells

- Task `.6` merged its cwd-only final-boundary fix after one review fix pass without a second task review; later full review/roasts and final suite cover unchanged product source, while its historical concurrent failure stays unattributed. Source: ledger Recovery/fix history in `.superpowers/sdd/ht-1urq-plan/metrics-evidence.json`; `run.md` review/roast records.

- The authoritative clean roast does not remove the ten retained outstanding extraction groups or the two open attribution gaps; earlier findings-ledger measurement rows reflect their original snapshot and are interpreted through supported later projection dispositions. Source: `findings-ledger.md`, `.superpowers/sdd/ht-1urq-plan/final-projection.json`, `run.md` root acceptance and report-status qualifiers.

- No sweep-fix or regression-pass product change was made to turn the final suite green; a quiet prepared rerun passed, leaving the intermediate failure unexplained. Source: `evidence/final-verification.md`, `friction.md`, root acceptance Recovery.
