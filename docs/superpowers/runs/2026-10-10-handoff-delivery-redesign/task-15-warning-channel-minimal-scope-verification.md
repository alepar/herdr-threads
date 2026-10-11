# Warning/channel minimized-scope source verification

Reviewer d_interface_final; source-only verification of selected proposal70fc974e754a9bbf80e01b619d4ac737e73773d409b74641cc35d76d4e639213, ACTIVE top and final Minimized scope disposition. The historical Refresh+100/mod_warning_open/15-source alternatives are DECLINED and were not used as requirements. No Cargo, source/Git/beads/thread mutation or runtime test was performed. This is not a new author approval gate or final cumulative D CLEAN. Routine corrected fixtures remain authorized.

Verdict on selected scope: source-compatible. No additional concrete defect identified in the cited eventual materializer→native counter→observer→admitted delivery path. Preserve Refresh160/Pull433,13 relevance sources, current schema and current kick tables. The pre-materializer registered-frame assertion is an unsupported latency expectation; direct canonical Pull same-native-version relevance remains required and independently implemented.

## Warn publication and finite attribution

service_events.rs publish_notify_internal commits physical programmatic Warn, service_notification_publications, immutable captured audience and service-notify send_attention job in the deciding transaction. Warn/count>0 job high is the audience ordinal maximum; Info has no such job. Existing materialization.rs send_unit dispatches the service-notify subject to service_notification_unit.

service_notification_unit performs one indexed next-recipient lookup for exact preparation with position<ordinal<=captured high. It inserts exact warning_id/seat/generation association with ON CONFLICT DO NOTHING, then checks recipient retirement and informational_notice_pending. The published programmatic warning recipient trigger in0008 maintains digest_programmatic_warnings in that transaction; later0025 preserves condition-specific counterparts. Newly inserted + nonretired + unoffered calls bump, incrementing recipient wake_work attention_version. An exact retry finds inserted=0 and cannot bump again. Different immutable warning_id while presentation remains Open has a different recipient association and can bump independently; generic one-Attention-per-episode does not suppress its Notice.

informational_notice_pending resolves exact warning, recipient projection ordinal and current occupant frontier. Missing projection is pending; a late projection gets its own ordinal above a previously carried higher-event frontier. notice_frontier requires exact current generation/execution. Notice source LIMIT occurs before hydration; notice allocation/settlement is distinct from receipt ACK and leaves frozen recovery descriptors for emission retry.

For r physical audience rows through this recipient, r successfully processed units reach it; an R-row job needs at most R+1 units including exhaustion. These are physical row ranks, not ordinal numeric distances. advance_work admits1..16 units and checks budget/5ms each iteration. Zero units with no error is an error; failure/progress.last_error is not a successful advance. At least one committed unit per successful selected advance supports the same conservative <=r / <=R+1 call bounds, conditional on fair job selection and continued successful storage/admission. No16-units-per-call or wall-clock SLA follows.

scheduler/deadlines.rs drive_work persists work_cursor between finite highwater pages, advances page cursor before the selected job's bounded advance, and retains per-job retry times on failures. New jobs cannot extend an active captured page; completion wraps discovery. This supports the stated existing fair-selection premise rather than an unconditional latency promise.

## Existing observer and Watch path

MOD_NOTIFY_TABLES already includes warning_recipients, digest_programmatic_warnings and wake_work. Those attribution/counter writes produce the existing mod observer hint; no new service-publication/work_jobs kick is necessary for eventual post-materializer delivery. Initial publication alone may legitimately produce no drain. Worker dirty passes and ten-tick safety pass remain the fallback; storage/admission failures retain existing retry/backoff limits.

Digest-free presentation_seat_view intentionally zeroes legacy pending counts but retains canonical native attention_version. For a primed quiet live registry entry, Some(last fingerprint) differs after bump; observe_fingerprint calls notify_at. An accepted sink Attention frame records last_attention_push under the entry lock. This is the existing publication cutoff for health, not a presentation descriptor, receipt ACK or new countdown. Actual lease/current binding then admits Watch V2 Pull, whose notice class reads the attributed programmatic projection. Native version requests a drain but does not decide canonical relevance or Notice applicability.

Watch V2 unconditionally performs its first Pull after accepted capability/lease before waiting for Attention frames. This covers retained already-published/materialized backlog on a new/reconnected channel despite initial digest-free has_pending false. Subsequent More performs bounded continuation with closing/owner guards. A silent missed live sink signal is not proof of model uptake; canonical retention/outbox/reconnect remain the selected recovery limits. This audit does not invent a guaranteed fresh signal after every downstream loss.

## Ordinary body and health remain separate

send_unit point-selects each frozen receipt_ordinal, then project_receipt applies effective receipt precedence and writes receipt_state. Only effective Pending with absent prior overlay bumps; terminal, waived/NotRequired and retired cases do not manufacture wake. The sole production pending receipt_state insert outside project_receipt is the canonical overdue-warning marker in schema.rs; its warning condition/attribution path supplies its own existing eventual native progress. Actual ACK and retirement writers install terminal state. Thus this inventory reveals no unsupported fresh ordinary producer bypassing both the initial bump and an existing warning/native progress route.

Ordinary job completion is R recipients+W warning positions+one terminal unit; recipient position r takes <=r successful units. Existing send_manifests/receipt_state/receipts/wake_work kicks and the same native fingerprint comparison drive live quiet-channel body drain. notify_at records the ordinary successful push cutoff even when a Message-only V2 page has no generic Attention/token. Supplemental generic-control Confirm bookkeeping is not substituted for this ordinary path. Receipt remains pending until actual receipt ACK; ordinary body delivery does not require an invented generic control.

## Clear and scope limits

Physical warning jobs walk finite membership intervals, then affected/finalize phases: K+3 units for K inspected intervals. attribute checks exact actionable/unoffered association; condition clear transitions fail warning_wakes_seat. Logical close/ACK/waiver/retirement is recognized by next admitted canonical Pull through existing exact relevance proof. Scoped passive Clear requires a delivery opportunity; no quiet idle wake solely to retract is selected. Direct ServiceNotify Warn Pull can recognize publication before native materialization using exact source10/11 publication+recipient identity, even with unchanged native metadata. Retain that test independently from corrected registered eventual-drain tests.

## Runtime obligations for final cumulative verdict

Review actual corrected real registered Warn and ordinary fixtures, including native unchanged before materializer, bounded exact advance_work units with processed/error checks, native bump→observer/pass→lease-admitted Notice/body, second distinct Warn while already Open, current-occupant/late attribution, real ACK and cutoff controls, retained backlog first Pull/reconnect, and passive Lazy/clear negatives. Source verification alone does not mark those proofs GREEN. If those existing routes fail their selected oracle, preserve the meaningful RED and raise the concrete defect before final CLEAN; do not add declined machinery automatically.

## Consulted production hashes

```json
{
  "src/store/service_events.rs": "eabdb053ede3417c74c845f3f63edcc7f42e93edb972259e03ec2297e2e805bb",
  "src/store/materialization.rs": "be2e3899024fe1564fed13f34a82ed6c3fe3c9b9d7f6a34c4aaa4c09cac14886",
  "src/store/attention.rs": "3ff3cbd8e6582ede2931ff28ea6080e4f2a561c058a0556804923cbe50b79926",
  "src/store/mod.rs": "65be9a2447a78459a040a04cd8ad5db3a78183f5a7edbdf50a5a46256bd1311d",
  "src/store/schema.rs": "770fd1962356a23236ca679b1268b591bb85eeccf5ae9b376742364e1ab726eb",
  "src/store/seats.rs": "aa4bf9d178a700e0ce17a1701345cd36bec21c66c7f243f8d2964ed1cf3fd49d",
  "src/store/invitation_content.rs": "1e9830929b63ed26443215ef7d9c0dd4ed0baac07801893f301c9035bcbb43ca",
  "src/store/messages.rs": "f582f890c1404805ab4f589a4f2c6e6f6536bec1e41adab6235dab55a883dff8",
  "src/store/receipts.rs": "377a7e459d89e15c0563f3360fe41c0a1e44886f7fb835ff0907d61f04d135f2",
  "src/service/kicks.rs": "87fdb4b090ec34062daaddc6ce959ffa9361dcdb71c970c481d082e7e9eba130",
  "src/service/mod_channels.rs": "87c4da68dc439d69b2e13b662509441ae78a619f3357bb106b6f1482dbb63667",
  "src/service/workers.rs": "a16eac6af86bc73740dfb57c753c262990c6135a4162abdf31343163e69b7564",
  "src/scheduler/deadlines.rs": "9d499794a7ba82c0b15c77a463b9cd32ce864851bfa0d13ba233e0d3c87a60bc",
  "src/cli/watch.rs": "df937177073bff2a994d5797ed2993d0bdb781c6114ff55c3990c5e53adbc108",
  "migrations/0008_digest_pending_paths.sql": "f1d0385323435a429e72478458cb1dbb1f3998dd7db3017677ba53d673740101",
  "migrations/0025_warning_notice_delivery.sql": "be0ff728fb17015517364604d2d2fe55a5a826665fb1f34e23ecf2f854431417",
  "migrations/0038_receipt_warning_origins.sql": "f75b12aa854d074560b65457bf570f3256634329dae648b55bdf9826c9a48333"
}
```
