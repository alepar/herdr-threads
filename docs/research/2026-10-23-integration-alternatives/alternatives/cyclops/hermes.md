# Cyclops x Hermes Agent: message injection and wake-up

- Alternative: Cyclops, https://github.com/cyclops-team/cyclops. Clone read (untrusted, not executed) at commit `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06). Paths written `cyclops:<path>:<line>` are relative to that clone root.
- Harness: Hermes Agent, https://github.com/NousResearch/hermes-agent. Clone read (untrusted, not executed) at `f3b5b980d6158794beb3d1044fd7b8ca4b8f0b23` (2026-10-23). Paths written `hermes:<path>:<line>` are relative to that clone root. The shared harness reference is `../../primers/hermes.md`.
- Tags: **VERIFIED** means read in source or docs. **INFERRED** means reasoned from those and not observed. **unknown** means not found. Neither Cyclops nor Hermes was run for this report.

## TL;DR

- Cyclops supports Hermes only through a **docs-only, unmeasured manifest** (`version_tested = "unverified"`). It was written on 2026-09-04, about six weeks before Hermes moved every session into one gateway process (v0.22.1). VERIFIED `cyclops:resources/manifests/hermes.toml:1-8,39`.
- **Injection and wake are one act.** The daemon pastes a single doorbell line into the Hermes composer with tmux bracketed paste, then presses Enter. The line is `[cyclops from A to B] <summary> | cyclops inbox claim m-att_<token>`. The body never enters the pane. The model has to run the claim command with its terminal tool to read the body. VERIFIED `cyclops:docs/development/DELIVERY.md:127-146`, `cyclops:resources/manifests/hermes.toml:102-108`.
- **There is no draft guard and no modal or permission guard for Hermes.** The manifest has no `human_input` composer rule and no blocked-state rules, and the gate holds only on positive evidence. The doorbell is therefore pasted and submitted into whatever has focus, whether that is a human draft, a busy turn, Hermes' dangerous-command approval overlay, or a sudo or secret prompt. Its Enter can **approve a pending dangerous command**, because the first approval choice, "once", is preselected. The gate behaviour is VERIFIED; the outcome in the pane is INFERRED (see Limitations).
- **Hooks give no injection and no ACK.** Only `on_session_start` and `pre_approval_request` are wired. In the daemon they count as diagnostic "hooks are firing" liveness and nothing more. VERIFIED `cyclops:resources/manifests/hermes.toml:58-64`, `cyclops:src/cyclopsd/src/ack.rs:177-198,244-300`.
- **On Hermes 0.22.1 and later the identity model probably breaks.** Cyclops attributes hook reports and `inbox claim` calls by walking the kernel process ancestry. Hermes now runs hooks and terminal tools in the shared gateway process, not under the pane. Claims are then likely refused or credited to the wrong pane (INFERRED, high confidence; see Limitations).

## Support status

| Item | Finding | Evidence |
|---|---|---|
| Listed? | Yes, as manifest `hermes` ("Hermes Agent"), among the website's *unverified* agents | VERIFIED `cyclops:resources/manifests/hermes.toml:36-43`; `cyclops:website/src/lib/components/Compatibility.svelte:22,47`; compiled in at `cyclops:src/cyclops/src/manifests.rs:137-138` |
| Measured? | No. "UNVERIFIED best effort from primary documentation, written 2026-09-04. No hermes binary was run and nothing was measured: this file binds the pane and declares the documented hook contract and nothing more." | VERIFIED `cyclops:resources/manifests/hermes.toml:2-4` |
| Class | "Generic": the universal tmux doorbell path, plus hook wiring rendered from the manifest's `[hooks.wiring]` table rather than a Rust template | VERIFIED `cyclops:src/cyclops/src/wiring.rs:1-17`; `cyclops:docs/guides/install.md:523` |
| Doc drift | `STATUS.md` says "Twelve detection manifests ship" and does not name Hermes. The directory holds 52 | VERIFIED `cyclops:STATUS.md:44-49`, `cyclops:tests/e2e/parity-check.sh:642` |
| Target Hermes version | Pre-gateway. The manifest's sources are the Hermes hooks, CLI and cli-commands docs of early September 2026. Hermes v0.22.1 (October 2026) routes classic CLI turns through the gateway | VERIFIED `cyclops:resources/manifests/hermes.toml:5-8`; primer section 1 |
| Project activity | 14 stars, 0 forks, last push 2026-09-08T06:48Z | VERIFIED, `gh api repos/cyclops-team/cyclops`, 2026-10-23 |

Verdict: **nominally supported, effectively unvalidated.** Everything Cyclops does for Hermes is its generic tmux mechanism. The Hermes-specific data (screen rules, hook names) is unmeasured, and part of it no longer matches current Hermes (see the toolbar glyph under Limitations).

## Injection mechanism

### Channel: the pane, by tmux bracketed paste

There is no hook output, no MCP server, and no use of Hermes' plugin APIs (`ctx.inject_message`, `pre_llm_call` context, `transform_tool_result`) or of its gateway RPC (`prompt.submit`, `session.steer`).

Code path (VERIFIED):

1. **Accept.** `msg.send` appends the message and one mailbox entry per recipient to the journal and fsyncs before replying. One FIFO worker runs per recipient. `cyclops:docs/development/DELIVERY.md:38-49`.
2. **Gate.** `admit`, at `cyclops:src/cyclopsd/src/delivery/gate.rs:1203-1325`:
   - pane present and alive (`:1209-1215`); a raw send returns here (`:1217`);
   - not in copy-mode (`:1220`); a manifest binds the pane (`:1224`);
   - one fresh capture (`:1230`); named blocks from screen rules (`:1242-1286`);
   - `composer_is_held` (`:1294`);
   - foreground binding (`:1305-1324`).
3. **Spool and paste.**
   - `load-buffer` into a per-attempt buffer, then `paste-buffer -p -d` (bracketed, deleted after use). `cyclops:src/cyclopsd/src/delivery/inject.rs:78-82,88-100,134`.
   - The manifest declares `method = "load-buffer + paste-buffer -p"` and `submit = "Enter"`. `cyclops:resources/manifests/hermes.toml:102-104`.
4. **Readback, then Enter.**
   - One capture checks whether the row reads back exactly. Either way Enter is pressed once: the record is `submitted` on an exact readback and `submitted_unverified` otherwise. `cyclops:docs/development/DELIVERY.md:63-68`.
   - Exact composer extraction needs `composer_prompt_regex` and `composer_continuation_regex` (`cyclops:src/cyclops-manifest/src/lib.rs:985-990`) plus an `IdleWithInput` rule (`cyclops:src/cyclopsd/src/delivery/inject.rs:635-645`), and `hermes.toml` declares none of them. With no proof the inject path returns an empty payload proof and the record is `submitted_unverified` (`inject.rs:227-237`, `gate.rs:667,714-718`). INFERRED: every Hermes doorbell records `submitted_unverified`.
5. **The model pulls the body.**
   - The submitted line becomes a Hermes user turn. The skill tells the model to run `cyclops inbox claim m-att_...` exactly as written. `cyclops:skills/cyclops/SKILL.md:200-233`.
   - The claim returns the envelope: `[cyclops <id>] TO: .. FROM: .. SUBJECT: ..`, then the body, then a `Reply:` hint, then `[cyclops:end <id>]`. `cyclops:skills/cyclops/SKILL.md:267-283`; the claim output is printed by the CLI at `cyclops:src/cyclops/src/main.rs:2992-3005` (`TO:` only when the daemon carries the recipient label). The daemon's `render_payload` (`cyclops:src/cyclopsd/src/delivery/terminal.rs:13-28`) has no `TO:` field.

### Payload format (VERIFIED, `cyclops:docs/development/DELIVERY.md:129-139`)

```text
[cyclops from implementer to reviewer] The rate limiter is ready for review. | cyclops inbox claim m-att_--AAAAAAQACAAAAAAAAAAQ
```

- **Recipients in the header:** up to three labels, then `+N`, or `to all`.
- **Token:** the 22-character token encodes the 128-bit attempt id.

### When it fires

As soon as the gate admits the attempt. There is no notion of a turn boundary: the gate does not hold for a working agent (`cyclops:docs/development/DELIVERY.md:96-103`). For Hermes this means:

- **Idle:** the line starts a turn.
- **Busy:** the outcome depends on Hermes' `display.busy_input_mode`. The default is `interrupt`, so Enter during a run redirects or interrupts the running turn. VERIFIED `hermes:hermes_cli/config_defaults.py:829`; primer section 9.
- **Modal open:** Enter goes to the modal (see Limitations).

The manifest itself says `safe_states = ["idle"]` (`cyclops:resources/manifests/hermes.toml:107`). The daemon never reads that field: `safe_states` occurs only in the struct and in tests (`cyclops:src/cyclops-manifest/src/lib.rs:936`). VERIFIED by grep.

### Size bounds

- **Summary:** a derived summary is one line of at most 200 characters. A sender-supplied summary has **no cap**; the CLI only warns above 160. VERIFIED `cyclops:docs/development/DELIVERY.md:40-44`.
- **Body:** has no pane bound, because it never goes to the pane. Cyclops itself sets no claim-output bound. The claim runs through Hermes' terminal tool, which keeps 40% head and 60% tail of output over 50,000 chars by default (configurable under `tool_output`) around a `[OUTPUT TRUNCATED ...]` marker (`hermes:tools/terminal_tool_result.py:151-154,230`, `hermes:tools/tool_output_limits.py:13`, `hermes:tools/tool_output_truncate.py:1-30`, hermes-agent at `f3b5b980`). VERIFIED. A larger body loses its middle while Cyclops has already marked it claimed (INFERRED).
- **Hermes paste collapse:** Hermes collapses a paste containing 5 or more newlines (6 or more lines) or 2,000 or more characters into `[Pasted text #N: L lines → <file>]` and expands it again on submit. VERIFIED `hermes:hermes_cli/cli_tui_mixin.py:1666-1673,1700-1707`; primer section 9.
  - A normal doorbell is one short line and is unaffected.
  - A sender summary of 2,000 or more characters would collapse. Readback would fail, but the text would still expand on submit (INFERRED).

### Framing

- **Per message:** one header line plus the claim command. The doorbell carries no static explanation.
- **Once per session:** the skill explains the protocol (`cyclops:skills/cyclops/SKILL.md`, 24,772 bytes). It is seeded at `~/.hermes/skills/cyclops/SKILL.md`. VERIFIED `cyclops:docs/guides/install.md:523`, `cyclops:src/cyclops/src/wiring.rs:801-805`.

### Dedupe and ACK

- **Dedupe:** strict per-recipient FIFO and one Enter per attempt. A repeat claim returns the same payload and "creates no second task". A claim made before the write withdraws the doorbell, so nothing is pasted. VERIFIED `cyclops:skills/cyclops/SKILL.md:278`; `cyclops:docs/development/DELIVERY.md:148-155`.
- **No hook ACK on Hermes.** The manifest declares no `ack`, no `turn_start` and no `turn_end`: "There is NO prompt-submit or stop event (pre_llm_call fires per model call, not per turn), so no start, ACK or end is declared". VERIFIED `cyclops:resources/manifests/hermes.toml:17-23,45-56`.
  - INFERRED: that reasoning is weaker than it looks. Hermes' `pre_llm_call` fires **once per turn** and its payload carries `user_message` and `turn_id` (primer section 2.2; `hermes:website/docs/user-guide/features/hooks.md` documents it as the `UserPromptSubmit` equivalent). It could have served as a payload-matched ACK.
- **Receipt tiers left for Hermes:**
  - **Screen tier.** Tier-2 evidence is `working_seen || (changed && id_staged)` (`cyclops:src/cyclopsd/src/delivery/gate.rs:2767`). `working_seen` needs the manifest's `status_working` rule ("thinking…" or "running…") to match. `id_staged` needs a composer extraction Hermes lacks (INFERRED false).
  - **No verifier.** After `SCREEN_ACK_DEADLINE` (5 s) the attempt closes as `notified` with no verifier. `cyclops:docs/development/DELIVERY.md:69-74`.
  - **Claim.** A claim by the exact recipient after Enter is a receipt. `cyclops:docs/development/DELIVERY.md:73-74`.

## Wake mechanism

### Exact mechanism

The doorbell's Enter *is* the wake. A pasted line plus Enter submits a user turn in Hermes' prompt_toolkit composer. VERIFIED `cyclops:skills/cyclops/SKILL.md:209-211` ("This is the wake-up signal, not the complete message"). Nothing in Cyclops is Hermes-native: it uses no `ctx.inject_message`, no gateway `prompt.submit`, no `/queue`, and no background-process notification.

### Idle and busy detection

The manifest's screen rules (VERIFIED `cyclops:resources/manifests/hermes.toml:66-100`):

| Rule | State | Region | Evidence flag |
|---|---|---|---|
| `title_useless` | unknown | pane title | UNMEASURED |
| `status_working` | working | bottom 4 non-empty lines contain `thinking…` or `running…` | `lifecycle_evidence = false` |
| `composer_empty_with_toolbar` | idle | bottom 4 lines match `^\s*❯\s*$` **and** `^⚕ .*│` | `lifecycle_evidence = false`; **no `composer_semantic`** |

None of these rules decides whether a doorbell is written:

- **No hold rule.** `composer_is_held` holds only for a `composer_semantic = human_input` match, an `IdleWithInput` state, or a delivery-owned staged or turn hold. The Hermes manifest has no rule that can produce the first two. VERIFIED `cyclops:src/cyclopsd/src/fusion.rs:2386-2398`.
- **Unknown state still passes.** When the foreground leader is the agent process itself, as in the classic CLI, the gate admits even when the state is `Unknown`: `binding.leader == binding.agent || det.state != AgentState::Unknown`. VERIFIED `cyclops:src/cyclopsd/src/delivery/gate.rs:1305-1313`.
- **`write_ready` never authorizes.** It is `false` for Hermes (`no_write_safe_composer_evidence`, `cyclops:src/cyclops-proto/src/state.rs:818-826`), but delivery treats it as "only a receipt-latency hint ... can never authorize a write" (`cyclops:src/cyclopsd/src/messaging_runtime.rs:173-178`).
- **Liveness idle admission is unreachable.** `liveness_admits_idle` requires a screen rule with `composer_semantic == Clean`, and Hermes has none. VERIFIED `cyclops:src/cyclopsd/src/fusion.rs:2428-2452`.
- **Conflicting docs, resolved by source.** `DELIVERY.md:105-109` says unverified manifests are "effectively a raw write with a receipt"; the goose, opencode and pi manifests say deliveries "fail closed (not write-ready)" (`cyclops:resources/manifests/goose.toml:25-26`, `opencode.toml:25-26`, `pi.toml:28-30`), but write-readiness never authorizes or refuses a write (previous bullet). Admit's source settles it for the classic CLI: the doorbell is written. VERIFIED.
- **The Ink TUI (`hermes --tui`) is most likely admitted too.** It is a `node` child started with plain `subprocess.call`, with no `start_new_session` (`hermes:hermes_cli/main_tui_launch.py:566-570`), so `node` stays in the Hermes process group. Cyclops' "foreground leader" is the tty's foreground process-group id (`tpgid`, `cyclops:src/cyclopsd/src/identity.rs:1065-1078`, via `fusion.rs:1915,2134-2136`), which is then the Hermes Python process itself, so `binding.leader == binding.agent` and an `Unknown` screen still admits (`gate.rs:1306-1315`). `foreground_not_agent` (`gate.rs:1316-1317`) fires only when a process in its own group owns the terminal. INFERRED from code (assumes the shell made `hermes` the group leader and Ink does not call `setpgid`); not run.

### Draft safety

**None for Hermes.** Cyclops' own documentation says so: "the seven unverified ones do not [carry a human-input rule], so a doorbell to one of those panes never detects a draft and is effectively a raw write with a receipt". VERIFIED `cyclops:docs/development/DELIVERY.md:105-109`; the skill repeats it for `--raw` at `cyclops:skills/cyclops/SKILL.md:247-249`.

- **No clear key.** The manifest declares no `clear_keys` (`cyclops:src/cyclops-manifest/src/lib.rs:925-928`), so Cyclops has no measured way to discard a botched paste.
- **The paste joins the draft.** A doorbell pasted into a half-typed human draft is appended to it, and Enter submits both (INFERRED from prompt_toolkit buffer semantics).
- **Gap race, for all vendors:** "A person can type between the final capture and the tmux write; there is no input lease across that gap." VERIFIED `cyclops:STATUS.md:106-108`.

### Latency

- **Design:** event-driven, with no polling, no timers and no retry clock. The delay is one fresh capture, spool, paste, a bounded readback re-read, then Enter. VERIFIED `cyclops:docs/development/DELIVERY.md:50-68`.
- **Hermes numbers:** unknown; none were measured.
- **Total to body:** the doorbell turn plus one model tool call (`inbox claim`) before the model sees the body (INFERRED).

### Failure modes

- **Held forever with no draft protection.** A held attempt waits for a pane event and pings the admin once after 120 s (`cyclops:docs/development/DELIVERY.md:24,54-56`). On Hermes the composer holds that can fire are only delivery-owned (an unconsumed doorbell or the turn it started), plus `foreground_not_agent` when a separate process group owns the terminal; a human draft never holds.
- **No automatic retry after a write.** A paste or submit failure becomes `attention_required`, and only an operator requeue resends it (`cyclops:docs/development/DELIVERY.md:76-80,228-231`).
- **A daemon restart after the paste** closes an attempt at `writing`, `submitted` or `submitted_unverified` as `attention_required`, unless the recipient already claimed it (then `notified`); pre-write attempts are rescheduled (`cyclops:docs/development/DELIVERY.md:203-210`).
- **Hermes-specific hazards** are listed under Limitations: an approval overlay, a sudo or secret prompt, the first-use hook consent prompt, a busy interrupt, and gateway attribution.

## Setup and installation footprint on Hermes

- **Opt-in.** Wiring happens only through `cyclops start --setup-only --wire-hooks`, which `scripts/install.sh` runs, and on later boots once consent is recorded in `~/.cyclops/vendor-wiring-consented`. It is skipped when `~/.hermes` does not exist; `CYCLOPS_NO_VENDOR_HOOKS=1` declines it. VERIFIED `cyclops:docs/reference/hooks.md:113-145`.
- **File written: `~/.hermes/config.yaml`.**
  - Without an existing column-0 `hooks:` key, Cyclops appends a fenced block: `# cyclops:begin` / `hooks:` / one entry per event / `# cyclops:end`.
  - With an existing bare `hooks:` key, the fenced entries go directly under it, at 2-space indent.
  - It refuses and asks for a manual merge when `hooks:` holds an inline value or an event is already configured.
  - The original is copied aside as `config.yaml.before-cyclops`. Uninstall removes only the fence.
  - VERIFIED `cyclops:src/cyclops/src/wiring.rs:35-39,187-196,511-620`; `cyclops:docs/reference/MANIFESTS.md:246-251,276-287`.

  ```yaml
  # cyclops:begin
  hooks:
    on_session_start:
      - command: "/path/to/cyclops hook on_session_start"
        timeout: 5
    pre_approval_request:
      - command: "/path/to/cyclops hook pre_approval_request"
        timeout: 5
  # cyclops:end
  ```

  The events come from `[hooks.wiring.events]` (`session_start = "on_session_start"`, `permission_request = "pre_approval_request"`, `cyclops:resources/manifests/hermes.toml:62-64`). Both are valid Hermes hooks (`hermes:hermes_cli/plugins.py:131,146`). VERIFIED.
- **Skill seeded:** `~/.hermes/skills/cyclops/SKILL.md`. It is created only below an existing private parent directory, never a new consumer directory. VERIFIED `cyclops:docs/guides/install.md:523`, `cyclops:docs/reference/hooks.md:126-130`.
- **Not handled:**
  - **Hook consent.** Cyclops sets neither `hooks_auto_accept` nor `--accept-hooks`. Hermes therefore asks `Allow this hook to run? [y/N]` on the CLI's terminal for each new `(event, command)` pair, and a gateway or non-TTY run silently leaves the hook unregistered. The manifest notes this and ships no modal rule for the prompt. VERIFIED `cyclops:resources/manifests/hermes.toml:24-28`; `hermes:agent/shell_hooks.py:748-768`; `hermes:website/docs/user-guide/features/hooks.md:2071-2081`.
  - **Profiles.** Only the default profile is wired; the path `~/.hermes/config.yaml` is fixed. Profiles under `~/.hermes/profiles/<name>/` and any `HERMES_HOME` override are not wired. VERIFIED from the manifest path and `cyclops:docs/reference/MANIFESTS.md:242` (home-relative paths only).
  - **No plugin, no MCP config, no system prompt edit.** VERIFIED (no other wiring shape for Hermes).

## Token cost

**Per message (INFERRED):**

| Item | Cost |
|---|---|
| Doorbell user turn | One line of about 100-250 bytes, roughly 30-70 tokens. It is a full new turn on an idle agent, so Hermes reprocesses the conversation, with whatever prompt caching the provider offers |
| Claim | One model-emitted terminal tool call (`cyclops inbox claim m-att_...`, about 20 tokens out) |
| Envelope | Body size plus about 100-200 bytes of framing (`TO/FROM/SUBJECT`, `Reply:` hint, end sentinel) |
| Reply | One more tool call (`cyclops reply ...`) |
| Busy recipient | An interrupt and redo under the default `busy_input_mode: interrupt` can cost far more than the bytes |

**Per quiet turn:** zero. No hook injects context; `on_session_start` returns are ignored, and the hook fires once per new session at the first turn (`hermes:website/docs/user-guide/features/hooks.md:907-925`). No polling. VERIFIED.

**Per session:** the skill is 24,772 bytes, about 6k tokens, when Hermes loads it. Hermes loads skills on demand, so only the description sits in the index (INFERRED from Hermes' skill model; not traced for this report).

**No priority or threshold:** every message wakes the recipient. INFERRED (no priority field in the delivery docs; see `overview.md`).

## Limitations and bugs

1. **The doorbell's Enter can approve a dangerous command.** VERIFIED in parts; the combined outcome is INFERRED. Hermes shows an approval overlay with choices `["once", "session", "always", "deny"]` and `"selected": 0` (`hermes:hermes_cli/cli_modal_mixin.py:1086-1095,1113-1123`). Enter while that overlay is up calls `_handle_approval_selection()` (`hermes:hermes_cli/cli_tui_mixin.py:1574-1577`), which submits the selected choice, `once` (`hermes:hermes_cli/cli_modal_mixin.py:1128-1149`). Cyclops' Hermes manifest has no `blocked_permission` or `blocked_modal` rule, so the gate sees `Unknown`, and for the classic CLI the leader is the agent, so it admits the doorbell (`gate.rs:1305-1313`).
   - **The wired hook does not prevent it.** The `pre_approval_request` hook is only an "attention edge" that never admits a pane (`cyclops:src/cyclopsd/src/selftest.rs:173-179`). No non-test path from a hook report to `BlockedPermission` was found by grep, so it cannot hold the gate either (INFERRED from grep). The manifest's `attention = "pre_approval_request"` key is not even a field of the `Hooks` struct (`cyclops:src/cyclops-manifest/src/lib.rs:288-350`), so serde drops it.
   - **Paste content does not matter.** A bracketed paste is one `Keys.BracketedPaste` event (`hermes:hermes_cli/cli_tui_mixin.py:1939`), not digit keystrokes, so the paste does not pick a numbered choice; the following Enter selects the default.
   - **Sudo and secret prompts:** Enter submits the buffer text, here the doorbell line, as the sudo password or secret value (`hermes:hermes_cli/cli_tui_mixin.py:1563-1572`).
2. **The hook consent prompt eats the doorbell.** On first use the CLI blocks in `input("Allow this hook to run? [y/N]: ")` (`hermes:agent/shell_hooks.py:762`). A doorbell pasted there is read as the answer. That answer is not `y`, so the hook is not approved and is asked again next time, and the doorbell is lost as a turn. Cyclops nevertheless records `notified` with no verifier after 5 s. INFERRED.
3. **A busy recipient's turn is interrupted.** The default Hermes `busy_input_mode` is `interrupt` (`hermes:hermes_cli/config_defaults.py:829`), and Cyclops never holds a working agent (`DELIVERY.md:100-101`). Unlike Claude, which queues typed input, a Hermes turn is redirected or interrupted by every doorbell that arrives mid-turn. INFERRED from both sources.
4. **Identity probably breaks on Hermes 0.22.1 and later.** INFERRED, high confidence, not observed.
   - **Ancestry is the authority.** Cyclops identifies callers only by peer credentials and an ancestry walk. "A process below an agent vendor inside a watched pane sends as that pane"; a process with no vendor ancestor is `admin` (`cyclops:docs/development/DELIVERY.md:257-266`). Hook reports are accepted only from inside the pane they report for (`cyclops:docs/reference/hooks.md:170-184`).
   - **Hermes moved execution out of the pane.** Since v0.22.1 every session's tools and shell hooks run in one gateway process. Shell hooks are spawned with `subprocess.Popen` from the agent's process, and that is the gateway (`hermes:agent/shell_hooks.py:480-507`; primer section 1).
   - **The gateway argv is not a vendor.** The gateway is started as `python -m hermes_cli.main ... gateway run` (`hermes:hermes_cli/gateway_runtime_start.py:52-79`). Cyclops' interpreter-aware matcher takes `hermes_cli.main` as the script basename, which does not match `hermes` (`cyclops:src/cyclopsd/src/fusion.rs:2896-2927`).
   - **Consequence (a):** if the gateway still descends from the first pane's `hermes` client, every Hermes session's `inbox claim` and hook report is attributed to *that* pane.
   - **Consequence (b):** if it was reparented, or runs as a service, they come from `admin`.
   - **Effect on claims:** the claim requires `record.recipient == claimant` (`cyclops:src/cyclopsd/src/mailbox/store.rs:303-309`). In case (b), and for every other pane in case (a), claims fail with `NotificationAttemptUnknown`. In case (a) the first pane's attempts can be claimed by the wrong session.
   - This is the same class of breakage the primer predicts for herdr-threads' own Hermes bridge (primer section 12, flag 1).
5. **The `on_session_start` liveness edge does not do what the manifest says.**
   - `admitting_edge_qualifies` compares the normalized event with `normalize_event("SessionStart")` = `sessionstart` (`cyclops:src/cyclopsd/src/ack.rs:177-198`, called at `:618`; normalization at `cyclops:src/cyclops-manifest/src/lib.rs:100-106`). Hermes reports `on_session_start` → `onsessionstart`, so the edge never qualifies as admission liveness. It only sets the `hooks verified` diagnostic (`ack.rs:244-300`). The daemon never reads `[hooks.wiring.events]` (grep: no use in `src/cyclopsd`). VERIFIED.
   - The hook also fires at the first *turn* of a new session, not at launch (`hermes:website/docs/user-guide/features/hooks.md:907-925`), so a freshly started, idle Hermes pane shows `hooks unverified`. VERIFIED.
6. **The screen rules have drifted from current Hermes.**
   - The idle rule requires a toolbar row matching `^⚕ .*│`. Current Hermes docs show ` ☤ claude-sonnet-… │ 12.4K/200K │ …`: a different glyph (☤, U+2624) with a leading space (`hermes:website/docs/user-guide/cli.md:224`). No `⚕`-prefixed status bar was found in `hermes_cli/`; `⚕` appears only in update notices (`hermes:hermes_cli/update_cmd_check.py:209-222`).
   - The default layout is now `modern`, with the status line under the composer (`hermes:hermes_cli/config_defaults.py:838-839`).
   - INFERRED: the idle rule never matches today, so Hermes panes read `Unknown`. That does not stop delivery (limitation 1), but it removes the only screen receipt evidence apart from `running…`.
7. **The reason given for "no ACK" overlooks a usable hook.** See Dedupe and ACK: `pre_llm_call` is per turn and carries `user_message`, so a hook-verified receipt was available.
8. **YAML merge by text.** Cyclops inserts its entries at a fixed 2-space indent under an existing `hooks:` key. If the operator's file indents `hooks:` children by 4 spaces, the result mixes indentation inside one mapping and probably fails to parse, which could break the whole Hermes config. INFERRED from `cyclops:src/cyclops/src/wiring.rs:572-598`; not tested.
9. **Unused manifest safety data.** `safe_states` and `unsafe_states` are declared (`hermes.toml:107-108`) but nothing in the daemon reads them, so they suggest a protection that does not exist. VERIFIED by grep.

## Would the generic mechanism work on Hermes, and how well

Using the primer:

- **Classic CLI (`hermes`, `hermes chat`), idle, empty composer, no modal:** it works. A one-line bracketed paste plus Enter is a normal user prompt, and Hermes handles bracketed paste (primer section 9). The model then needs a terminal tool call to claim. In gateway mode that call is subject to the attribution problem above, so on v0.22.1 and later the claim step is likely to fail even when the wake succeeds. INFERRED.
- **Busy:** poor. It interrupts by default. With `busy_input_mode: queue` it would behave like Claude.
- **Modal, approval, sudo, secret or consent prompt:** unsafe (limitations 1 and 2).
- **Ink TUI (`hermes --tui`):** likely admitted like the classic CLI (same process group), so the same hazards apply; the screen rules were written for the classic REPL. INFERRED.
- **Better Hermes-native paths that Cyclops ignores** (primer sections 4, 5 and 2.2):
  - `ctx.inject_message(..., session_key=…)` or gateway `prompt.submit` wake an idle session with no keystrokes, queue for a busy one, and cannot clobber a draft or answer a modal;
  - `session.steer` and `transform_tool_result` deliver mid-turn;
  - a `pre_llm_call` shell hook could inject the body and earn an exact ACK.

## Clever ideas worth stealing

- **Body out of band, behind a claim token.** The pane line is tiny and identical in shape for every vendor. The body sits behind an authenticated, idempotent claim (`m-att_` encodes the exact attempt). The token also makes a pre-write claim withdraw the doorbell, so nothing is pasted (`DELIVERY.md:148-155`). For herdr-threads' Hermes path, a short wake marker plus `pre_llm_call` delivery already avoids the extra tool call. A wake marker that carries an attempt id would still let us correlate "this turn was our wake" in the `pre_llm_call` payload's `user_message`.
- **Honest receipt vocabulary.**
  - States: `submitted`, `submitted_unverified`, `notified(verified_by hook|screen|none)`, `attention_required`.
  - Nothing is retried on a timer; operator requeue only.
  - Each fact is separate: accepted, then notified, then claimed, then completed.
- **Event-driven holds.** A held attempt is released by pane events, never by a timer, with one admin ping after a long hold.
- **Manifest-as-data with auto-dismiss modal rules** (`decline_keys`, bounded by `MAX_DECLINES`, screen re-read before the confirming key). For Hermes, an approval-overlay rule that *holds* is the missing half.
- **Fenced, reversible config edits.** Begin and end markers, a `.before-cyclops` backup, "wire twice is wire once", and "unwire restores the operator's bytes" (`wiring.rs:1-17`). A good pattern for any edit we make to a harness config.
- **Counter-lesson for us.** Kernel-ancestry identity is elegant, but Hermes' gateway shift breaks it outright. For Hermes, the stable key is the per-command `HERMES_SESSION_ID` the gateway injects (primer section 6), plus a pane-side witness. Herdr-threads should not depend on process trees or inherited env for Hermes either.

## Verification

Adversarial pass, 2026-10-23, against the same clones (cyclops `a84d44b`, hermes-agent `f3b5b98`) and `gh api`.

- Checked and confirmed: manifest contents and line ranges; `admit` steps and the `leader == agent || state != Unknown` clause; `composer_is_held`; `liveness_admits_idle`; `write_ready` as a hint only; `safe_states` unused outside tests; `attention` absent from the `Hooks` struct (no `deny_unknown_fields` on it); `on_session_start` normalizing to a non-admitting edge; the claim's `recipient == claimant` filter; the interpreter-aware argv matcher; the gateway's `python -m hermes_cli.main ... gateway run` argv; the DELIVERY.md, hooks.md, MANIFESTS.md, STATUS.md and install.md citations; Hermes' `busy_input_mode` default, `modern` layout, approval choices with `selected: 0` and Enter handling, sudo/secret Enter handling, bracketed-paste binding, consent prompt, `on_session_start` timing, `pre_llm_call` as the `UserPromptSubmit` equivalent, the `☤` status bar, and `⚕` only in update notices; skill size 24,772 bytes; repo stats.
- Changed: the Ink TUI claim (it was "held with `foreground_not_agent`"; Cyclops' leader is the foreground process-group id, and `node` shares Hermes' group, so it is most likely admitted); the claim-envelope renderer citation (the CLI's `print_claim_payload`, not the daemon's `render_payload`, which has no `TO:`); the paste-collapse threshold (5 newlines, not 5 lines); the "fail closed" attribution (pi also says it; goose and opencode say it of write-readiness, which does not gate); the restart failure mode (post-paste attempts only); line ranges for `lib.rs`, `ack.rs`, `selftest.rs`; the hold failure mode; repo stats now cited to `gh api` directly; the "never a gate block" claim for `pre_approval_request` marked INFERRED from grep.
