# Task 4 implementation report — ht-0lo.3.1

Status: DONE_WITH_CONCERNS; Important I1 ADDRESSED with meaningful RED/GREEN and mandatory lint. Fresh scoped re-review/literal CLEAN is coordinator-owned before integration. Earlier build-budget and migration-integration concerns remain below.

Worktree: `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.1`; branch `task-ht-0lo.3.1`; base `957544e8`.

## Implementation and integration interface

`HandoffArgs` exposes `body: Option<String>`, `file: Option<String>`, and `stdin: bool`. Clap's required, exclusive `instruction_source` group selects exactly one source. Legacy positional remains after `--`. Dispatch calls existing bounded `input::read_body` before constructing `HandoffRequest`, so the downstream interface remains one acquired `body: String` without source/path fields. The reader admits nonempty UTF-8 through 65,536 bytes and preserves NUL, whitespace and newlines. Request/protocol body validation now follows that ordinary-message contract; topic and goal validation stays separate.

Task 10 owns confirmation/deadline policy and diagnostic readiness. This change does not select invitation policy, add those CLI options, or change readiness guards. Task 2's new invitation field must receive mechanical `None` adaptations if needed at integration.

Frozen-body tests cover explicit launch, existing-recipient delivery, and bootstrap new-tab paths using actual file parser acquisition, source mutation/deletion after staging, lost replies, and retry. Both maximum ordinary text and maximum six-byte JSON escaping run through each path. A separate stdin test drops the source reader before replay and compares the exact repeated keyed send. Native argv is checked for absence of body and source path.

## Body-bearing serializer inventory

| Envelope | Body occurrences / escaping | Limit and exact preflight |
| --- | --- | --- |
| Public `SendMessage` / bootstrap commands | One body; six JSON bytes/input control byte | Existing global 1 MiB frame retained; fixtures serialize/decode actual wire requests before canonical calls. |
| Bootstrap canonical identity | One body | 128 KiB → 512 KiB; exact `encode_identity` before journal publication and canonical persistence. |
| Canonical recovery result | One identity/body | 256 KiB → 1 MiB; existing exact persistence serializer retained; CreatedPane and Cancelled recovery fixtures survive reopen. |
| Canonical completed wrapper | One identity/body plus retained report | Existing 2 MiB; actual maximum body + existing bounded quote-heavy native/report fixture. |
| Public completed + recovery reply | Two identities/bodies together | Existing global 1 MiB frame retained; actual combined `WireResponse` serializer/decode fixture covers U+0001, NUL, whitespace. |
| Local bootstrap progress | One identity/body plus request/creation metadata | 128 KiB → 512 KiB; existing request-bearing capacity preflight before host call retained. |
| Bootstrap journal origin | One identity/body | Existing formula `2 * MAX_IDENTITY_BYTES + 4096` unchanged; 266,240 → 1,052,672 bytes; exact header+semantic+two-newline preflight before allocator reservation/write. |
| Bootstrap linked child progress | One canonical completed wrapper when present | Existing 2 MiB; exact serializer before linked write and canonical completion remains. |
| Bootstrap terminal | Completed identity/body plus exact original JSON nested as string | Existing formula `MAX_COMPLETED_BYTES + 2 * MAX_BOOTSTRAP_ORIGIN_BYTES + 4096` unchanged; 2,633,728 → 4,206,592 bytes; exact terminal serializer before canonical completion remains. |
| Existing-recipient delivery origin | One payload/body | 65,536 → 524,288 bytes; exact origin preflight before allocator reservation/write. |
| Delivery terminal | Exact original JSON nested as string; progress contains no body | 131,072 → 1,048,576 bytes; exact preview serializer before canonical completion, same serializer before file write, bounded metadata/read/take/decode on replay. |
| Legacy handoff intent | One acquired request/body | Existing generic journal semantics preserved; body acquisition/validation remains bounded at 65,536 bytes. Legacy progress contains no body and retains existing 4 MiB metadata cap. |

A 65,536-byte U+0001 or NUL body contributes exactly 393,216 JSON bytes per occurrence. Nesting that original JSON string contributes 458,752 bytes for those body escapes. Two direct copies contribute 786,432 before metadata. These actual serializer fixtures justify the changed body-bearing ceilings; creation/attachment remain 128 KiB, linked local remains 2 MiB, completed remains 2 MiB, and global frame remains 1 MiB. Bootstrap derived formulas remain structurally unchanged by coordinator ruling. Oversized independent metadata/native/report checks retain their limits and are not truncated or reclassified.

Coordinator scope ruling: existing generic `Journal::load` / `decode_reader` historical acceptance is unchanged, including its existing unbounded generic read. No general journal cap is introduced. New handoff acquisition and owned retained-envelope loaders use bounded reads; fresh review must check every new path and exact pre-effect serializer checks.

## Migration and compatibility

Immutable `0028_handoff_topology.sql` bytes are untouched. New provisional `0029_handoff_body_bounds.sql` rebuilds only the parent identity and recovery tables at original names, copies all columns verbatim, and restores unchanged indexes/triggers. Existing populated databases disable foreign keys before BEGIN (SQLite ignores this pragma inside a transaction), restore the original setting after success/error, validate full DDL and `foreign_key_check` before commit, and roll back on failure. No rename rewrites foreign-key targets and no JSON/key/digest is reserialized.

The real v28 fixture contains identity, attempt, creation, attachment, recovery and completion rows. It snapshots every column of all six bootstrap tables, upgrades, compares exact values, reopens, and compares canonical completed/recovery results. A second fixture checks original identity bytes/keys and accepts a newly created maximum escaped body after upgrade. Python SQLite smoke also exercised fresh migration chains with FK ON/OFF and populated physical-table preservation; this supplements, and does not replace, Rust canonical upgrade tests.

Known integration seam: Tasks 1, 2 and 4 own isolated provisional migration 29 candidates. Coordinator will assign integration versions 29/30/31 in ready completion order, with fresh schema seam review; no receipt-first ordering is assumed. Task 1's receipt-table rebuild also needs the same FK-outside-transaction migration discipline; shared wrapper consolidation is coordinator-owned at integration.

## TDD and verification evidence

RED before production changes:

- Shared initial run failed test-harness arity (missing `guarded_retry` JSON argument), corrected; this was not counted as behavioral RED.
- Corrected shared queued session 67639 was stopped while artifact-lock queued, per coordinator; no production effect/evidence loss.
- Private `nice cargo test --locked --all-features --lib handoff_instruction_sources`: exit 101; 0 passed, 1 failed, 3596 filtered; source parser rejects valid `--file` as unexpected. First private compile took 10m24s with cloned-target fingerprints and concurrent foreign compilers; the test itself took 0.05s. This is cold-ish initial evidence, not an incremental timing claim.
- The immutable baseline test binary independently failed `store::topology_handoff::tests::maximum_escaped_body_identity_survives_canonical_encoding`, `cli::handoff_delivery::tests::maximum_escaped_delivery_origin_and_terminal_survive_cleanup_retry`, and `cli::handoff::tests::handoff_stdin_acquisition_is_bounded_and_preserves_exact_message_bytes`: each exit 101, 0 passed/1 failed, rejected old body validation/1,024-byte cap.

Formatting and whitespace: `cargo fmt --all`, `git diff --check` passed before GREEN lane. GREEN command results, actual serialized whole-envelope sizes, clippy, final self-review, process cleanup and commit will be recorded below.

## Files

Production: `src/archival_legacy.rs`, `src/cli/{commands,handoff,handoff_delivery,journal,topology_handoff}.rs`, `src/protocol/handoff.rs`, `src/store/{schema,topology_handoff}.rs`, `src/store/topology_handoff/persistence.rs`, new `migrations/0029_handoff_body_bounds.sql`, `TRUST-POLICY.md`.

Tests: `tests/cli/{handoff,handoff_delivery,topology_handoff}.rs`, `tests/store/{archival_legacy,topology_attempts,topology_handoff,topology_handoff_schema}.rs`, `tests/handoff_topology_cli.rs`; protocol/identity inline unit fixtures in the production files above. Existing `src/cli/input.rs` is reused unchanged.

## First GREEN selection and traced corrections

Own `cargo metadata --locked --no-deps --format-version 1` confirmed workspace_root and target_directory are this exact isolated task tree. Commands use its private persistent target and `CARGO_BUILD_JOBS=1`; no Cargo overlaps within the lane.

`nice cargo test --locked --all-features --lib maximum_ -- --nocapture` (session 25436) compiled in 8m35s and ran tests in 21.52s: 8 passed, 3 failed, 3588 filtered. The sandbox emitted `nice: setpriority: Operation not permitted`; the compile/test continued. Coordinator-directed CLI-only escalation will repeat this exact selection before final claims. Coordinator's liveness sample during compilation measured rustc PID 80912 elapsed 8m09, CPU 4m39/current64.6%, RSS3.8GiB, running. This build exceeds the repository's incremental budget and is reported honestly; host contention was measured, no timeout/assertion/budget has been relaxed.

Actual serializer sizes from that run (65,536 U+0001 bytes under ordinary bounded metadata): canonical identity 394,089; combined completion+recovery response frame 792,412 (NUL same size; 3-byte whitespace body frame 5,990); delivery original 394,067 and nested terminal 460,051; bootstrap original 395,314. The initial raw bootstrap progress fixture serialized to395,674 but was invalid and is superseded by the corrected actual request-bearing fixture.

Failure tracing and corrections:

1. New bootstrap file retry selected `CompleteReply`, which existing behavior recovers atomically via status in the same call. The established `downstream_wrapper_reply_loss_recovers_atomic_canonical_report` test explicitly expects success. Changed the new fixture to `CompleteBefore`, the established injected failure before wrapper completion, so the first call genuinely leaves retained work for retry. The first-error, frozen-original and no-relaunch assertions are preserved.
2. New raw progress fixture claimed `possible_creation=false` while retaining creation evidence, violating its phase invariant; its witness also used the unrelated default endpoint. It now builds the actual request-bearing possible-creation envelope with identity-matching workspace/cwd/env/label/focus/witness. This corrects fixture reachability, not a production acceptance check.
3. Existing `composition_maximum_legal_escaped_serializer_memory_and_budgets` reached canonical completion but real archival scan vetoed coverage. Source tracing found hardcoded128KiB request-bearing progress loaders in `src/archival_legacy.rs`, plus obsolete128KiB delivery-terminal and64KiB delivery-original loaders. Coordinator approved this additional write surface. They now reuse shared handoff limits. Legacy Handoff and HandoffDelivery origins receive a typed512KiB archival cap; generic non-handoff `RECORD_CAP`64KiB and header4KiB remain unchanged. All scanner byte/memory/time assertions remain unchanged pending rerun.

Added focused real scanner regression `tests/store/archival_legacy.rs::maximum_escaped_handoff_bodies_survive_real_archival_origin_and_terminal_loaders`: maximum escaped origins for all three modes, delivery terminal with exact surviving original and after original cleanup, coverage and byte-preservation assertions. Existing composition regression exercises the full bootstrap submission/linked/completed/terminal bundle and deciding path. `TRUST-POLICY.md` now records the scanner's same typed handoff limits. No historical bytes/digests are rewritten.

The populated migration fixture was also tightened during pre-GREEN self-review: a genuine populated v28 image is written with `VACUUM INTO` a unique private temp DB, closed/reopened before upgrade, and closed/reopened again after upgrade; all six tables and canonical histories must match. Existing repeated-initialize cases independently cover same-connection idempotency.

Additional production file: `src/archival_legacy.rs`; additional focused test file: `tests/store/archival_legacy.rs`. Corrected candidate formatted and whitespace-checked; verification remains pending.

## Corrected GREEN evidence

CLI-only `require_escalated` with `nice cargo` prefix was approved for all subsequent checks after the established sandbox priority failure. Own private target and jobs1 remain in use. No source edits occurred during these builds.

- Exact repeat `nice cargo test --locked --all-features --lib maximum_ -- --nocapture` (session19842): exit0, build2m42s, tests6.57s, 11 passed/0 failed/3588 filtered. U+0001 canonical identity394,089; combined reply792,412 (NUL same; whitespace5,990); delivery origin394,067/terminal460,051; bootstrap origin395,314 and corrected request-bearing progress395,817 bytes.
- The existing unchanged-budget composition stress passed with actual components: identity523,022, report1,048,576, original523,206, submission524,288, linked child1,573,216, terminal2,311,759; 5 pages/12 reads; maximum page-buffer capacity4,932,473; maximum scan297ms/deciding173ms/foreground wait174ms; total1,098ms; baseline live9,110,272/sampled peak22,195,952 bytes. No numeric byte/memory/time assertion or timeout was raised.
- `nice cargo test --locked --all-features --lib handoff` (session82829): exit0, build0.05s, 237 passed/0 failed/3362 filtered, tests33.42s. Covers all three CLI modes, parser/source boundaries, dropped-stdin retry, historical frozen intent/digest semantics, native argv/routing, unknown-create/start, completed/terminal replay, prepublication overflow and independent native/report refusal controls.

The first corrected edited-source build2m42s remains above the stated one-minute incremental budget and is a reported limitation. Warm selection build0.05s demonstrates artifact reuse; it does not erase the edited-source timing. No build-profile/cache-invalidating workaround was applied.

`nice cargo test --locked --all-features --test combined topology_handoff` (session1538): first combined build4m09s, tests1.05s, 70 passed/3 failed/528 filtered. One established exact-bound recovery test successfully retained1,048,576 bytes and rejected max+1 at the DDL, then failed only its stale final literal262,144. Coordinator approved mechanical `tests/store/topology_attempts.rs` expansion: name and final expected bound now1MiB; exact-bound historical JSON whitespace and max+1 rejection remain unchanged.

The two new migration fixtures failed before migration at `begin_pending` with CallerUnverified: their old-schema setup lacked the cooperative sender seat/binding. They now call the existing SQL-only `thread` / `joined_agent` prerequisites as appropriate, assert schema version28 after seeding, and keep the physical version28 reopen assertion before upgrade. These initial failures provide no migration evidence; the corrected migration tests must pass before completion. No production migration change was made in response. This corrected integration-test-only candidate is formatted and awaits its coordinated lane; the passing lib artifact still matches production source.

Coordinator authorized an independent direct-artifact archival check while the Cargo lane was occupied: `target/debug/deps/combined-c319dcf33c89ec96 archival_legacy --nocapture` (session25263), exit0, 33 passed/0 failed/568 filtered,2.26s. Provenance: exact own combined binary produced by session1538, including the unchanged corrected archival production and new scanner test; subsequent source edits touched only topology fixtures outside this selection. No stale changed topology case was run or claimed. The new all-three-origin and delivery-terminal survival/cleanup regression passed, together with existing corruption/contradiction/caps/namespace/read-only/coverage controls. No Cargo was started for this independent check.

Prebuild fixture audit traced canonical caller verification completely: claim instancei/seats/targetp/generation1/TopLevel/Codex/session/execution00000000-0000-4000-8000-000000000001 matches resolved native seat(s,i,p,gen1,targetgen0), the live `cooperative_top_level` binding, hostbootb/epoch1, fresh observedtarget(p,b,epoch1,gen0), and joined membership(t,s). Both identities retain namespacei,/state,/host.sock. All compound/child keys are replaced for the second maximum-body identity and its digest recomputed. Populated completion/recovery evidence uses matching workspacew, endpoint/host.sock, panew:p2, recipientpeer, terminal/incarnation and report digest. SQL-only prerequisite helpers were independently replayed against genuine migrations1..28 with FK ON: inserts passed, user_version stayed28, foreign_key_check empty. This is supplemental setup evidence, not a substitute for the pending Rust migration/reopen result. Remaining28 literals in touched suites identify historical cutpoints; latest assertions are29, the approved recovery boundary is1MiB, and creation/attachment128KiB remain unchanged. Temporary DB cleanup follows existing best-effort RAII so an earlier fixture failure cannot cause a double panic.

## Actual CLI artifact provenance, setup RED and cleanup

Coordinator authorized direct `handoff_topology_cli` checks before the next Cargo lane. Exact compile-time provenance: `target/debug/deps/combined-c319dcf33c89ec96.d` records `CARGO_BIN_EXE_herdr-threads` as this task's `/target/debug/herdr-threads`. That executable's mtime17:41:41 follows last production edits17:26:50; combined binarymtime17:42:35, both from session1538. Invoking its `handoff --help` confirmed the new exclusive `<BODY|--file <FILE>|--stdin>` grammar and65,536-byte contract. It is not a copied stale executable.

`HT_LEAK_RUN_ID=C7A28315-94B3-4771-B97C-18F1E5260203 target/debug/deps/combined-c319dcf33c89ec96 handoff_topology_cli --nocapture` first failed all20 before setup because private Unix socket binding was denied by sandbox. Exact CLI-only escalated repeat (session96253) ran7.84s,18 passed/2 failed. Both failures were fixture `seal_snapshot_stage` at503: snapshot target set incomplete (`directory_search_discovers_canonical_name_with_unrelated_topic`, `pending_public_postpublication_lock_failure_after_attached_reports_continuation`). Exact individual reproductions passed0.29s/1.33s.

Source tracing establishes the cause: existing fixture ignored the `SnapshotStageProgress` result and immediately sealed. Production `stage_snapshot_targets` intentionally commits a nonempty prefix when its5ms writer quantum expires; under parallel load only part of the two-target fixture can be committed. The same correction already exists in `tests/hook_entrypoint.rs`: consume each reported prefix until complete, then seal. Applied that setup correction in the already-declared `tests/handoff_topology_cli.rs`, with batches<=16, positive/exact progress assertions and the same original5s CallBudget. No production change, serialized-test escape, timeout increase or assertion weakening. The corrected whole20-case CLI selection must now run against a rebuilt combined artifact.

Scoped `scripts/check-no-leaked-processes --run-id C7A28315-94B3-4771-B97C-18F1E5260203` initially required CLI-only escalation because sandbox denied `ps`; exact escalated check passed `no leaked test processes`. It was repeated after both failed-case reproductions and again passed. No shared server/foreign process was touched. Source candidate formatted/whitespace-clean; zero Cargo/owned-child processes at lane wait.

Coordinator also authorized direct unchanged journal coverage while Cargo was occupied: `target/debug/deps/herdr_threads-9111c9f9be8710f5 cli::journal --nocapture` (session48541), exit0,54 passed/0 failed/3545 filtered,2.53s. Provenance: exact lib artifact from session19842, mtime17:36:40; production journal source16:56:35 and unchanged journal tests16:31:06. Covers allocator/publication, unknown outcomes, exact original actor/keys, historical omission/digest compatibility, bounded headers/pagination and retry preflight. No Cargo or OS child process was started; newer combined fixture edits are outside this artifact selection.

## Final rebuilt integration checks

Coordinator granted the exclusive lane with an explicit warm-cache worker experiment: `CARGO_BUILD_JOBS=4` (previously1), same private persistent target, nice priority, one Cargo invocation, unchanged profiles/budgets.

- `nice cargo test --locked --all-features --test combined topology_handoff` (session58430): exit0, build6.71s/tests1.01s;73 passed/0 failed/528 filtered, no warnings. Both genuine v28 migration fixtures passed, including physical database reopen, exact six-table historical preservation, canonical completion/recovery equality, FK ON restoration and a new maximum-escaped identity accepted after upgrade. The exact1MiB retained result/max+1 rejection control passed.
- `HT_LEAK_RUN_ID=420AB070-0B34-4F79-89DE-5816FB3770D4 nice cargo test --locked --all-features --test combined handoff_topology_cli -- --nocapture` (session92028): exit0, build0.14s/tests14.09s;20 passed/0 failed/581 filtered. This rebuilt artifact includes the staged-prefix fixture correction. Both previously load-sensitive setup cases and all actual CLI/owned-child behavior passed without timeout/budget changes.
- `scripts/check-no-leaked-processes --run-id 420AB070-0B34-4F79-89DE-5816FB3770D4`: exit0, `no leaked test processes`. CLI-only escalation is required for process visibility in this environment.
- Final `cargo fmt --all`: exit0. Required clippy is in progress; no completion or clean-lint claim yet.

The warm edited-combined fixture rebuild6.71s is below the incremental budget with jobs4. This observation is not a controlled comparison with prior production-source rebuilds, and does not erase the earlier2m42s budget violation.

## Final lint, self-review and concerns

`CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=<own-tree>/target nice cargo clippy --locked --all-targets --all-features -- -D warnings` (session33069, CLI-only escalation): exit0,43.72s, no warnings or diagnostics. Exclusive Cargo lane released immediately afterward. Final `git diff --check` passed; `git diff -- migrations/0028_handoff_topology.sql` is empty. No full-suite or redundant default-feature check was run; coordinator owns mandatory pre-merge default-feature gate/fresh review.

Self-review checked every acquired-source branch and body-bearing serializer listed above: exact bytes are acquired before intent construction, only body String is retained, body validation is independent of identifier validation, actual origin serialization precedes ordinal reservation, actual identity serialization precedes bootstrap publication, and delivery terminal serialization precedes canonical completion. Existing bootstrap terminal/request-bearing progress preflights remain in place. Generic journal historical loading and non-handoff archival record/header caps remain unchanged. The complete migration DDL audit and genuine populated/reopened v28 fixtures verify both changed bounded tables without altering other table/history blobs, compound child keys, digests or immutable triggers. All changed paths are within the declared surface or explicit coordinator-approved expansions. No new process-spawn helper, shared server control, real-user configuration write, push or stash was introduced.

Fresh review should independently trace: every new bounded source/owned retained loader and exact pre-effect serialization; all repeated body/nested-original occurrences and structurally unchanged derived bootstrap formulas; migration FK disable/restore/rollback discipline and six-table history preservation; typed archival widening with generic caps unchanged.

Concerns: the first edited-production corrected build2m42s exceeded the one-minute incremental budget under measured host/compiler contention; initial cold-ish cloned-target compiles were much longer. No assertion, timeout, profile or budget was relaxed. Final warm edited-combined rebuild6.71s and lint43.72s passed the stated budgets using approved jobs4, but are not a controlled demonstration that every production-source edit now meets one minute. Provisional local migration29 must be serially renumbered at integration after Task2's invitation migration29 and other ready schema candidate, preserving immutable historical migration bytes and receiving fresh schema/FK seam review. Task10 confirmation/deadline/readiness remains deliberately outside this completed Task4.

All final selected test processes exited, and scoped CLI leak check passed. No owned Cargo or child process remains active.

Commit: `1fe52281` — `feat: freeze bounded handoff instruction sources and envelopes`. Final `git status --short` is empty; HEAD differs from BASE957544e8. Commit contains20 scoped files including the new provisional migration; report is coordinator-owned integration documentation outside the task branch. Initial git staging was sandbox-blocked at the worktree index; exact scoped `git add`/`git commit` CLI escalation succeeded. No push or stash.

## Round 1 review — Important I1

Read full `task-4-review.md` and receiving-code-review skill. Verified the issue against actual call ordering: explicit-pane `handoff::start` validates/selects/preflights startup input, constructs frozen HandoffPlan, calls Journal::record, and only afterward calls resume (canonical Begin and native launch). The new journal origin-cap match covers Delivery/Bootstrap but omits legacy Handoff, while the archival loader already applies512KiB to that owned kind. Historical generic load/decode acceptance does not excuse publication of a newly oversized retained envelope.

Prepared targeted `tests/cli/handoff.rs::legacy_handoff_origin_overflow_refuses_before_publication_or_effects` through actual parsed explicit-pane start. It retains legitimate --new-thread/--pane/codex grammar, maximum U+0001 body and two --model=<12000 U+0001> values; validates request, native raw-byte limits and registered Codex policy. Actual semantic serialization must exceed512KiB even before header/newlines. The producer attempt must leave no published intents, unchanged next-ordinal, zero canonical commands/native starts; a subsequent ordinary frozen intent must be ordinal1. No production code changed before meaningful RED. Source fixture audited/frozen pending the coordinated Cargo lane.

I1 meaningful RED: `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=<own-tree>/target nice cargo test --locked --all-features --lib legacy_handoff_origin_overflow_refuses_before_publication_or_effects -- --nocapture` (session27727, scoped CLI escalation), exit101; build19.14s/tests0.14s;0 passed/1 failed/3599 filtered. Actual admitted semantic serialized537,942 bytes, above524,288 even before header/newlines. The actual start route published the oversized intent; test failed specifically at the no-published-intent assertion. Native-byte/registered-Codex grammar/request checks and startup input preparation all passed first. This is behavioral RED, not setup/compile failure. No OS test children were spawned. Cargo lane released immediately.

Minimal production correction after RED: legacy `IntentKind::Handoff` now shares the existing512KiB `MAX_DELIVERY_ORIGIN_BYTES` match arm with HandoffDelivery in Journal::record_locked. Exact encoded header+body+two-newline preflight therefore runs before allocator reservation/publication for all three handoff kinds. No ceiling, generic historical loader, serializer, digest, key or effect ordering changed. Corrected source formatted/whitespace-checked and frozen pending GREEN/lint lane.

### ADDRESSED I1 — final verification

All corrective checks used own persistent target, CARGO_BUILD_JOBS=4, nice priority, CLI-only escalation and one invocation at a time in the granted exclusive lane. Production remained unchanged until recorded meaningful RED.

- Exact I1 regression GREEN (session65566): exit0, build9.95s/tests0.12s;1 passed/0 failed/3599 filtered. Same actual semantic537,942>524,288 bytes. Actual producer now returns error with zero published intents, unchanged ordinal bytes, zero downstream canonical command calls/native starts, and next ordinary intent ordinal1.
- `nice cargo test --locked --all-features --lib maximum_instruction_file_freezes_through_explicit_launch_retry`: exit0, build0.03s/tests0.21s;1 passed/0 failed/3599 filtered. Existing regression loops both65,536-byte ordinary and six-byte-escaped file bodies, deletes/mutates source, and proves exact lost-reply retry/no relaunch/native argv body-path exclusion under ordinary metadata.
- `nice cargo test --locked --all-features --lib cli::journal` (session31204): exit0, build0.03s/tests2.82s;54 passed/0 failed/3546 filtered. Shared writer historical loading/omission/digest/key/allocator/retry protections passed.
- `cargo fmt --all` and `git diff --check`: exit0.
- Mandatory `nice cargo clippy --locked --all-targets --all-features -- -D warnings` (session81821): exit0,16.41s, no warnings/diagnostics.

Both corrected incremental build9.95s and lint16.41s meet the one-minute budgets for this correction; earlier candidate budget overruns remain visible and are not erased. Cargo lane released immediately on completion, before report/commit. These unit selections start no OS children, all owned threads completed, and zero Cargo/test processes remain. No unchanged whole CLI/topology/archival suite was redundantly rerun, per coordinator instruction.

Fix self-review: exactly one production match-arm change and one actual producer regression; legacy Handoff uses the same512KiB writer/scanner cap, no global/derived/metadata cap increase, no historical generic Journal::load/decode_reader change, no serialization or historical byte/key/digest rewriting. Fresh review should check scoped I1 closure and produce literal CLEAN before integration.

I1 fix commit: `d6f7ed94` — `fix: preflight legacy handoff origin before publication`, following original candidate `1fe52281`; original BASE remains957544e8. Final `git status --short` is empty and HEAD differs from BASE. Ready for coordinator-owned fresh scoped re-review.
