# Pi harness adapter

Status: proposed design for review; production implementation has not started. Epic: `ht-jr1g`.

## Goal and requirements

Make the upstream pi terminal coding agent a supported herdr-threads harness: automatic enrollment, bounded in-context delivery, priority-correct idle wake, honest receipt attribution, isolated setup, and launch/handoff. The user assigned this session research, design, coordination and review; Sol implementers own production changes. The user requires the common adapter boundary and a presentation of the design and trust-policy changes before implementation.

Normative inputs are AGENTS.md and TRUST-POLICY.md. The [research package](research/report.md) establishes external capabilities; [repository seam analysis](research/candidates/architecture.md) identifies existing integration points. Research observations establish evidence, not exact-version admission gates.

## D1. One adapter and one in-process extension

Register `PiAdapter` under `src/harness/pi/`; native integration lives under `integrations/pi/`. Reuse `PushChannel`, `cooperative_mod_delivery`, and `IdleWake::Channel`, with no composer policy. Pi's extension can start a run in its existing process, so no host typing or external RPC server is needed. Consume opencode's landed channel-only wake changes before modifying shared registration or scheduler expectations. No new neutral trait is currently justified.

Support the terminal CLI, with an explicit `ctx.mode === "tui"` witness. `hasUI` is insufficient because RPC also exposes UI capabilities. Print, JSON, RPC and arbitrary SDK hosts do not enroll. Missing mode or required APIs disables the integration capability with a local diagnostic; no version string authorizes a fallback.

Without a live extension/channel there is no idle wake. Supported turn callbacks may recover delivery and reconnect the channel; no composer fallback is claimed. Named and herdr-core seats can use the same in-process channel once admitted. The existing herdr-core launch composition gaps remain owned by their shared-core tasks, and qualification must state their outcome.

## D2. Session and process identity

Use the exact session manager ID, never a session filename, title or pane label. The extension exports its current session and own PID in adapter-owned environment variables. The adapter validates these against the innermost matching pi process, with actual entrypoint/process evidence and a PID witness. It must never recognize arbitrary `node` as pi or reuse an outer pi's inherited PID/session. Native `PI_SESSION_ID` is corroborating input, not a reason to accept stale extension state.

Current names proposed for the contract are `HERDR_THREADS_PI_SESSION`, `HERDR_THREADS_PI_PID`, and `HERDR_THREADS_PI_MODE`. A nonterminal extension instance clears inherited ownership markers and stays inert. Every event payload freezes session, PID, execution generation and role. Startup/switch refreshes markers before model tools execute. Custom remote shell implementations which remove these fields have no attribution claim and are not silently repaired.

Move `PI_CODING_AGENT` from unsupported markers to pi's registered metadata in the same commit. Role attribution is cooperative under `cooperative_env_session`, not a new authentication boundary.

Pi does not supply a universal child-role marker. The official example's JSON/print children are excluded by mode and PID checks. Do not allocate seats for them or grant them parent permissions. Third-party interactive child launchers require an explicit cooperative role contract before they can be supported; missing evidence is not inferred as top-level. The shared skill states this limit once.

## D3. Lifecycle and resource ownership

Use `session_start` and its actual reason: startup, reload, new, resume or fork. Resume uses the same native ID; new/fork use the successor ID. Session shutdown/replacement fences the old execution before any async operation can complete, stops and reaps its watch child, and invalidates old delivery/wake candidates. A successor never ACKs an old frame merely because it inherited a pane.

Lifecycle registration uses the registered adapter contract. It reports the corresponding startup/resume/clear event through existing hook plumbing, preserving daemon-authoritative C1 and A2 decisions. Compaction restates the fixed guidance on compact re-entry. Reload reconstructs state from the current native session and fresh daemon admission; it does not replay an old process-global binding claim.

Check-in is marked successful only after its actual result; the next eligible callback retries failure. Watch `no_binding`/session mismatch invalidates the local bound mark. One extension owner per process/execution prevents a global plus project copy from opening duplicate watches. Disposal is idempotent, and every callback checks its frozen execution before and after awaiting anything.

## D4. Delivery, evidence and receipt timing

A successful `sendMessage` invocation is not a receipt predicate. The API is fire-and-forget, message-end callbacks precede entry insertion, and session-tree entries can exist before disk persistence. The extension tracks a bounded batch with exact IDs, full framed bodies, session/execution and a private nonce.

The admission predicate is observation of the exact full custom frame in the current session's model-bound context, together with its matching current-branch custom-message entry. Only then send `watch ack --via context` for eligible complete IDs. No invented row ID and no `--via append` based on void return. Truncated items remain unsettled and follow the existing continuation path. Invitations are delivered but never accepted automatically.

This predicate claims entry into the extension-observed session context, not provider receipt, model consumption or disk flush. Later extension transformations and provider/transport failure are explicit accepted limits. If the context path or matching session entry is unavailable, keep receipts pending; never downgrade the evidence to queue acceptance.

The existing watch protocol supplies daemon ordering, priority admission, bounded pages, presentation epochs, ACK retry and replacement/disability handling. Reuse its actual wire behavior and framing goldens. Deduplicate in-flight records by exact delivery identity and context version. Clear local state only on admitted completion or explicit generation teardown; retries cannot manufacture a second receipt or acknowledge changed/truncated content.

## D5. Idle, busy and wake are separate states

The bridge makes a fresh state decision immediately before invoking pi, in the same synchronous JavaScript turn, with no await between the final session/state checks and `sendMessage`. It never uses a remembered busy snapshot to authorize `triggerTurn`.

| State | Body delivery | Wake |
|---|---|---|
| Positively settled idle, passive item | Stage with `deliverAs: "nextTurn", triggerTurn: false`; observe on the next real context | None |
| Positively settled idle, daemon-admitted waking item | Stage body as `nextTurn` without starting a run, then submit the existing content-free wake marker | One in-process wake; ACK waits for body context evidence |
| Positively active model/tool run | Steer complete framed body into that run using the verified active-run path | None; no composer or separate wake submission |
| Manual compaction, retry gap, shutdown, uncertain state | Retain bounded pending items until a qualified callback | None |

`agent_end` is not final idle. Use `agent_settled` and a fresh `ctx.isIdle()` for idle eligibility. Conversely, `!isIdle()` is not enough to authorize steering with `triggerTurn: true`: manual compaction can be nonidle without a running agent. Maintain a positive active-run interval from start/context events, close it at end and before compaction, and revalidate before delivery. The implementation must demonstrate that its exact interval never includes a non-running phase that would turn a passive item into a new run.

A waking item arriving while busy is served in-turn where possible. An aborted or otherwise unconsumed item retains its daemon-derived wake intent until settled; an item already observed in context never causes a later wake. Passive items never acquire wake eligibility through local retries. A wake carries only the existing marker; it does not duplicate bodies.

The extension never edits the user's composer. TUI selectors and extension dialogs have no universal public blocked-screen getter, so real-binary draft/modal tests must qualify the in-process behavior. If those tests show interference, the affected wake path stays disabled until an evidence-backed guard exists; do not invent a screen heuristic.

## D6. Setup and launch

Resolve `PI_CODING_AGENT_DIR`, otherwise `~/.pi/agent`. The explicit launch profile maps to that override. Install only the owned extension and shared skill, with manifest digests and idempotent status/unsetup handling. Preserve modified or foreign files. All development setup uses a scratch agent root and HOME; no real user config is touched. No permission grant is added merely to make setup green.

`--kind pi` launches the interactive executable in its pane. The adapter owns option parsing, model/provider selection, profile-root mapping and startup input composition. `--model` maps to pi's `--model`; the upstream value accepts `provider/id` with an optional thinking suffix. If `--provider` is supplied it must accompany a model. Initial interactive options cover model/provider, thinking, continue/resume/fork and resource paths only where their current documented grammar is captured in tests; unknown or disabling options are refused. Positional startup input is owned by the handoff composer, never appended twice. Accept the documented interactive options and refuse print/JSON/RPC/SDK/server or extension-disabling forms, ambiguous positional bodies and secret-bearing API-key arguments. Startup text is the existing one-line handoff summary plus its exact recovery command, passed as one native argument. Never use an exact pi version as a launch prerequisite.

The implementer must test npm/interpreter and native binary process shapes against captured facts, especially inherited environment and resume/fork. Unsupported wrapper shapes fail with the existing admission diagnostics rather than guessing.

## D7. Trust-policy amendments

Amend the same implementation commit's normative policy:

- Name pi's session/PID/mode claims under existing cooperative environment provenance and its supported lifecycle paths.
- Define the context-plus-current-branch receipt predicate under `cooperative_mod_delivery`, including no ACK for queue acceptance and no proof of disk/provider/model consumption.
- State channel-only wake and no-channel behavior, including absence of native fallback.
- State excluded nonterminal/unknown-child modes, custom shell attribution limitations, stale callback fencing and downstream context-transform loss.

No seat merge, heuristic seat move, new receipt provenance constant, adversarial caller verification or peer wire change is proposed.

## D8. Implementation boundaries and qualification

After user design review and opencode integration, dispatch Sol tasks with exclusive paths and a frozen extension/Rust event contract. Rust identity/lifecycle/delivery declaration and extension implementation can proceed independently once that contract is agreed. Setup/launch follows the Rust skeleton; public registration and shared tests have one owner. Qualification and documentation follow integrated code, not isolated mocks alone.

Required evidence includes malformed/inherited identity refusal; mode and child exclusion; resume/fork/reload/compaction; passive idle silence; threshold-driven wake; busy delivery and stale-busy race; exact receipt evidence; truncation/replay/late callbacks; setup ownership; launch grammar; channel loss/reconnect; draft/modal behavior; and owned process/temp cleanup. Use focused Rust and JS tests plus real-binary fake-model probes. A live OpenRouter/DeepSeek wake matrix requires separate coordinator permission to use the existing credential.

Run the repository's clippy, neutral-core, wire-compat, formatting and default-feature checks through the required heavy/toolbox wrappers. Final review-ready handoff includes an independent super-roast, an unfiltered full suite and scoped leak checks on the integrated tree, and live e2e evidence or an explicit credential-related qualification gap. The main coordinator owns merge; this branch never pushes.

## Open decisions before implementation

The model-free probe confirms the chosen queue/context ordering and stale-busy hazard. The implementer must demonstrate the active-run state machine through tool, retry and compaction phases, and qualify TUI draft/modal behavior before support is declared complete. Review the proposed TUI-only initial scope and context-observation accepted limit before implementation. A live credentialed wake run remains separately authorized.
