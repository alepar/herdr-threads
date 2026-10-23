# hcom x Antigravity CLI (agy)

- Upstream: https://github.com/aannoo/hcom (MIT, 597 stars / 94 forks, latest release v0.7.31 on 2026-10-21; see `overview.md` "License and maturity" for the GitHub API facts).
- Clone used: `alts/hcom` (scratchpad), `git clone --depth 1`, HEAD `de33e31a0cdb` (2026-10-21). The clone was read only; nothing in it was built or executed.
- Citations: `path:line` relative to the clone root. Our side: `primers/agy.md` (§ numbers) and herdr-threads paths.
- Tags: **V** = VERIFIED (read in source or docs), **I** = INFERRED, "unknown" = no source answers it.

## Support status

**Native, released.** **V**

- `IntegrationSpec ANTIGRAVITY`: `name: "antigravity"`, `aliases: ["agy"]`, `cli_binary: "agy"`, `released: true` (src/integration_spec.rs:1663-1730).
- Added in 0.7.19 by a contributor (CHANGELOG.md:376, PR #55). Later entries:
  - "Sent Antigravity instructions on each invocation" (:265).
  - "Required Antigravity to complete requests after acknowledgment" (:367, issue #57).
  - "Updated Antigravity prompt and approval-dialog detection" (:144).
- agy reuses the **Gemini hook family**. Hook command names are `gemini-sessionstart|beforeagent|afteragent|beforetool|aftertool|sessionend`, and the agy flavour is selected out of band by `ANTIGRAVITY_AGENT=1` (src/integration_spec.rs:1683-1689; src/hooks/gemini.rs:674-702).
- Launch is `hcom agy …`. That runs agy inside hcom's own PTY proxy (`uses_pty_default: true`, `background: HeadlessPty`, initial prompt through `--prompt-interactive`, resume through `--conversation`) and adds `ANTIGRAVITY_AGENT=1` and `HERDR_AGENT=antigravity` to the PTY runner env; `HERDR_AGENT` is a hint for Herdr, which reads it from the pane's foreground process (`hcom pty`) (src/integration_spec.rs:1699-1713; src/launcher.rs:691-705, 723).
- Validated agy versions: hcom's screen fixtures and comments cite agy **1.2.x / 1.2.16** (src/pty/screen.rs:542, 1746; src/pty/vt_compat.rs:243) **V**. Whether hcom was exercised on 1.3.x: unknown. There is no version gate for agy; the only version gate in this area is Gemini's (src/hooks/gemini.rs:842-843) **V**.

Per-message flow in one paragraph:
1. `hcom send` writes a row to SQLite and TCP-pokes the recipient's PTY delivery thread.
2. When the screen-scraped gates pass, the thread **types a metadata-only `<hcom>[intent:thread #id] a -> b</hcom>` line** into agy's composer and presses Enter.
3. The woken turn's `PreInvocation` hook returns the bodies as an **`ephemeralMessage`** inject step.
4. The cursor advances once the hook's stdout is flushed.

## Injection mechanism

### Hooks registered

All hooks go into one `hcom-lifecycle` group of the **global** `~/.gemini/config/hooks.json` (src/hooks/antigravity.rs:162-227). **V**

| agy event | hcom handler (name) | Output on agy | Purpose |
|---|---|---|---|
| `PreInvocation` #1 | `gemini-sessionstart` (`hcom-sessionstart`) | `{"injectSteps":[{"ephemeralMessage": <bootstrap>}]}` on **every** model call | bind conversation → instance; (re)inject bootstrap |
| `PreInvocation` #2 | `gemini-beforeagent` (`hcom-beforeagent`) | `{"injectSteps":[{"ephemeralMessage": <preamble + batch>}]}` or `{}` | deliver pending messages |
| `PostInvocation` | `gemini-afteragent` | `{}` (agy no-op) | none for agy |
| `Stop` | `gemini-sessionend` | `{"decision":"allow"}` | mark `listening` (turn end) or soft-stop |
| `PreToolUse` `.*` | `gemini-beforetool` | `{"decision":"allow"}` | status "running <tool>" |
| `PostToolUse` `.*` | `gemini-aftertool` | `{}` | none for agy |

- hcom does **not** use agy's own `SessionStart` event. Its "sessionstart" is a `PreInvocation` handler that runs before every model call (src/hooks/antigravity.rs:112-116, 163-170) **V**.
- Every hook runs with a 15 s timeout ("agy default is 30s; 5s was tight under cold-start + busy sqlite", src/hooks/antigravity.rs:157-160) **V**.

### Serialization

src/hooks/gemini.rs:598-669 **V**:

```rust
let supports_inject = matches!(hook_name,
    "gemini-sessionstart" | "gemini-beforeagent" | "gemini-afteragent");
if !supports_inject { return Some(json!({})); }      // PostToolUse: exactly {}
return Some(json!({ "injectSteps": [ { "ephemeralMessage": ctx } ] }));
...
// agy noop: PreToolUse REQUIRES a decision field — `{}` is interpreted as deny
if matches!(hook_name, "gemini-beforetool" | "gemini-sessionend") {
    Some(json!({ "decision": "allow" }))
```

- `ephemeralMessage` is the only step type used. There is no `systemMessage`, no `userMessage` and no downgrade path (grep for `ephemeralMessage` across the clone: src/hooks/gemini.rs and tests/cli_smoke.rs only; plain `ephemeral` also matches unrelated relay/Codex code) **V**.
- Primer semantics (§2.3, verified by our probe): `ephemeralMessage` is "visible to the model for one request only, never persisted".

### Code path for one delivery

1. `dispatch_gemini_hook` reads the stdin JSON and detects agy. It checks `ANTIGRAVITY_AGENT` first; failing that, it falls back to the payload schema (`toolCall`, or `conversationId` plus one of `workspacePaths|invocationNum|fullyIdle|…`) and logs a one-time warning (src/hooks/gemini.rs:674-737) **V**.
2. It opens the DB and then applies `hook_gate_check`, which requires `HCOM_PROCESS_ID` (src/hooks/gemini.rs:755-774; src/hooks/common.rs:121-127) **V**.
3. `handle_beforeagent` resolves the instance by process binding, refusing when the conversation is bound to another instance (src/instance_binding.rs:1216-1231). It updates the directory from `workspacePaths[0]`, because the agy hook cwd is agy's config dir (src/hooks/gemini.rs:171-189, 336-342) **V**.
4. `assemble_gemini_family_lifecycle_outputs(is_agy=true)` (src/hooks/common.rs:209-259) **V**:
   - if there are unread messages, it emits `ANTIGRAVITY_DELIVERY_ACTION` followed by the formatted batch and keeps a **deferred** ack;
   - if there are none and the bootstrap has already been announced, it emits nothing.
5. `serialize_hook_result` builds the JSON. stdout is written and flushed, and **only then** does `commit_delivery_ack` advance the per-instance cursor. The advance is forward-only and fenced by `created_at` (src/hooks/gemini.rs:818-832; src/hooks/common.rs:314-346) **V**.

### When it fires

- **Turn start.** `PreInvocation` with `invocationNum` 0 delivers on *any* turn: the human's own prompt, or hcom's typed wake **V** (handler has no prompt filter, src/hooks/gemini.rs:328-406).
- **Mid-turn.** Every subsequent `PreInvocation`, that is, before each model call that follows a tool result, delivers anything that arrived meanwhile **V** (same handler; agy semantics primer §2.5).
- **Not at PostToolUse.** It is explicitly skipped so that no ack is committed for output agy would drop:
  > "Antigravity PostToolUse cannot inject model context. Preparing delivery here would attach a delivery_ack that dispatch would commit even though serialization emits `{}`." (src/hooks/gemini.rs:464-469) **V**
- **Not at PostInvocation.** It is a no-op for agy: "PostInvocation fires after each model invocation, including between tool calls. Stop/turn-end is the reliable idle boundary." (src/hooks/gemini.rs:415-419) **V**

### Payload format and framing

Each delivery's `ephemeralMessage` is the agy preamble, a `\n\n---\n\n` separator, then the batch (src/bootstrap.rs:182-183; src/hooks/common.rs:224-226; src/hooks/gemini.rs:400; src/messages.rs:548-668) **V**:

```
## HCOM MESSAGE — handle the delivery below this turn
For `intent=request`: an ACK alone does not complete it, and no turn is auto-created to resume after you go idle. Do the requested work and `hcom send` the result in this same turn. ...

<hcom>[request:thread-x #42] luna → nova: <text> | [<hints>]</hcom>
```

- A multi-message batch is `[N new messages] | m1 | m2 …`.
- The per-instance or config `hints` are appended to every delivery.
- At most one one-time tip is appended per delivery, inside the `<hcom>` wrapper: the first-seen thread tip wins, else the first-seen intent tip (src/messages.rs:623-648).
- The bootstrap from the first `PreInvocation` hook is a *separate* inject step in the same model call. The two handlers emit independent `injectSteps` arrays, and how agy merges two hooks' steps in one event is unknown (I: presumably concatenated).

### Size bounds

- `MAX_MESSAGES_PER_DELIVERY` = 50 and `MAX_MESSAGE_SIZE` = 1 MiB (src/shared/constants.rs:19, 22; src/hooks/common.rs:261-267) **V**.
- No byte cap on a delivery and no truncation was found. A single ephemeral step can therefore approach 50 MiB (I).

### Dedupe and ack

- Ack is a single forward-only `last_event_id` cursor that advances on stdout flush. There is no confirmation that agy accepted or applied the step. **V** (commit order) / **I** (no confirmation: no transcript read exists in the agy path).
- herdr-threads confirms a `systemMessage` by finding its persisted `SYSTEM_SDK` record in `transcript_full.jsonl` (primer §10). hcom cannot do the equivalent, because an ephemeral step is not persisted (primer §2.3) **I**.
- Malformed hook output is dropped silently together with its sibling steps (primer §2.3), and the cursor has already moved by then. That would be a silent loss (I). Normally this cannot happen, because `ephemeralMessage` is a supported step.
- The thread and intent tips are marked "seen" while the batch is being *prepared*, before the ack commits (src/messages.rs:625-650). A failed write therefore loses the tip but not the message **V**.

### Bootstrap

- On first bind: `inject_bootstrap_once` (src/hooks/gemini.rs:306-319).
- On every later model call: the "recurring" path, which returns the full `get_bootstrap` text again (src/hooks/gemini.rs:254-278) **V**:
  > "agy fires PreInvocation before every model call … refresh its bootstrap: agy's ephemeralMessage is not persisted in the conversation trajectory."

  A smoke test pins this behaviour (tests/cli_smoke.rs:1085-1122) **V**.

## Wake mechanism

**Mechanism.** Typing into the PTY that hcom itself owns. **V**

1. The per-agent `hcom pty` process runs `run_delivery_loop` (src/delivery.rs:1580). It blocks on its localhost notify port: `IDLE_WAIT` is 30 s, with a 250 ms retry while gated (src/delivery.rs:1327, 1462).
2. `hcom send` connects to that port to wake it (overview "Architecture").
3. When the gate opens, it builds the wake text with `build_wake_inject_text` (src/delivery.rs:585-605, 2058-2072) **V**:

   ```rust
   let preview = format!("<hcom>{inner}</hcom>");   // inner = "[intent:thread #id] sender -> recipient"
   if preview.chars().count() > max_len || preview.contains('@') { "<hcom>".into() } else { preview }
   ```

   - `max_len` is the terminal width minus 15.
   - Thread and sender names lose `@` and any embedded `<hcom>` tags, and whitespace is collapsed (src/delivery.rs:607-644).
   - Several pending messages are shown as `[N new messages] | <first>`.
   - The body never goes into the typed text.
   - Claude and Codex get a bare `<hcom>`; agy, Gemini and OpenCode get the preview "for human visibility", because their hook output is not shown in the TUI (src/delivery.rs:578-584).
4. The text goes to the PTY through a localhost TCP inject server, with all C0 control characters except tab stripped (so no ESC and no newline), followed by a separate `\r` (src/delivery.rs:1296-1321) **V**.
5. The woken turn's `PreInvocation` delivers the bodies.

**Idle and busy detection.** The gate runs in order and fails fast (src/delivery.rs:1008-1087; agy gates at src/integration_spec.rs:1690-1698) **V**.

1. `require_idle`: DB status must be `listening`.
   - It is set by the agy `Stop` hook when `fullyIdle == true` or `terminationReason ∈ {NO_TOOL_CALL, NO_TOOL_CALLS}`, followed by a notify poke (src/hooks/antigravity.rs:809-832; src/hooks/gemini.rs:532-539).
   - Each delivery's ack sets `active` (src/hooks/common.rs:336-345); `PreToolUse` sets an active tool status (src/hooks/gemini.rs:428-453).
   - A stuck `active` with output quiet for 10 s is flipped back to `listening` (`pty:recovered`, src/delivery.rs:2130-2165).
2. `block_on_approval`: a screen scrape. It needs two of three signals: the "Requesting permission for:" marker, an exact question line ("Run this command?" and so on), and a dialog footer ("tab Amend" / "↑/↓ Navigate") (src/pty/screen.rs:534-570; src/pty/shared.rs:205-206). No latch is applied for agy.
3. `block_on_user_activity`: **disabled for agy** (src/integration_spec.rs:1694). No rationale is given in the source. The keystroke cooldown is still checked unconditionally right before Enter (src/delivery.rs:2269, 2292).
4. Submit-settle: 1.5 s after an observed prompt submit (src/delivery.rs:930, 1051-1059).
5. `require_ready_prompt`: the footer must show `? for shortcuts` (src/integration_spec.rs:1672). agy hides that footer when a draft is present and replaces it with `esc to cancel` when busy, including on the permission menu (`tests/fixtures/agy/composer/*.read-detection.txt` in herdr-threads; primer §8).
6. `require_prompt_empty`: the bottom-most screen row starting with `>` must hold nothing, or only dim (placeholder) text (src/pty/screen.rs:1131-1166).

**Draft safety.**
- **Two independent signals** stand between the gate and a human draft: the footer disappears and the `>` row is non-empty **V**.
- After typing, hcom requires the scraped composer to equal the injected text **exactly** (`PromptOwnership::Exclusive`) before it presses Enter. It re-checks that condition, plus approval and keystrokes, from a fresh snapshot immediately before each Enter (src/delivery.rs:1332-1364, 2255-2309) **V**.
- **Gap:** on `Mixed` (the human typed during the window) or on lost ownership, hcom refuses to submit but **does not remove the text it typed**. The `<hcom>[…]</hcom>` preview stays inside the human's draft, and a later human Enter would submit both together. The `MixedPrompt` and ownership-lost branches only change state (src/delivery.rs:2276-2286, 2310-2323) **V**.
- herdr-threads' agy path has `PokeCapabilities::NONE` (no stash) and clear key `ctrl+u` (primer §10). That gives a similar refuse-to-submit stance with the same leftover risk; whether our Herdr path removes partial wake text is a separate check (I, not examined here).

**Latency.** I, from constants; nothing measured.
- Notify is immediate.
- The gate passes immediately when idle, or after up to 1.5 s of submit-settle.
- The render poll runs every 10 ms, Enter follows, and two hook processes then spawn before the first model call.
- Expected: about one second from `send` to the model seeing the body.
- Timeouts: render 10 s, text clear 2 s with up to 3 Enter retries (backoff 400/800/1600 ms), cursor verification 10 s (src/delivery.rs:1329-1330, 1415-1419, 1512-1513, 2403) **V**.

**Failure modes.**
- **Verify timeout with mail still pending.** agy gets `Retry` for up to 3 inject attempts and then `Reset` back to `Pending`, which **re-arms indefinitely**. Claude, by contrast, fast-fails to `WakeUnacknowledged` (src/delivery.rs:1382-1398, 2521-2570) **V**.
  - If the hooks stop firing (for example, one malformed key elsewhere in the shared `hooks.json` silently drops the whole file, primer §2.1), every cycle still submits a real agy turn that never advances the cursor. That is an unbounded paid wake loop, throttled only by the timeouts and the `Stop`-driven idle gate (I).
- **Soft-stop at turn end.** `Stop` with a `terminationReason` other than `NO_TOOL_CALL(S)` and `fullyIdle != true` soft-stops the instance (status `inactive`, `exit:<reason>`) and, for agy, also clears the process binding (`keep_process_binding: false`) (src/hooks/gemini.rs:532-563; src/hooks/common.rs:1565-1591) **V**. The next `PreInvocation` would then need `bind_session_to_process` "Path 2" (restore from `life.stopped` by session id, src/instance_binding.rs:657-713) to re-attach (I: not traced end to end).
  - Whether agy reports `fullyIdle:true` on `ERROR`, `max_steps_exceeded` or a user cancel is unknown.
  - If it does not, the agent stops being woken until a human prompt produces a new delivery ack and then a `NO_TOOL_CALL` stop (I). `pty:recovered` only handles `active`, not `inactive` (src/delivery.rs:2137-2138) **V**.
- **Wake with nothing pending.** Mail can be delivered mid-turn between the gate and Enter. The typed preview then becomes a user turn with no body, and the agy branch emits nothing (src/hooks/common.rs:228-233) **V**. The cost is one short model turn (I).
- **Wake during a live subagent.** agy's parent screen during subagent work shows an empty `>` composer and `? for shortcuts … · 1 subagent(s)` (herdr-threads fixture `tests/fixtures/agy/composer/subagent-generating.read-detection.txt:15-20`) **V**. The parent's own `Stop` has already set `listening`, so every hcom gate passes and hcom types a new turn while the child runs (I). herdr-threads' `AgyComposer` reads this screen as `Unreadable` and does not wake (primer §10).

**Identity across subagents and /clear.**
- The hooks carry no parent or role field for agy children (primer §7). A child's `PreInvocation` arrives with the parent process's `HCOM_PROCESS_ID` and a fresh `conversationId`.
- `bind_session_to_process` "Path 3" binds that unknown conversation to the process's instance and moves the process binding to it (src/instance_binding.rs:717-735) **V**.
- No agy-specific child detection exists: there is no `presence/*.lock` or subagent-listing check in src/hooks/gemini.rs or src/hooks/antigravity.rs (grep `subagent|presence`) **V**.
- So an agy subagent invocation is very likely treated as the instance itself. It gets the bootstrap ("Your name: luna"), it can be handed pending mail as ephemeral context, and the cursor is acked on it (I, strong).
- `/clear` takes the same Path 3, which writes the new `conversationId` into the instance's `session_id` (src/instance_binding.rs:717-736, session_id write at :727-729). The `already_bootstrapped` check (`name_announced != 0` and matching `session_id`) then passes on the very first model call, so the recurring bootstrap is emitted from the first call after `/clear` (and likewise on a subagent child's first call) (src/hooks/gemini.rs:262-276) **V** (code reading; not run).

## Setup and installation footprint

`ensure_hooks_installed` runs at launch: verify first, write only if needed, refuse to launch on failure (src/launcher.rs:633-670, 1909). For agy it writes **global** files: **V**

1. **`~/.gemini/config/hooks.json`** (or `$GEMINI_CLI_HOME/.gemini/config/hooks.json`):
   - It merges an `hcom-lifecycle` key and preserves other keys (Herdr's `herdr` block, for example).
   - It writes atomically, refuses to touch an unparsable or non-object file, and verifies after the write (src/hooks/antigravity.rs:118-255, 306-649; src/runtime_env.rs:111-118).
   - Removal deletes the key, and the file too when it is left empty (src/hooks/antigravity.rs:270-304).
2. **Each hook command is a shell guard** (src/hooks/antigravity.rs:78-110):

   ```sh
   sh -c 'command -v hcom >/dev/null 2>&1 && ANTIGRAVITY_AGENT=1 exec hcom gemini-beforetool || { printf %s eyJkZWNpc2lvbiI6ImFsbG93In0= | base64 -d; exit 0; }'
   ```

   - If hcom is uninstalled, fail-closed hooks (PreToolUse, Stop) still answer `{"decision":"allow"}`. The fallback JSON is base64-encoded to survive the nested quoting.
3. **`~/.gemini/antigravity-cli/settings.json` `permissions.allow`**, when `auto_approve` is on, which is the default (src/config.rs:239, 353):
   - 38 rules, `command(hcom|uvx hcom <cmd>)` for 19 "safe" subcommands: `send start help list events listen relay config transcript archive bundle status term hooks …` (src/hooks/antigravity.rs:665-728; src/hooks/common.rs:52-72).
   - Removal strips only those rules (:730-783).
   - agy rewrites this file itself and may drop unknown keys (primer §7).
4. Process environment: `ANTIGRAVITY_AGENT=1`, `HERDR_AGENT=antigravity` (src/launcher.rs:691-705), `HCOM_PROCESS_ID` (src/launcher.rs:2060).
5. Hooks for agy stay **persistent and global**. Claude, Codex, OpenCode and Pi moved to per-launch injection (CHANGELOG.md:172). The hooks therefore fire in **every** agy session on the machine, hcom-launched or not:
   - The gate makes them inert, but only after the process has spawned, created the hcom directories and opened the SQLite DB with its migration check (src/hooks/gemini.rs:755-774).
   - A pre-gate exists for `beforeagent` only (:739-753) **V**.

## Token cost

Byte counts come from the template constants in src/bootstrap.rs (measured with a scratch script that read the file, not by running hcom). Tokens are estimated at about 4 bytes per token (I).

| Item | Size | Persistence | |
|---|---|---|---|
| Recurring bootstrap, on **every model call** (turn start and after every tool result), whether or not mail is pending | ≈3.35 KB of template text (IDENTITY_HEAD 247 + MESSAGES 392 + CAPABILITIES 1768 + RULES 147 + IDENTITY_TAIL 276 + notice 39 + DELIVERY_AUTO 440), plus the `<hcom_system_context>` wrapper, the active-instance list, script names and optional tag/relay notices and user notes → **≈850-1000 tokens per model call** | ephemeral: billed on every request and never accumulated in history | V (constants) / I (tokens) |
| Per delivery: `ANTIGRAVITY_DELIVERY_ACTION` | 358 B ≈ 90 tokens | ephemeral | V |
| Per delivery: `<hcom>` batch | header `[intent:thread #id] sender → recipient: ` (~30-50 B) + body + `\| [hints]` on every delivery + first-time tips | ephemeral | V |
| Per wake: typed preview user turn | `<hcom>[request:thread #42] luna -> nova</hcom>` ≈ 40-80 B (terminal width − 15 at most); agy wraps it as `<USER_REQUEST>` plus `<ADDITIONAL_METADATA>` local time (primer §2.3) | **permanent** user turn | V / I |
| Quiet turn | Every model call still carries the ≈1k-token bootstrap. There is no "unchanged" short-circuit; the bootstrap is re-rendered each time | — | V |

Hook process overhead per agent step:
- 2 processes per model call (both `PreInvocation` hooks), sequencing unknown;
- 1 `PostInvocation`;
- 2 per tool call (`PreToolUse`, `PostToolUse`);
- 1 per `Stop`.

Each process opens SQLite (src/hooks/gemini.rs:755-769) **V**. A ten-tool-call turn (11 model calls) is therefore about 54 hcom processes (11 × 3 invocation hooks + 10 × 2 tool hooks + 1 `Stop`) and about 11 bootstrap copies (≈10k input tokens) before any mail (I, arithmetic).

For comparison, herdr-threads on agy:
- the skill is listed once (primer §3/§10);
- a quiet `PreInvocation` is one round trip that prints nothing (`Unchanged`; AGENTS.md "Token-efficient communication");
- the wake text is the fixed `herdr-threads: wake up` with no metadata.

## Limitations and bugs you can see

1. **The body vanishes after one model call (I, strong).**
   - The mail goes in an `ephemeralMessage`, which the model sees "for one request only" (primer §2.3).
   - The persistent transcript keeps only the typed metadata preview.
   - A request that takes several tool calls loses its text after the first tool call, and the model has to remember it in its own reasoning.
   - hcom re-sends the *bootstrap* every call for exactly this reason (src/hooks/gemini.rs:254-257) but never re-sends messages, and it acks on first emit.
   - The `ANTIGRAVITY_DELIVERY_ACTION` preamble ("do the work … in this same turn") and issue #57 ("acking a request and going idle before doing the work", src/bootstrap.rs:177-181) are consistent with that failure mode.
2. **No accepted-by-harness confirmation.** The ack fires on stdout flush, and ephemeral steps leave no transcript record to confirm against (I).
3. **About 1k tokens of bootstrap on every model call**, plus hints on every delivery (V/I; see the token table). This is against our rule (a).
4. **No delivery priority or threshold.** Every message wakes an idle agy (overview "Wake policy") **V**. This is against our rule 1.
5. **Subagent and parent identity are conflated** for agy children: bootstrap, mail consumption and ack can land on a child conversation (I; mechanism V, src/instance_binding.rs:717-735).
6. **hcom wakes while an agy subagent is still working** (I; screen fixture V).
7. **A lost submit leaves the preview inside the human's draft** (V, src/delivery.rs:2310-2323).
8. **An unbounded retry loop is possible** when hooks are broken, at the cost of one model turn per cycle (V for the `Reset → Pending` loop; I for the trigger).
9. **Possible delivery stall** after a non-`NO_TOOL_CALL` turn end without `fullyIdle` (V for the code; whether agy reports `fullyIdle` that way is unknown).
10. **Global, fail-closed `PreToolUse` on every agy session.** It exists only for status tracking.
    - A crash is handled by the panic guard, which returns Allow (src/hooks/gemini.rs:791-800) **V**.
    - A 15 s timeout, for example SQLite busy under load, blocks the tool call. agy's `PreToolUse` fails closed (primer §2.3) (I).
    - Every plain agy session pays about 5 hook process spawns per tool step (3 per model call, 2 per tool call), each opening the DB before the gate rejects it (V/I).
11. **Auto-approved `hcom term` is a cross-agent keystroke channel.** `term` is in `SAFE_HCOM_COMMANDS`, so `command(hcom term)` is auto-allowed in agy, and the bootstrap advertises `term inject <name> ['text'] [--enter]` (src/hooks/common.rs:67; src/bootstrap.rs:105) **V**. An agy agent can therefore type, and press Enter, into another agent's PTY without an approval prompt. That could include answering that agent's permission menu (I). `hcom config` is also auto-allowed (V).
12. **`block_on_user_activity` is off for agy.** The first keystrokes of a human who starts typing just as the gate passes can interleave with the typed preview (I). The exclusive-ownership check then prevents a submit (V) but leaves the mixed text in place (item 7).
13. **agy version coverage.** The scrape strings come from 1.2.x captures. The version of the "Updated … detection" entry in 0.7.28 is not stated (CHANGELOG.md:144). Approval detection keys on exact English question strings, so a new permission kind or wording falls through to `prompt_has_text` or `not_ready` (both safe) (I).
14. Two hooks in one `PreInvocation` each return `injectSteps`. Merge order and failure isolation are unknown. Primer §2.3 warns that malformed output drops sibling steps; whether "siblings" spans hooks is unknown.
15. Linux and agy: the hcom code is OS-neutral apart from the Windows `where` wrapper (src/hooks/antigravity.rs:90-98) **V**. Live validation status on Linux is unknown.

## Clever ideas worth stealing

1. **Exclusive-ownership submit check** (src/delivery.rs:1332-1364, 2267-2286).
   - Type the wake, then press Enter only if the scraped composer equals the typed text exactly; a substring match counts as "mixed" and is refused.
   - Re-check from a fresh snapshot before every Enter retry, and confirm success by the cursor advancing rather than by the screen.
   - For us: Herdr's `pane send-text` followed by a re-read could adopt the "exact equals" rule before sending Enter, if we do not already. Worth checking against `AgyComposer`.
2. **Approval scrape that needs two of three signals** (marker, exact question line, dialog footer), so stray scrollback cannot flip the status (src/pty/screen.rs:534-570). It is a candidate cross-check for our `permission-prompt` parse.
3. **A missing binary can never break agy** (src/hooks/antigravity.rs:78-110). Each hook command is wrapped in `command -v hcom || printf <base64 fallback JSON>`, so fail-closed events answer allow even after an uninstall. If we ever register a fail-closed event (`PreToolUse`, `Stop`), the same guard applies. Our plugin's hooks call `herdr-threads` directly (I: not checked here).
4. **Turn-end classification** from `Stop` `terminationReason` plus `fullyIdle`, which tells "turn ended" apart from "session gone" (src/hooks/antigravity.rs:809-832). It is a cheap, hook-native busy/idle flag (primer §9 lead), with the caveat in limitation 9.
5. **`workspacePaths[0]` as the project dir** rather than the hook cwd (src/hooks/gemini.rs:171-189). It matches the primer note that the hook cwd is the `hooks.json` directory.
6. **Schema-based harness detection fallback** when the env marker is missing (src/hooks/gemini.rs:674-702). It is robust against agy sanitizing the hook env (primer §2.4 conflict), and it logs once.
7. **Do not prepare or ack mail on a hook whose output the harness discards** (`PostToolUse` → `{}` with no ack, src/hooks/gemini.rs:464-469). We already avoid tool hooks on agy, but the rule generalizes: the ack is tied to an output channel that is known to inject.
8. **A human-visible wake preview.** Because agy's `systemMessage` is hidden in the TUI (primer §2.3), a human watching our pane sees only `herdr-threads: wake up`. hcom's ≤ (width − 15) metadata line, `@`-stripped and tag-stripped, tells the human who wrote and on which thread. It costs about 40-80 permanent bytes per wake, against R18's content-free rule, so it would have to be an explicit, size-stated trade-off and not a default.
9. **Counter-lessons from hcom's choices** (I):
   - `ephemeralMessage` is the right carrier for *re-offerable, idempotent* context such as a check-in digest, and the wrong one for mail that must survive a multi-step turn. Our `systemMessage` with transcript confirmation and a `userMessage` downgrade is the stronger delivery design.
   - Whatever we put in an ephemeral step must not be acked on emit.

## Verification

Adversarial pass against the clone at `de33e31a0cdb` (read only), the GitHub API and `primers/agy.md`.

- Checked and held: GitHub facts (597 stars, 94 forks, MIT, v0.7.31 published 2026-10-21); `ANTIGRAVITY` spec fields and gates; CHANGELOG lines 144/172/265/367/376 and their versions; hook registration, 15 s timeout, shell guard with base64 fallback; `serialize_hook_result` (inject only on the invocation hooks, `{"decision":"allow"}` on PreToolUse/Stop); env-then-schema agy detection; `hook_gate_check` on `HCOM_PROCESS_ID`; stdout-flush-then-ack ordering; PostToolUse and PostInvocation no-ops for agy; `ANTIGRAVITY_DELIVERY_ACTION` (358 B) and the bootstrap template byte counts; 50-message / 1 MiB limits; wake preview builder, sanitizing, tool split and C0 stripping; exact-equality `PromptOwnership` and the Mixed/ownership-lost branches leaving typed text (the leftover-wake resubmit path is Claude-only, src/pty/shared.rs:232-233); `verify_timeout_decision` (Claude fast-fail, others Retry x3 then Reset to Pending without resetting `inject_attempt`); `pty:recovered` only from `active`; agy approval scrape (two of three signals, 1.2.x question list); agy prompt scrape; Path 3 binding; 19 safe commands x 2 prefixes = 38 rules with `term` and `config`; `auto_approve` default true; the herdr-threads subagent fixture lines 15-20; primer §2.3/§2.4/§2.5/§7 statements cited.
- Changed:
  - `/clear` bootstrap claim was wrong: Path 3 sets the instance `session_id`, so the recurring bootstrap is emitted from the first model call after `/clear`, not skipped.
  - Process count for a ten-tool-call turn corrected from about 34 to about 54; plain-session spawns per step from about 6 to about 5.
  - Framing: preamble and batch are joined by `\n\n---\n\n`, not a blank line; at most one tip per delivery.
  - `HERDR_AGENT`/`ANTIGRAVITY_AGENT` are added to the PTY runner env; `HCOM_PROCESS_ID` cited at its own line.
  - `ephemeral` grep scoped to `ephemeralMessage`.
  - Citation ranges for the permission rules and their removal; added that agy soft-stop also drops the process binding.
- Not checked (left as stated, with their I/unknown marks): agy runtime behaviour (merge of two hooks' `injectSteps`, `fullyIdle` on errors), latency, and whether hcom ran on agy 1.3.x.
