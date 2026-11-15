# Pi harness adapter

Status: proposed design for review; production implementation has not started. Epic: `ht-jr1g`.

## Goal and requirements

Make the upstream pi terminal coding agent a supported herdr-threads harness: automatic enrollment, bounded in-context delivery, priority-correct idle wake, honest receipt attribution, isolated setup, and launch/handoff. The user assigned this session research, design, coordination and review; Sol implementers own production changes. The user requires the common adapter boundary and a presentation of the design and trust-policy changes before implementation.

Normative inputs are AGENTS.md and TRUST-POLICY.md. The [research package](research/report.md) establishes external capabilities; [repository seam analysis](research/candidates/architecture.md) identifies existing integration points. Research observations establish evidence, not exact-version admission gates.

## D1. One adapter and one in-process extension

Register `PiAdapter` under `src/harness/pi/`; native integration lives under `integrations/pi/`. Reuse `PushChannel`, `cooperative_mod_delivery`, and `IdleWake::Channel`, with no composer policy. Pi's extension can start a run in its existing process, so no host typing or external RPC server is needed. Consume opencode's landed channel-only wake changes before modifying shared registration or scheduler expectations. D3 adds one neutral output-policy setting to keep channel mail out of lifecycle output; all other behavior uses existing seams.

Support the terminal CLI, with an explicit `ctx.mode === "tui"` witness. `hasUI` is insufficient because RPC also exposes UI capabilities. Print, JSON, RPC and arbitrary SDK hosts do not enroll. Missing mode or required APIs disables the integration capability with a local diagnostic; no version string authorizes a fallback.

Without a live extension/channel there is no idle wake. Supported turn callbacks may retry admission and reconnect the channel; they do not substitute a hook-body delivery path. With the channel disabled or unavailable, ordinary explicit inbox remains the delivery fallback and no composer wake is claimed. The adapter reports that it has no hook turn-delivery fallback (`binding_turn_delivery` false); existing live-channel handling still governs availability. Named and herdr-core seats can use the same in-process channel once admitted. The existing herdr-core launch composition gaps remain owned by their shared-core tasks, and qualification must state their outcome.

## D2. Cooperative role, session and process identity

Use `sessionManager.getSessionId()`, never a filename or pane label. The bundled extension is the producer of an explicit cooperative role assertion. At initial TUI startup it declares `top_level` only when no inherited foreign integration owner or explicit child marker is present. Capture inherited markers before writing any new ones. An inherited `HERDR_THREADS_PI_PID` different from the current PID makes the role unknown-child and disables enrollment; do not overwrite that evidence and promote the process. A cooperating child launcher can explicitly set `HERDR_THREADS_PI_ROLE=child`; child and malformed/unknown roles never open a top-level channel. The ordinary interactive extension then stamps `HERDR_THREADS_PI_ROLE=top_level`, `HERDR_THREADS_PI_SESSION`, `HERDR_THREADS_PI_PID`, and `HERDR_THREADS_PI_MODE=tui` from its own current context.

The adapter requires that explicit role, valid session, TUI mode and matching current harness-process PID. Identify the innermost pi entrypoint/native executable, never arbitrary `node`; a foreign ancestor PID cannot qualify a nested process. The public launch shim scrubs all these ownership markers before launching a fresh pane. Native `PI_SESSION_ID` is corroborating input only. Move `PI_CODING_AGENT` out of unsupported markers in the same registry commit.

This is a cooperative assertion under `cooperative_env_session`, not native role attestation. A third-party launcher that deliberately strips inherited markers and starts an interactive child is indistinguishable from a new ordinary TUI. That accepted limit is explicit; such launchers must participate in the child marker contract. SDK multi-session hosts, nonterminal children and custom remote shell implementations without these claims are outside initial support. No heuristic moves or ends a seat.

## D3. Lifecycle, guidance and ownership

`session_start.reason` distinguishes startup, reload, new, resume and fork. Establish a fresh local execution owner, refresh environment claims and perform canonical lifecycle admission. Startup/reload can initialize idle directly from a fresh current-session `ctx.isIdle() === true`; they do not need a previous `agent_settled`. Otherwise begin in Unknown and wait for a qualifying callback. Resume claims the existing ID; new/fork use their native successor ID. Pi metadata never merges seats.

Lifecycle output has a separate route from mail. Add a neutral `OutputPolicy` setting, defaulting to existing behavior, for channel-owned delivery. Under that setting the shared hook path performs lifecycle registration but composes only fixed guidance, routing and admission diagnostics, omitting inbox offers and receipt-bearing bodies; pi encodes this as `ObserverOnly`. It grants no hook-context receipt and does not consume omitted delivery. The extension keeps this bounded guidance until `before_agent_start` can return it in a persistent custom message, or a completed turn boundary can append it. Watch is the sole mail/receipt path. This is the one justified neutral policy addition; no pi-name branch is added to core.

Fixed guidance is inserted once after startup, compact re-entry and successful tree navigation, never on every ordinary context call. It is tracked separately from mail IDs and never ACKed as mail. A failed lifecycle call does not mark the extension bound; the next eligible callback retries it. Missing or stale channel admission reports the existing unavailable state rather than injecting apparent success.

On `session_shutdown`, fence the old owner first, stop/reap its watch and cancel its timers. Report the existing session-end binding event as a best-effort liveness disconnect, never retire or end a seat. Remove only environment fields still equal to that owner's values; a successor's values must survive late cleanup. A failed daemon disconnect leaves only the existing conservative stale-liveness limit; closed channel ownership grants no idle wake. A successor must complete lifecycle admission before receiving or ACKing.

On `session_tree`, keep the native session and canonical binding, increment the local branch revision, invalidate unconsumed proposal/context evidence, recheck the current branch, and restage fixed guidance. Previously valid receipts remain settled. Unacknowledged items absent from the selected branch return to pending; items still present await exact context observation. There is no attempt to retract historical mail or reverse receipts.

Only one integration owner exists per process. A duplicate global/project installation stays inert. Reload establishes a new owner and reconciles only this integration's known unacknowledged custom frames against the current branch and daemon replay. This design places no message bodies in pi's native nextTurn/steer/follow-up queues, so old native queue cancellation is not required or claimed.

## D4. Bounded delivery and honest receipt evidence

The watch child provides canonical framed items, priority/deadline keys and wake eligibility. WatchLine carries no creation key: tie-breaking preserves first-observed canonical watch order, matching the existing delivery assembler, with an unchanged ordinal for a retained ID on re-offer. Exact age ordering across owner replay cannot be reconstructed from this interface; this inherited limit adds no invented timestamp or wire extension. Keep bodies in an extension-owned buffer until a qualified boundary. Its aggregate limits are 256 records and 2 MiB of UTF-8 body/frame data, plus bounded metadata; one admission page remains at most 32 items/64 KiB under the existing limits. These are design bounds to test, not measured resource claims. Retain the best pending items by the existing priority/deadline/stable-tie ordering. At capacity a better item displaces a worse unsubmitted item; no displaced item is ACKed or marked displayed. An oversized item remains canonical and uses the existing bounded continuation route.

Watch input remains readable at capacity: parse each bounded frame, retain an admitted wake-control bit independently of body storage, and discard excess bodies without settlement. Thus a late p0 can replace passive content and authorize a wake even when the body cache was full. Set a replay-needed flag whenever any eligible body is not retained, whether an incoming body is discarded or a resident body is displaced. Once capacity is released (including after an admitted page is acknowledged), consume that flag to schedule one watch replacement and invoke canonical pending replay/refill; do not depend on remaining re-offers from the old watch. Preserve the flag if replay fails, and re-arm it if replay again drops any body. Do not reconnect repeatedly merely because the buffer is full: a new replay requires released capacity, with failed attempts subject to bounded backoff. Replacements and ACK retries are execution-fenced and use the existing bounded backoff. Pressure tests must cover more than the cache limit, urgent arrival at saturation, and eventual replay after space is released, including lower-ranked incoming overflow with no resident displacement and all old-watch re-offers exhausted; a failed replay keeps durable mail pending and reports channel degradation rather than inventing completion.

At every admission boundary sort the entire retained pending union before choosing the next page; arrival order and watch-page boundaries do not decide model order. Move that exact page to a proposed record with a private nonce, session, execution, branch revision and full bodies. Never clear it merely because the hook returned. A later callback proves acceptance, refusal or a changed generation. Bound proposed records within the same aggregate budget.

The receipt predicate remains exact complete frame observation in the current session's model-bound context, together with its matching accepted current-branch custom-message entry. Then send `watch ack --via context` for those eligible IDs. A callback return, void sendMessage result, preview entry, native queue acceptance or message-end event is insufficient. No invented stored row ID, no automatic invitation acceptance, no truncated-item ACK.

The observed context and accepted session projection are cooperative evidence, not disk flush, provider receipt or model consumption. Later handlers may transform context and transport can fail; document these accepted limits. If returned drafts were changed/rejected or a cancellation left entries without another model request, receipts stay pending. Reconcile exact nonce/body matches before retrying so an accepted-but-unobserved proposal is not blindly duplicated. Failed ACKs retry only the already evidenced exact IDs, under their original execution and branch evidence.

## D5. Boundary delivery and content-free idle wake

Bodies enter pi through explicit native boundary results, never through queued sendMessage calls:

| Callback/state | Action |
|---|---|
| `before_agent_start` | Return one custom message containing pending guidance and the sorted next mail page; it is persisted before the first model request |
| `turn_end`, `outcome === "completed"` | Append the sorted next page to the existing proposed entries; request `continue: true` only when this invocation added unseen mail |
| `agent_before_settle`, `outcome === "completed"` | Final opportunity for newly arrived mail using the same bounded draft operation |
| Error, aborted, manual compaction or unqualified boundary shape | Do not propose continuation; preserve pending mail for a later qualified boundary |
| Settled idle with no daemon-admitted waking item | Retain bodies locally; no run starts |
| Settled idle with admitted wake intent | Submit only the existing content-free wake marker; its `before_agent_start` injects the sorted body page |

Preserve other extensions' `event.entries` and `continue` values. Do not return unconditional continuation; a page already proposed or consumed cannot request another step. Bound retries to one proposal per retained ID per outer native run. If another extension rejects/alters a proposal, or the run ends with a non-completed outcome, suspend automatic wake for that unchanged attention rather than repeatedly starting outer runs. Keep mail unacknowledged for a later user-driven run or genuinely new attention; routine settlement and unchanged reoffers cannot release this suspension. Retained item witnesses plus at most 256 recent wake keys bound novelty tracking. If this history saturates while wake is blocked, absence from the cache cannot prove new attention: suspend automatic wake conservatively until a permitted user/owner/channel transition, while canonical mail remains pending. This is a wake-only degradation under rejected/no-progress saturation, not receipt settlement. The real-SDK probe confirms completed boundary continuation stays inside one outer run. It also found that pre-settle proposals can revive synthetic error/aborted outcomes despite documentation's hard-exit wording: the explicit completed-outcome guard is mandatory. Actual cancellation can commit entries while suppressing continuation; the D4 predicate handles that without a false receipt.

Idle eligibility requires the current admitted TUI owner, live channel, fresh `ctx.isIdle()`, no observed blocking extension-UI prompt, and no wake in flight. Initialize it at successful startup/reload as in D3, then refresh on `agent_settled`. Track `ui_prompt_start`/`ui_prompt_end` spans conservatively; they do not prove coverage of every built-in selector. No composer text is edited. Built-in draft/modal behavior remains a real-TUI qualification gate; an interfering path stays disabled until an evidence-backed guard exists.

Make the final identity/idle/wake-intent checks and sendMessage call in one synchronous continuation with no await. The submitted marker contains no body. A start/context event for the captured owner proves progress; its actual delivery clears only the corresponding consumed wake intent. If no start, context or settled event appears within a 30-second observation interval, mark the attempt unreceived and suspend further automatic submits for that unchanged attention. Re-enable only on a later authentic session event, newer attention or successful channel replacement, after fresh idle checks. This bounded no-progress policy deliberately prefers an explicit degraded wake over duplicate automatic runs; it does not promise autonomous recovery from every runtime failure.

## D6. Setup and launch

Resolve `PI_CODING_AGENT_DIR`, otherwise `~/.pi/agent`. The explicit launch profile maps to that override. Install only the owned extension and shared skill, with manifest digests and idempotent status/unsetup handling. Preserve modified or foreign files. All development setup uses a scratch agent root and HOME; no real user config is touched. No permission grant is added merely to make setup green.

`--kind pi` launches the interactive executable in its pane. The adapter owns option parsing, model/provider selection, profile-root mapping and startup input composition. `--model` maps to pi's `--model`; the upstream value accepts `provider/id` with an optional thinking suffix. If `--provider` is supplied it must accompany a model. Initial interactive options cover model/provider, thinking, continue/resume/fork and resource paths only where their current documented grammar is captured in tests; unknown or disabling options are refused. Positional startup input is owned by the handoff composer, never appended twice. Accept the documented interactive options and refuse print/JSON/RPC/SDK/server or extension-disabling forms, ambiguous positional bodies and secret-bearing API-key arguments. Startup text is the existing one-line handoff summary plus its exact recovery command, passed as one native argument. Never use an exact pi version as a launch prerequisite.

The implementer must test npm/interpreter and native binary process shapes against captured facts, especially inherited environment and resume/fork. Unsupported wrapper shapes fail with the existing admission diagnostics rather than guessing.

## D7. Trust-policy amendments

Amend the same implementation commit's normative policy:

- Name pi's session/PID/mode claims under existing cooperative environment provenance and its supported lifecycle paths.
- Define the context-plus-current-branch receipt predicate under `cooperative_mod_delivery`, including no ACK for returned drafts or queue acceptance and no proof of disk/provider/model consumption.
- State channel-only wake and no-channel behavior, including absence of native fallback.
- State excluded nonterminal/unknown-child modes, custom shell attribution limitations, stale callback fencing and downstream context-transform loss.

No seat merge, heuristic seat move, new receipt provenance constant, adversarial caller verification or peer wire change is proposed.

## D8. Implementation boundaries and qualification

After user design review and opencode integration, dispatch Sol tasks with exclusive paths and a frozen extension/Rust event contract. Rust identity/lifecycle/delivery declaration and extension implementation can proceed independently once that contract is agreed. Setup/launch follows the Rust skeleton; public registration and shared tests have one owner. Qualification and documentation follow integrated code, not isolated mocks alone.

Required evidence includes malformed/inherited identity refusal; mode and child exclusion; resume/fork/reload/compaction; passive idle silence; threshold-driven wake; busy delivery and stale-busy race; exact receipt evidence; truncation/replay/late callbacks; setup ownership; launch grammar; channel loss/reconnect; draft/modal behavior; and owned process/temp cleanup. Use focused Rust and JS tests plus real-binary fake-model probes. A live OpenRouter/DeepSeek wake matrix requires separate coordinator permission to use the existing credential.

Run the repository's clippy, neutral-core, wire-compat, formatting and default-feature checks through the required heavy/toolbox wrappers. Final review-ready handoff includes an independent super-roast, an unfiltered full suite and scoped leak checks on the integrated tree, and live e2e evidence or an explicit credential-related qualification gap. The main coordinator owns merge; this branch never pushes.

## Open decisions before implementation

The model-free probes confirm boundary commit/context order, same-run continuation and the need to guard non-completed outcomes. The implementer must qualify the bounded replay path, lifecycle and TUI draft/modal behavior before support is declared complete. Review the proposed TUI-only initial scope and context-observation accepted limit before implementation. A live credentialed wake run remains separately authorized.
