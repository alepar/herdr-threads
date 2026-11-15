# Codex queue-backed wake

## Goal

Wake a qualified idle Codex session through its persistent input queue while preserving herdr-threads delivery, attribution and user-interaction guarantees. Queue only `herdr-threads: wake up`; hooks continue to carry bodies and canonical delivery observations.

Status: reopened by explicit user requirement change; observed-idle enqueue with best-effort cancellation is permitted. The current amendment below supersedes the historical rejection and conflicting requirements. Production remains unchanged while target/control and retry details are settled. Design/quality owner: snSGPb0CT1uM. Implementor: snSGte2clRpf. Queue: ht-d2jq. Source baseline: main 1cf51f9a4; live spike: 8625c066e, `docs/superpowers/runs/2026-11-14-codex-queue-wake/evidence/report.md` on branch `spike/codex-queue-wake`. The spike is not merged into this document's baseline.

## Requirement baseline

Source: user in this design session: “Own comprehensive Codex queue wake design and quality; full instructions are already in thread dzU0QoN9Dew.” The handoff mnSGdbzbn5j9 records the originating user's request for a separate design/quality owner and implementor and agreement that queueing is user input, not regular message delivery. This design introduces engineering decisions, not additional claims of human approval.

Requirements: preserve hooks and receipt semantics; never use queue input for message bodies; improve qualified idle waking without exact-version gates; keep cooperative same-user attribution; give the implementor bounded assignments and enforce design quality. Existing AGENTS.md delivery rules and TRUST-POLICY.md A2/A3/A4 remain normative. Any proposed weakening requires an explicit decision and matching policy change, not an implementation shortcut.

## Current amendment: observed-idle enqueue

Source: the user in the design-owner session, after reviewing the negative qualification:

> I think we are unfortunately already loose with "requirement that busy sessions receive messages through hooks without an extra wake afterward".
> I think this should be mostly covered by the case of us not appending to the queue when we explicitly see agent is alredy busy. So cancellations should be rare, and its fine if they dont come thorugh

This is an explicit change to the acceptance contract. The following rules supersede conflicting text in the historical design and qualification decision below. The experimental observations remain valid; their consequence changes.

### Accepted behavior

Only enqueue after current canonical wake eligibility and receiver/UI observations admit an idle wake. An explicitly busy receiver is never queued. Unknown activity is not idle evidence. Recheck canonical owedness as late as practical before add. Preserve priority/threshold, exact target, interruption, blocked/modal UI and initial draft/focus restrictions. Queue only the existing content-free marker; hooks retain body delivery and all receipt authority.

A receiver can become busy after the check. Its hooks may deliver the mail before the queued marker runs; a later extra turn is an accepted residual race. Request cancellation when the owned pending marker becomes obsolete, but cancellation is best effort. Failed, raced or unavailable cancellation does not invalidate the feature. This acceptance applies to the race, not to deliberately enqueueing into a known busy session or repeatedly retrying an uncertain add.

Remove receiver hook veto from the proposed implementation. Do not add prompt suppression, text-equality ownership detection, queue-ID hook correlation or special hook failure handling. A visible blocked turn is unnecessary when an occasional ordinary wake turn is accepted. Exact queue IDs remain useful for pending cancellation and diagnostics, not model-delivery receipts.

### Minimal control and retry direction

Keep the neutral distinction between body delivery and wake transport. The Codex adapter owns queue protocol and effective executable/profile/storage observation; canonical reservation and priority/owedness stay in core. Keep the managed-daemon attribution refusal. Native fallback is selected before a possible queue effect for unqualified/dormant targets.

Persist one logical enqueue attempt before sending. A known accepted add counts as a submitted wake, never body delivery. A lost add reply is not retried blindly: exact correlation may recover a pending entry, but absence proves neither failure nor delivery. Cancel only exact owned entries, never matching user text. Queue deletion is best-effort cleanup, not a proof that no turn will run and not permission to replay the same wake through native typing.

Do not retain the historical proposal's blanket requirement for successful cancellation or proof of zero latent effects as an activation prerequisite. Conversely, user acceptance of cancellation failure does not by itself authorize repeated adds or transport retries for the same unresolved logical wake. The amended implementation plan must specify durable deduplication of our own attempts, the existing wake ladder/contact semantics, scope changes and newer-attention behavior without claiming upstream idempotency or exactly-once execution. That remaining policy is design work, not grounds to reimpose the rejected race-free prerequisite.

### Remaining design work and tests

Resolve supported exact receiver/storage capture and bounded helper behavior using existing source evidence. An unknown configuration uses native before enqueue; no effective-storage guessing. Settle conservative unknown-add retry and restart behavior. Prefer the smallest supported implementation; do not build hook veto or a generalized queue broker.

Revised acceptance: no add when already observed busy; eligible idle p0 wakes and p1/T0 stays silent; racing busy delivery may leave one previously submitted marker; cancellation failure is recorded and tolerated; no automatic duplicate add for the same uncertain attempt; unchanged hook body/ACK behavior; no queueing of real message content; existing UI/identity/dormant guards preserved. Fault-injection tests distinguish definitive pre-effect failures from ambiguous acceptance. Live characterization of the accepted residual race is evidence, not a failing gate.

The design owner will issue revised implementation tasks after target/control and retry details are concrete. The earlier negative qualification is complete under its original stricter contract and does not need to be rerun to prove the same race again.

### Authorized first implementation slice: target/control adapter

Task ht-d2jq.3 is authorized independently of scheduler/storage activation. Add a neutral WakeControl port with observe, add, list and exact-ID delete; default adapter support is absent. Scope includes canonical binding/native session/execution/generation and full process fingerprint/kernel boot. The bounded adapter descriptor holds only required paths/file identities, never environment or argv dumps. Add has no body parameter: it constructs only the existing wake marker. Outcomes distinguish accepted ID, definitive pre-effect refusal and uncertain effect. Neither list nor delete supplies receipt or arrival authority; no adapter-internal add retry.

Initial capture is Linux foreground-only. Observe the exact receiver executable, process fingerprint, current session writer-lock file and persistent queue database from proc file descriptors. Deduplicate descriptors by device/inode, require one distinct supported queue database, reject deleted/replaced/ambiguous evidence and recheck identity. Unsupported operating systems/configurations or unreadable observations use native before add. This is observed capability/layout support, not a version-number admission gate.

Source inspection found that the queue retains a detached SQLite connection for data_version polling, independently of its pool. A session writer lock alone does not establish loadedness: a surviving recorder can retain it after a failed shutdown. Canonical connected binding/session, normal SessionEnd disconnect, live fingerprint and matching lock jointly support the observation; failed SessionEnd plus retained recorder remains a limitation. The implementation must not label FD presence as proof of dispatchability. Retain exact source passages with the implementation evidence.

The preferred helper uses the captured executable, private empty HOME/CODEX_HOME/XDG/cwd, cleared inherited environment and an explicit captured sqlite_home. No receiver hooks, credentials, profile or command-line overrides are copied. Only initialization and queue control calls are allowed; no thread load/resume/start. Before implementation, qualify that exact private-home/shared-SQLite recipe in an isolated no-auth check. Failure requires a new explicit recipe review. Verify the helper addresses the same database identity before add.

After helper initialization and storage verification, invoke a neutral final-authorization callback immediately before the first add bytes. Core owns its canonical scope/owedness and existing host idle/UI checks. A false/error result writes no add bytes. This prevents helper startup from separating the last idle check from enqueue unnecessarily; it does not promise an atomic check across processes.

Initial refusal bounds: capture budget 100ms, at most 1024 descriptors, env 64KiB, argv 32KiB, paths 4KiB, descriptor 16KiB and IDs/correlation 128 bytes. Helper operation budget is at most 2s within its caller budget, reserving 250ms for cleanup and refusing startup with less than 500ms available. One helper per target at a time, no unbounded waiting or background owner. JSON line 256KiB, total stdout 1MiB, stderr 8KiB; list 32 entries/page and at most four pages, with explicit incomplete result. These are engineering bounds to test, not measured latency guarantees; process cleanup must account honestly for a nonquiescent helper.

Any partial write of add bytes makes transport failure uncertain; no retry/native fallback follows from an error after that boundary. Known accepted ID remains accepted even if cleanup reports an error. Deterministic tests cover partial writes, initialization and final-callback refusal, mismatched files, overflows, timeouts, descendant cleanup and incomplete list. The slice adds no scheduler selection or persistent wake policy; those follow a separate reviewed retry contract.

Final source-contract ruling: the isolated no-auth private-home/shared-SQLite check passed: helper loaded/list stayed empty before and after add/list/delete, queue database device/inode matched, no turn or auth operation ran, and owned scratch/processes were cleaned. Retain its source/check artifacts with the adapter implementation. The final add callback is mutable and fallible, invoked once after helper initialization and exact database/receiver revalidation. False or error means no add bytes are written.

Cooperative observed-loadedness is accepted for this slice, without a cross-process runtime-query prerequisite. Normal SessionEnd invalidates canonical eligibility before persistence cleanup; a missing session lock also refuses queue. Missed SessionEnd combined with failed writer cleanup can retain stale eligibility in a surviving process and admit a marker for an unloaded session, possibly retained until explicit resume. That combined failure is an accepted observation limit, now recorded in TRUST-POLICY. It is not evidence that lock presence means dispatchability. Explicit End cannot manufacture dormant native authority; existing canonical fallback rules remain decisive. With these rulings, ht-d2jq.3 is authorized to proceed without further design confirmation for routine implementation choices.


### Authorized second implementation slice: durable attempt ledger

Task ht-d2jq.4 adds storage and ports only. Existing wake_work is a mutable per-seat reservation slot and can be cleared by completion, recovery or invalidation; it cannot retain an uncertain external queue effect. Keep a separate node-local append-by-attempt ledger, without cascading foreign keys to live seats, bindings or wake_work. No scheduler invocation or queue operation is introduced in this slice.

Replay cohort is `(local instance, registered adapter, adapter-produced replay scope)`. Codex supplies `session:<canonical native-session UUID>`, bounded to 128 bytes. It intentionally survives process/binding/generation/storage-path changes. This is conservative suppression, not physical database identity or continuity evidence: copied profiles with the same native UUID share suppression. Freeze physical executable/database descriptor and original seat/host/mapping/scope separately. Keep an origin-seat index so later native selection cannot overlook an earlier possible queue effect. Independent cohort rows are permitted, but their existence grants no native fallback authority.

One unresolved intent per cohort is enforced transactionally. The row freezes attempt, original scope, adapter/replay key, descriptor (16KiB), correlation (128 bytes) and every component of LogicalAttentionFrontier: invitation, addressed receipt and actionable warning each have decision sequence plus event offset; lazy has its own scalar. A maximum sequence across those components is insufficient. Retain revision, submit-owner boot, arm nonce, phase, cleanup quarantine and bounded diagnostic. Owned queue IDs are separately keyed by exact attempt/id, at most 128 unique IDs per intent.

Prepare validates the current canonical reservation, connected exact binding/session/execution/process/boot and scoped owedness before creating a missing row. Repeating an existing attempt cannot alter frozen fields. Arm repeats canonical checks and CASes Prepared to Armed with the invocation nonce; only the transition winner receives a send ticket. Replaying the same request/nonce must not return send authority again. The successful durable commit precedes any add bytes. If its response is lost, the row stays Armed and the caller does not send. This deliberately accepts lost wake opportunity rather than duplicate submission.

Record-result checks the exact ticket/revision/owner. Accepted stores its ID and cleanup quarantine atomically; Unknown retains possible effect. A definitive pre-effect refusal can become NoEffect, but cleanup uncertainty still blocks reuse. Crash recovery changes no intent back to Prepared and grants no add ticket. It can conservatively classify old Armed work as Unknown/quarantined. Queue-ID reconciliation validates and deduplicates owned exact IDs; completeness/absence does not establish cancellation, arrival, receipt or no effect.

For this storage-only slice, only definitive no-add/no-effect with confirmed cleanup is resolved automatically. Armed, Accepted, Unknown and cleanup-quarantined records remain unresolved and cannot be evicted. Do not add a generic caller-asserted resolution flag or an operator endpoint. Task .5 must define canonical obsolescence/served-attention resolution and cleanup-quarantine disposition before enabling production. New frontier, seat contact, queue absence or delete result alone is not resolution. Best-effort cancellation remains accepted; this ledger is not an attempt to restore the former race-free-turn requirement.

Initial caps: 4096 intent rows, 128 owned IDs each, 32 recovery summaries per page with descriptors omitted. At capacity refuse new queue preparation before effects; native fallback is a later policy decision and must still inspect applicable unresolved effects. Prune only nonquarantined NoEffect tombstones no longer referenced by active/last wake_work; prepare's current-reservation validation prevents a stale cached reservation from resurrecting a pruned attempt. Never evict unresolved records for space. Use the next coordinated additive node migration, not a baseline edit.

Required tests use two connections for arm races and commit-durable SQLite settings for restart/crash cases. Cover reopening each phase, lost arm response/no second ticket, stale nonce/boot/revision, immutable scope/correlation/frontier, independent components of frontier, ID+cleanup atomicity, unchanged uncertainty on contact/new frontier/disconnect, recovery after origin retirement, per-cohort conflict, copied-profile suppression, caps, safe tombstone pruning and bounded pagination. Scope remains storage, neutral replay key and port definitions; no external queue effects or production activation.

## Historical conditional design (superseded where the amendment differs)

## Evidence and decision

The live spike observed a loaded embedded TUI waking via shared storage, draft preservation, normal UserPromptSubmit context, and exact root UUID targeting while a side session was displayed. It also observed duplicate turns from repeated identical client IDs, obsolete markers starting an extra turn after busy delivery, interrupted queues waiting for continuation, and unloaded sessions retaining markers until explicit resume. A separate stdio app-server added/listed/deleted entries without loading the target. These are observations of the exercised configuration, not general guarantees.

Choose a narrowly scoped queue control adapter with add/list/delete rather than a bare CLI substitution. A CLI enqueue cannot support exact cancellation or unknown-outcome reconciliation. Keep the existing native route where queue qualification is unavailable before any possible queue effect. Do not enable automatic queue dispatch until stale-marker admission is qualified. A periodic cancel loop alone is insufficient: dequeue may win before cancellation, and an absent row does not say which won.

This is a conditional architecture. If the receiver has no supported pre-inference suppression or equivalent atomic conditional dispatch, preserve native waking and record the missing upstream capability. Do not ship dormant speculative markers or relax delivery rule 4 to make the feature pass. A hook veto may prevent inference yet still produce a turn-start event or UI entry; qualification must name the actual observable guarantee, including whether this satisfies “no extra turn.”

## Current seams

`src/harness/adapter.rs` separates DeliveryMechanism from IdleWake but currently represents only Native and Channel. `src/harness/registry.rs::delivery_pairing_allowed` couples HookPull to Native. `src/host/native.rs::submit_prompt_mode` rechecks host identity, idle state, empty composer, focus dwell and canonical owedness immediately before typing. `src/ports.rs::WakeTarget` contains binding, harness and pid, but not sufficient exact queue scope. `src/scheduler/mod.rs::AwaitingReceipt` uses seat-level contact as a native wake heuristic; that cannot be elevated into proof that a particular queued submission arrived.

The queue path must enter the existing reservation/fencing machinery, not bypass it in a background adapter task. Keep adapter details under `src/harness/codex/` or existing Codex-owned paths. Core works through typed neutral ports; no harness-name branches. Existing managed-daemon attribution refusal remains.

## Key abstractions and ownership

1. **Wake transport policy** describes eligible ways to request a turn, independently of body delivery. Preserve HookPull and its exact current receipt protocol. Add a neutral programmatic wake facility only with the operations required below. Registry validation accepts the explicitly supported HookPull/programmatic pairing; it must not pretend queue control is PushChannel or reuse mod registration/health.
2. **Queue target observation** is adapter-produced configuration evidence, tied to canonical binding ID, native session, execution, generation, process fingerprint including boot/start stamp, executable identity and effective storage configuration. A home-directory string alone is insufficient. Daemon admission checks A2 and uniquely associated receiver ownership. The observation grants no continuity or binding migration.
3. **Wake effect intent** is a durable reservation extension identifying one logical effect and its frozen target, originating attention witness, correlation ID, possible queue IDs, current state and history. It survives daemon restarts independently of today's attention. A newer frontier never erases an unresolved older effect.
4. **Queue control port** exposes bounded add, exhaustive-within-bound list and exact-ID delete, with explicit definitive refusal versus unknown effect. Its adapter owns protocol details and child lifetime. Correlation IDs identify ours; they do not deduplicate add.
5. **Receiver arrival/admission observation**, if qualified, names the exact target and owned wake attempt through supported upstream evidence. It permits wake bookkeeping only. Bodies, ACKs and invitation acceptance keep their existing paths.

Avoid a generic persistent RPC framework. One scoped helper per bounded operation group is the initial option; it must be joined/stopped before the operation returns. If measured startup cost cannot fit existing wake budgets, propose a bounded reusable owner explicitly before implementation; do not hide a long-lived server or extend deadlines silently.

## Target and eligibility

Queue qualification requires a current top-level hook-capable foreground receiver with an exact native session, uniquely associated live process and observed persistent storage. Unknown/memory-only storage, unknown executable/configuration, stale bindings, unavailable control operations or unqualified hook admission select native before enqueue if its existing guards permit it. No exact release-number probes are operational prerequisites. Capability failures are scoped to the observed executable/configuration identity and failed operation; changing that identity invalidates the observation. Transient failures do not permanently disable unrelated hooks.

No latest-binding, focused-chat or first-target selection. A single eligible exact session can be targeted. Multiple owed sessions in one process require an explicit policy backed by canonical per-binding owedness; initial queue admission refuses this ambiguous case and uses existing process composer semantics before effects. This conservatively preserves /side behavior until exact multi-session policy is qualified. A duplicated native session loaded in multiple receiver processes is not a uniquely owned target; queue admission refuses.

Preserve priority/threshold admission and current receipt/invitation versus per-binding lazy-row rules. An ACK in a sibling does not prove this binding displayed a lazy row. A queue retry cannot manufacture new attention. A live healthy writable mod route still excludes another wake route.

Initial rollout retains current modal, blocked-screen, interruption, draft and focused-empty-dwell restrictions even though queue storage can preserve a draft. This avoids turning transport capability into permission to interrupt the user. Readable empty composer is only UI evidence; it does not prove idle or loaded session state. Missing evidence defers. Dormant targets use existing guarded native waking before enqueue; no queue `--resume`, thread load or interruption override.

## Durable effect state machine

The states below are proposed internal meanings, not new wire names. Reuse existing attempts where possible.

| State | Evidence | Allowed next action |
|---|---|---|
| Prepared | Canonical reservation and frozen scope committed; no add may have run | Recheck guards, then durably enter Adding; or abandon without external effect |
| Adding | Add may execute; commit precedes helper write | At most one add invocation for this intent; recover by observation, never replay add |
| Pending | Add reply or exact correlation listing supplies owned IDs | Revalidate owedness; observe arrival or cancel exact pending IDs |
| CancelRequested | Old effect is obsolete, scope changed, or shutdown requires removal | Delete only exact owned entries; continue bounded reconciliation |
| Arrived | Qualified receiver evidence ties arrival to owned effect | Stop wake retry; ordinary delivery still decides body display and ACK |
| Cancelled | Qualified cancellation proves no latent effect remains | Release suppression; any still-owed attention needs a fresh canonical decision |
| Uncertain | Lost add/delete reply, missing row without arrival, or ambiguous ownership | Retain fence; no resend/native fallback, even on newer attention; reconcile or report operator limitation |
| RefusedBeforeEffect | Definite no-add outcome | Existing guarded native fallback may be considered |

Adding is conservative: crash before the actual write can leave an uncertain intent. This intentionally trades liveness for avoiding duplicate external user turns. A list with no match cannot distinguish “never added” from “consumed before list.” A delete success is not yet proof of no in-flight dispatch unless upstream ordering establishes that guarantee; qualification owns this boundary. A timed-out helper must be fully quiescent before any negative observation could be considered final. Killing its parent alone is insufficient.

Persist every discovered queue ID before acting on it. Discover duplicate owned rows by exact correlation and exact target, then cancel all within the bounded reconciliation page; do not delete by wake-text equality or touch user queue entries. Unexpected overflow retains Uncertain and prevents activation; it does not discard evidence to fit a limit. Rollback or local persistence failure after enqueue remains an external-effect uncertainty.

Recovery scans durable unresolved intents, including retired/rebound seats, before dispatching new effects for those scopes. Cleanup may address the old queue scope to remove owned effects, but grants no authority to wake that old receiver. Native fallback is prohibited until the old effect is definitively absent from both pending and in-flight dispatch. A new process, storage selection or daemon boot does not establish that absence: old storage can replay on resume.

## Race handling and receiver contract

Race R1: idle check; user starts work; enqueue; busy hook serves mail; turn ends; queued marker drains. Cancel-on-busy narrows the window but cannot close it. A supported receiver admission mechanism must reject obsolete owned markers before inference, checking exact canonical attempt/scope and current owedness. Ordinary user prompts, including a user typing the marker, must not be suppressed merely by text equality. If upstream does not expose an unambiguous correlation field, matching an active intent by timing and marker text is insufficient.

Race R2: add commits upstream; reply is lost; daemon crashes. Durable Adding prevents replay. List can recover pending IDs; if the row has disappeared and no exact arrival exists, keep Uncertain.

Race R3: delete races with dequeue. Record exact delete result without converting disappearance to arrival, cancellation or receipt. Qualification must establish whether true delete excludes an in-flight claim. If it does not, receiver admission remains necessary.

Race R4: admission check succeeds; another path serves or retires attention before inference. Define the receiver's linearization point and existing tolerated observation gap explicitly. Do not claim atomicity between the daemon's database and Codex's turn executor. If the proposed guarantee cannot be met across that boundary, report it and keep automatic queue selection disabled.

Race R5: side session or another process contacts the daemon. Existing seat-level contact remains historical native wake evidence only. It cannot clear a queue-effect fence or prove root-session arrival. Qualification must establish exact arrival identity without widening hook provenance. No queue operation ACKs a receipt.

Interruption, modal entry, target retirement, generation change, threshold reduction and in-turn service invalidate admission and request cancellation. Failure to cancel does not authorize a fallback. Queue storage may outlive both processes; graceful shutdown alone cannot be the correctness mechanism.

## Budgets, wire and operational behavior

Use existing CallBudget/cancellation and owned-child infrastructure. No helper starts if insufficient budget remains to initialize, run and reap it. No authentication copying in production and no writes to real profile configuration. Control processes must never resume/load a target, infer caller identity from inherited pane variables, or hold a new user binding. Startup/config side effects of app-server must be audited before qualification.

Concrete byte, row, concurrency and time bounds are implementation prerequisites in ht-d2jq.3, derived from actual protocol responses and existing scheduler limits. Limits are not performance claims. Parsing must bound response size before deserialization; stderr is bounded diagnostics, not model context. Oversized/malformed/partial replies cannot become definitive no-effect outcomes.

Any new protocol fields default compatibly and any gated variant advertises its capability. Unknown peers retain existing native behavior before effects. Once an effect exists, downgrade does not erase it; old binaries must not blindly resume native wake over an unresolved effect. Design the mixed-version/rollback fence before enabling production.

No new routine model-visible text. Wake text stays the existing 22 ASCII bytes, `herdr-threads: wake up` (verify literal length before implementation). Diagnostic state belongs in existing doctor/wake evidence; any new string requires an explicit byte-size account. Do not deliver queue bodies, correlation IDs or repetitive transport explanations to the model.

## Acceptance and rollout

Production automatic queue selection remains disabled until all mandatory qualification rows pass. A successful characterization test of an unsafe behavior is not an acceptance pass. No user-facing toggle is invented to bypass a failed contract.

| Area | Required checks |
|---|---|
| Policy | p1/T0 silent; p0/T0 wakes; T=-1 and T=3; threshold change during attempt; invitations and explicit ACK remain p0 |
| Delivery | Complete production hooks deliver bodies and settle only eligible canonical receipts; truncation/flush failure unchanged; no extra inbox call for the ordinary single-item case |
| Activity | Busy before selection, during add, before dequeue and after cancellation; served-in-turn prevents obsolete inference; unknown/blocked/interrupted stays deferred |
| User UI | Draft preserved; model picker unchanged; no queued turn bypasses modal hold; focus dwell preserved |
| Identity | Root versus /side; sibling display and ACK; multiple sessions/processes; storage overrides, memory-only storage, stale executable/profile and managed daemon refusal |
| Durability | Every local commit/RPC boundary; lost add reply; duplicate correlation rows; lost delete reply; dequeue/delete race; missing row; daemon/TUI restart and target replacement |
| Fallback | Unsupported before add permits guarded native; unknown/pending effects prohibit it; dormant exact resume remains native; retired-target cleanup cannot wake |
| Resources | Bounded fake peers, oversized replies, helper crash/hang and descendant cleanup; no leaked test processes or temp files |

Deterministic fake-peer and store/scheduler tests establish state transitions. Real isolated Codex tests using production ht hooks establish upstream behavior and attribution; test-only token echo alone is insufficient. Record native session IDs, queue IDs, model-request/turn observations and sanitized traces, separately from receipt evidence. Measure enqueue-to-hook timing separately from model completion; two samples establish no latency guarantee. Run relevant tests and required fmt/clippy/neutral/wire/default checks. A release still runs RELEASE.md gates; this epic itself authorizes no release.

## Work ownership and unresolved decisions

Beads is the task-state authority: ht-d2jq.1 qualification (implementor), .2 design (design owner), .3 target/control adapter, .4 durable reconciliation, .5 selection/admission integration, .6 integrated qualification and final review. The implementation tasks are gated by the settled design; their initial briefs are conditional, not approval to build speculative abstractions.

Unresolved gates: pre-inference veto semantics and exact queue correlation at hooks; delete/dequeue linearization; receiver-loaded and storage evidence; same-session multiple receivers; bounded helper startup/config side effects; mixed-version rollback fencing. ht-d2jq.1 supplies first evidence. The design owner will settle the gate and refine task boundaries before production implementation, then review actual code and traces. If no compliant queue route exists, retain native behavior, preserve the findings and explicitly report that outcome rather than declaring the requested feature delivered.


## Qualification decision — 2026-11-14

Reviewed evidence: spike branch commit `455b2c919`, parent `8625c066e`, especially `docs/superpowers/runs/2026-11-14-codex-queue-wake/evidence/admission-qualification.md`, `admission-source-inspection.json`, run-7 results/RPC capture/driver, run-8 results and `admission-cleanup-check.json`. The design owner inspected these artifacts directly. This decision supersedes the draft's pending qualification language above; the conditional architecture remains a record of requirements, not an implementation authorization.

**Do not implement or enable automatic queue waking through the tested enqueue/list/delete plus UserPromptSubmit-veto route.** Keep production native waking and hook delivery unchanged. Qualification ht-d2jq.1 fulfilled its negative-evidence acceptance criteria; that does not complete the feature epic.

Run 7's explicit block prevented observed model sampling but left a visible blocked/completed turn. Its full hook payload contains native session/turn and prompt fields but no queued-submission or client correlation ID. Hook failure exited 1 and allowed inference. The hook had already lost access to the pending queue row: list was empty and deletion returned false. Pending-only lost-add-reply recovery worked, with no inference claim for vanished rows. Run 8 reached an ownership screen before a second SessionStart, so competing dispatch was not tested.

The retained source shows peek/copy, start-if-idle, then deletion; the dispatch mutex is local to a service instance. A controller may successfully delete a row after a receiver copied it but before it starts. That true-delete race is a source-derived possibility, not a live forced result. Both the observed false-delete boundary and source-derived true-delete boundary defeat cancellation as a general no-latent-turn proof. The report correctly separates these evidence classes.

CODEX_HOME was captured from environment, not hook payload. Effective sqlite_home/CODEX_SQLITE_HOME configuration and shared queue namespace still lack the necessary receiver observation. The same-home ownership screen is not proof of exclusivity across all queue-storage configurations. Cleanup artifacts report unchanged source credentials, no private refresh and no remaining tagged processes/scratch roots; the worktree has no build artifacts. Repository checks are the implementor's recorded successful run, not a new full-suite or release qualification by this design review.

Reopening requires evidence of a supported mechanism that identifies an owned submission, conditionally admits it before obsolete runtime turns are created, and accounts for unknown enqueue/cancel effects without unsafe retries or fallback. These are required semantics, not a request to invent a new protocol or patch upstream. A proposed alternative must also resolve exact receiver/storage scope, interruption/UI policy and mixed-version recovery. No version pin, text/timing heuristic, silent extra turn, or fail-open hook is an acceptable substitute.

The design-owner quality review accepts the evidence as a bounded negative qualification. It makes no universal claim that every present or future Codex programmatic wake route is impossible. Production tasks stay blocked; the design and evidence should be retained through normal repository integration. No further live experiments are assigned.

Integrated evidence: [initial spike report](evidence/report.md), [admission qualification](evidence/admission-qualification.md).
