# Hermes experimental adapter

The adapter, owned Python plugin and bounded driver are implemented and tested with
source-shaped synthetic APIs. Native recognition, actual context delivery and
cooperative model/receipt acceptance remain unverified. Do not treat setup, an
enabled config entry, a fixture or a replay as a native PASS.

## Operator scope

Select the intended Hermes executable and profile under the same environment used
for setup and launch. `setup hermes --profile NAME`, `setup-status hermes --profile
NAME` and `unsetup hermes --profile NAME` resolve that explicit profile with the
captured official APIs. Omission selects `default`, not the sticky active profile.
Named profiles must exist. Lexical HERMES_HOME is preserved while ownership/locking
binds its physical home. Bare setup considers all detected registered adapters;
bare status/unsetup considers every registration. A bare command refuses --profile.

Setup installs three owned files under the selected home's
`plugins/herdr-threads`: `plugin.yaml`, `__init__.py`, `bridge_config.json`. A private
ownership transaction records their exact digests and generation. Native inspection
may perform normal Hermes initialization, permissions, backups, locks and recovery;
it neither grants permission nor rewrites config values. Enable with the printed
`hermes --profile NAME plugins enable herdr-threads` command and retain native consent.
Installed, configured enabled, activation, callback observation and admission are
separate facts. Unsetup preserves modified/foreign assets, YAML and native enable
selection, reporting residue and manual disable guidance.

Managed launch uses one policy-owned `chat`, one explicit --cli and one profile.
Captured model/provider options and chat -q/--query occupy their actual parser
positions. Gateway, Desktop, TUI/native/oneshot/resume, arbitrary subcommands,
suppressed plugins and approval bypass flags refuse. No yolo, accept-hooks,
HERMES_ACCEPT_HOOKS or hooks_auto_accept is inserted. A private modified Herdr must
advertise the actual guarded process-hint capability; a hint remains cooperative
recognition input. A successful managed launch is wake-only, unregistered, and is
not a hook check-in or receipt. Hermes has no composer provider or in-turn poke.

## Runtime and evidence

The bridge captures runtime/build identity once in its initialized native context,
including actual interpreter and agreeing loaded module origins. It keeps that
startup observation for its lifetime; later source edits need no restart. This is
not continuous loaded-byte attestation. Canonical seat identity, binding generation
and continuity are decided independently by the daemon.

One native effective-config reader periodically supplies an observed timeout. The
read is not cancellable, and its value is not a dispatcher snapshot. Unknown,
failed, pending or stale observations skip delivery. Callback access is nonblocking;
one decreasing local deadline, owned-child termination/reaping and replay guards
bound delivery. A hung reader can survive bounded unload; no replacement reader is
started while it survives. Report that failure rather than claiming clean unload.

Two independent domains are `native_callback` with `native_shape_observation`
origin and `bridge_envelope` with `bridge_envelope` origin. Their qualified-turn
and qualified-post-tool milestones describe their exact source/runtime/contract.
The post-tool callback forwards allowlisted IDs/type shapes only, never tool
arguments/results/errors. It has no check-in, attention or receipt effect.
Native start/reset notifications alone cannot attach a seat. Qualified turns route
Startup/Current/Clear through the durable Rust journal; no native Resume is invented.

Callback return, lifecycle ACK, timely client evidence replies, native context
acceptance, API message delivery, model consumption and explicit model-issued
accept/read/ACK are separate observations. Pinned native context enters the **user
message**, not the system prompt. Ordinary string `api_content` is skipped for
MoA/codex_app_server; multimodal and spill paths need their own actual measurement.
`_persist_disabled` may omit the hook entirely. Zero child callback samples cannot
PASS child delivery. Cooperative receipts retain `cooperative_top_level`; default
text inbox's separate `cooperative_inbox_display` claim is not model-consumption
proof. No callback or driver output is a daemon acknowledgment.

## Adapter author boundary

Implement `HarnessAdapter` in one Rust module (submodules allowed), then add one
same-binary static `Registration::new(&ADAPTER)` with module/build wiring and fixtures.
Use the actual trait in `src/harness/adapter.rs` and registration in
`src/harness/registry.rs`, rather than adding core switches on the harness name.
Metadata owns ID, context spelling, host aliases and supported scope; registry
validation rejects collisions and Human aliases. Typed admission belongs to that
registration; it grants parser capability, never canonical caller authority.

The production registry contains three adapters: Claude, Codex, Hermes. Test-support
builds add the fourth fixture that exercises generic consumers. `adapters --json`
is same-binary metadata discovery, not installation or native recognition. Optional
installer/launch/composer/canary providers declare capabilities explicitly; absent
providers yield unavailable operations. Installer policy owns hook inspection and
skill destinations; Hermes declares no installer skill destination. External native
recognition, captured APIs/fixtures and canary companions remain separate prerequisites.
An adapter without native evidence domains does not inherit another adapter's proof.

Claude/Codex operational hooks use versionless registered strict contracts and
optional runtime metadata. Setup/launch resolve the selected executable and owned
configuration separately. Do not manufacture an installed-version witness, build
hash, runtime identity or rich composer/receipt capability from valid input.

## Measurement contract

Use the exact frozen Threads SHA and runnable binary digest after independent code
reviews/fixes. Prepare a private schema1 input under an owned `/private/tmp` root;
review its complete concrete argv/environment/actions before any live permission
decision. The driver Python interpreter is explicit:

```sh
HT_PROBE_PYTHON=/absolute/driver/python \
  sh scripts/native-hermes-probe.sh --preview --input /private/tmp/owned/input.json \
  --result /private/tmp/owned/preview.json
```

Preview validates input but starts nothing. Its input digest binds the private argv
document; public output contains hashes rather than personal paths or raw native
payloads. Input keys are `schema_version`, `producer`, `isolation_root`, `profile`,
`home`, `physical_home`, `state_root`, `host_endpoint`, `installation_token`,
`interpreter`, `launcher`, `source_root`, `runtime_identity`, `threads`, `host`,
`api_mode`, `timeout_seconds`, `owned_services`, `cleanup_argv`, `stages`. Optional
native inputs are `native_scope`, `runtime_command_file` and `model_evidence`. `threads` binds path,
SHA256 and source_commit; `host` additionally marks modified=true and the required
`agent_start_process_hint_v1` capability. These explicit source identities are not release
versions or fabricated kernel witnesses. Normal generic launch performs the actual
peer/capability/start negotiation.

`owned_services` contains at most one private_host and one private_daemon with
reviewed explicit argv. Only processes started in new sessions by this driver are
signaled/reaped. `cleanup_argv` is the reviewed exact private namespace shutdown;
it runs under a separate one-second cleanup reserve even after measurement timeout.
It must close owned native resources before their server exits. Do not put a shared
server or foreign tab/pane in that input. Output confirms owned process reaping;
the coordinator must additionally check private pane/descendant/native-reader
settlement and UUID-scoped leak results. Failed cleanup remains FAIL.

`--dry-run` requires producer=synthetic_fixture and explicit temporary stand-ins.
Dry children receive only an allowlisted noncredential environment and isolated
HOME, CODEX_HOME and CLAUDE_CONFIG_DIR. Its strict stage observation is `{schema_version:1,stage,provenance:"synthetic_fixture",
samples,observed,supported}`. The named Python regression executes this real command
orchestration. Native commands cannot reuse observed:true fixture JSON as proof.

`--native` requires producer=prepared_native plus settled scope: recognition_only,
native_callbacks or live. Each scope has separate necessary authority; the option
does not grant it. Generic guarded-launch stdout supplies only the managed-launch
predicate. The existing selective canary receives the parent's unchanged official
runtime-command input, exact profile/home/generation/private endpoint/Threads binary
and same-binary descriptors. It supplies actual loader gates and independent native
domain reply milestones; it does not prove returned context or model consumption.
Source capture/recognition/callback/model stages remain separate. Missing native
context/model observations stay INCONCLUSIVE; no unperformed stage passes.

All matrix verdicts are PASS, FAIL, INCONCLUSIVE or SKIPPED, with nonzero sample
requirements. Unknown dependencies, API forms, configured timeout, enablement,
credentials and mode-specific context delivery cannot PASS. Resume, compression and
uncaptured native abandonment forms stay explicit. Raw stdout/stderr is bounded
and discarded after parsing; secrets/native user/tool bodies never enter results.
Result files are new private files and never overwrite a failed attempt.

Live cooperative verification requires genuine native model-issued action evidence
and matching canonical seat/binding/receipt records. Operator/driver setup commands
are not model actions. Use an unhinted request to check mail; supplying exact CLI
commands or IDs in a launch prompt cannot prove hook-context delivery. The live observer reads the captured physical Hermes home's `state.db` and the
captured Threads instance's `threads.sqlite3` at
`STATE/instances/SHA256(raw host-endpoint bytes)/threads.sqlite3`, with its actual
private `namespace` UUID; no root-database fallback in SQLite read-only mode without
importing Hermes. The private `model_evidence` expected predicates are
`session_id`, `after_id`, `user_row_id`, `started_at`, `finished_at` (epoch seconds,
maximum 120-second window), `context_marker`, `seat_id`, `generation`,
`execution_id`, `target_id`, `thread_id`, `message_id`. They are reviewer inputs,
never supplied observed/PASS verdicts. Actual active persisted rows supply observations.

A selected ordinary-string `api_content` containing the expected marker proves
only prepared user-message persistence. It does not prove callback return,
successful API send or model consumption. Other selected API modes remain
unqualified. The persisted schema has no callback `turn_id`: the observer scopes
a fresh user row through the next active user row and joins assistant terminal
call IDs to exactly one completed native tool row. That boundary is not an exact
callback-turn witness. Calls must be plain foreground argv for the exact Threads
binary; wrappers, shell chains and background/polling forms remain inconclusive.
Source-defined matching global scope/complete cooperative options are supported.
Inboxes/reads report actual model call and successful terminal result only.
Accept/ACK additionally require the live canonical Hermes binding and a fresh
matching canonical actor/generation/session/execution observation within the call
interval (100ms persistence-clock tolerance). ACK reads the current manifested
`receipt_state` projection, with legacy `receipts` only for non-manifested sends;
automatic inbox-display acknowledgment cannot prove an explicit model ACK.
Raw user/tool content and IDs are
never copied into public results. Missing/stale/mismatched observations cannot PASS.

The observer cannot demonstrate how the model learned an action. A prompt
containing the expected marker, exact target/message IDs or Threads command is
unqualified for context attribution. Callback-return, exact callback-turn/API
send/model-consumption and child/reset observations still require separately
captured producers; no domain reply or caller JSON supplies them. The driver
therefore keeps `native_acceptance=UNMET`, even when individual measured action
stages PASS.
Final live measurement and acceptance belong to the coordinator, not leaf dry tests.


The native input now requires `launch` with exact private `pane`, `terminal` and
`agent_name`. The driver constructs the digest-bound Threads command with
`--state-dir`, `--host-endpoint`, `--json`, `launch --pane ... --kind hermes
--name ... -- --profile NAME --cli`. The adapter owns the emitted `chat` token;
passing `chat` as a caller subcommand is refused. A native guarded-launch stage
cannot substitute an echo or another executable/operation/scope. PASS requires
the actual report's pane, name, composed argv, resolved launcher, selected home,
profile and prelaunch identity/observation scope. It means managed launch only.
The separately constructed private-host `agent get` matches the actual
`result.agent` pane/terminal/name/Hermes label and nonpending status; it is an
advisory recognition observation, not kernel epoch/incarnation attestation.

An optional strict `measurement` object in both driver and captured companion
input enables separate selective native API diagnostics. Its fields are
`schema_version:1`, a fresh nonnil `invocation_id` UUID, exact `target`, expected
`seat`, bounded `context_marker`, and `child_form` (`explicit_parent_callback`
or `persist_disabled_no_callback`). The captured official trace/native-driver
argv is unchanged; no bootstrap parsing or reconstruction occurs. The default
canary result shape and domain credit stay unchanged. Opt-in output has a separate
`measurement` scope `selective_native_api_invocation_not_model_delivery`.

The producer captures the actual official dispatcher's returned context, then
matches a fresh completed event/operation and prepared kind in the private context
journal to the current canonical Hermes binding. Only a derived context hash and
byte count leave private memory. A declared parent-qualified child must return
the fixed read-only restriction and conserve bounded private context/attention
and canonical table snapshots. This measures a selective declared-child API
invocation; it does not establish that a running native subagent dispatched a
callback. A native form known to suppress callbacks is explicitly skipped.
Declared-reset None is insufficient: the producer requires an unchanged current
context plus new-session reset hint, then a qualified new-session return,
consumed hint, immutable prepared Clear operation, and same-seat canonical
execution/generation/session transition. This is selective reset API behavior,
not an actual interactive CLI reset or model delivery.

Missing, zero, ambiguous, oversized, stale or mismatched samples remain
inconclusive; child-state conservation is an observed bounded before/after
projection, not atomic proof of all concurrent activity. All selective reads use
the actual hashed instance and namespace, bounded private journal history and
read-only SQLite deadlines. Real native/model acceptance remains UNMET until
the coordinator performs separately authorized, reviewed final-build measurements.
