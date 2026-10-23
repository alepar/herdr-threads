# herdr-threads x OpenCode: message injection and wake-up (ours)

Subject: herdr-threads at `2de3da1c4` (main), against OpenCode (https://github.com/anomalyco/opencode, formerly sst/opencode). OpenCode surfaces are cited from the primer `docs/research/2026-10-23-integration-alternatives/primers/opencode.md` (whose source citations are against the OpenCode `v1.18.1` tag and `dev` HEAD). No OpenCode source clone was re-read for this report, so an OpenCode claim here is only as strong as the primer's. Every herdr-threads claim below was read in this repository. Each is marked **VERIFIED** (read in code or docs) or **INFERRED** (deduced, not run). Nothing was executed against OpenCode.

## 1. Support status

- **Planned. There is no adapter.** VERIFIED:
  - `builtins()` registers Claude, Codex, Hermes and agy only (`src/harness/registry.rs:1482-1490`).
  - There is no `src/harness/opencode*` and no `integrations/opencode/` (the directory listing has `agy claude codex hermes skill`).
  - `README.md:101` lists "Pi, OpenCode | Planned".
  - `bd search opencode` returns no issues, so no bead tracks the work.
- **Prior research:** two sentences in `docs/research/harness-adapters-2026-10-03/other-harnesses-and-prior-art.md:7,11` (OpenCode plugins are JS/TS hook objects; `shell.env` carries optional `sessionID`/`callID`). VERIFIED.
- **The only OpenCode-specific code is a negative test.** Herdr's agent kind `"opencode"` is the fixture for an "unrecognized" agent, which a cooperative wake refuses before any prompt (`src/host/native.rs:5019-5045`). VERIFIED.

What an OpenCode agent in a Herdr pane gets today:

- **No hooks.** Nothing is installed into OpenCode, and `setup` and `launch` know no `opencode` harness. VERIFIED: the registry lookup has no such id.
- **No in-turn delivery** and **no wakes** (section 3).
- **Manual CLI use only.** The model can run `herdr-threads ...` through its bash tool if it somehow knows to. How such a call is attributed is a trust gap (section 7, L2).

## 2. Injection mechanism

### 2.1 Today: none

No herdr-threads code path puts a message into an OpenCode model's context. VERIFIED by absence: no adapter, no `HookStep`, no `DeliveryPolicy`, no plugin.

The only route is the model itself running `herdr-threads inbox`, whose compact rows would come back as bash tool output (`src/protocol/output_compact.rs`). Even that route is unreliable: the call resolves to a non-agent caller, and without a saved pane context it has no seat (section 7, L2). INFERRED that it fails or acts as a person; not run.

The skill is the only static surface that could reach OpenCode, and it does so only by accident (section 4).

### 2.2 The machinery an OpenCode integration would plug into (VERIFIED, neutral core)

The core is already harness-neutral. An OpenCode adapter would declare policies, not add branches.

**Delivery declaration** (`src/harness/adapter.rs:447-472`):

- `DeliveryPolicy { mechanism: PushChannel | HookPull, provenance, idle_wake: Native | Channel, wake_text }`.
- `Registry::new` accepts only two pairings (`src/harness/registry.rs:1270-1283`):
  - `PushChannel` + `cooperative_mod_delivery`, with either wake kind;
  - `HookPull` + `cooperative_hook_delivery` + `Native`.
- No declaration (`None`) means "hooks deliver and the native wake applies" (`adapter.rs:229-231`).

**Push channel (the Claude mod's shape):**

- The mod spawns `herdr-threads watch --harness claude --session <sid>` and reads JSON lines (`integrations/claude/mod/hooks/register.js:242`).
- `watch --harness` accepts any id from `Registry::push_channel_ids()` (`registry.rs:1434-1443`, used by `src/cli/commands.rs:1216,2556`). The refusal for other harnesses is the frozen `not_claude` wire code, already named neutrally `NO_PUSH_CHANNEL` (`src/protocol/watch.rs:318-327`).
- The channel itself is therefore not Claude-only.
- ACKs go back via `watch ack --session <sid> --via context|submit|append <ids>` (`register.js:476`; enum `ModDeliveryVia` at `watch.rs:172-179`). Those three vias map one-to-one onto OpenCode surfaces (section 6).

**Hook pull (agy's shape):**

- A `HookStep` runs inside `herdr-threads hook <id>`. It sends `DeliveryPull` to the daemon, emits native stdout, and later sends `DeliveryConfirm` (`adapter.rs:807-829`, `src/cli/hook.rs:312,3461`).
- This shape assumes the harness spawns a command per hook event. OpenCode hooks are in-process JS, so a plugin would have to shell out on every hook: one process per tool call, bounded at 1.5 s (`hook.rs:19-21`). INFERRED: a poor fit compared with the push channel.

**Framing** (`src/delivery/frame.rs:9,133-163`):

- The frame opens with `[herdr-threads] new messages:`.
- Each item then gets one header line, `[herdr-threads] <kind> <id> in <thread> from <sender>[ [pN]][ [human]]…:`, followed by a body indented two spaces.
- A frame never starts with `/`. This matters for OpenCode, where a leading `/` or `@` in the composer opens autocomplete (primer §7).
- The JS `frame()` in `register.js` is the reference implementation, with golden fixtures shared with Rust (`frame.rs:1-4`). An OpenCode JS plugin could reuse it verbatim.

**Bounds:**

| Bound | Value | Source |
| --- | --- | --- |
| Body | 8 KiB, then a `body <id>` continuation | `watch.rs:63` |
| Watch page | 32 items, 64 KiB | `watch.rs:65-67` |
| Hook-pull page | 32 blocks, 64 KiB | `src/service/delivery_pull.rs:37,40` |
| Re-offers | spaced 60 s, at most 3 per generation | `src/delivery/ledger.rs:22-24` |

**Ordering:** priority asc, then deadline asc (none last), then age (`src/delivery/assemble.rs` `batch_order`, mirrored in `register.js`). VERIFIED by AGENTS.md, which cites these files.

**ACK provenance** (TRUST-POLICY.md:303-305):

- `cooperative_mod_delivery` is defined in Claude terms: an answered tool result that carries the mod's context, `$.session.append` resolving without `deny`, or `$.prompt.submit`.
- An OpenCode plugin's predicates would be different calls, such as a `tool.execute.after` output mutation or a `promptAsync`/`noReply` 2xx. The policy text must name them before an OpenCode plugin may claim this provenance. AGENTS.md requires this: "a new way to attribute an action needs a provenance value defined in the policy". VERIFIED text; the conclusion that it needs an edit is INFERRED.

## 3. Wake mechanism

### 3.1 Today: refused

**The native wake is refused** at `cooperative_wake_ready` (`src/host/native.rs:1682-1699`). If Herdr's `agent` field for the pane is not a registered host kind, the result is `Err("no recognized harness agent (agent opencode)")`. This happens before the composer read and before `agent prompt`. VERIFIED, with test coverage at `native.rs:5043-5044`.

A cooperative wake also requires a bound harness (`native.rs:1720-1722`), and nothing can bind as `opencode`. VERIFIED.

**Nothing is observed either:**

- `observed_ui` returns `Unknown` with no composer read for an unregistered kind (`native.rs:2032-2052`).
- `status_ui` maps `blocked` to `ApprovalOrQuestion` only for registered kinds (`native.rs:2354-2366`).
- So an OpenCode pane is never read, typed into or prompted. VERIFIED.

Consequences:

- **Draft safety:** trivially safe, because nothing is typed.
- **Latency:** not applicable. An OpenCode agent learns of mail only if it runs `inbox` on its own initiative or a human tells it to.

### 3.2 What the existing native wake would do if OpenCode were registered (VERIFIED for the generic path; INFERRED for OpenCode)

The generic wake, in `submit_prompt_mode` (`native.rs:1784-1960`):

1. Rechecks `agent get` within 750 ms (`native.rs:55`). The status must be `idle` or `done` (`native.rs:53`), and pane, terminal and incarnation must be unchanged.
2. Reads the composer through the adapter's `ComposerPolicy::read` over `agent read --source detection` (`native.rs:1876-1878`; trait at `adapter.rs:1352-1359`). A non-empty or unreadable composer refuses the wake.
3. If the pane is focused, requires the composer to have been seen empty for `FOCUSED_EMPTY_MILLIS = 60_000` (`native.rs:71,1744-1781`).
4. Herdr types the wake text with `agent prompt <pane> <text>`, bounded at 2 s (`native.rs:57,1896-1900`).

The text is `MARKER` (`herdr-threads: attention pending; run herdr-threads inbox`), or `WAKE_UP` (`herdr-threads: wake up`) when turn-start injection exists (`src/notification/policy.rs:14-21`). Neither text starts with `/` or `@`, so neither would open OpenCode's autocomplete. INFERRED as safe on that axis.

What it would take for OpenCode:

- An OpenCode `ComposerPolicy` (a parser for the OpenTUI composer).
- Herdr's idle/busy detection for kind `opencode`, which is unknown: the primer §8 marks it INFERRED.
- `clear_key`/`restore_text` for the poke path. The clear must not be ESC: on OpenCode, ESC while busy is `session_interrupt` (primer §7).

### 3.3 The better path OpenCode offers (INFERRED design, from the primer's VERIFIED surfaces)

OpenCode's server API makes `IdleWake::Channel` natural, as for Claude:

- An in-process plugin watches `session.status` for idle and then calls `client.session.promptAsync` (primer §2-3).
- It never touches the composer, so it cannot clobber a draft at the server level.
- A human's draft can still race a submitted wake turn: the turn starts while the human is typing. Only a TUI plugin can see the draft (`TuiPromptRef.current.input`, primer §7). The Claude mod's equivalent is a `promptRead` check with a 120 s draft hold (`register.js:16,536-546`).

## 4. Setup footprint

**Today: none.** No OpenCode config, plugin dir, MCP entry or env var is written. VERIFIED: no adapter.

**Accidental skill exposure:** `setup` for Claude writes `<claude-config-root>/skills/herdr-threads/SKILL.md` (`src/harness/claude.rs:1516-1523`). When that root is `~/.claude`, OpenCode would list the skill as well, because it scans `~/.claude/skills/**/SKILL.md` (primer §6). INFERRED. Two caveats:

- When `CLAUDE_CONFIG_DIR` points elsewhere (as with an aisw profile), OpenCode does not see the skill, because it resolves the directory under home (primer §6).
- Where it does see it, the skill describes commands whose caller attribution OpenCode cannot satisfy (section 7, L2).

**A future integration would need:**

- a server plugin file under an OpenCode config dir (`~/.config/opencode/plugin/` or `OPENCODE_CONFIG_DIR`), or an npm spec in `opencode.json` `"plugin"`;
- optionally a TUI plugin through `tui.json`, for draft-aware wakes;
- the skill copied into `<opencode-config>/skills/` (primer §2, §6, §7).

Isolated tests can use `OPENCODE_CONFIG_DIR` / `OPENCODE_CONFIG_CONTENT` without touching `~/.config` (primer §2, INFERRED there).

## 5. Token cost

**Today:**

- Per message: 0, because nothing is injected.
- Per quiet turn: 0.
- Static: the skill description in OpenCode's skill listing (about 350 bytes of frontmatter `description`, `integrations/skill/SKILL.md:3`), only when the accidental exposure above applies. The 48 KB body loads only on demand. VERIFIED size; that OpenCode lists only the description is INFERRED.
- A manual `inbox` costs the model one tool call plus compact rows.

**Projected for a push-channel plugin that reuses the core** (INFERRED, from VERIFIED constants):

- One frame per batch: a 29-byte `[herdr-threads] new messages:` line, plus per item a header of roughly 60-120 bytes and the body (at most 8 KiB, then a continuation line).
- Wake prompt: `wake up` (7 bytes), or 22 bytes for `herdr-threads: wake up`.
- Quiet turn: 0 bytes, provided the plugin's `messages.transform` and `tool.execute.after` handlers return the input unchanged when the queue is empty. The Claude mod does the same (`register.js:833`: `if (!batch.text) return result`).
- If a plugin used `experimental.chat.system.transform` it would alter the system prompt per request and defeat prompt caching (primer §2). Avoid it.

## 6. What our adapter traits would need, and which OpenCode surfaces fit

### Recommended shape: `PushChannel` + `cooperative_mod_delivery` + `IdleWake::Channel`, the Claude-mod analogue

| herdr-threads piece | OpenCode surface (primer section) | Notes |
| --- | --- | --- |
| `AdapterMetadata.host_kinds: &["opencode"]` | Herdr reports `opencode` (`native.rs:5044`; §8) | Makes the pane recognised and the A4 guard effective (L2). |
| `agent_env_markers` | `OPENCODE=1` and `AGENT=1` set by the main process (§10) | `OPENCODE` with `person_guard: true`. `AGENT` is too generic to be a guard. |
| `caller_env: CallerEnvShape` | `process_names: ["opencode"]`; `pid_var: Some("OPENCODE_PID")` (§10); `session_var`: a variable **our plugin exports** through the `shell.env` hook (for example `HERDR_THREADS_OPENCODE_SESSION`), because OpenCode exports no session id by default (§10) | The variable name must be ours, not a guessed native one. `role`: a witness that refuses child sessions from the task tool (their `sessionID` has a parent). |
| `BindingPolicy` | one TUI process hosts many sessions (§10) | A pane does not map one-to-one to a session. The seat binds the session the plugin observes the human driving (`tui.session.select` / `api.route.current` from a TUI plugin), or the session that last ran a hook. Open design question. |
| `DeliveryPolicy` (push, mod provenance, Channel) | — | Passes `delivery_pairing_allowed` (`registry.rs:1270-1283`) unchanged. |
| channel transport | the plugin spawns `herdr-threads watch --harness opencode --session <id>` through Bun `$`/`child_process` (`PluginInput.$`, §2) | It reuses `watch`, its grace and stall timers, and the ACK ledger as they are. |
| `via=context` (in-turn, rule 3) | `tool.execute.after`: mutate `output.output` (§2), or `experimental.chat.messages.transform` per step (§2) | `messages.transform` is ephemeral and runs after every tool round. It is the closest match to "no extra model call", but it is experimental. Under the version rules (AGENTS.md "Harness and Herdr versions"), detect a missing hook and fall back to `tool.execute.after`; never gate on a version. |
| `via=append` (silent, p > T, rule 2) | `client.session.prompt({noReply:true, parts:[{type:"text", synthetic:true, text: frame}]})` (§3) | Durable in history and starts no loop. `synthetic` hides it from the TUI user-text view. |
| `via=submit` / Channel idle wake (rules 2, 4, 6) | `event` hook on `session.status` = idle, then `promptAsync({parts:[{text:"wake up", synthetic:true}]})` (§2-3) | **Rule 4 hazard:** a prompt sent to a busy session is not rejected but steered into the running loop (§3). The plugin must gate on idle itself, or a p1 send extends a busy turn. |
| draft safety | a TUI plugin's `TuiPromptRef.current.input` and `api.state.session.status` (§7) | Without it, the server-only plugin cannot see the draft. Mirror the mod's 120 s draft hold. |
| compaction | `experimental.session.compacting` (§2) | Can carry a one-line "pending threads" note across compaction. |
| `ComposerPolicy` (native fallback only) | OpenTUI composer read through Herdr | Needed only if `IdleWake::Native` is kept as a fallback. The clear key must not be ESC (§7). |
| TRUST-POLICY.md | A3 rows 303-305 | Define the OpenCode predicates for `cooperative_mod_delivery`, or add a new provenance, in the same commit (AGENTS.md "Trust policy"). |

### Alternatives that fit worse

- **`HookPull`:** a process per hook event. In-process JS hooks make that pointless overhead.
- **External HTTP via `--port` / `serve` + `attach`:** this needs managed launch, so a `LaunchPolicy` that adds `--port` and sets `OPENCODE_SERVER_PASSWORD` (§3-4). The daemon would then hold OpenCode HTTP credentials and speak a third-party API. Use it only if a plugin is unacceptable.
- **Herdr keystroke wake:** the weakest option (§12 option 4). It brings ESC-interrupt and autocomplete hazards, and it is not draft-aware without a parser.

### 2.x caution

On `dev` (2.x), the v1 hook names are gone. A native durable inbox, `POST /api/session/:id/synthetic` with `delivery: steer|queue` and `resume`, maps almost directly onto "wake iff p <= T" (primer §9). INFERRED: plan the plugin with a thin per-generation shim, keyed on the observed API rather than a version string.

## 7. Known limitations

- **L1. No delivery and no wake.** An OpenCode agent misses every item until a human tells it to run `inbox`. VERIFIED (sections 2.1, 3.1).
- **L2. Caller attribution falls through to "person"** (trust gap, VERIFIED by reading; not run).
  - OpenCode sets no registered env marker, so `harness_marked` is false (`src/cli/output.rs:165-177`). `caller_env::derive_current` returns `NoHarness`, and `caller::derive` maps that to `CallerOrigin::Human` (`src/cli/caller.rs:69-80`).
  - The command then takes the saved-pane-context path (`src/cli/mod.rs:3535-3550` acts on an `Agent` origin only).
  - The A4 agent guard looks for an env marker or a Herdr-reported harness kind (`src/cli/mod.rs:355-371`). `is_harness_agent_kind("opencode")` is false (`src/protocol/authority.rs:127-131`), so the guard does not trip.
  - Result: the OpenCode model can run `herdr-threads human me init` in its own pane and act as an `operator_human` person seat. Its messages would be framed `[human]`.
  - This contradicts the intent of A4 ("refused where the system sees evidence of an agent", TRUST-POLICY.md:307). It is still within the cooperative model: A4 is best effort.
  - Fix: register even a minimal OpenCode adapter that has host kind `opencode` and env marker `OPENCODE`, before any delivery work. The guard then refuses this path.
- **L3. Pane-to-session cardinality.** One TUI hosts many sessions and child sessions (primer §10). Seat binding through `CallerEnvShape` assumes one native session var per caller. INFERRED: the role witness must reject task-tool child sessions, which have `parentID`.
- **L4. The skill reaches OpenCode only by accident** (`~/.claude/skills`, section 4) and then describes commands that cannot attribute the caller (L2). INFERRED.
- **L5. `cooperative_mod_delivery` is Claude-worded** (TRUST-POLICY.md:304). Reusing it for OpenCode needs a policy edit. VERIFIED text.
- **L6. Unknowns:**
  - how Herdr classifies OpenCode idle and busy;
  - the tool timeout for a long-poll custom tool;
  - whether the TUI renders `synthetic` API parts (primer open questions);
  - whether Herdr's `agent.prompt` typing into OpenTUI triggers the 150-character paste-summary collapse (primer §7). Our wake texts are 22 and 56 bytes, so INFERRED no.

## Verification

Adversarial re-read, against the code:

- **Wake refusal.** Rechecked that the refusal happens at the host-kind lookup, before the bound-harness check: `native.rs:1694-1699` precedes `1700-1723`.
- **`observed_ui`.** Rechecked that it returns before any composer read: the `by_host_kind` miss at `2043-2048`.
- **Push channel.** Rechecked that `push_channel_ids` is registry-driven, not hard-coded. The assert at `registry.rs:2651` lists `["claude"]` only because only Claude declares `PushChannel`.
- **L2, first draft.** It said a Human origin is "refused". Corrected: `Human` is returned when no marker is set (`caller.rs:76-77`), and refusal applies only to `Refused` origins on identity-needing actions (`cli/mod.rs:1170-1173`).
- **Skill path.** Rechecked that it comes from the resolved config root, not a hard-coded `~/.claude` (`claude.rs:1516-1523`), so the exposure is conditional.
- **OpenCode file and line citations** come from the primer and were not re-read against an OpenCode clone in this pass. No clone of anomalyco/opencode is present under `scratchpad/alts`; `opencode-sentinel-src` is a different project.
