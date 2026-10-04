# Harness adapters and Hermes integration

Epic: `ht-3bi`. Design input: approved static registry and Python bridge architecture, research package dated 2026-10-03. Implementation is on `harness-adapters`; this document does not authorize a product version bump, release, shared-server restart or coordinator-owned final sweep.

## Goal

A harness author can implement one Rust adapter module, register it once in the same binary, and obtain generic CLI selection, setup/status, contract/version discovery, health, doctor and host lookup. Claude and Codex retain their behavior, while Hermes interactive CLI receives bounded turn-boundary cooperative context through an owned Python plugin and supports explicit invitation acceptance, inbox reads and ACKs in an isolated native acceptance test.

## Scope and approval

The user approved the architecture preview, the same-binary static registry, broad 0.3.0 scope and Python bridge, and invoked autonomous super-auto with reviews retained. This spec concretizes those choices. `TRUST-POLICY.md` remains normative. Hermes support initially covers interactive CLI, default or explicitly selected profile, first qualified attachment, subsequent turn context and documented reset. Gateway, Desktop, TUI, automatic restored-session continuity without native resume evidence, exact compression-triggered recovery, composer stashing and pokes during a turn remain unsupported.

This branch does not include cli-names-handoff/v0.2.2. `threads-main` at w4:p1 owns serialized integration into main, final full sweep, cleanup and later approved 0.3.0 release.

## Architecture and boundaries

The binary contains a static ordered registry of adapter registrations. Registration checks duplicate IDs and host kinds. One module owns each harness's identity, metadata, contracts, recipe tables, installed admission witness, native decode/encode, setup transaction invocation/assets, runtime attribution, launch policy and optional composer/canary facilities. A single registration is the only required edit outside that module, excluding module declaration/build wiring and fixtures. There is no inventory/linker discovery, dynamic library loader, external plugin protocol or ACP runtime replacement.

Core discovers an adapter by typed ID and performs canonical seat resolution/check-in, attention budgeting, neutral guidance rendering, command construction/escaping, output flush and existing offer bookkeeping. The daemon decides all seat, binding, continuity, receipt and authority transitions against its canonical view. Adapters provide observations and cooperative claims; they do not move seats, authorize receipts or turn native history/ancestry into continuity.

Existing Claude JSON installation transactions, Codex JSON/TOML ownership transactions and Claude prompt-suggestion consent remain concrete backends. Move their calls behind adapters without redesigning their file formats or adopting a general configuration framework.

## One author interface

The following is the required public author interface shape; shared value types can use existing names where their meaning matches. Associated admission state is important: Codex keeps its typed installed-schema witness, and core never publicly downcasts it.

```rust
pub trait HarnessAdapter: Send + Sync + 'static {
    type Admission: Send + Sync + 'static;

    fn metadata(&self) -> &'static AdapterMetadata;
    fn contracts(&self) -> &'static [ContractDescriptor];
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget)
        -> InstallObservation;
    fn admit(&self, observation: &InstallObservation, budget: &CallBudget)
        -> AdmissionDecision<Self::Admission>;
    fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder;
    fn classify(&self, input: &HookInput) -> ContractObservation;
    fn decode(&self, admitted: &Self::Admission, input: &HookInput)
        -> Result<DecodedEvent, DecodeFailure>;
    fn encode(&self, admitted: &Self::Admission, event: &DecodedEvent,
              offer: &NeutralOffer) -> Result<EncodedOutput, EncodeFailure>;
    fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget)
        -> RuntimeAttribution;
    fn setup(&self, request: &SetupRequest, budget: &CallBudget)
        -> Result<SetupOutcome, SetupFailure>;
    fn status(&self, request: &StatusRequest, budget: &CallBudget)
        -> SetupStatus;
    fn unsetup(&self, request: &UnsetupRequest, budget: &CallBudget)
        -> Result<RemovalOutcome, SetupFailure>;

    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> { None }
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy> { None }
    fn canary_strategy(&self) -> Option<&dyn CanaryStrategy> { None }
}
```

`AdapterMetadata` declares a valid agent ID, display label, accepted legacy context spelling, executable lookup descriptor, Herdr host-kind aliases, supported setup scope and event-processing budget policy. Optional provider traits are shared capability types; authors implement only ones used. Their absence means a concrete unsupported diagnostic, never success.

A generic registry registration constructor erases `Self::Admission` internally. An admitted handle binds the registration identity, recipe/capability result and opaque adapter state. The internal blanket wrapper invokes the same adapter with its own typed admission state; erasure/downcast, if used, occurs only in this generic wrapper, never in core consumers or brand switches. A handle for one registration cannot be used with another. Production handles cannot be manufactured from a version string. No cloning or serializing an installed witness into a client authority claim.

`AdmissionDecision` represents listed/schema-matched/optimistic/refused with existing operator diagnostics and evidence meaning. `classify` is independent of admission, so refused or malformed native input can still report honestly classified violations. Classification cannot grant decoding capability. Native input capabilities and optional poke declarations remain recipe-bound; verified-by-use evidence never adds a capability.

`DecodedEvent` contains normalized role (`TopLevel`, `Subagent`, `Unknown`), native session reference, immutable external event ID, lifecycle/Current/observer intent, event metadata, delivery eligibility and bounded runtime attribution. `Unknown` cannot check in. `NeutralOffer` contains bounded fixed guidance plus escaped peer data and ready argv. `EncodedOutput` explicitly declares context-bearing versus observer-only output; observer-only output has no offer-consumption token.

## Identity and serialization

Use one registry-backed `AgentHarnessId`, not another closed enum of Claude/Codex/Hermes. Parsing an operational ID requires a registered adapter. Separate `OccupantHarness::Agent(AgentHarnessId)` and `OccupantHarness::Human`; Human has no adapter, recipe, hooks or launch policy. Preserve `Copy` where feasible through static registered identity references; otherwise deliberate ownership changes are implementation details, not wire changes.

Protocol and SQL use lowercase ID strings. Existing `codex`, `claude` and `human` serialize byte-identically. Context/journal fields preserve existing `Codex`, `Claude`, `Human` spellings through a context-specific serializer/deserializer; Hermes uses the adapter-declared `Hermes` spelling. Do not change old cache/journal format_version simply for Rust type cleanup. Deserialization accepts the documented legacy spellings in their existing domains; it must not silently coerce unknown IDs into Claude or Human. Unknown registry identity in a stored binding is a bounded unsupported diagnostic and cannot confer agent authority. Do not delete its history or end a binding merely because this binary lacks an adapter.

Every CLI selector derives admitted choices from the registry, including hook, setup, unsetup, launch, version/contract listing and doctor filters. Human-specific commands retain separate semantics. Existing syntax and argument order remain unchanged except the addition of Hermes/profile scope where required. No non-Codex-defaults-to-Claude fallback remains.

## Event metadata, contracts and evidence

Registry descriptors declare native event name, class, qualification fields, lifecycle buffering/holding rules, output capability and required verification milestones. Claude/Codex retain existing lifecycle-plus-tool verification, contract canonical JSON/hashes and the Codex resumed-rollout creator-version suppression. New metadata must not perturb old contract IDs merely because descriptors gained extra operational fields.

Event names are bounded ASCII identifiers admitting underscores in addition to existing names; accept no whitespace/control characters, arbitrary paths or unbounded strings. Contract classification validates declared JSON fields, while value constraints remain decoder failures rather than invented contract violations.

Hermes has separate descriptors for native callback projections and the bridge transport/envelope. Its projection records only allowed identity/shape information; normalized attachment/Current/Clear is synthesized by the bridge protocol and is never labeled a native SessionStart or native Resume event. Include origin/contract domain in evidence so a bridge envelope's success cannot verify a different native payload contract. Contract/recipe output and release rows expose that distinction.

Hermes tool observation is nonconsuming and uses `post_tool_call`, a native fail-open observer whose return is ignored. Do not register `pre_tool_call`: its errors/timeouts fail closed and could alter tool approval semantics. The native post-tool callback has session_id/task_id/tool_call_id/turn_id/api_request_id but no parent_session_id. Associate it only with an exact session/turn already qualified by pre_llm_call in the bounded role map; missing association remains unattributed, never presumed top-level. It may contribute callback-projection/version evidence, but never allocates/checks in a seat, fetches/consumes attention, injects context, changes approval decisions or produces a receipt. Tool arguments/results/error text are never forwarded. Bounded field-shape/type observations describe their exact native observer origin. A required post-tool milestone is named explicitly, never called pre-tool verification.

`RuntimeIdentity` distinguishes optional normalized release version from exact bounded identity key and provenance. Hermes preserves base_version, derived_version, commit, source, dirty and distance when reported by the runtime resolver; identity is unavailable if capture fails. A dirty/development commit has its own key and cannot become a verified bare release row. Claude/Codex normalized release keys and transcript attribution remain unchanged. PATH install observation is never runtime attribution. Manifest/release tooling must reject or distinctly encode development identities rather than strip suffixes to fit semver.

Do not infer a tested Hermes recipe floor from upstream date or local install presence. Define a candidate recipe for a captured exact runtime/build and explicit API contract, then record native test results. Unverified candidates are new/optimistic or refused according to the declared policy, never listed as verified. If no honest candidate admission can be established, keep native acceptance blocked and report the exact missing evidence.

## Health, doctor and compatibility

All active health aggregation, observation lanes, harness states, capability descriptions and doctor rendering iterate the registry. Per-adapter status carries independent installed, enabled, observed, admitted and runtime/evidence outcomes; installed plugin files alone never mean usable Hermes integration.

Shipped Health hello and its `HarnessHealth { codex, claude }` remain a frozen compatibility projection, derived from generic results. This is the explicit historical two-name exception, not the author interface. Add a separately negotiated `harness.health_v2` capability/request/result with an extensible ID-keyed map. New doctor queries it and uses legacy Health plus existing harness.states when unavailable. Old clients can read new daemon Health unchanged; new clients can talk to old daemons without requiring Hermes fields. Do not bump protocol version for this feature unless a concrete incompatible operation proves capability negotiation insufficient. Product release version is coordinator-owned.

Existing harness.states already has a vector and can enumerate Hermes without changing its container. Additional runtime identity details requiring new strict wire fields use a negotiated v2 result rather than adding unknown fields to an old strict struct. Legacy manifest keys and Claude/Codex state precedence remain compatible. Test every advertised capability has a handler.

## Setup and ownership

Adapter setup requests carry explicit scope resolved once from an environment snapshot: Claude/Codex retain config-root semantics; Hermes receives default or named profile and resolved profile home. Do not discover a user's arbitrary active profile heuristically. If a selected profile cannot be resolved with the supported native API, refuse with an actionable command; do not install into an unrelated default home.

`setup hermes` stages bundled `plugin.yaml` and Python entrypoint assets under the selected `$HERMES_HOME/plugins/herdr-threads`, using an owned manifest/fingerprint and atomic write/rename helpers. Exact native filenames/entrypoint schema follow the audited plugin loader and native tests. Store asset digests and the installed Rust executable path in an owned asset, never a shell command string. Paths containing spaces must work.

Installation and native enabling are distinct. Setup prints the profile-specific `hermes plugins enable herdr-threads` instruction. It does not rewrite user YAML, grant trust/consent or add --accept-hooks, HERMES_ACCEPT_HOOKS, hooks_auto_accept, --yolo or native permission bypasses. Read-only native status checks may establish enabled state only for the selected profile under a bounded probe that reads native configuration without plugin discovery or importing/executing third-party plugins. A same-interpreter config-loader helper is acceptable; do not add a Rust YAML rewrite/parser merely for this. Disabled status is reported explicitly; launch refuses rather than silently proceeding without hooks.

Unsetup removes only exact owned files and empty owned directories. Foreign files, modified assets, profile YAML and plugin enabled-selection entries remain untouched; report residue and native disable guidance. Interrupted install recovery and changed-file refusal use the same ownership principles as existing backends. Existing Claude/Codex manifests, adoption, conflict/refusal, baseline and prompt-suggestion restoration tests must pass unchanged.

## Hermes Python bridge

The bundled bridge uses the native plugin registration API and the standard library only. It does not replace the PTY/runtime, monkeypatch Hermes internals, run a daemon or transfer conversation history. Allowlist metadata: bridge schema/version, native callback name, platform, session_id, explicitly present parent_session_id, native turn/event identifiers, documented reset reason, callback shape facts and captured runtime identity. Tool arguments/results, raw user message, conversation history, model output and tool bodies must never enter Rust stdin or logs. Opaque IDs stay opaque strings within current bounded text limits.

Native pre_llm_call explicitly present `parent_session_id == ""` qualifies top-level for the captured API. A nonempty valid parent identifies a child. Missing/wrong-typed/overlong parent evidence is Unknown, never assumed top-level. `_persist_disabled` child suppression can mean no callback; absence makes no positive role claim. Child callbacks receive only the existing fixed read-only/subagent restriction if the captured API supports safe child context, and never check in/consume an offer. On-session-start is a role-incomplete observer and never registers.

Bridge state is bounded per process and keyed by session plus native turn identity. First qualified pre_llm_call requests Startup/attach, even with history. Later turns request Current only after lifecycle state is durably acknowledged by Rust; merely scheduling/starting a subprocess must not mark attachment complete. A documented CLI on_session_reset records bounded pending Clear metadata for the newly named session; the next qualified callback performs Clear/recovery. Unknown session rotation attaches a new execution and never infers repair. on_session_end is per conversation and neither ends bindings nor clears durable context. No native resume discriminator means no cooperative_continuity repair on restored seats.

Prefer Rust-owned durable local lifecycle journal/state: the bridge forwards native turn/session/reset observations, and Rust selects first attach versus Current using that state. If bridge state selects the transition, the Rust response must explicitly acknowledge accepted lifecycle state and exact replay must use the same event ID. Failed output/timeout before acknowledgment cannot downgrade the next call to unregistered Current. Restart tests must ensure a process-local empty cache does not replace a live binding on every ordinary turn or fabricate Resume. Native `turn_id` supports stable retry identity when captured; otherwise a bridge-generated ID is stable only within that one invocation/retry and is labeled synthesized.

The bridge invokes the owned absolute `herdr-threads hook hermes` executable with argv, `shell=False`, bounded JSON stdin and bounded stdout. No comment ownership marker becomes an argv token. Use sub-timeouts below the captured native callback deadline and below existing lifecycle/tool budgets. A callback exception, timeout, nonzero exit, malformed/oversized result or unavailable binary returns no context and allows Hermes to continue. Terminate/reap only bridge-owned subprocesses. Avoid unbounded communicate buffering; Rust is trusted owned output but the bridge still enforces a small response limit.

The post-tool observer returns None, catches all bridge exceptions and performs only bounded work within a native fail-open callback. All fallible observer bookkeeping is nonblocking and bounded. There is no pre-tool registration. Native callback deadline configuration below our bridge budget must be reported as a limitation; abandoned native workers can still complete late, so event replay/generation tests must prevent stale late output from replacing later state.

Runtime identity is captured once per bridge load with a bounded operation tied to the running Hermes interpreter/module location and selected profile, never a PATH Hermes executable. The official resolver can spawn git. A bounded child using the same interpreter/import roots may invoke that resolver and be killed/reaped on timeout, provided tests establish it identifies this runtime's install. If that cannot be established, use an unavailable identity rather than a guessed release. No resolver call occurs per hot callback.

Rust output flush records only the existing offer/output claim; the Python return is offered context, not proof native Hermes accepted it or the model consumed it. A discarded callback, output loss or native spill must not auto-accept an invitation or ACK a receipt. Explicit normal CLI cooperative actions retain cooperative_top_level and text inbox's separate cooperative_inbox_display observation.

## Launch, host and wake

Core retains guarded Herdr start, seat/hold/live-agent checks and correlated ObservedStartup. Adapter launch policy validates supported argv forms and produces exact native argv/wrapper policy. Codex owns its --no-daemon probe/rewriting, config insertion for supported interactive/exec/exec-resume forms, sandbox/approval behavior and uncaptured codex resume refusal. Claude remains unchanged. Do not use shell concatenation or reorder caller bytes.

Hermes permits only natively captured interactive CLI forms in the selected enabled profile. Refuse Gateway/TUI/Desktop, unverified one-shot/resume forms or overrides that suppress plugins; distinguish refusal from an accepted launch. Preserve native permissions and do not enable hooks on launch. Launch's managed_launch claim is wake-only, unregistered and replaced only by qualified lifecycle check-in.

Host kind and environment evidence of agent presence derive from registry descriptors, preserving existing Claude/Codex markers. Adding recognized Hermes kind to me-init refusal is a documented TRUST-POLICY A4 update: it is positive best-effort agent evidence, not adversarial identity verification. Environment markers must be captured and explicitly declared; do not invent generic marker heuristics.

Ordinary wake compares the current bound registered harness and live Herdr kind and retains idle/done recheck and focused/blocked/working refusals. Hermes declares no composer reader, composer_stash or poke_during_turn. Missing composer capability suppresses soft pokes with a named limitation; it must not accidentally change ordinary wake eligibility or retries.

## Storage migration

Keep migrations 0001, 0009 and 0012 unchanged. Add the next forward migration rebuilding active closed harness columns in occupant bindings, harness_version_evidence and harness_unattributed while preserving indexes, keys, references and rows transactionally. Resolve actual current schema version/DDL before naming the migration; do not race concurrent coordinator migrations.

Use bounded lexical ID constraints rather than enumerated brands for storage, and enforce registry recognition plus Human/agent role/provenance invariants in canonical Rust decisions. Removing brand checks must not let an arbitrary SQL string confer cooperative authority. Generic agent receipt eligibility replaces `IN ('claude','codex')` with registry/canonical occupant classification, preserving exact binding generation and cooperative_top_level checks.

Legacy schema verification remains version-aware: validate old literal checks only for their original schema, then migrate. New verifier checks new constraints/indexes. Reopening every supported old schema must preserve seat IDs, historical harness spellings, receipt state/provenance, human waivers, managed-launch placeholders, evidence rows and unattributed reasons. Fresh schema and migrated schema have equivalent invariants. Never end a binding because a persisted unknown adapter is absent; return unsupported operational behavior without silently rewriting history.

## Canary and author proof

Registry discovery emits IDs, contract-domain IDs, recipe/version identity descriptions and canary strategy descriptors. Existing Claude/Codex npm canaries remain supported. Hermes uses an explicit adapter-owned Python/source probe/install companion, or selected installed runtime for local native validation; npm installation cannot stand in for Hermes installation. Shared runner discovers descriptors instead of extending a central brand list for every adapter. A strategy may say unsupported automatic installation, but its result is skipped/inconclusive, never PASS.

Release manifest generation iterates registered adapters. Native observer contracts, normalized bridge contracts and exact runtime keys stay distinguishable. No unperformed keyed model case becomes verified because a setup asset exists or replay succeeds. Hermes CI model-free capture can verify load/invocation plumbing only; explicit model interaction is separately recorded.

A test-only minimal adapter, registered in an injected registry with no composer/canary capability, exercises selectors, hook decode/encode, setup/status, contracts, doctor/health enumeration and host lookup. It requires no core brand branches and returns named unsupported operations. Core can support a genuinely new primitive through a shared capability addition; independent Herdr host recognition, native captured fixtures and installer companions are explicit non-interface prerequisites.

## Focused acceptance and validation

1. Existing Claude/Codex hook parse/render/admission/contract and cooperative/context replay tests pass; contract IDs and protocol/context fixtures remain stable. Codex schema witness cannot be manufactured, cross-adapter handles cannot decode and resumed creator-version suppression remains.
2. Registry rejects duplicate IDs/kinds, unknown CLI IDs and Human-as-agent. Minimal adapter author proof passes generic discovery without adding production brands. Unknown stored identity yields diagnostic, not authority/default Claude.
3. Setup regressions cover existing manifests/adoption/interruption/refusal/exact removal and native permission consent. Hermes isolated profile install/disabled/enabled/status/unsetup covers modified/foreign assets, profile separation and paths with spaces; no real user config changes.
4. Bridge fixtures prove no raw content reaches Rust, bounded IDs/bytes/cache/output, parent absence/child suppression, first qualified attach/later Current/reset, failed-first-attach retry, process restart and replay, observer-only nonconsumption, exact session-role association and absence of pretool registration. Timing/failure cases cover Rust error/hang/oversized output, native abandonment and late/stale event handling without receipts or leaked children.
5. Migration fixtures reopen every supported schema and compare keys/history/receipts/human waivers/evidence. Hermes canonical check-in and inbox display work; arbitrary harness strings and observer/child inputs cannot gain authority.
6. Old/new daemon-client compatibility covers frozen Health, capability absent fallback and harness.health_v2; registry health/doctor/state iteration includes Hermes. Generic canary discovery preserves old cases and reports missing Hermes credentials/installer as inconclusive.
7. Isolated named private Herdr native acceptance captures plugin discovery/native enable/refusal, guarded launch, actual qualified callback and delivered turn context, child behavior and explicit cooperative invitation accept/read/ACK. Reset/resume/compression outcomes are recorded individually. Native receipts preserve declared provenance. A fixture or configured file is never native PASS.
8. Run relevant cargo tests only, cargo fmt, `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, and before coordinator merge `nice scripts/check-default-features`. Tests use isolated HOME/config roots, test_support-owned children and appropriate explicit test target/nextest groups. Stop every owned helper/private server before reporting. No branch full-suite sweep: coordinator owns it and scoped process leak checks.

Review the architecture and implementation independently before coordinator handoff. Report failed/inconclusive native matrix entries honestly; unverified lifecycle/launch forms stay unsupported rather than being declared parity.

## Decisions, alternatives and bounded open issues

- Chosen registry + associated-admission interface over enum facade: the facade retains per-brand edits. Chosen Python native plugin over Hermes shell hooks: shell projection drops parent_session_id. Chosen profile-specific file staging and native enable guidance over YAML rewriting/auto-consent.
- Chosen negotiated extensible health over breaking strict Health hello or protocol bump. Legacy projection is the sole deliberate fixed two-name compatibility seam.
- Recommended lifecycle authority is Rust-owned durable state; Python observes native identities/reset and avoids treating timeout as successful attachment. Implementer must resolve the existing journal API and native turn retry semantics before claiming restart/replay behavior.
- Exact Hermes candidate recipe/build, safe runtime identity capture, native enabled-status query and allowed launch argv require a focused source/native feasibility proof during adapter development. Capture these decisions in the nested Hermes spec; no optimistic floor or automatic resume repair is implied here.
- Chosen post_tool_call observer over pre_tool_call because native pretool exceptions/timeouts block tools. If API capture cannot supply exact session/turn role/version association, leave that evidence milestone inconclusive; do not change native approval behavior to make a test pass.
- Existing HarnessEvidence has deny_unknown_fields, strict release-version normalization and an ASCII-alphanumeric event validator. New origin/identity/event semantics cannot silently reuse it. Prefer a separately negotiated hook.harness_evidence_v2 if rich fields are necessary; a field-compatible adapter-specific exact identity grammar is acceptable only when old daemon handling cannot collapse a development identity into release success. Preserve legacy Claude/Codex command semantics. An old daemon lacking the new capability yields explicitly skipped Hermes persistence, never falsely successful attribution. Potential structured runtime-identity wire/storage evolution must use compatibility-aware results and forward migration, or separate explicit identity key without extending old strict structs. Exact implementation is a nested evidence design decision, preserving release/development distinction.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

Implementation owner: record material deviations, captured Hermes version/build and API/launch/profile forms, targeted test commands/results, native matrix outcomes and explicit limits, review resolutions and coordinator handoff commit. Do not label skipped native cases verified, do not claim final sweep or merge before coordinator evidence, and do not bump/publish the release from this branch.
