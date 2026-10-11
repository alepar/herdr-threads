# Task 10 implementation report — ht-0lo.3.2

Status: DONE_WITH_CONCERNS (implementation committed; focused checks/gate passed; independent review and final branch readiness remain parent-owned).

Own branch `task-ht-0lo.3.2`; BASE `d4d32b1b67ed6aa15c314a0d872d50e37e25bb9b`.
Own worktree and persistent target: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2` (`target` inside this root).

Applied TDD and SDD implementer workflow. No subagents, Herdr commands, real user configuration writes, main merge, push/stash or foreign process changes. Read AGENTS, TRUST-POLICY, task brief, root/handoff normative design and settled producer reports/interfaces. Coordinator owns review, integration, epic/run ledger, final post-roast checks and notifications.

## TDD and verification log

All Cargo uses this exact private target, CARGO_BUILD_JOBS=4, nice, locked/all-features and coordinator-exclusive lane. Source freezes for every command. Inherited reviewed/gated baseline is not rerun as a cold or full-suite baseline.

First RED command: `CARGO_TARGET_DIR="$PWD/target" CARGO_BUILD_JOBS=4 nice cargo test --locked --all-features --lib maximum_instruction_file_freezes_through_explicit_launch_retry`; session98303, full log `/private/tmp/ht-task10-red.log`. Existing actual file-source retry fixture now asserts emitted child invitation Some(Lazy), no invitation deadline, and addressed Ordinary body; no producer edits precede this command. Result: meaningful runtime RED, invitation confirmation None versus expected Some(Lazy), selected1/failed1, runtime0.15s, first-new-path build9m06s. Initial nice syscall denied by sandbox but Cargo continued; next commands use scoped CLI escalation. At own process check, cargo PID49742 had elapsed2m26s and own rustc PID49781 was active (~30.9% CPU). This first build at a new source/worktree path exceeds60s raw wall time. It does not by itself measure same-path incremental regression or establish that the one-minute incremental budget passed; later same-path measurements are separate. No unsupported cause claim.

## Scope admissions

Original brief files plus coordinator-authorized source `src/ports.rs`, `src/harness/adapter.rs`, `src/harness/{claude,codex}.rs`, `src/harness/hermes/launch.rs`, `src/test_support/mod.rs`, `src/host/native.rs`, `TRUST-POLICY.md`. Additional fixtures: `tests/contracts.rs`, `tests/cli/follow_retry.rs`, `tests/integration/{channel_activation,topology_activation}.rs`, `tests/lifecycle_ux.rs`, `tests/store/{archival_legacy,topology_handoff}.rs`. Optional owned launch descriptor needs honest absence, not a fabricated fingerprint. Hermes diagnostic readiness is likewise advisory; actual selected profile/API/native grammar and any used configuration rechecks remain safety guards. Mechanical fixture changes preserve historical omitted handoff policy and existing assertion strength.


## Incremental evidence (ongoing)

- session59547 `/private/tmp/ht-task10-policy-red.log`: actual independent flags parser, Claude missing readiness and Hermes missing readiness failed before their producer changes;3 selected/3 failed, same-path build1m17s, runtime0.50s. This breaches the60s incremental budget; no profile/budget changes or causal assertion.
- session86270 `/private/tmp/ht-task10-policy-first-green.log`: compile-only failure from four mechanical fixture adaptations; excluded from behavioral evidence. Coordinator admitted `tests/cli/cooperative.rs` policyNone literal. Corrected before retry.
- session20603 `/private/tmp/ht-task10-policy-first-green-retry.log`: build6m21s, runtime1.52s,4 selected/1 passed/3 failed. Parser passed; capability-before-effects meaningful RED (unexpected success). Claude/Hermes retained readiness veto in registry was still unfixed. No overall GREEN claimed.
- session17988 `/private/tmp/ht-task10-policy-green-second.log`: build1m42s, runtime0.71s,5 selected/4 passed/1 failed. Original four now GREEN. New linked wrong-key assertion incorrectly used an unrelated unregistered operation, whose legacy bypass is intentional; this is fixture error, not behavior RED. Corrected to actual opposite registered child-role keys.
- session21659 `/private/tmp/ht-task10-linked-green.log`: build20.65s, runtime0.41s,1 selected/1 passed. Exact modern Lazy/RequireAck matching invitation and addressed Ordinary send pass; confirmation/invite duration/message duration/registered child-role key/mode changes refuse, total_changes unchanged. Historical None test preserved.
- session18202 `/private/tmp/ht-task10-canonical-first.log`: build44.12s, runtime0.292s,3 selected/3 failed before effects. Fixture prerequisites: static ADVERTISED omits conditional invitation policy capability, so actual producer refusal was correct; not behavioral RED. Owning canonical fake discovery was corrected to advertise its implemented policy handlers; helper visibility warning corrected without suppression. Scoped `scripts/check-no-leaked-processes --run-id 80b233de-c5be-4901-bcd4-2d20457b23e1` passed.
- session80567 `/private/tmp/ht-task10-canonical-second.log`: build1m15s/runtime5.144s;10 selected/7 passed/3 failed. Explicit pane fixture incorrectly used Human for RequireAck; canonical warnings initially omitted real daemon bounded materialization. These fixture prerequisites were corrected; not behavioral RED. Run ID6d6be017-45cd-43e5-9be7-b77771496bd2 cleanup passed.

Further explicit admissions: `src/harness/hermes/runtime.rs` separate launch-input fingerprint retaining full original observation hash and validation; `src/harness/registry.rs` exactly two shared diagnostic-only readiness veto removals; `tests/store/topology_attachment_guard.rs` parameterized real canonical identity before digest/Begin. No root run ledger edits.

Frozen semantics: explicit policy/overrides freeze before staging; omitted numeric None freezes the choice of canonical default, resolves once at corresponding canonical child creation, and exact-key operation replay preserves stored duration/deadline/outcome after config changes. No new default-read/status protocol. Historical absent policy uses serde skipNone and exact prior omitted child fields, preserving digests.

Readiness: hook descriptors are optional honest None. Registration/scope identity, native binary/API/profile/home, argv/environment, seat/occupant/empty shell and host guards remain; used config rechecks are retained. Hermes launch-input stamp removes only four known schema3 top-level diagnostic values (`enabled`, `disabled`, `config_quality`, `fallback_kind`); full original stamp, every validation/fallback rejection and native/source/module/interpreter/environment/bootstrap/API input remain. TRUST wording updated in same source state. Embedded guide remains198lines (<200).


## Canonical producer seam and correction evidence

Task7 missed the actual native publication lineage: `src/store/messages.rs` leaves immutable `author_kind` NULL, whose native/built-in actor-seat interpretation is explicitly retained by migration0002. The original Task7 triggers and bounded projector required explicit native and therefore produced a warning but no sender origin for real guarded handoff sends. Actual public guarded handoff tests exposed this RED after executing real bounded work materialization. Coordinator admitted the seam fix. All six migration0035 trigger predicates and the bounded receipt projector now accept explicit native OR historical NULL only with no service author, a canonical same-instance actor seat and the exact published recipient association. No author/history rewrite. Tests cover actual public ordinary send through prepare/permit/mutate, actual guarded handoff Begin→children→Complete, genuine NULL v34 populated upgrade/rollback/bounded bootstrap/reopen, NULL/no-actor and service/built-in/wrong-audience/raw-overlay negatives. Fresh independent provenance/schema seam review is required and parent-owned.

- session2943 `/private/tmp/ht-task10-canonical-third.log`: build35.00s/runtime1.505s;4 selected/1 passed/3 failed. Both absent capabilities and send-time-only partial capabilities refuse before journal/native effects. Explicit-pane claim equality was a fixture mismatch. Two canonical overdue warning-origin failures are actual producer seam RED. Scoped run556a3b76-d269-4cab-a659-6ea345315f74 cleanup passed.
- session45171 `/private/tmp/ht-task10-focused-regression-first.log`: warm build0.03s/runtime29.152s;302 selected/272 passed/30 failed. This first regression selection exposed the categories below, not30 new behavioral REDs. Scoped run84fe3976-c867-4431-b9d4-900ed3bce5a4 cleanup passed.
- session27261 `/private/tmp/ht-task10-corrections-first.log`: compile-only missing import and all-None fixture type inference; excluded from behavioral evidence and corrected together.
- session5512 `/private/tmp/ht-task10-corrections-retry.log`: same-path build1m02s/runtime17.975s;108 selected/99 passed/9 failed. Actual ordinary NULL-native warning, genuine NULL upgrade/reopen, origin negatives, both canonical handoff warning cases, explicit maximum body, independent parser/capability and default-before/after tests passed. Remaining retained fixture/status and Failed-readiness issues corrected below. Scoped runc41eca11-afbf-4732-b46d-b31324499c2e cleanup passed.
- session8502 `/private/tmp/ht-task10-retained-readiness-corrections.log`: build1m00s/runtime5.418s; exact remaining9 selected/9 passed. Scoped run3a359388-2658-4ebe-ac66-74f8830e193c cleanup passed.

### First regression's30 failure classifications and exact assertion changes

Three FencedClient cases (`handoff_fenced_begin_precedes_effects_complete_follows_successful_flush`, `handoff_fenced_complete_survives_local_removal_failure_then_cleanup_only_retry`, `registered_handoff_report_replay_and_completed_cleanup_keep_frozen_identity`) incorrectly classified the newly required capabilities read as a mutation. Read-only discovery forwards without changing Begin-before-child/Complete-after-flush assertions. `maximum_instruction_file_freezes_through_explicit_launch_retry` uses a genuine Agent fixture and asserts the complete expected Agent claim; no weakening of equality.

The two canonical warning failures are the NULL-origin seam described above. The21 delivery/history cases were affected by pre-Begin capability refusal or inconsistent old Begin versus new read-only Status fixtures: retained Human actor prerequisite; retained Agent evidence and elected-daemon prerequisite; preterminal/surviving-progress oversized and unknown-staged-field refusals; both publication-sync failure cases; canonical completed binding/archive/restart retry; strict completed cleanup; directory-sync/write/flush/intent/progress cleanup retries; conflicting terminal; changed participation; terminal corruption/unknown nested/unknown header; copied UUID/root/socket guards. The producer now uses the existing exact frozen typed Status on pending and terminal Agent paths. Only actual canonical Completed skips capability/Begin; pending/new still require both semantic capabilities before effects, and unknown/unavailable status refuses. Canonical fake routes Status to actual SqliteStore.delivery_query. Terminal fake validates the full exact frozen envelope; historical corruption/refusal assertions remain. The Human negative fixture uses unchanged legacy child and Begin/Complete producers to assemble genuine v1 staged/report/terminal encoding under its original frozen Human bytes; the current Agent-only envelope and zero-call/zero-cleanup Human wrapper refusal remain unchanged.

The two obsolete Hermes veto assertions now prove missing/disabled/failed/foreign/partial/wrong-endpoint readiness is diagnostic with honest hook=None and actual submission, while missing selected native API still refuses before seat/native/record effects. Independent used-asset/config/binary/native-profile drift negatives remain. `publication_public_route_refuses_unavailable_daemon_before_publication` expects the truthful earlier HostUnavailable discovery refusal and retains zero-publication assertions.

### Test fixture prerequisites and limits

Canonical fake discovery advertises only conditional invitation/bootstrap handlers actually implemented by its canonical delivery/topology handlers. The partial-cap negative independently omits invitation.confirmation_v1 while send-time receipts remain present. Canonical receipt tests run normal `materialization::advance_work` before `scan_due`; they do not fabricate receipts, registration, acceptance or ACK. Combined missing-readiness Hermes test uses the actual guarded NativeLauncher and canonical Begin/Create/Invite/Send/Complete; recipient remains unregistered, deadline warns original sender, and a later explicit canonical ACK clears the warning. Launch itself supplies no ACK/acceptance/registration. Explicit duration7s and invitation11s are distinct; omitted numeric defaults are tested before child creation and after child creation/lost reply/retry.

All Cargo commands use own exact CARGO_TARGET_DIR, CARGO_BUILD_JOBS=4, nice, --locked --all-features and source freeze until drain. First new-worktree9m06 build, later production rebuild6m21 and same-path measurements are reported separately. Multiple same-path builds exceeded60s; no speed budget/profile was raised, and no unsupported warm-cache or regression-cause claim is made. No full-suite/default-feature readiness gate was run by this task; parent owns expensive final readiness/default checks after review.


## Final focused regression records

- session18203 `/private/tmp/ht-task10-final-lib.log`: build1m03s/runtime19.976s,146/146 PASS. Added actual already-joined ended-binding recipient (canonical Participants/SeatInspect, accepted old invitation, no new invitation) plus real message deadline and sender warning. Existing/new-tab max-body fixtures now assert canonical independent7s message and11s invitation values, unchanged by exact retry. All delivery retained/corruption, linked-policy, mapped cooperative and Hermes native/used-configuration controls pass. Scoped ddc9be9a-9829-4252-985e-f7bdd7afc0e6 cleanup passed.
- session29363 `/private/tmp/ht-task10-mapped-combined.log`: compile-only previously missed adapter_smoke mandatory hook consumer. Coordinator admitted `tests/harness/adapter_smoke.rs` mechanical Some assertion under genuinely installed synthetic hook, preserving exact fingerprint comparison. No behavior evidence from compile failure.
- session88290 `/private/tmp/ht-task10-mapped-combined-retry.log`: build24.28s/runtime5.839s,209/209 PASS; mapped contracts/archival_legacy/topology_handoff/lifecycle_ux/adapter_smoke. Scoped7e6b0cac-6e3a-41b9-84da-461e6e79e0bf cleanup passed.
- session18275 `/private/tmp/ht-task10-mapped-integration.log`: build11.11s/runtime6.068s,11 selected/9 passed/2 failed. Two manual modern replay constructions incorrectly used historical confirmation=None; canonical OperationPayloadMismatch refusal was correct. Coordinator admitted deriving confirmation and independent invitation/message overrides from exact original frozen policy in those two constructions. Historical omission, digest/key equality, tamper/zero-effects assertions retained. Scoped6df90e93-84fe-4b80-aa25-9270f0185c0f cleanup passed.
- session87900 `/private/tmp/ht-task10-mapped-integration-retry.log`: build2.36s/runtime2.610s, exact2 corrected integration cases PASS. Scoped67141972-8943-4c6e-80d3-5bca364d5b1a cleanup passed. All11 mapped cases thus accounted after correction, without repeating the9 prior-green cases.

The new actual canonical completed/pending capability test uses real SqliteStore status after real Complete committed with a lost reply. Unsupported pending status refuses before Begin or child rows. Completed presentation allows only the exact validated typed Status, performs no capability read/Begin/child operation, and leaves canonical counts unchanged. This is preservation evidence, not invented RED for previously correct behavior.

Task7's approved coverage used explicitly native synthetic author rows and did not cover actual producer NULL-kind lineage. This task's seam fix is distinct new integration evidence and requires independent fresh review; previous Task7 approval is not claimed as approval of this correction.

### Reproducible focused commands

All run from ownworktree, with CARGO_TARGET_DIR set to the exact ownworktree/target, CARGO_BUILD_JOBS=4, a unique HT_LEAK_RUN_ID as listed above, and full logs at listed paths:

```sh
nice cargo nextest run --locked --all-features --lib -E 'test(cli::handoff::) | test(cli::handoff_delivery::) | test(cli::topology_handoff::) | test(cli::launch::) | test(harness::launch::tests::) | test(store::control::tests::topology_attachment::)'
nice cargo nextest run --locked --all-features --lib -E 'test(cli::handoff::) | test(cli::handoff_delivery::) | test(cli::launch::task23_tests::) | test(publication_public_route_refuses_unavailable) | test(receipt_origin_actual_ordinary_send) | test(receipt_discovery_populated34_upgrade) | test(receipt_discovery_orphan_wrong_audience)'
nice cargo nextest run --locked --all-features --lib -E 'test(actor_prerequisite_retained_human_delivery) | test(fix_retry_publication_sync_failure) | test(retry_canonical_completed_after_both_bindings) | test(retry_directory_sync_failure_after_unlink) | test(retry_failed_write_and_flush_retain) | test(retry_intent_removal_failure) | test(retry_progress_removal_failure) | test(hermes_readiness_is_diagnostic_but_missing_native_api)'
nice cargo nextest run --locked --all-features --lib -E 'test(cli::handoff_delivery::) | test(cli::launch::task23_tests::) | test(maximum_instruction_file_freezes_through_bootstrap) | test(cli::follow::tests::) | test(cli::cooperative_tests::) | test(store::control::tests::topology_attachment::)'
nice cargo nextest run --locked --all-features --test combined -E 'test(contracts::) | test(archival_legacy::) | test(topology_handoff::) | test(lifecycle_ux::) | test(adapter_smoke::)'
nice cargo nextest run --locked --all-features --test integration -E 'test(channel_activation::) | test(topology_activation::)'
nice cargo nextest run --locked --all-features --test integration -E 'test(elected_live_registered_phase_replays_recheck_current_caller_recipient_and_payload) | test(elected_public_new_tab_completes_one_native_create_and_start_then_replays_history)'
nice cargo nextest run --locked --all-features --lib -E 'test(cli::follow::follow_retry::) | test(handoff_policy_completed_canonical_status_bypasses_missing_capabilities_but_pending_refuses)'
```

The final146 selection's cli::follow::tests:: term selects no follow_retry tests; the final separate correctly registered follow_retry selection above closes that coverage. No misselection is counted as evidence.


## Final task handoff

Commit: `cccec16b42467b747ebf6389b89a8d99c7e6a6da` (`Freeze independent handoff receipt and invitation policies`), own task branch `task-ht-0lo.3.2`. Git status empty after commit;39 admitted files. Shared report is the only run document edited by this task; no root ledger/runstate changes.

- session45234 `/private/tmp/ht-task10-final-follow-status.log`: build34.24s/runtime0.702s,13/13 PASS. Actual registered follow_retry12 plus real canonical completed-versus-pending contract1. Scoped aee3e563-1f89-47f1-a17c-fb81f2f0fd09 cleanup passed.
- mandatory first clippy session14465 `/private/tmp/ht-task10-clippy.log`: failed large_enum_variant because independent policy fields increased Top::Handoff to424 bytes versus next216. No lint suppression. Audited all three Top::Handoff consumers and local clap Args-for-Box impl; narrow Box<HandoffArgs> changes storage only.
- session25944 `/private/tmp/ht-task10-box-parser.log`: build14.99s/runtime0.314s,5/5 PASS. Actual all-route independent parser policy and positional/file/stdin byte preservation, max body and acquired-once retry. Scoped83a699d0-8f85-4d7c-b9f6-2285fab8495a cleanup passed and final recheck passed.
- mandatory exact clippy retry73650 `/private/tmp/ht-task10-clippy-retry.log`: `CARGO_TARGET_DIR=<exact ownworktree>/target CARGO_BUILD_JOBS=4 nice cargo clippy --locked --all-targets --all-features -- -D warnings` PASS exit0,41.53s. No source edits after this gate.
- final `cargo fmt --check` session41881 `/private/tmp/ht-task10-fmt-final.log` PASS exit0; `git diff --check` PASS; embedded guide198 lines (<200). Earlier fmt35527 also passed before narrow parser storage correction; final check is authoritative.

Final parser command:
```sh
nice cargo nextest run --locked --all-features --lib -E 'test(handoff_policy_parses_independent_deadlines_before_body_effects) | test(handoff_instruction_sources_preserve_message_bytes_and_exclude_conflicts) | test(handoff_stdin_acquisition_is_bounded_and_preserves_exact_message_bytes) | test(acquired_stdin_survives_retry_after_reader_is_unavailable) | test(maximum_instruction_file_freezes_through_explicit_launch_retry)'
cargo fmt --check
nice cargo clippy --locked --all-targets --all-features -- -D warnings
```

All selected tests accounted for across final relevant selections; no full-suite/default-feature claim. Every focused suite used explicit registered lib/combined/integration targets, source freeze, captured logs and owned leak UUID. No test/daemon/private host children remain. No shared Herdr actions, real user config writes, external messages, main merge, push or stash.

Historical Human negative is a local v1 encoding compatibility fixture using the pre-existing Client keyed legacy API, not a canonical live-Human authorization positive. Its original Human semantic remains unchanged and current wrapper refuses before canonical calls/output/cleanup. Canonical status positive coverage separately uses actual SqliteStore committed completed Agent operations after binding/archive/restart and actual lost Complete response.

Concerns/limits: first new-worktree path build9m06 and later production rebuild6m21 are distinct from same-path incremental evidence. Multiple actual same-path builds (1m17,1m42,1m15,1m02,1m03) exceeded the60s budget; latest parser incremental14.99s and final lint41.53s pass their budgets, but do not erase earlier overruns. No full suite, speed profile/budget alteration or unsupported attribution of cause. Parent owns fresh Task10 review, explicit all-six-trigger/bounded-projector canonical NULL provenance/schema seam review, integration, PR roast, final default-feature/expensive readiness at cleared SHA, and main-tab notification. Task10 leaf completion is not merge authorization or branch-ready assertion.
