# Full-capacity Watch triage

Status: **CONFIRM — the original resumed continuation comes from an unfinished relevance sweep, independent of full ordinary capacity. Healthy slow background completion is also confirmed. The pre-Watch paced barrier completed but removed the required real Attention stimulus.** The next narrowly supported correction is to relocate that same barrier after the initial terminal reply is held and256 offers are proved, before taking the normal baseline and releasing any response/frame. Actual validation remains required; source assessment is not a PASS claim. Preserve the original strict first-resumed-response and no-spin assertions.

## Scope and actual evidence

Inspected D at `5f879abb3e6609fa2b7ed2695bfd551a3e590536`, tree `32d4d3f958a1bfff50a8fb28dbcf9d5b71b43597`, clean. D is ROOT's `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`. This report is the sole authorized write. No Cargo, builds, tests, Herdr commands, subagents, Git mutations or source edits were performed.

`/private/tmp/ht-owner-final-5f879abb/nextest-run.log:318–339` records this integration case failing at `tests/integration/mod_delivery.rs:2730`, with `page.more == true` where false was required. The matrix ran 314 tests: 313 PASS / 1 FAIL, 10.598 seconds. Execution metadata pins the before/after source and binaries; scoped leak check reports none for run `001ab0f8-eb24-44a1-a2c2-16f816bc08e3`.

The earlier `/private/tmp/ht-resume-activation-082385c7/nextest-run.log:290` records this exact case PASS in 10.020 seconds. The capacity fixture and relay did not change between 082 and 5f; integration-file changes concern separate notice tests. That prior success is evidence of one working execution, not proof that its stable-state assumption always holds.

## Established call flow

1. `Rig::activation` starts an actual isolated elected daemon and workers. Its bootstrap confirming invitation is accepted before the capacity fixture begins (`mod_delivery.rs:361–478`).
2. The test freezes UTC, sends 257 ordinary 6000-byte messages through canonical RPCs, then starts one real Watch. The replay seeding goes directly to upstream; no source or ledger SQL is fabricated (`2613–2657`).
3. The relay holds actual Pull responses and actual Attention forwarding. The test releases each continuation. It reaches a terminal response within 64 observations, proves the offered set equals precisely messages 1–256, proves no duplicates, and finds message 257 intact in the normal carry (`2658–2710`). All these assertions passed before the observed failure.
4. It releases the terminal response, reads real Watch output, checks all 257 receipts remain pending and no mod ACK occurred. Only then does it forward one actual held Attention (`2711–2722`). `CapacityFlow` proves that the resumed Pull request occurred after the successful Attention write, with no extra earlier Pull (`1427–1592`).
5. The resumed daemon response is a Page. Its first assertion, `more == false`, fails. The subsequent empty offered/truncated/items checks and normal-state equality were **not executed**. The old log does not show those values. The saved Watch capture belongs to the separate activation-output producer, not this failed response.
6. Production Watch drains every Page with `more == true`; after `false`/Unchanged it waits for channel input (`src/cli/watch.rs:632–713`). The relay's ordering is therefore a valid causal observation. It does not prove canonical background state stayed unchanged.

## Owner replay and ordinary continuation analysis

`DomainService::presentation_pull` snapshots known/blocked offers, free slots and due stamps before calling the store (`src/service/delivery_presentation.rs:159–211`). It obtains the per-lease owner snapshot, applies successful store progress after ledger installation, and preserves the owner permit throughout (`273–289`).

The first Watch starts after bootstrap and before any ordinary delivery to B, so its first owner snapshot is empty. `ModChannelRegistry::presentation_replay_snapshot` captures `known.stamps` only for `entry.owner_replay == None`; `apply_presentation_replay` installs even an empty successful capture. Subsequent calls use that saved remaining set and never refill it from the growing ledger (`src/service/mod_channels.rs:547–630`). No replacement owner occurs in this fixture. Thus the new owner-replay path is not an explanation for recapturing all 256 newly offered messages after the terminal response.

UTC remains frozen; every offer's `last_at` equals that frozen instant. The service blocks offers that are not due. Once the ledger holds 256, `ordinary_runnable` returns only whether known due keys exist; it ignores retained unknown normal carry at zero slots (`src/store/invitation_content.rs:1848–1883`). Offer recording does not evict at capacity (`src/delivery/ledger.rs:183–201`). The fixture uses only two seats, so seat-LRU pressure is absent. This rules out the simple unknown-carry spin and elapsed-time reoffer explanations under the observed unchanged receipt/binding scope.

The Notice-clear change contributes `(!carried_notices.is_empty() && controls.active_epoch.is_some())` to `nonordinary_more` (`invitation_content.rs:2548–2551`). It cannot be blamed without evidence of an actual carried Notice and active control epoch; the failed assertion currently supplies neither.

## Concrete independent continuation hypothesis

Every ordinary publication enqueues `send_attention` work (`src/store/messages.rs:882–898`). Workers later update `work_jobs` through `commit_work_prefix` (`src/store/work.rs:64`) and may project receipt state. Migration 41 advances the global `presentation_warning_fence` on every work-job update, receipt-state/receipt mutation, and several other canonical mutations (`migrations/0041_presentation_relevance.sql:276–300,384–436`). These triggers are not limited to changes that make a warning actionable.

`CapacityFlow` controls downstream traffic only. Neither the terminal reply hold nor the receipt inspections stop daemon materialization. Reads of pending receipts do not assert that send jobs, warning sources, or the global relevance fence are stable.

Each Pull calls `presentation_relevance::advance` exactly once. A cached Absent proof is reused only if its saved fence and notice frontier match current values. Otherwise it captures a new bounded sweep, which may return Unknown while source/fallback traversal remains. Unknown means `proof.more == true` (`src/store/presentation_relevance.rs:1178–1278`). That independently reaches Page.more through either frozen-outbox `replay` (`invitation_content.rs:1656–1658`, `2045`) or fresh `nonordinary_more` (`2548–2583`), even when every ordinary offer is blocked and normal carry must remain parked.

This is a complete possible call flow, but **the supplied failed run does not identify its actual branch**. A current or previous outbox, newly pending control, changed publication/context frontier, or owner/ledger invariant violation must be distinguished using actual metadata. A `more: true` value alone cannot establish which source requested continuation.

## Minimal next evidence, parent-authorized fixture diagnostic

Preserve the exact case, assertions, response gates, 64-observation bound, UTC freeze and first resumed Pull. Add diagnostic-only reads/capture to this existing integration fixture. No additional Pull, sleep, timeout increase, production logging, model-visible output, real user state or acceptance of `more: true`.

Capture at the held initial terminal and held resumed response:

- Normalized request/reply metadata: ordinal, owner lease/claim identity, result kind, more, offered/truncated IDs, item kinds and IDs, emitted control stamps, token presence/identity and output byte count. Omit message bodies.
- One coherent read-only SQLite snapshot: normal tuple (summarize carry IDs/body lengths), all nonordinary cursor pairs and publication/passive counters, outbox token/more/masks/descriptor kinds, control state, and eligible invitation count.
- Relevance proof JSON or its complete bounded fields: claim, fence/frontier/native version, absent/witness, all 13 source high/after/exhausted/candidate/judge states; current global fence and notice frontier. Current fence must be sampled coherently with proof data.
- Bounded summary of work jobs by kind/status and relevant source counts. These disambiguate ongoing materialization without dumping traffic or bodies.

The parent has separately authorized a fixture-only helper author to add normalized response, outbox/control, publication and relevance metadata and then conduct the exact-case probe. Await that actual evidence before a CONFIRM verdict. If the new run passes, retain its diagnostic state and report that it did not reproduce; do not treat success as causal confirmation or silently expand to a broad rerun.

## Decision pending actual metadata

If the resumed proof is Unknown with nonexhausted sources and owner/ordinary outputs stay empty, the observed continuation comes from bounded relevance maintenance. Then determine which canonical producer changed its fence and whether the test's stable-input premise was unmet. If the proof is Absent and every nonordinary gate is exhausted, investigate actual outbox/source and owner/ledger state instead. A fix must preserve exact normal carry, finite ordinary parking, every independent runnable source, and real channel causality.

No source fix or completion clearance is granted by this static triage.

## Diagnostic successor and isolated non-reproduction

Inspected `cecd6154a91e626325bfc2c46c720a38d9dd4228`, tree `371cb4e5a496d0d85315df76b383b28f94d43082`: only the existing capacity integration test changes (40 insertions, 2 deletions). It adds bounded metadata to the unchanged `more == false` assertion and preserves every original assertion, real Pull, relay gate and timing bound. Production is identical to 5f.

`/private/tmp/ht-owner-capacity-diagnostic/execution.json` records exactly the one requested selector, exit 0, command elapsed 6.234 seconds and leak exit 0 for `a11cc82c-bc6a-4700-8cae-69cae2179b86`. `run.log` records 1 PASS, Nextest summary 5.997 seconds. Because metadata is printed only on failure, this run provides no resumed-state diagnostic and does not establish the original failure's cause.

The helper reads consumer, proof and canonical metadata in separate SELECTs without a surrounding read transaction. Interpret a global fence mismatch cautiously: workers may advance it between those reads. A persisted resumed proof with `absent=false` and nonexhausted slots is stronger evidence that the actual Pull requested relevance continuation. The parent is preparing one same-selection 314-test run to seek that evidence. Parallel tests use separate databases; they can change scheduling/load, not directly mutate this fixture's fence.

## Actual diagnostic reproduction: causal CONFIRM

Read `/private/tmp/ht-owner-capacity-diagnostic-314/nextest-run.log:333` and `execution.json`. Source remains clean `cecd6154` / tree `371cb4e5a496d0d85315df76b383b28f94d43082`, with unchanged 5f production and the same installed executable hash. Actual result: 314 selected/run, 313 PASS / 1 FAIL, 10.671 seconds. Scoped leak exit 0 under `1be2e461-2511-4008-ad6d-02c8e772eb4b`.

The held initial terminal was Pull 34; the first causal resumed Pull was 35. Its reply has `more:true`, zero offered/truncated counts, `items:[]`, `controls:[]`, `token:null`, `output_bytes:0`.

Before the resumed Pull, the saved proof was Absent, fence 2117, native version 129, witness null. Source slot 10 (`work_jobs`, pending `send_attention`) had captured high 259 and reached after ordinal 259, exhausted=true. All other source slots were exhausted with zero highs.

After the resumed Pull, the **persisted proof itself** is absent=false, fence 2121, native version 131, witness null. Slot 10 still captures high 259 but is now after ordinal 144, exhausted=false, with no suspended candidate/judge. All other slots remain exhausted/empty. This is a new bounded source traversal, not a read-order artifact from comparing global-fence SELECTs. The saved proof cannot be Unknown merely because the later diagnostic read observes a newer fence: the actual Pull persisted its restarted traversal.

The normal consumer still has carry_count=1, ordinary_done=1, the same ordinary version and passive generation. Publication after/high remain equal at 263; context, initial, dirty and revisit cursors are exhausted. Control counter remains zero with no active/pending/allocated epoch. Both outbox tokens are null and both descriptor lists empty. `outbox_more` changes from 0 to 1 because the fresh-path `nonordinary_more` includes `proof.more`; there is no frozen outbox replay. No owner replay output, logical due output, notice output or independent control allocation occurred.

Therefore the exact resumed path is:

`successful initial Absent proof and terminal Page` → `canonical background mutations invalidate cached proof` → `Attention-released actual Pull` → `advance() snapshots pending send_attention range` → `bounded schedule stops before range exhaustion` → `Relevance::Unknown` → `proof.more=true` → `nonordinary_more=true` → `empty Page.more=true`.

The incrementing native version is consistent with ongoing send materialization. Source inspection supplies its concrete producers: `send_unit` projects recipient receipts (`materialization.rs:591–593`); durable work completion updates the fenced `work_jobs` row. The diagnostic does not log every individual mutation, so it does not attribute all four fence increments to exact rows. That narrower uncertainty does not affect the confirmed continuation source or fixture-premise defect.

## Smallest deterministic test repair contract

Add one condition-based canonical barrier **after the 257 canonical sends and before `rig.watch(SESSION_B)`**. Use the existing bounded `wait_until(..., STEP, ...)` and read-only `rig.db()` to wait until there are no incomplete `send_attention` jobs in this isolated fixture database:

```sql
SELECT NOT EXISTS (
  SELECT 1 FROM work_jobs
  WHERE kind='send_attention' AND status IN ('pending','failed')
)
```

The schema restricts statuses to pending/failed/complete; equivalently require status='complete' for every such row. Scope to all fixture send_attention jobs, including any bootstrap publications, not only the 257 newly sent message IDs. A failed job must prevent barrier success, not be treated as drained. Attach a bounded kind/status count summary to a failure if convenient. Do not write job status, proof JSON, ledger state or receipts.

Why this is causal: canonical send publication synchronously inserts each finite job before the RPC returns. The test sends nothing after this point. `commit_work_prefix` durably reaches complete and excludes that job from source slot 10; later turns cannot reopen the same completed job in this fixture. Waiting for that actual producer completion removes the pending range which the failing proof demonstrably rescanned. A mere idle-lane count, unchanged fence sampled twice, or arbitrary sleep would not establish the same condition. Other clock-driven receipt warnings cannot newly become due because UTC is frozen before these sends; this fixture has no new invitations/notices or lifecycle mutation during the observed drain.

Keep all existing observations and invariants unchanged:

- Real Watch and relay; no extra diagnostic/recovery Pulls, no fake capabilities or frames.
- Frozen UTC; 257 pending ordinary receipts; exactly 256 unique offers and the unknown identity retained.
- The existing maximum 64 initial Pull observations and no Pull before a successfully forwarded real Attention.
- **The very first resumed Pull must still be Unchanged or an empty Page with `more:false`.** Do not replace this with an arbitrary “eventually quiet” loop.
- Exact normal carry/cursor/partial snapshot equality; zero mod ACK; actual owned-child EOF/reconnect-grace cleanup.

The barrier may increase setup time while actual jobs finish; it must use the current STEP budget, not increase it. It neither weakens production relevance guarantees nor consumes offers: materialization projects pending state, and the existing receipt/no-ACK assertions verify that distinction. This is a fixture-only repair; production continuation and owner replay stay unchanged.

Parent should validate the admitted fixture change with the exact case and then the same frozen 314 selection, checking scoped leaks. This triager performed no execution and grants no post-fix PASS claim. If the strict assertion still fails, use the retained diagnostics and investigate the newly identified producer instead of widening the terminal allowance.

**Triage complete and quiescent.** Only this report was written; no processes were started or left running.

## Completion barrier failed: proposal withdrawn pending runtime diagnostics

Read the parent-owned `/private/tmp/ht-owner-capacity-completion-c2a8abf7/run.log` and `execution.json`. At source `c2a8abf787518913b26e93fa5edf904cb9c2493d`, tree `43cc3cc4b6092b3f4887f244b8699c2056c22b01`, the exact case timed out waiting for all send_attention jobs before Watch. Nextest records 27.385 seconds for the case (27.391 summary); scoped leak exit 0 for `bae3d150-60c6-433f-b9d3-b5a7e87b9667`. No Watch starts in this failed execution. No counts, remaining-job state or worker health were printed, so this execution alone cannot distinguish failed work, no progress, or healthy but slow progress.

My previous proposal correctly removed the actual relevance source if reached, but incorrectly assumed that passive waiting would make the whole backlog complete within STEP. That feasibility assumption is now withdrawn. The original Unknown-proof diagnosis remains confirmed by the independent cecd314 reproduction.

New static trace:

- `DeadlineDriver::drive_work` (`src/scheduler/deadlines.rs:335`) discovers at most **one** work job per call (`limit:1`) and advances up to16 units of that job. Work discovery has no UTC-based eligibility filter (`src/store/mod.rs:982–1077`); retry guards use monotonic time.
- `DriveOutcome::progressed_with_more` continues immediately only when the selected job progresses **without completing** (`deadlines.rs:110–115`). It does not continue solely because discovery has additional jobs.
- Once a tiny send_attention job completes, `start_deadline_worker` breaks the inner loop absent due continuation (`src/service/workers.rs:994–1006`) and returns to its five-second safety tick.
- A deadline-origin commit cannot kick its own lane: `src/store/mod.rs:165–166` explicitly removes the origin lane. The earlier canonical send requests supplied foreground kicks, which stop during a read-only completion barrier.
- Frozen UTC does not freeze monotonic pacing or retry gates. No inspected pending-work/advance-work code requires a registered Watch.

Therefore a concrete hypothesis is healthy one-completed-job-per-safety-tick progress after seeding, far too slow to finish a residual hundred-job backlog within20 seconds. This is not yet an attribution of the c2a8 run because that run lacks job/counter metadata.

Parent has authorized the existing helper to add diagnostic-only baseline/timeout snapshots: coherent status counts and aggregate positions/completed_units; a bounded sample of remaining IDs/ordinals/positions/high_water/errors; deadline lane commit count, completed passes/idle events, failure attempts and health. Preserve STEP, probe cadence and the predicate. A small monotone complete-count increase with zero failed jobs and clean lane health supports the scheduler hypothesis; failed/error rows require a different investigation.

After that actual evidence, reconsider preparation that deterministically drives the real worker's finite work through existing test-support kicks, or a precisely bounded independent relevance-continuation contract. Do not silently replace the original first-resumed assertion with eventual quiet, pause real infrastructure, fabricate completion, or adjust production scheduling as a fixture shortcut. No revised repair is cleared yet.

## Retained barrier diagnostics: healthy slow progress confirmed

Independently read `/private/tmp/ht-owner-capacity-barrier-retained-diagnostic/run.log:21` and its `execution.json`. At `34c465e916f0d63df717dd7c41d38b967432d930`, tree `b065f1eaa49934b37cd771f6a0e548f075c399e4`, the exact case failed its unchanged barrier (24.780-second case; 24.785-second Nextest summary). Scoped leak exit0 for `b2788256-34fc-4c1b-9a0d-e953904bbaf1`.

Actual retained state:

| Value | Before barrier | Timeout |
|---|---:|---:|
| send_attention complete | 133 | 142 |
| send_attention pending | 124 | 115 |
| failed jobs | 0 | 0 |
| pending position/completed units | all0 | all0 |
| pending high_water | all2 | all2 |
| deadline health | null | null |
| deadline idle events | 121 | 126 |
| monotonic ms | 35717 | 55729 |
| UTC ms | 1791753173030 | 1791753173030 |

All completed send jobs reached position/high_water2 and completed_units2. Error summaries and the bounded sample of20 pending jobs have null last_error. The completed prefix advanced from ordinal135 to144; pending tail begins at145. Two receipt-timer jobs were already complete and remained complete. Thus this execution made real healthy progress but left115 unstarted jobs after20 seconds. It was neither a frozen monotonic clock nor a failed-job barrier. It does not establish exactly one completion per five seconds: additional due continuations/kicks can cause extra driver calls, as the nine completions versus five newly completed worker waits illustrate. The precise source conclusion is that completion of one work job alone does not drive the remaining discovered backlog immediately.

The intermediate diagnostic attempt `61233ec1` also timed out, but daemon stderr redirection hid its snapshots. Do not use it as additional causal evidence. The retained34c run supplies the missing actual state.

## Accepted correction boundary (implementation and verification owned by parent)

Parent reports source review approved and author is now authorized to add **setup-only orchestration through existing `LaneProbe`**, retaining the canonical all-send-attention-complete predicate:

1. After all257 canonical send RPCs, before any Watch starts, enter the existing single STEP-bounded barrier.
2. If incomplete send work exists, issue one initial `rig.probe.kick_registered(Lane::Deadlines)`.
3. Thereafter issue at most one successor kick for each newly observed `registered_idle_events(Lane::Deadlines)` value, while incomplete send work remains. The event is an orchestration opportunity, never the success predicate. If the lane does not complete another pass, do not flood repeated kicks.
4. Declare setup complete only on the existing canonical SQL condition: all fixture send_attention rows complete, complete count equals total, total at least257. Failed rows remain incomplete. Keep the same20-second STEP and existing polling cadence.
5. Stop orchestration before `rig.watch`; perform no deadline kicks in either initial Watch drain or the first post-Attention response. Keep all original receipt,256-offer,257th-carry,64-observation, causal-channel, first-resumed-more=false and cleanup assertions.

This advances the genuine isolated daemon's existing worker through ordinary wake signals, with its existing per-call budgets, canonical commits and failure reporting. It does not overwrite SQL, advance clocks, manufacture presentation output or alter production scheduling. It specifically removes the healthy unfinished producer that invalidated the supposedly stable proof. Use the retained diagnostics if the predicate still fails; do not extend STEP or reinterpret a partial count as success.

Causal diagnosis and accepted scope are complete. Parent owns implementation, strict checks, exact-case validation and the same314 selection. **No tests or processes were run by this triager; only this report was changed.**

## Stepped pre-Watch barrier removed the channel stimulus

Read `/private/tmp/ht-owner-capacity-stepped/run.log`. At `c51c4a403f9702d3556e6c964357d98b7c29e167`, the exact case passed canonical completion and initial finite drain, then failed at `mod_delivery.rs:1565`: `expected an actual queued daemon Attention`. Case26.260 seconds, Nextest summary26.261; parent reports scoped leak0 under run prefix `deca0652`. This is a new missing-stimulus failure, not failure of the completion predicate or resumed more=false assertion (the resumed Pull never occurred).

Exact source explanation: V2 `presentation_seat_view` intentionally uses a cheap fingerprint with `attention_version` but all pending counts zero (`src/store/seats.rs:4641–4668`). `ModFingerprint::has_pending()` ignores the version and checks only those counts (`src/ports.rs:2589`). On a freshly registered Watch the registry's first fingerprint pushes only if has_pending; later readings push when the fingerprint changes (`src/service/mod_channels.rs:476–497`). Consequently, completing all send materialization **before** registration leaves no subsequent version change and no initial pending-count trigger. The ordinary initial Pull can still find all bodies, explaining why the drain passes while the real channel frame never appears. My earlier pre-Watch boundary failed to preserve this independent positive fixture prerequisite.

Parent's relocation proposal is supported as a narrow next trial:

1. Restore the original order: seed257, register actual Watch, run the unchanged bounded initial drain until its terminal reply remains held, and prove the complete256-offer set.
2. At that point run the same single STEP-bounded, per-idle-generation paced canonical completion barrier. Watch is already registered. The terminal response stays held, so the consumer cannot issue another Pull; CapacityFlow still holds all real Attention frames.
3. Stop deadline kicks as soon as the canonical completion predicate holds. Only then take normal-state baseline/diagnostics, release the terminal response, inspect output/receipts, and explicitly forward one actual queued Attention using the existing causal gate.
4. Retain the strict first resumed quiet response, exact carry tuple, no ACK, pending receipts and owned-child cleanup.

This preserves the real version-change path available while the Watch is live. At resumed Pull time all send jobs are complete, so source10 has no pending range; even an invalidated cached absence can snapshot an empty range and become Absent within that call. The terminal-held interval permits no hidden consumer drain or fabricated frame.

**Validation limitation:** relocation alone is not a mathematical guarantee that a frame was queued under every possible scheduling/coalescing order; if all work finishes before the registry establishes a previous fingerprint, the same stimulus prerequisite can still be absent. Keep the actual `attention_seen>0` and `requests==terminal` witnesses (already checked by `release_attention_after`) mandatory. Bounded diagnostics should retain those counters and before/after worker state if they fail. A successful exact run must establish the real frame witness, and the same314 selection must still be checked. Do not turn a missing frame into synthetic Attention or relax the assertion.

Source evaluation is complete; this proposed relocation has no execution clearance from the triager. Only the report was modified; no Cargo, tests, Herdr or processes were run.
