# Pi adapter architecture: repository findings

This memo describes existing repository seams, not confirmed pi capabilities. Pi API and runtime facts must come from the separate primary-source research. The inspected base was `c4c5ecb9e`; opencode references below describe the concurrent `opencode-harness` branch and must be reconciled after it lands.

## Recommended shape, conditional on pi evidence

Use `src/harness/pi/` for Rust behavior and `integrations/pi/` for the native extension. Register one `PiAdapter` through `builtins()`. Prefer the existing `PushChannel`, `cooperative_mod_delivery`, `IdleWake::Channel` combination with no `ComposerPolicy`, provided the extension can observe session-local delivery and safely trigger an idle turn. This is the shape of `src/harness/opencode/delivery.rs` on the opencode branch. A typed composer fallback is a distinct capability needing its own capture-based safety proof; it is not required to make a channel adapter work.

The extension should run `watch` as an owned child for its exact current top-level binding. Native session changes must fence outstanding callbacks, queued deliveries, delivery evidence and wake intent. Kill/reap the old watch and establish the successor binding/channel; do not relabel old work with a new session ID. Canonical daemon admission remains the authority. Channel replacement/grace and reconnection are already modeled by the push infrastructure.

There is no demonstrated need for a new neutral trait merely to add a second channel-only harness. Opencode adds `Registry::wakes_only_through_channel` and the scheduler's `no_native_wake`; these suppress native reservations/pokes for targets whose only possible wake path is their channel, even while the channel is absent. `Registry::idle_wake_on` combines live channel, host capability and adapter composer capability. The branch's sender warning and dormant-target behavior are still under review; reuse their landed form, not a second pi-specific implementation.

## Existing seams and their implications

| Concern | Existing seam | Pi design requirement |
|---|---|---|
| Identity | `CallerEnvShape` in `src/harness/adapter.rs`; process names, process matcher, session variable, PID variable, role witness and marker variables | Declare only observed process/session facts. A generic `node` name is insufficient; match pi's actual entry point or a verified extension-supplied PID. Missing session and ambiguous role must refuse rather than borrow the pane's identity. |
| Children | `CallerSession::{Subagent,Marked,MalformedMarker,Unattributable}` and `BindingPolicy` | Do not infer top-level from missing third-party child metadata. Decide which supported native/extension child paths have evidence, and document unsupported ones. `registered_child_sessions` is appropriate only if a real parent registration path exists. |
| Hook admission | `HookAdmissionPolicy::RegisteredContract` or `QualifiedCallback` | Opencode's registered contract is a simpler starting point than Hermes' startup-qualified bridge. Exact event registration must agree with the payload; no executable-version probe is an operational prerequisite. |
| Lifecycle | `DecodedEvent`, `EventIntent::Lifecycle`, `EventKind::{Startup,Resume,Clear,Compact}` | Map actual session-start/switch/fork/compaction behavior. Only an admitted top-level lifecycle can enroll under the default policy. Current/tool events do not allocate. |
| Enrollment fallback | `EnrollmentPolicy::WitnessedStartAndUnownedTurn` | Hermes uses this for a witnessed CLI start that enrolls without context, then a qualified first turn. Do not copy it unless pi genuinely needs the split and can produce the witness. |
| Turn state | `HarnessAdapter::turn_state_edge` and `TurnEdge::{Busy,Idle}` | Agy maps invocation/stop. If pi has reliable lifecycle boundaries, report them; unknown remains unknown. A channel's own exact session state still gates self-wake. |
| Context | `encode`, `EncodedOutput`, `OutputPolicy` | Fixed guidance belongs on lifecycle/compact re-entry when recoverable there; ordinary quiet callbacks output nothing. Decode and encode must distinguish context-bearing output from control acknowledgements. |
| Delivery | `DeliveryPolicy` and existing watch/ACK protocol | Keep bounded frame ordering, priorities, truncation, deduplication and channel generation fencing. A pending item's `wakes` state comes from the daemon, not a local reimplementation of thresholds. |
| Setup | `default_config_root`, `setup_environment_inputs`, scope resolution, `setup/status/unsetup`, doctor projection | Install only owned extension/skill assets into the selected isolated root; preserve foreign or modified assets and user's settings. No permission or plugin enablement may be inferred from a file being installed. |
| Launch | `LaunchPolicy` | Keep interactive grammar adapter-local; map explicit profile directory, model and startup input through existing APIs. Refuse modes which cannot preserve the pane/session contract. |

`src/harness/unsupported.rs` already carries the `PI_CODING_AGENT` agent marker. Move its ownership into pi metadata in the same change that registers the adapter; otherwise registry validation or unsupported-harness classification can conflict. Registry tests currently use `pi` as an unknown ID and pin exact builtin/push-channel lists. Replace those negative fixtures with an actually unknown identifier and update the lists intentionally.

## Receipt evidence is the gating decision

`DeliveryPolicy`'s admission comment explicitly requires evidence that the frame entered the session: a stored row or a model request containing it. A return, 2xx or queue acceptance alone is not delivery evidence. The generic provenance name does not relax that predicate.

Opencode's bridge waits for its append's returned stored-message ID, carries it through `watch ack --via append --row`, and goes channelless when it cannot obtain a row. Its busy wake-pending mark is cleared only when a model step's parent message proves that it covered the append, not merely when any later model step starts (`integrations/opencode/plugin/herdr-threads.js`, append/drain, delivery recording and assistant parent checks). Pi's `sendMessage` may have a different return contract. Do not copy opencode's successful-return ACK logic if pi returns void or only schedules work.

Select one evidence-backed pi predicate before implementation: for example, observing the exact custom message in the session manager after durable append, or observing its exact complete frame in the next model-bound context. The research must establish whether such an observation is available, when it runs, what can remove/filter the message afterwards, and whether it applies while idle and while busy. If no honest mod-delivery predicate exists, retain ordinary hook-context delivery for the appropriate callbacks and leave receipts pending until that path's actual predicate holds. Do not invent a row ID or record a queue as an append.

A stored append and a guarantee of a turn are separate. Busy delivery arriving during a final response, abort or rejected tool can miss that turn. Retain wake intent until actual model-bound evidence consumes the item, or until a subsequent idle eligible wake starts a turn. No busy wake, no spurious wake for p1 at T=0, and no wake after the item was served in-turn. Preserve user drafts and modal input even when the wake uses an in-process API.

## Trust-policy changes expected

Review and amend the following in the same implementation commits where behavior changes:

- C1/C2 enrollment: identify pi's admitted lifecycle and how resume/fork/session switches preserve or replace native identity. Never merge seats or move/end a binding from a local hint.
- A2/A3 environment attribution: name the extension-carried session/PID/role values and their cooperative origin. Existing `cooperative_env_session` can cover this, as in opencode; do not call plugin-authored variables native attestation.
- A3 `cooperative_mod_delivery`: state pi's exact observation predicate and any recorded evidence. This is required even if the provenance constant is reused.
- A4 channel wake: explain the channel-only failure mode, ordinary turn-delivery fallback and lack of dormant wake when no extension/channel is active.
- Accepted limits: scope unsupported SDK/RPC/child modes, extension reload or session-switch ambiguity, and any gap between observed context and the final model request. Do not import opencode's hidden-session or server/attach limitations unless pi actually has them.

A standard native hook's `cooperative_hook_context` completion means its complete context output was written and flushed; it is explicitly not proof the model consumed it. It is distinct from the stronger adapter-specific channel predicate and from agy's transcript-confirmed `cooperative_hook_delivery`. Mixing these paths creates false evidence. Invitations remain separate from receipt ACKs, and truncated items never auto-ACK.

## Shared-file coordination

Wait for and rebase on opencode's landed main before shared-core implementation. Expected shared edits are registration/module exports and unsupported marker removal, registry/adapter seam tests, setup/help/doctor goldens, package validation, release-smoke and wake-e2e dispatch, shared skill/docs and trust policy. Opencode already changes `src/harness/registry.rs`, `src/harness/adapter.rs`, `src/scheduler/mod.rs`, `src/host/native.rs`, `src/daemon/health.rs`, `src/app.rs` and broad integration expectations. Treat the review's wake-status changes as an integration dependency.

Known shared launch gaps `ht-8wrr` and `ht-vxdp.11` can bypass adapter-composed argv on herdr-core terminals. They belong to the coordinator's shared work. Pi must not silently claim that path qualified or fix it independently.

## Required evidence and tests before calling support complete

1. Decode/contract and identity tests: missing/malformed ID and role, native versus nested process shape, stale inherited environment, child refusal, switch/resume/fork/compaction and failed check-in retry. Test old session callbacks after a successful switch.
2. Extension tests: passive idle append; p0/T=-1/T=3 wake decisions; busy in-turn delivery; arrival during final response, abort and tool rejection; exact evidence before ACK; truncated bodies; replay/dedup; stale watch generation; reconnect, stopped watch and missing API degradation; late callback after disposal.
3. Setup and launch tests: isolated root, idempotency, modified/foreign preservation, ownership cleanup, model passthrough, startup summary as one argument, rejected headless/RPC or disabling flags, no secret argv and no real config access.
4. Real-binary model-free tests: same extension installation and public launch entrypoints as production, isolated host/state/HOME, scripted provider, observed session and transcript evidence, draft/modal preservation, process cleanup. A mocked extension API alone cannot qualify pi's semantics.
5. Live wake e2e after credential approval, priority/receipt/invitation cases and busy delivery, then the required integration suite/leak checks on the merge tree. Current handoff requires heavy/tb for builds; preserve the repository lint, wire, neutrality, default-feature and speed gates.

Open design questions are pi's exact receipt observation, safe per-tool environment injection, supported child role evidence, what session switching does to deferred messages, whether native startup enrolls before the first model turn, and how the extension detects terminal versus nonterminal modes. These should be resolved by the primary-source dossiers and runtime probe rather than a new generic framework.
