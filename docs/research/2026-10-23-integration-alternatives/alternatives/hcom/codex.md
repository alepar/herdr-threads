# hcom x OpenAI Codex CLI: injection and wake

- Upstream: https://github.com/aannoo/hcom. MIT licence, 597 stars and 94 forks, last push 2026-10-21, latest release v0.7.31 (2026-10-21), previous v0.7.30 (2026-10-15). All from the GitHub API on 2026-10-23.
- Clone: `alts/hcom` in the scratchpad, HEAD `de33e31a0cdb` (2026-10-21). I read it and executed nothing from it. Citations are `path:line` relative to the clone root.
- Harness reference: `../../primers/codex.md` (Codex 0.163.0). hcom's real-tool tests pin `@openai/codex@0.162.1` (`scripts/mock-tools.pins:6`).
- Tags: **V** = VERIFIED (read in source or docs), **I** = INFERRED, "unknown" = not found.

## 1. Support status

**Native, first-class.** The README lists "Codex CLI | automatic | `hcom codex`" (README.md:180) **V**. The integration spec `CODEX` (src/integration_spec.rs:1457-1528) declares:

- JSON-on-stdin hooks;
- PTY wrapping by default (`uses_pty_default: true`);
- every PTY delivery gate on (`require_idle`, `require_ready_prompt`, `require_prompt_empty`, `block_on_user_activity`, `block_on_approval`);
- ready patterns `› ` and `» ` (the second is the Ultra-effort composer);

**V**.

There is a real-binary test suite: `tests/real_tool_codex.rs` drives the pinned Codex against a localhost mock of the Responses API. It covers the full lifecycle and an approval gate that holds a message (`tests/real_tool_codex.rs:1-17, 32-60`) **V**. The suite is `#[ignore]` and runs explicitly through `just ci` (justfile:174) **V**.

Architecture in one line: `hcom codex` launches Codex inside hcom's own PTY proxy (`hcom pty`). Per-run `-c hooks.*` overrides install 6 hooks. The hooks deliver message bodies **inside a turn**, and the PTY proxy **types `<hcom>` + Enter** to wake an idle session **V** (details below).

## 2. Injection mechanism

### 2.1 Hooks declared (V)

`src/hooks/codex.rs:47-62` (`CODEX_HOOK_COMMANDS`):

| Codex event | hcom command | Matcher | Role |
|---|---|---|---|
| `SessionStart` | `codex-sessionstart` | `startup\|resume\|clear\|fork` | bind the session to the process, set `listening`; **no injection** |
| `UserPromptSubmit` | `codex-userpromptsubmit` | none | delivers **only if the prompt is exactly `<hcom>`** |
| `PreToolUse` | `codex-pretooluse` | `Bash\|apply_patch\|spawn_agent` | status detail only; never injects |
| `PostToolUse` | `codex-posttooluse` | none (every tool) | **mid-turn delivery**; also clears an approval block |
| `Stop` | `codex-stop` | none | set `listening` and ping the PTY delivery loop |
| `Interrupt` | `codex-interrupt` | none | set `listening` (`interrupted`) and ping the loop |

The design notes sit in the comment at `src/hooks/codex.rs:38-46`:

- There is deliberately no `PermissionRequest` hook, because it "fires before Codex picks a reviewer, so under auto-review it marked rows blocked while nobody was asked".
- `PostToolUse` matches every tool "so an approved apply_patch/MCP call still clears a block … and delivers mid-turn".

**V**

### 2.2 Code path (V)

1. Codex spawns `hcom codex-posttooluse` (or `codex-userpromptsubmit`) with the hook JSON on stdin. `dispatch_codex_hook_native` (src/hooks/codex.rs:1034-1083) parses stdin and opens the SQLite DB. It then applies `hook_gate_check`: the hook is inert unless `HCOM_PROCESS_ID` is set, i.e. unless hcom launched this process (src/hooks/common.rs:121-127).
2. Instance resolution goes through the process→session binding (`resolve_instance_codex`, src/hooks/codex.rs:698-704). Codex-internal threads are filtered out first: subagents (`agent_id` present) and memory consolidation (no `transcript_path`, and either a cwd named `memories`/`memories_v2` or a session id that differs from the bound one). See `is_internal_thread`, src/hooks/codex.rs:771-794.
3. `handle_posttooluse` (src/hooks/codex.rs:918-938) calls `prepare_codex_delivery`. `handle_userpromptsubmit` (:858-881) does the same, but only when `prompt.trim() == "<hcom>"`. For any other prompt it marks the row `active` and returns a no-op.
4. `prepare_codex_delivery` (:756-762) calls `common::prepare_pending_messages`. That reads unread rows, caps them at 50 and formats them **without** committing the cursor (src/hooks/common.rs:304-311, 348-396). The result sets the same text into both fields:

   ```rust
   HookResult::Allow {
       system_message: Some(prepared.formatted.clone()),   // TUI echo for the human
       additional_context: Some(prepared.formatted),       // model context
       delivery_ack: Some(prepared.ack),
   }
   ```

5. `dispatch_result_to_stdout` (:981-1031) writes `{"systemMessage":…, "hookSpecificOutput":{"hookEventName":…, "additionalContext":…}}`. **Only after `stdout.flush()` succeeds** does it call `commit_delivery_ack` (:1007-1014). The commit advances the cursor forward-only, is fenced by the row's `created_at`, and sets status `active` / `deliver:<sender>` (src/hooks/common.rs:313-345).

### 2.3 Payload format (V)

`messages::format_messages_json` wraps the batch in `<hcom>…</hcom>` (src/messages.rs:655-668). The pieces are:

- A single message reads `[intent:thread #id] sender → recipient (+N others): text`.
- A multi-message batch reads `[N new messages] | msg | msg …`.
- Then `| [<hints>]` is appended to **every** delivery when per-instance or config hints are set.
- One-time tips are appended on the first receipt of a thread or intent.

See src/messages.rs:548-652. The body is raw text, with no escaping of a body that contains `</hcom>` (V: `format_messages_json` wraps the output of `format_hook_messages` unchanged, and `validate_message` rejects only control characters and oversize bodies, src/messages.rs:127-150, 655-671; the wake *preview* does strip the tags, src/delivery.rs:630-644. The effect on the model is I).

### 2.4 When it fires

| Moment | Fires? | |
|---|---|---|
| Mid-turn, after **any** tool (shell, `apply_patch`, MCP, spawn_agent) | yes, `PostToolUse` `additionalContext` | V |
| Turn start of a **human** prompt | **no**: it only flips status to `active` (src/hooks/codex.rs:870-873). Mail waits for a tool call or the post-turn wake | V |
| Turn start of the `<hcom>` wake prompt | yes, `UserPromptSubmit` `additionalContext` | V |
| Session start | no message delivery. The bootstrap is static `-c developer_instructions=<TOML string>` (src/tools/codex_preprocessing.rs:207-276; src/hooks/codex.rs:854) | V |
| End of turn (`Stop` block-and-continue) | **not used** for Codex: `handle_stop` returns `{}` (src/hooks/codex.rs:940-949, 989) | V |

### 2.5 Size bounds

- At most 50 messages per delivery (`MAX_MESSAGES_PER_DELIVERY`, src/shared/constants.rs:19; `limit_delivery_messages`, src/hooks/common.rs:261-267). Each message is capped at 1 MiB on send (src/shared/constants.rs:22; src/messages.rs:143) **V**. hcom has no per-delivery byte cap **V**.
- Codex's own hook-output spill applies: above about 2,500 tokens the output goes to disk and the model gets a head-and-tail preview (primer §1.4). hcom commits the ack on stdout flush, not on model receipt. So a big batch is marked delivered while the model sees only a preview (I).
- The same text is also emitted as `systemMessage`. That is UI-only, so it adds no model tokens but doubles the bytes on stdout (V for emission; primer §1.4 for "not model context").

### 2.6 Framing, dedupe, ack (V unless noted)

- **Ack** is a single forward-only per-instance cursor over the global `events` table. It is committed after a successful flush. A delayed ack never rewinds a newer one, and a batch read from a row that was since replaced by stop+resume is voided (src/hooks/common.rs:313-334).
- **Dedupe** relies on cursor monotonicity alone. Codex runs hooks sequentially within a turn, so PostToolUse and UserPromptSubmit cannot both deliver the same rows (I).
- **Crash window:** a hook that flushes and then dies before the commit redelivers the batch on the next hook (at-least-once) (I, from the commit order).
- **Tip side effect:** tips are marked "seen" during formatting, before the commit (src/messages.rs:625-650). A failed flush still burns the one-time tip (I).
- **Ordering:** by global event id. There is no priority.

## 3. Wake mechanism

### 3.1 Mechanism: PTY keystroke injection of `<hcom>` (V)

The PTY proxy's delivery thread (`run_delivery_loop`, src/delivery.rs:1580ff, generic branch :1850ff) does the following:

1. **Waits** on its localhost TCP notify port. `hcom send` connect-and-closes every registered port (overview §Architecture). The fallback poll is 30 s when idle (`IDLE_WAIT`, :1462) and 250 ms while blocked (`RETRY_DELAY`, :1327).
2. **Checks pending** (`db.has_pending`) and moves to `Pending` (:1975-1987).
3. **Evaluates the gate** (`evaluate_gate`, :1008-1087) and **re-checks pending** right before typing (:2032-2038).
4. **Types** `WAKE_TRIGGER = "<hcom>"` for Codex. The comment says "Claude/Codex hooks show the full delivery in the TUI, so they only need a trigger" (:2058-2072, constant at :561). `inject_text` connects to the PTY's local TCP inject server and writes the text with every C0 control character except tab stripped, which also blocks ESC (:1299-1313).
5. **Phase 1 (render):** polls the scraped composer every 10 ms for up to 10 s, until the composer text equals `<hcom>` **exactly** (`PromptOwnership::Exclusive`). A composer that contains `<hcom>` plus other text is `Mixed`, which means "refusing automatic submission" (:1330-1413, 2230-2342).
6. Right before Enter it re-checks ownership, approval and user activity from one fresh snapshot, then sends `\r` (:2263-2308; `inject_enter` :1316-1321; `pre_enter_settle_ms: 0` for Codex, integration_spec.rs:1470).
7. **Phase 2 (clear):** the composer must empty within 2 s. Otherwise Enter is retried up to 3 times with 400/800/1600 ms backoff, re-checking ownership each time (:1416, 1513, 2344-2431).
8. **Verify:** the hook-side cursor must advance within 10 s. If it does not but nothing is pending any more, that counts as success. For Codex a timeout retries the whole injection up to 3 times, then "resets" back to `Pending` (:1382-1398, 2433-2571).

The Codex `UserPromptSubmit` hook then sees the prompt `<hcom>` and returns the batch as `additionalContext` (§2.2). The wake prompt itself carries no content (V).

### 3.2 Idle vs busy detection (V)

Gate order, from src/delivery.rs:1008-1087 and the gate rationale at integration_spec.rs:1487-1499:

1. **`require_idle`**: the DB status must be `listening`. For Codex only hooks set it: `SessionStart`, `Stop` and `Interrupt` (src/hooks/codex.rs:843-849, 946, 958-964). `UserPromptSubmit` sets it to `active` (:809-811, 871). Codex is **excluded from the "output stable 10 s → listening" recovery** that other tools get: "Codex Stop/Interrupt hooks own idle status; quiet terminal output can still mean active work" (src/delivery.rs:2128-2130).
2. **Approval**: hcom latches on the terminal title containing `Action Required`. It reads the last complete OSC 0 or OSC 2 title in a 4 KB rolling buffer (src/pty/screen.rs:76, 83-120, 360-372). It also scrapes visible dialogs: "allow command?", "[y/n]", "press enter to confirm or esc to cancel" and so on, excluding the transcript viewer (`is_codex_approval_visible`, src/pty/screen.rs:504-535). A negative scrape must hold for 400 ms before the latch clears (`APPROVAL_SCRAPE_CLEAR_MS`, src/delivery.rs:944-975). **Correction to the mapper note:** the approval signal is the OSC 0/2 *title*, not OSC 9. The real-Codex test says so explicitly: "Detection does NOT rely on Codex's OSC9 notification … under hcom's headless PTY … Codex falls back to the Bel backend and emits no OSC9" (tests/real_tool_codex.rs:44-55). The `OSC9` wording in the src/delivery.rs:660-662 doc comment is stale.
3. **User activity**: no keystroke from the human within 500 ms (`USER_ACTIVITY_COOLDOWN_MS`, src/pty/shared.rs:35). Every forwarded stdin byte refreshes it (src/pty/mod.rs:1338).
4. **Submit settle**: no injection within 1.5 s of an observed composer non-empty→empty edge. This covers the lag before `UserPromptSubmit` flips the status to `active` and prevents a double delivery (src/delivery.rs:926-930, 1046-1059).
5. **Ready prompt**: `› ` or `» ` must be visible. Codex startup is treated as not ready: the `>_ OpenAI Codex` header followed by a `loading` line, or any `enter … · esc …` modal footer such as trust, update or startup-hook review (src/pty/screen.rs:249-270, 456).
6. **Empty composer**: `get_codex_input_text` finds the **bottom-most** `› `/`» ` row (history above can contain `›`). Text after the glyph counts as placeholder unless its non-dim cells strictly outnumber its VT100 *dim* cells (a tie is placeholder) (`is_dim_after_prompt`, src/pty/screen.rs:686-731, 1073-1135). On the startup screen it returns `None` (unknown), never "empty".

### 3.3 Draft safety (V unless noted)

- Text after the prompt glyph whose non-dim cells outnumber its dim cells is `prompt_has_text`, and the gate stays shut (src/delivery.rs:1066-1081; src/pty/screen.rs:691-731). A typed draft is all non-dim, so hcom does not type beside one (I: a short draft appended to a still-visible dim placeholder could be misread as empty).
- The exclusive-ownership check before *every* Enter, retries included, means a human who starts typing after hcom typed `<hcom>` is never auto-submitted (src/delivery.rs:1343-1364, 2267-2286, 2368-2388).
- Residual hazard (I): hcom has already typed `<hcom>` when the human starts typing, so the human's draft now begins with `<hcom>`. hcom gives up (`Mixed`) but does not remove its text. The `leftover_wake` path that re-submits a lone leftover `<hcom>` exists only for Claude (src/delivery.rs:892-898, 1071-1076). For Codex, a stranded `<hcom>` is therefore read as `prompt_has_text` and blocks all later wakes until the human clears or submits it.
- Codex's paste-burst heuristic turns an Enter within 120 ms of a fast burst into a newline (primer §7). hcom writes `<hcom>` as one 6-byte write, waits for the render, and sets no settle for Codex. It relies on the Enter retry to recover (I; whether the first Enter is ever swallowed in practice is unknown).

### 3.4 Latency (I)

The notify ping is immediate. The minimum after a `Stop` is the remaining 0.5 s user-activity cooldown plus a 10 ms render poll, then the Enter and the hook round trip: well under a second when nobody is typing. A wake that lands just after a human submit waits ≥1.5 s. Missed pings fall back to the 30 s idle poll.

### 3.5 Failure modes (I unless noted)

- **Hooks not running in the hcom env → bare wake turns.** The PTY loop sets `listening` itself at startup (src/delivery.rs:1633). Codex is excluded from stability recovery. If the Codex hooks run but `hook_gate_check` fails, for example because they execute in a shared app-server daemon whose environment lacks this pane's `HCOM_PROCESS_ID` (primer §2.1, §9), every wake submits a bare `<hcom>` that nothing fills. Verify then times out and the retry types again. `Reset` returns to `Pending` without resetting `inject_attempt` (src/delivery.rs:2559-2570, V), so this repeats indefinitely, each attempt a full model turn.
  - Codex's bootstrap (`DELIVERY_AUTO`) does **not** contain the "a prompt that is only `<hcom>` is a wake trigger … end your turn" rule that Cursor and Copilot get (src/bootstrap.rs:189-215, 505-506, 610, V). The model may therefore "answer" an empty wake.
  - hcom does not pass `--no-daemon`. It only scrubs `CODEX_EXEC_SERVER_URL` from children (integration_spec.rs:1473; launcher.rs:916-917, 951, V). Whether per-run `-c hooks.*` session flags and the hook env survive a TUI that attaches to an already running shared daemon: **unknown**. CI exercises 0.162.1, but its test environment may never have a pre-existing daemon.
- **Missed `Stop`/`Interrupt` → never woken.** If a turn ends without either hook (crash, hook timeout), the status stays `active` forever for Codex, because of the stability-recovery exclusion. Mail then arrives only at the next human-driven tool call.
- **Wake after a human Esc.** `Interrupt` sets `listening` (`interrupted`), so hcom types `<hcom>` about 0.5 s after the human stops typing. That restarts work the human may have just stopped deliberately. Contrast Codex's own durable queue, which does not auto-dispatch after an interrupted turn (primer §3).
- **Composer-visible-while-busy.** Codex shows its composer during a turn, and Enter there *steers* (primer §7). The only guard against typing mid-turn is the hook-set status. Any desync that leaves `listening` during a turn makes hcom steer the running turn with `<hcom>`, which is harmless for content but costs a turn.
- **Screen-scrape drift.** Ready glyphs, the dim placeholder, modal footers and approval strings are tied to the TUI's rendering. The changelog shows a steady stream of such fixes ("Fixed Codex counting as ready while a startup or update screen was open", CHANGELOG.md:78; `/voice` redraw holding approval, :45; "Codex 0.157 startup waits and Ultra-effort prompt detection", :206) (V).

## 4. Setup and installation footprint on Codex

Per launch, through argv only (V). `prepare_per_run` (src/hooks/codex.rs:240-336) builds:

- `-c features.hooks=true`
- `-c hooks.<Event>=[…]` for all 6 events, merged after any caller `-c hooks.*` (:147-211)
- `-c hooks.state={<key> = {trusted_hash = "<currentHash>"}, …}`: **hcom pre-trusts its own hooks**. It obtains the hashes by spawning `codex app-server --listen stdio://` with the same overrides and calling `hooks/list` (:1362-1450). It keeps only `source == "sessionFlags"` entries whose command it owns, fails the launch unless all 6 are found (:281-310), and caches the result under the hcom dir keyed by `codex --version` output plus a declaration digest (:258-268, 311-312). The comment explains why the user's `config.toml` trust is not copied: "would pin a launch-time snapshot above any trust the user grants mid-session" (:242-245).
- `-c developer_instructions=<TOML string>`: the bootstrap (the README puts the CLI primer at about 700 tokens, README.md:231). It is prepended to a CLI-supplied `developer_instructions` with a `---` separator (src/tools/codex_preprocessing.rs:207-276). A value that exists only in `config.toml` is overridden rather than merged (I: a `-c` override replaces that key).
- `-c sandbox_workspace_write.writable_roots=[…, <hcom dir>]`. Per its own comment this replaces the list, and "roots set only in project or system config are dropped" (src/tools/codex_preprocessing.rs:10-77).
- `-c sandbox_workspace_write.network_access=true`, unless the CLI args already set `sandbox_workspace_write` or its `network_access` key (a `config.toml` value is not consulted, so it is overridden). The stated reason is "Seatbelt otherwise denies terminal Unix sockets (kitty/tmux)" (:278-317); that it also lets `hcom send` reach the localhost TCP notify ports is I. INFERRED side effect: this grants the workspace-write sandbox general outbound network, not only localhost.

Persistent file writes into `$CODEX_HOME` (V):

- `rules/hcom.rules`: execpolicy `prefix_rule(... decision="allow")` for the `hcom` safe subcommands (`send, start, help, list, events, listen, relay, config, transcript, archive, bundle, status, term, hooks, …`), plus `--help`/`-h` for each tool. It is synced on every launch (rewritten when its content differs) when `auto_approve` is on (the default is `true`, src/config.rs:353) and deleted when it is off (src/hooks/codex.rs:338-352, 1558-1608; src/hooks/common.rs:52-71). Auto-allowing `hcom config` and `hcom relay` lets the agent change hcom settings and relay membership without approval, and auto-allowing `hcom term` ("Terminal admin: screen query, injection", src/commands/term.rs:1-17) lets it read and type into other agents' PTYs without approval (I).
- Legacy cleanup: on launch hcom removes old global `hooks.json` entries, their `config.toml` `hooks.state` keys (re-keying the user's shifted handlers), a pre-hooks `notify = [… codex-notify]` and `hcom-hook-trust.toml` (src/hooks/codex.rs:354-520). History: "Moved … Codex … hooks to each hcom launch. Removed legacy global hcom hooks and Codex's hook-trust bypass" (CHANGELOG.md:172). An earlier trust fallback "could bypass [unrelated project and plugin hooks'] trust checks", fixed in GHSA-pwv3-8r7h-p373 (CHANGELOG.md:228).

Runtime footprint: one `hcom pty` proxy per agent, owning the PTY, plus two localhost TCP listeners (notify and inject). Neither listener authenticates, and the inject server also answers a "screen" query (src/pty/inject.rs:9-35, 55-60) (V). Any local process can read the agent's screen and type into it (I). The C0/ESC stripping lives in the client-side `inject_text` (src/delivery.rs:1299-1313), not in the server, which has no byte filter (src/pty/inject.rs, V by absence); `hcom term` writes raw bytes to the same port (src/commands/term.rs:87-93).

## 5. Token cost

| Case | Model-visible tokens | |
|---|---|---|
| Per session | Bootstrap in `developer_instructions`, about 700 tokens, resent with every request (prompt-cached) | V (size: README.md:231) / I (caching) |
| Per delivered message | `<hcom>` + `[intent:thread #id] sender → recipient: ` + body + `</hcom>` (roughly 15-25 tokens of framing), plus `\| [hints]` on **every** delivery if hints are configured, plus a one-time tip per new thread or intent | V (format) / I (token estimate) |
| Wake overhead (idle recipient) | One extra user turn whose prompt is `<hcom>` (about 3 tokens), plus the full turn's system and history re-read, plus whatever the model generates | V (mechanism) / I (cost) |
| Mid-turn delivery | Only the `additionalContext` block; no extra turn | V |
| Quiet turn / quiet tool call | **0**: a no-op hook prints nothing (`output = None`), and `Stop` prints `{}` (src/hooks/codex.rs:988-1006) | V |
| Process cost per tool call | 2 hook processes (PreToolUse for Bash/apply_patch/spawn_agent, PostToolUse for every tool), each opening SQLite | V (count) |

Every pending message wakes the recipient: there is no priority, threshold or silent send (overview §Wake policy). That makes the wake-turn cost per message, not per batch, unless several messages queue up before the gate opens (I).

## 6. Limitations and bugs visible in the source

1. **No delivery at a human's turn start.** By design `UserPromptSubmit` injects only for `<hcom>`. A human-driven turn with no tool calls ends, and *then* hcom spends a separate wake turn (src/hooks/codex.rs:870-873) **V/I**.
2. **No `Stop` block-and-continue.** Mail that arrives during the final sampling step gets a whole new wake turn instead of a continuation (primer §1.5) **V/I**.
3. **Bare-wake loop when hooks are inert** (§3.5) and no "ignore bare `<hcom>`" instruction for Codex **V/I**.
4. **Stranded `<hcom>` blocks all later wakes** for Codex, because `leftover_wake` is Claude-only (§3.3) **V/I**.
5. **Ack on flush, not on receipt.** Combined with Codex's ~2,500-token spill, large batches are acknowledged while the model sees only a preview (§2.5) **I**.
6. **Body can contain `</hcom>`** and break the wrapper. Only the wake preview strips these tags (§2.3) **I**.
7. **Approval detection is pure scrape**: the title string plus English dialog phrases. No `PermissionRequest` hook, by choice (src/hooks/codex.rs:41-46) **V**.
8. **Shared app-server daemon not addressed.** No `--no-daemon`, no daemon-env handling found (§3.5) **V (absence) / unknown (effect)**.
9. **Self-granted hook trust** sidesteps Codex's per-hash review for hcom's own hooks. That is legitimate (the user ran `hcom codex`), but it means Codex's `/hooks` review never fires for them **V/I**.
10. **Sandbox widening** (`network_access=true`, writable_roots replaced) and the persistent `rules/hcom.rules` auto-allow in the user's real `$CODEX_HOME` **V**.
11. **Unauthenticated localhost inject/screen port** **V/I**.

## 7. Clever ideas worth stealing

1. **`PostToolUse` with no matcher, as model context.** It covers `apply_patch`, MCP and every other tool, where herdr-threads uses `PreToolUse ^Bash$` (primer §10, §11.4). It is also the natural hook to deliver bodies or a digest after the tool result, without a model `inbox` round trip.
2. **The `UserPromptSubmit` filter on the exact wake string.** Only our own wake prompt pulls mail in at turn start, so a human's prompt is never padded with peer mail. Whether that is *better* for us is debatable (we deliver a digest at every turn start), but keying behaviour on "this prompt is our content-free wake" is a cheap, harness-neutral signal.
3. **Pre-trusting per-run hooks through `hooks/list` + `-c hooks.state.<key>.trusted_hash`.** This gives zero-touch trust for session-flag hooks, cached per (Codex version, declaration digest), without writing `config.toml`. If herdr-threads ever moves to per-launch hooks, this is the working recipe (src/hooks/codex.rs:240-336). It also shows that `hooks/list` reports `source` and `currentHash`, which is a cheap way to *diagnose* untrusted hooks.
4. **Excluding internal threads.** Subagent (`agent_id`) and memory-consolidation hooks are recognised by a missing transcript and a `memories*` cwd, so they never rebind or flip status (src/hooks/codex.rs:764-794). That is relevant to our seat attribution for Codex.
5. **`Interrupt` hook as an idle edge.** Codex fires no `Stop` on Esc. hcom uses `Interrupt` to return to idle (src/hooks/codex.rs:951-967). Whether a *wake* should follow an interrupt is the policy question in §3.5.
6. **Draft-safety primitives from owning the PTY:**
   - VT100 dim-attribute placeholder detection;
   - exclusive-ownership checks before every Enter;
   - a submit-settle window against the hook-lag double delivery;
   - an approval latch with a negative-hold timer against redraw flicker;
   - startup-screen detection (`loading`, `enter … · esc …` footers).

   Several of these (the dim check, the startup footer, the 400 ms approval latch, the 1.5 s submit settle) translate directly into our Herdr-composer classifier and `agent_status` gating (src/pty/screen.rs:249-270, 686-731; src/delivery.rs:926-975).
7. **Verify by cursor advance, not by the screen.** A wake counts as successful only when the hook-side ack moves (src/delivery.rs:2433-2475). That gives a closed loop from typed wake to model receipt, which our nudge path could mirror with the delivery ledger.

## Verification

Adversarial pass on 2026-10-23 against the clone at `de33e31a0cdb` and the GitHub API.

- Checked and confirmed: repo metadata (597 stars, 94 forks, MIT, v0.7.31 / v0.7.30 dates); the `CODEX` spec and gates; `CODEX_HOOK_COMMANDS` and its design comment; every hook handler, `hook_gate_check`, `is_internal_thread`, `dispatch_result_to_stdout` (commit after flush, `{}` for Stop); the 50-message cap and the 1 MiB send cap; the `<hcom>` wrapper and format; `DELIVERY_AUTO` lacking the bare-wake rule (Cursor/Copilot only); `prepare_per_run` (the `hooks/list` preflight over `codex app-server --listen stdio://`, `sessionFlags` filter, count check, version+declaration cache); `developer_instructions` prepend; writable-roots replacement; the gate order and constants (500 ms, 1.5 s, 400 ms, 250 ms, 30 s, 10 s, 2 s, Enter backoff 400/800/1600 ms, 3 inject retries then `Reset` without clearing `inject_attempt`); the Codex exclusion from stability recovery; the OSC 0/2 title and approval phrases; the startup-screen detection; the unauthenticated inject server with a screen query; the CHANGELOG lines; the real-Codex test and `just ci` step.
- Changed: the memory-thread rule (no transcript AND (memories cwd OR differing session id)); the `prepare_raw_messages` line range; the `</hcom>` claim upgraded to V for the code path, with the send validator cited; the dim-placeholder rule (non-dim must strictly outnumber dim, so "a non-dim character" was wrong); `network_access` (only CLI args are checked, and the stated reason is terminal Unix sockets; localhost TCP marked I); `hcom.rules` (synced rather than always rewritten, fuller command list including `term`, corrected line range, `hcom term` auto-allow noted); the inject-server note that the byte filter is client-side only.
- Not re-verified: primer and overview cross-references (taken as cited); Codex-side behaviour (spill threshold, paste-burst, daemon env), which remains I or unknown as marked.
