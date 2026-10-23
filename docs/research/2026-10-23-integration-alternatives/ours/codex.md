# herdr-threads x OpenAI Codex CLI: message injection and wake

What herdr-threads does today for Codex, read from the code on `main` at `2de3da1c4` (2026-10-23). The sections match the alternative reports. Harness facts come from the Codex primer (`../primers/codex.md`, `rust-v0.163.0`).

Tags: **VERIFIED** means read in this repo's source, tests or docs (cited as `path:line`). **INFERRED** means reasoned from the code and not observed running. **unknown** means not found.

---

## 1. Support status

- **Supported, as a hook-pull-free "standard hook" harness with native idle wake.** The adapter does not override `delivery_policy()`, so it gets the default `None`: "hooks deliver and the native wake applies (delivery-policy DP1)" (`src/harness/adapter.rs:228-231`). VERIFIED.
  - Pending items reach the model through hook `additionalContext`, including message bodies (§2).
  - An idle session is woken by Herdr `agent.prompt` typing a short prompt (§3).
- **Admission does not probe the version.** `hook_admission_policy()` is `RegisteredContract` (`src/harness/codex.rs:1451-1453`). `admit()` returns `ContractDeclared` whenever the executable is on `PATH`, or when hook selection was not requested (`src/harness/codex.rs:1594-1601`). Each hook payload is then classified against the declared `CONTRACT` shape (`src/harness/codex.rs:52-133`, `classify` at `:1609-1618`). VERIFIED. This matches the AGENTS.md "Harness and Herdr versions" rule.
- **Documentation drift: `integrations/codex/README.md` describes an older, stricter regime.**
  - It says unlisted versions are refused (`README.md:25-37`) and that health reports `harness.codex = unsupported` (`README.md:3-6`).
  - `Declaration::health_capability` does still return `Unsupported`, because native invocation transport and model receipt are unproven (`src/harness/codex.rs:321-333`).
  - The daemon's own observation is `ContractDeclared`, with "rich optional capabilities unavailable" (`src/harness/codex.rs:1430-1449`).
  - The README lists five hook groups and leaves out `UserPromptSubmit`, which `DECLARATION` installs (`src/harness/codex.rs:272-309`).
  - The module doc comment (`src/harness/codex.rs:1-12`) is stale in the same way.
  - VERIFIED: the drift itself. INFERRED: that the README is stale rather than the code being wrong.
- **What "unsupported" covers.** It means the **`updatedInput` command rewrite** (the scoped `HERDR_HOOK_INVOCATION` export, `src/harness/codex.rs:924-937,1149-1164`) and the **model-issued receipt**. Both are `NativeSupport::Unsupported` in the only recipe (`src/harness/codex.rs:202-209`). Delivery and wake do not depend on either. VERIFIED.

## 2. Injection mechanism

### 2.1 Surfaces used

Six hook groups go into `$CODEX_HOME/hooks.json` (`DECLARATION.owned_hooks`, `src/harness/codex.rs:272-309`). Each one runs `herdr-threads … hook-event codex --event <Event>`, with the state dir and Herdr endpoint pinned (`src/harness/setup.rs:109-115`, `integrations/codex/README.md:224-227`). Timeouts are 10 s, and 3 s for `SessionEnd` (`src/harness/codex.rs:1204-1224`). VERIFIED.

| Event (matcher) | Purpose | Model-visible output |
|---|---|---|
| `SessionStart` (none; `source` startup/resume/clear/compact/fork) | Lifecycle enroll plus durable Lifecycle CheckIn | instruction, ready commands, overview, recovery rows, inbox page |
| `SubagentStart` | child context | `CHILD_RESTRICTION` only |
| `PreToolUse` (`^Bash$`) | tool-boundary attention | digest plus inbox page, only when the digest token advanced |
| `UserPromptSubmit` | turn-start attention (ht-mu3). It is decoded as a tool-boundary `Current` check-in (`src/harness/codex.rs:1185-1186`) | same as PreToolUse |
| `SubagentStop`, `SessionEnd` | disconnect the named binding (ht-54r.11) | none |

Not used: `PostToolUse`, `Stop` (block and continue), async hooks, MCP, `codex queue` / `thread/queue/*`, `turn/steer`, `thread/inject_items`, `notify`, `thread/status/changed`. VERIFIED by their absence from `DECLARATION` and the adapter (primer §10 agrees).

### 2.2 Code path for one hook invocation (VERIFIED)

1. **Entrypoint.** `run_process_with` reads at most 64 KiB of stdin (`src/cli/hook.rs:72`). The hook needs `HERDR_ENV` and `HERDR_PANE_ID`, otherwise it treats the session as foreign (`src/cli/hook.rs:4207-4215`).
2. **Decode.** `CodexAdapter::decode` checks that the registered event equals the payload's `hook_event_name`, then runs `parse_shape` (`src/harness/codex.rs:1627-1651,1166-1205`). A `UserPromptSubmit` keeps `native_event = "UserPromptSubmit"`, so the output's `hookEventName` matches the event that ran (`:1640-1645`). A `PreToolUse` for any tool other than `Bash` is `Invalid` and produces no output (`:1189-1192`).
3. **Check-in, tool-boundary class.** `bridge::tool_boundary_check_in` (`src/harness/bridge.rs:1248-1395`):
   - It reads the seat's attention digest (`AttentionDigestDelivery`) and compares its token with the execution's stored mark (`:1271-1356`).
   - If the token has not advanced, it returns empty text, so the hook prints nothing.
   - Otherwise it makes one non-durable `CheckIn { mode: Current }` (`:1357-1365`) and renders `render_context(...)` plus the check-in offer through `encode_hook` (`:1375-1386`).
4. **Inbox page.** The page is read only when `hook_inbox_page(policy, live)` holds and the boundary text is not empty (`src/cli/hook.rs:2609-2626`). For Codex the policy is `None` and no mod channel exists, so the page is always due (`src/delivery/routing.rs:35-41`). `hook_inbox::read_page` issues an `InboxBatchV2` read with these limits:
   - `PAGE_ROWS = 8`, `PAGE_READ_BYTES = 16 384` (`src/cli/hook_inbox.rs:27-29,91-137`).
   - A 500 ms budget of its own (`src/cli/hook.rs:2897`).
5. **Compose.** `compose_context_limited` builds the fixed section and the escaped peer-data section, budgeted to `MAX_CONTEXT = 4096` bytes (`src/cli/hook.rs:74,666-812`). `hook_inbox::render` then appends **whole items only**, in page order, while two limits hold (`src/cli/hook_inbox.rs:224-309`, called at `src/cli/hook.rs:4052-4054`):
   - the total stays within 4096 bytes;
   - the JSON-escaped form stays within 7 168 bytes.
6. **Encode.** `adapter::encode_context` emits `{"hookSpecificOutput":{"hookEventName":…, "additionalContext":…}}` (`src/harness/adapter.rs:2063-2082`). It never emits `permissionDecision` or `updatedInput`. The Codex-specific `encode_native_context` adds a 4 608-byte envelope cap (`src/harness/codex.rs:1135-1146`).
7. **Emit, then complete.** `emit` writes and flushes stdout (`src/cli/hook.rs:4255-4266`). Only after a successful flush does `LazyCommit::submit` run, with a 300 ms cap (`src/cli/hook_inbox.rs:31,357-406`):
   - `CompleteInboxDelivery { via: HookContext }` for the lazy rows and ACK-eligible receipts shown;
   - `AdvanceCursors { in_turn }` for the rows displayed.

### 2.3 Timing (VERIFIED unless marked)

- **Turn start.** The `UserPromptSubmit` hook fires on every submitted prompt, whether typed by the human or typed as a wake. Its context lands before the prompt (primer §1.4).
- **Mid-turn.** The `PreToolUse` hook fires before each **Bash** call. Codex inserts the context after the call and before its output (`docs/evidence/codex-158-live-hook-capture/report.md`, cited at `integrations/codex/README.md:58-63`). Mail that arrives during a turn reaches the model at its next Bash call.
- **Not covered mid-turn.**
  - INFERRED, from the matcher and the `parse_shape` reject: turns that only call `apply_patch`, MCP tools, web search, `sleep` and similar tools get no mid-turn delivery.
  - A turn's tail after its last Bash call gets none either.
  - There is no `Stop` hook, so mail that arrives after the last tool call waits until the session is idle. It is then woken only if its priority is `<= T` (§3); otherwise it waits for the next human turn.
- **Session start, resume, clear and compact.** `SessionStart` runs the lifecycle check-in. Resume, clear and compact add hot-thread recovery rows (`src/cli/hook.rs:2884-2911`).

### 2.4 Payload and framing (VERIFIED)

The `additionalContext` of a delivering check-in is plain text with these parts, in order:

1. **`TOP_LEVEL_INSTRUCTION`** (600 B, `src/harness/mod.rs:178`): "Complete hook/channel deliveries … ACK eligible receipts automatically … When waiting for replies, finish your turn; hooks notify you. Do not poll …".
2. **`CHILD_RESTRICTION`** (414 B, `src/harness/mod.rs:184`).
3. **`Hook command routing (JSON data): …`**, which is `null` when the budget is tight (39 B minimum, `src/cli/hook.rs:688-724`).
4. **`CODEX_COMMAND_GUIDANCE`** (490 B, `src/harness/codex.rs:454`), carried as `OutputPolicy.extra_guidance` (`:1619-1626`).
5. **The ready-commands block** (`actions.render`, top level only).
6. **`PEER_DATA_NOTICE`** (114 B), then `peer_data: "<JSON-escaped digest summary and offer>"` (`src/cli/hook.rs:777-786,815-818`).
7. **`INBOX_HEADER`** (85 B), then `inbox_peer_data: "<JSON-escaped compact inbox rows>"` (`src/cli/hook_inbox.rs:38-39,268-272`). The rows are the ordinary text-inbox rows, for example `message m1 t#1 from peer [p2] bytes` followed by an indented body (`tests/cli/hook_inbox.rs:104-107`).
8. **A continuation line when coverage is incomplete**: "More inbox content may remain: run herdr-threads inbox; follow its next: commands." (`src/cli/hook_inbox.rs:60-67`).

Sizes were measured from the constants. Peer text only ever appears JSON-escaped inside the two containers (`src/cli/hook.rs:566-572`). **Codex does not use the R17 compact frame** (`[herdr-threads] <kind> <id> …`, `src/delivery/frame.rs`). That frame is used only by the watch/channel and `DeliveryPull` paths (`src/protocol/delivery.rs`). VERIFIED: the only `delivery::frame` importer is `src/protocol/delivery.rs`.

### 2.5 Bounds

All VERIFIED unless marked.

| Bound | Value | Where |
|---|---|---|
| stdin | 64 KiB | `src/cli/hook.rs:72` |
| `additionalContext` | 4 096 B (encoder rejects above) | `src/cli/hook.rs:74`, `src/harness/adapter.rs:2068` |
| Codex envelope | 4 608 B | `src/harness/codex.rs:1143` |
| Escaped inbox block | 7 168 B | `src/cli/hook_inbox.rs:34` |
| Rows per hook | 8; 16 KiB read | `src/cli/hook_inbox.rs:27-29` |
| Hook budget | 1.5 s tool and turn; 5 s lifecycle | `src/cli/hook.rs:68-70` |
| Inbox read | 500 ms | `src/cli/hook.rs:2897` |
| Completion | 300 ms | `src/cli/hook_inbox.rs:31` |

- **Codex's own limit.** Hook output above about 2 500 tokens is spilled to disk (primer §1.4). The 4 KiB cap stays well under it, so spill never happens. INFERRED.
- **What fits in practice.** The fixed section alone is about 1.75 KB (items 1 to 4 plus the two notices, before ready commands). Roughly 2.0 to 2.3 KB is left for bodies, and JSON escaping inflates quotes, backslashes and newlines.
  - A single body larger than about 2 KB never fits in a hook. It is not partially shown: `whole()` requires the item complete (`src/cli/hook_inbox.rs:140-158`).
  - Such a body yields `Incomplete` coverage and the continuation line, so the model must run `herdr-threads inbox`, which costs a tool call.
  - INFERRED: the arithmetic comes from the measured constants. The exact ready-command size per offer was not measured.

### 2.6 De-duplication and coalescing (VERIFIED)

- **Digest-token coalescing per execution mark.** An unchanged token means no output and no check-in (`src/harness/bridge.rs:1338-1356`). The mark only grows (`stored.join`).
- **Per-binding read cursors.** `AdvanceCursors` over the rows displayed means a row shown whole is not shown again to that binding (`src/cli/hook_inbox.rs:388-405`).
- **The attention mark is handed back for commit only when the encoder consumed the context and coverage was not `Unavailable`** (`src/cli/hook.rs:4116-4123`). INFERRED: it is committed after `emit`, as the lazy completion is. The mark-commit call site was not traced.
- **Re-offer bounds** (60 s spacing, 3 per generation) live in `src/delivery/ledger.rs` (AGENTS.md "What we do today"). Not traced for this hook path: unknown whether the ledger applies here or only to `DeliveryPull`.

### 2.7 Acknowledgement (VERIFIED)

- **Display is the ACK.** Message rows shown whole complete through the daemon's canonical writer after flush (`CompleteInboxDelivery`, `via: HookContext`), with no model action.
- **Rows with an `ack_candidate`** (ACK requests) are only shown if the daemon advertises `HOOK_CONTEXT_RECEIPTS_V1`. Otherwise they stop the shown prefix and stay for the text inbox (`src/cli/hook_inbox.rs:230-243`).
- **Invitations** are presented, but accepting one is a separate model action. Invitations are recorded as offered in-turn only when `in_turn` (`src/cli/hook_inbox.rs:383-387`).
- **Flaw: "flushed to stdout" is treated as "seen by the model".** INFERRED. The hook cannot observe Codex rejecting or dropping its output: for example an invalid `hookEventName`, or a Codex that ignores `additionalContext` for an event. A receipt would then be ACKed without delivery. The codebase records this as cooperative provenance (`cooperative_hook_context`, `src/delivery/routing.rs:32-34`), not native-verified receipt. That is honest, and it is also why `model_receipt` stays `Unsupported`.

### 2.8 Subagents (VERIFIED)

- **`SubagentStart`** gets `CHILD_RESTRICTION` only.
- **A subagent's routine Bash call** gets a read-only notice from its own binding's digest mark (`subagent_notice`, `src/cli/hook.rs:2709-2776`). It never checks in, accepts or ACKs.
- **Subagent detection.** `CODEX_THREAD_ID` that differs from the session id marks a subagent for CLI callers (`src/harness/codex.rs:1378-1390`). Hook payloads carry `agent_id` and `agent_type` (`CONTRACT`).

## 3. Wake mechanism

### 3.1 Who decides (VERIFIED)

1. **Eligibility.**
   - The store computes a seat's `wake_frontier`: the newest published delivery row with priority `p <= T` (every priority when `T < 0`), unread by some connected top-level binding (`src/store/lazy_delivery/wake.rs:59-75`).
   - Pending invitations, ACK-required receipts and actionable warnings also select a wake (`src/notification/policy.rs:66-88`). These are always p0 (AGENTS.md rule 5).
   - A disconnected binding holds no wake.
2. **Scheduling.**
   - Publication upserts `wake_work` for each local recipient seat (`src/store/lazy_delivery/staging.rs:19`). A commit to `wake_work` kicks the wake lane (`WAKE_TABLES`, `src/service/kicks.rs:133-145,290`). `lazy_recipients` itself kicks no lane (`:241-242`).
   - `wake_batch_delay_ms` defaults to 0, i.e. no batching (`src/daemon/settings.rs:54`).
   - The minimum spacing between wake reservations is 30 s (`src/daemon/settings.rs:53`). The retry ladder is 30 s, 60 s, 120 s, then 300 s, capped at the last step (`src/notification/policy.rs:49,231-258`).
   - INFERRED: a wake that is submitted but not acted on is repeated at those intervals while the item is still unread.
3. **Dispatch.** `NativeWakeDispatcher::attempt` (`src/notification/dispatch.rs:153-547`) runs these checks in order:
   1. host boot and epoch fence;
   2. fresh target observation;
   3. cooperative structural identity;
   4. refusal on `HumanInput` or `ActiveTurn`;
   5. the composer-process choice for multi-binding panes (ht-327: one process, one composer);
   6. `safe_wake_target`;
   7. `is_current` on the reservation;
   8. `owes_wake`: no wake for a binding already served in its own turn (ht-705, ht-yha.62);
   9. wake-text selection;
   10. `submit_prompt`.

### 3.2 What is typed (VERIFIED)

- **Text selection.** `CodexAdapter::binding_wake_text` returns:
  - `WAKE_UP` = `herdr-threads: wake up` (22 B, content-free, R18) when the reached binding's provenance is `cooperative_top_level`, meaning it was registered through the hooks;
  - otherwise `MARKER` = `herdr-threads: attention pending; run herdr-threads inbox` (`src/harness/codex.rs:1393-1399`, `src/notification/policy.rs:14,17`).
- **Fallbacks to `MARKER`.**
  - The dispatcher falls back to the harness default (`registry.wake_text("codex")`), which is `MARKER` because Codex has no delivery policy, when the text evidence is not current, when sibling bindings disagree, or for a recovery hint (`src/notification/dispatch.rs:444-506`).
  - It also uses `MARKER` when ids are parked for `inbox` (`holds_parked`), which does not apply to Codex.
- **Why the content-free prompt works.** The typed prompt triggers `UserPromptSubmit`, whose context carries the pending rows (§2). The wake carries no content.
- **Transport.** It is Herdr's `agent prompt <pane> <text>` RPC (`src/host/native.rs:1897-1900`), not `send-text` plus Enter. The poke spike found that on Codex 0.160.0 an immediate `send-text` followed by `enter` is swallowed, and recommended `agent prompt` (`docs/evidence/poke-spike/findings.md:45,68`). How Herdr types internally (bracketed paste, delay before Enter) is unknown. It is Herdr's code, not ours. The primer's 120 ms paste-burst hazard (§7) is therefore Herdr's to handle.

### 3.3 Idle and busy detection (VERIFIED)

- **Herdr `agent_status`** must be `idle` or `done` on a fresh witnessed `agent get` taken immediately before the prompt. `working`, `blocked` and `unknown` are refused (`src/host/native.rs:53,1682-1740`).
  - Herdr derives this status from screen inference: Codex's OSC spinner or live timer means working, and a working-to-idle transition is debounced over three confirmations, up to 700 ms (`docs/evidence/nudge-input-safety/active-turn.md`, reading Herdr 0.9.3 `src/detect/manifests/codex.toml`).
  - It is not a native busy/idle signal.
  - The agent kind must match the bound harness (`codex`), in the same pane and terminal, and in the same server incarnation and epoch (`src/host/native.rs:1692-1723,1827-1845`).
- **Approval or question UI.** `safe_wake_target` refuses `ActiveTurn` and `ApprovalOrQuestion` (`src/host/native.rs:1354-1366`).
- **No native signal is consumed.** There is no `notify`, no app-server `thread/status/changed` and no `turn/completed` (primer §2.3). INFERRED: idle detection is only as good as Herdr's screen manifest for the installed Codex TUI.

### 3.4 Draft safety (VERIFIED)

- **Composer read.** Before submitting, the dispatcher runs `agent read <pane> --source detection` and parses it with `codex/composer.rs`:
  - It finds the last `› ` row plus the following two-space continuation rows (`src/harness/codex/composer.rs:8-33`).
  - The composer is `Empty` only when it holds exactly one row equal to the placeholder `Ask Codex to do anything` (`:44`).
  - Anything else is `Text`, `Unsafe` or `Unreadable`, and the wake is refused: "composer is nonempty or unreadable" (`src/host/native.rs:1876-1878`).
- **No stash.** `composer_stash` is `Unsupported` in the recipe (`src/harness/codex.rs:152-154,208`). A draft is never cleared and retyped; the wake is refused and retried on the refusal backoff (`src/notification/dispatch.rs:516-519`).
- **Focused pane.** A focused pane must be observed with an empty composer for **60 s** before a wake (`FOCUSED_EMPTY_MILLIS`, `src/host/native.rs:71,1744-1782,1879-1884`). Unfocused panes have no such wait.
- **After submission.** A read-only verification checks whether the wake text is still held in the composer (`WAKE_TEXTS`). If so, it reports `Unsubmitted` and **never presses Enter**, because the composer might now hold the human's input as well (`src/notification/dispatch.rs:97-131`).
- **Residual risk.** INFERRED: a keystroke the human types between the composer read and `agent prompt` (a window of about 750 ms to 2 s) could merge with the wake text. The poke spike observed that composer text and a poke merge with no separator (`docs/evidence/poke-spike/findings.md:50`). The 60 s focused-empty window reduces this risk but cannot remove it.

### 3.5 Latency

- **Unfocused idle pane.** Seconds: a kick on publication, then the RPC chain, with `PROMPT_RECHECK_MILLIS = 750` and `PROMPT_SUBMIT_MILLIS = 2000` (`src/host/native.rs:55-57`). The release e2e asserts "wakes within seconds" (AGENTS.md "Pre-release wake e2e"). VERIFIED as a gate; per-wake numbers are not recorded here.
- **Focused idle pane.** At least 60 s, plus the refusal backoff granularity. VERIFIED: the constant. INFERRED: the total.
- **Repeat wakes.** 30 s minimum, rising to 300 s (§3.1).

### 3.6 Failure modes

- **Untrusted or changed hooks.** INFERRED. Codex runs a hook only after the user trusts its hash (`integrations/codex/README.md:79-89`).
  - If the hooks are untrusted, a session never registers through hooks. Its bindings lack `cooperative_top_level` provenance, so it gets the `MARKER` wake, which names `inbox`. That degrades gracefully.
  - If a hook command changes after registration (re-setup, upgrade) and is not re-trusted, the binding keeps its provenance. The session would then get the content-free `wake up` with no `UserPromptSubmit` delivery behind it. The model sees a bare `herdr-threads: wake up`.
  - `wake_text_evidence_current` may catch this case; how precisely was not traced (unknown).
- **Shared app-server daemon.** Since about 0.157, Codex's TUI attaches to a machine-wide daemon that runs hooks with the environment of whichever pane started it, so `HERDR_PANE_ID` belongs to another pane.
  - Mitigations: an opt-in, owned shell-startup block that exports `CODEX_EXEC_SERVER_URL=` inside Herdr panes (`src/harness/codex/pane_env.rs:1-31`), or `--no-daemon`.
  - CLI callers under `codex app-server --managed-daemon` are refused (`src/harness/codex.rs:1363-1376`). Whether the hook path detects a wrong pane is unknown; the mitigation is advisory, not enforced.
- **Composer grammar drift.** INFERRED. If the placeholder text or prompt glyph changes in a new Codex release, every read becomes `Text` or `Unreadable`. Wakes then stop: this fails safe, but silently, apart from the refusal diagnostics.
- **Interrupted turn.** INFERRED. After Ctrl-C, Herdr's status should return to idle, and the wake proceeds normally. This differs from `codex queue`, which does not dispatch after an interrupt (primer §3).
- **`/side` (ephemeral) sessions.** They get their own binding through `SessionStart source=fork`. One process holding several bindings is woken through the process's single composer (ht-327; `src/notification/dispatch.rs:326-374`). This is not a correctness gap; the release e2e has opt-in cases for it.

## 4. Setup footprint

All VERIFIED. Source: `src/harness/codex.rs:1259-1299`, `integrations/codex/README.md`.

- **`$CODEX_HOME/hooks.json`** (default `~/.codex/hooks.json`): six appended groups, recorded in a private ownership manifest, with timestamped 0600 backups before any change. Setup never writes Codex trust; the user reviews the hooks at the next interactive start or with `/hooks`, and Codex records `[hooks.state."<key>"] trusted_hash` in `config.toml`.
- **Optional, with consent: `$CODEX_HOME/rules/herdr-threads.rules`.**
  - An execpolicy `prefix_rule` allowing `herdr-threads` and `ht`.
  - `prompt` rules for `human`, `setup`, `unsetup`, `doctor fix` and `internal installer-integrations`.
- **Optional, with consent: the pane-env block** (`CODEX_EXEC_SERVER_URL=`, guarded by `HERDR_ENV=1`) in `.zshenv`, `.bashrc` plus the login file, and the fish `conf.d` drop-in.
- **Skill file** at `<config root>/skills/herdr-threads/SKILL.md` (`src/harness/codex.rs:2435-2443`).
- **Legacy sandbox network allowance.** Still managed for measured 0.159.2 and 0.159.3 only (`SANDBOX_MEASURED_VERSIONS`, `src/harness/codex.rs:473`); historical.
- **Per-call permission.** Every model-issued `herdr-threads` call needs outside-sandbox execution through a rule or an approval (`CODEX_COMMAND_GUIDANCE`). This is a recurring friction point, absent on Claude.

## 5. Token cost

All INFERRED (bytes / 4 ≈ tokens) from VERIFIED constants.

- **Quiet turn or quiet tool call: 0 model tokens.** The hook still runs on every turn start and every Bash call: a process spawn, a daemon socket round trip and one digest read, within the 1.5 s budget, on the tool's critical path. An unchanged token means empty stdout (`src/harness/bridge.rs:1346-1354`).
- **Fixed overhead per delivering check-in: about 1.75 KB, roughly 440 tokens, before any message.** Items 1 to 4 plus the two notices (§2.4), plus the ready commands and the digest line. It is **repeated at every boundary where the digest advanced**, not once per session.
  - This conflicts with AGENTS.md "Token-efficient communication" (a): "Static explanations live once per session in the skill, never repeated per message or per turn".
  - It also conflicts with (b): "Per-message framing is the minimum structure needed".
  - By comparison, the R17 frame used by the Claude mod, agy `DeliveryPull` and watch is one header line plus a body.
- **Per message:** the compact inbox row plus body, JSON-escaped, which costs about 5 to 15 % extra for ordinary prose and more for code or quoted text, plus the 85 B `INBOX_HEADER`.
- **Large bodies** (over about 2 KB) cost one extra `herdr-threads inbox` tool call, plus that output (with 8 KiB truncation, `src/protocol/watch.rs`).
- **Wake:** 22 B typed (`herdr-threads: wake up`), plus one full model turn started by the prompt, plus the `UserPromptSubmit` delivery above. With the `MARKER` fallback, the model also spends a tool call on `herdr-threads inbox`.
- **Permission prompts.** Without the rules file, each model `herdr-threads` call can cost an approval round trip (human time, not tokens).

## 6. Known limitations

1. **Bash-only mid-turn coverage** (`^Bash$`; `parse_shape` rejects other tools, `src/harness/codex.rs:1189-1192`). VERIFIED.
2. **No end-of-turn delivery.** There is no `Stop` hook, so p1 mail that arrives after the last Bash call waits for the next human turn at T = 0. That is by contract (rule 2), but a `Stop` block could deliver p0 mail before idle instead of waking afterwards. VERIFIED: absence. INFERRED: the improvement.
3. **Repeated static instruction** on every delivering check-in (§5). VERIFIED.
4. **4 KiB context cap with about 1.75 KB fixed.** That leaves about 2 KB of body room, while Codex allows about 10 KB (2 500 tokens) before spilling. INFERRED: the room arithmetic.
5. **Screen-scraped idle and composer state**; no native idle edge is consumed. VERIFIED.
6. **No draft stash on Codex** (`composer_stash: Unsupported`). A draft blocks wakes indefinitely, by design (rule: never clobber). VERIFIED.
7. **Focused panes wait 60 s** before a wake. VERIFIED.
8. **ACK on flush, not on model-side acceptance** (§2.7). VERIFIED design; it is a cooperative provenance.
9. **Shared-daemon pane-identity leak**, mitigated only by an opt-in environment block or a flag (`pane_env.rs`; Herdr PR #4667 per the primer). VERIFIED.
10. **Hook trust by hash.** Every hook command change needs re-review (`RETRUST_WARNING`, `src/harness/codex.rs:1227-1229`). VERIFIED.
11. **Documentation drift** in `integrations/codex/README.md` and the `codex.rs` module doc (§1). VERIFIED.
12. **Beads cited in code:** ht-mu3 (UserPromptSubmit turn start), ht-54r.9 and ht-54r.11 (subagent notices and end events), ht-327 (multi-binding `/side`), ht-yha.62 and ht-705 (wake owed after in-turn display), ht-p03.20 (native matrix). The `bd` CLI panicked on search in this checkout (a dolt "nil parent" context error), so open beads were not listed (unknown).

## 7. Improvements: which primer surfaces fit our adapter traits

Codex is already supported, so this section lists the primer surfaces that would close the gaps above and the trait work each would need.

| Gap | Primer surface | Trait or code work | Fit |
|---|---|---|---|
| Bash-only mid-turn (6.1) | `PostToolUse` with matcher `*` (context after any tool), or keep `PreToolUse` and widen the matcher | `DECLARATION.owned_hooks` entry; `parse_shape` accepts the event and other tool names; `CONTRACT` gains `PostToolUse`. No neutral trait change | high, small change. Re-trust needed |
| Bash-only, latency on the tool path | **Async** `PostToolUse` hook (`async: true`, drained after each sampling step, primer §1.6) | same, plus the setup writes `async`. The 1.5 s check-in moves off the tool's critical path. ACK-on-flush still holds. Cost: async results are held to a safe boundary and cannot wake an idle session | medium. Needs a live capture |
| End-of-turn p0 (6.2) | `Stop` hook `decision:"block"` + `reason` (primer §1.5) | a new neutral `DeliveryPolicy` hook, such as "end-of-turn continuation", with a neutral default of none; a `CONTRACT` entry; loop bound via `stop_hook_active`. Must honour rule 4 and must not start turns for p > T | medium. Removes some nudges; spends one sampling round |
| Repeated instruction (6.3) | none needed: AGENTS.md (a) already says to put it in the skill or `SessionStart` once | neutral core: emit `TOP_LEVEL_INSTRUCTION`, `CHILD_RESTRICTION` and `CODEX_COMMAND_GUIDANCE` only on lifecycle check-ins, and move tool-boundary delivery to the R17 frame (`src/delivery/frame.rs`) | high. Saves about 440 tokens per delivery |
| Screen-scraped idle (6.5) | `notify` `agent-turn-complete` (fire-and-forget idle edge); app-server `thread/status/changed` | a `HostPort` or adapter idle-signal input. The app-server needs the shared daemon, which conflicts with the `--no-daemon` advice | low to medium |
| Draft-safe wake without typing (6.6, 6.7) | `codex queue --thread ID --message "herdr-threads: wake up"` / `thread/queue/add` (server-side idle check, never touches the composer; primer §3) | `IdleWake` gains a third variant (for example `HarnessQueue`) beside `Native` and `Channel` (`src/harness/adapter.rs:454-458`), plus a `Registry::new` pairing. The adapter maps a binding to the Codex thread id from the hook `session_id` (fork caveat, primer §9). It needs the shared daemon or a working cross-process SQLite watcher (contested for `--no-daemon`, primer §3) | promising. The first experiment: does `codex queue` reach a `--no-daemon` TUI within about 20 s? Not dispatched after an interrupt; ephemeral `/side` refused |
| Busy-session mid-turn without a tool call | `turn/steer`, plus the model's `sleep` tool returning early on input | an app-server client in the daemon; same daemon reachability problem | low today |
| Bigger bodies in-hook (6.4) | Codex allows about 2 500 tokens before spill (`output_spill.rs`) | raise `MAX_CONTEXT` per adapter (an `OutputPolicy` field) instead of the shared 4 096 constant | medium. Keep it under the spill threshold |

**Not worth pursuing.** MCP push: Codex logs `resources/updated` and `list_changed` notifications and drops them (primer §4). `thread/inject_items` (silent history) needs the app-server and has no clear advantage over `UserPromptSubmit` context for p1 "next turn" semantics.

---

## Verification

After drafting, every claim was re-read against the code:

- The hook-inbox page path was confirmed for Codex (`routing::hook_inbox_page(None, false) == true`). A first reading of the primer suggested a digest-only hook.
- The fixed-section byte counts were measured from the source constants.
- `UserPromptSubmit` is in `DECLARATION` and in `encode_event_context`.
- Wake-text selection was confirmed in `dispatch.rs` and `codex.rs:1393`.
- The 60 s focused-empty rule was confirmed at `native.rs:71`.
- The admission path is `RegisteredContract` with no version probe (`codex.rs:1451,1594`).

Not verified here: Herdr's `agent.prompt` typing internals; whether `wake_text_evidence_current` catches the re-trust case; whether the re-offer ledger applies to the standard hook path.
