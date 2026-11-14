# opencode harness adapter

Epic `ht-vxdp`. Design 2026-11-14, coordinator seat `snSGlLkotnaR` (tab `opencode`). Status: draft, revised after design review 1 ([review-design-1.md](review-design-1.md)); and after the live probe ([probe-real-model.md](probe-real-model.md)); awaiting the user's decisions (§Decisions for the user).

## Goal

An opencode session in a Herdr pane (or on a named seat or herdr-core terminal) is a full herdr-threads seat, on a par with Claude Code: it enrolls on its own, gets its check-in context, receives message bodies in its context without calling `inbox`, is woken when idle, and supports `launch` / `handoff --kind opencode`.

It gets there as one more `HarnessAdapter` in `src/harness/opencode/` plus one dependency-free plugin file, on the existing neutral core.

## Inputs

- Surface research, with evidence and confidence tags: [research-opencode-surface.md](research-opencode-surface.md) and `captures/`. Observed on opencode 1.18.1; source read at `anomalyco/opencode` `v1.18.1`. Nothing below is pinned to that version.
- Worked example: the agy adapter run, `docs/superpowers/runs/2026-10-09-agy-harness-adapter/`.
- Normative: `TRUST-POLICY.md`, and `AGENTS.md` ("Harness-neutral core", "Harness and Herdr versions", "Delivery contract", "Token-efficient communication").

## What opencode offers (confirmed)

| Need | opencode surface | Confidence |
|---|---|---|
| Process identity | One Bun-compiled binary, one process (`comm=opencode`). Plugins run in a worker thread of it. A shell-tool command is a direct child. opencode exports `OPENCODE=1`, `AGENT=1`, `OPENCODE_PID`. | observed |
| Session id in the shell | None natively. The plugin hook `shell.env` gets `{sessionID, callID}` per tool call and adds variables; a subagent's call gets the child's id. | observed |
| Session lifecycle | Session created lazily at the first prompt (`session.created`). `/new` and session switch emit no event. `--continue`/`--session` keep the id, `--fork` and `/new` make a new one, compaction keeps it (`session.compacted`). | observed |
| Subagents | The `task` tool creates a child session with native `info.parentID`. Every hook fires with the child's `sessionID`; `parentID` comes from `session.created` or `client.session.get`. | observed |
| Add text without a turn | `client.session.prompt({noReply: true, parts})` stores a user message and returns it. Idle: no turn, in context for the next one. Busy: consumed by the next step of the running turn. | observed (mock model) |
| Start a turn from the plugin | `client.session.promptAsync` starts a turn on an idle session in-process, with no HTTP port, leaving a typed composer draft untouched. On a busy session the prompt is queued into the running turn. | observed (mock model) |
| Idle/busy | `session.status {sessionID, status: idle|busy|retry}` events. Permission and question waits stay `busy`. | observed |
| Turn-start and post-tool hooks | `chat.message` (push a synthetic text part, hidden in the TUI) and `tool.execute.after` (mutate the tool output). | observed |
| Install | Plugins load from `{plugin,plugins}/*.{js,ts}` in `$XDG_CONFIG_HOME/opencode` (and `OPENCODE_CONFIG_DIR`, `~/.opencode`, project `.opencode/`). Skills load from `skills/**/SKILL.md` in the same directories. | observed |
| Permission | `permission.bash` rules, last match wins; the plugin `config` hook receives the merged config, mutable. | rules observed; `config`-hook grant unconfirmed |
| Launch | `opencode [--model p/m] [--agent A] [-c | -s ID [--fork]] --prompt TEXT`; `--prompt` submits at startup. `--pure` disables plugins. | observed |

## Design

### D1. Shape: a push channel, woken by the plugin itself

opencode is declared like Claude's mod: `DeliveryPolicy = (PushChannel, cooperative_mod_delivery, IdleWake::Channel)`, `wake_text = WAKE_UP`.

The plugin runs `herdr-threads watch --harness opencode --session <id>` as a child, reads its schema-1 JSON lines, puts items into the session and reports `watch ack`. This reuses the daemon's whole push path (registry, stall detection, wake suppression, receipts) with no new protocol.

There is **no composer policy** in this epic: no composer read, no blocked-screen recognizer, no typing into the TUI. Reasons:

- The plugin wake is in-process, leaves the user's draft alone, and knows the session's exact busy state. A typed wake has none of these.
- The TUI is hostile to a typed wake: the Help overlay leaves an empty-looking composer visible, the session-screen empty composer has no placeholder, `ctrl+c` on an empty composer exits opencode, and the home-screen composer would start a new session.
- A push harness also wakes on named seats and herdr-core terminals, where no host can type.

The cost is the dormant cases in D8, where no channel is live. A typed fallback for those is a possible follow-up, not part of this epic.

### D2. Identity and session claim

- `AdapterMetadata`: id `opencode`, label "opencode", executable `opencode`, env marker `OPENCODE` (moved out of `src/harness/unsupported.rs`, where it is registered today as an unsupported harness), host kinds `["opencode"]` (the kind Herdr reports, probe item 5).
- `CallerEnvShape`: `process_names: ["opencode"]`, `pid_var: OPENCODE_PID`, `session_var: HERDR_THREADS_OPENCODE_SESSION`, and a role witness that reads `HERDR_THREADS_OPENCODE_PARENT`.
- The plugin's `shell.env` hook sets all three variables on every shell call that has a `sessionID` (tool calls and the user's `!shell`), so a nested opencode never inherits a stale value:
  - `HERDR_THREADS_OPENCODE_SESSION` = the call's `sessionID`;
  - `HERDR_THREADS_OPENCODE_PARENT` = the root top-level session id for a child (walked up `parentID`, bounded to 8 levels), empty for a top-level session, `?` when the lookup failed;
  - `HERDR_THREADS_OPENCODE_PID` = the plugin's own process id.
- Role witness: `HERDR_THREADS_OPENCODE_PID` must equal `OPENCODE_PID`, else `Unattributable` (the variables were inherited from an outer opencode, for example by a nested `--pure` one). Then an empty parent is `TopLevel`, a valid one is `Subagent{parent}`, anything else is `MalformedMarker`: an unknown role fails toward child.
- A `herdr-threads` call under an `opencode` process with no session variable is refused `session_claim_required` naming the variable (plugin missing or `--pure`). A user `!shell` command claims the session it was typed in, as a user's `!` command does under Claude.
- Session ids are validated as `ses_[0-9A-Za-z]{1,64}`.
- Liveness fingerprint: the `opencode` process (`OPENCODE_PID` record). One process serves one pane, so no `process_spans_seats`.

### D3. Enrollment and lifecycle

The plugin keeps one **bound session**: the top-level session whose last lifecycle check-in succeeded. Every top-level `chat.message` runs exactly one registered hook command (`herdr-threads … hook-event opencode <Event>`, payload on stdin), so the daemon hears from the session every turn, as it does for Claude and Codex, and a failed check-in is retried on the next turn:

| Trigger in the plugin | Event and payload | Adapter decode |
|---|---|---|
| `chat.message` for a top-level session that is not the bound one, first in this process | `SessionStart`, `source: startup` (or `resume` when the session was not created in this process) | `Lifecycle(Startup | Resume)`, top-level, with context |
| same, later in the process, session created in this process (`/new`, `--fork`) | `SessionStart`, `source: clear` | `Lifecycle(Clear)` |
| same, later, session older than the process (session switch) | `SessionStart`, `source: resume` | `Lifecycle(Resume)`: C1 continuity by native session id |
| `session.compacted` for the bound session (also mid-turn, on auto-compaction) | `SessionStart`, `source: compact`; its output is appended (D4) | compact re-entry (fixed hook section restated) |
| any other top-level `chat.message` of the bound session | `UserPromptSubmit` | ordinary turn hook: quiet while the channel is live, delivers pending items when it is not |
| first `chat.message` of a child session | `SubagentStart` | child restriction, answered locally, no daemon call |

- The payload is `{event, source, session_id, parent_session_id?, cwd}`, modelled on Claude's `SessionStart` so the decode needs no new core concept. The role is payload-declared from opencode's native `parentID`, the same class of evidence as Claude's and Codex's payload roles.
- The hook's context output is added to that same user message as a synthetic text part in `chat.message`, so the first turn sees its check-in. It is hidden in the TUI.
- `hook_admission_policy` is `RegisteredContract`; admission is versionless (`contract_declared`), no version probe.
- No `local_evidence`, no `hook_step`, no `turn_state_edge`: the channel serves delivery, and the plugin holds the turn state.
- A session is marked bound only when its lifecycle hook succeeded; until then every turn retries it. A watch refusal `no_binding` or `session_mismatch` clears the bound mark, so the next turn checks in again.
- The plugin's own appends and wake prompts also raise `chat.message` (probe). The plugin recognises them by the message ids and in-flight marks it keeps, never by payload shape: an append runs no hook, and a wake prompt runs the turn hook of the bound session and is never a lifecycle trigger.
- **One plugin instance per process.** opencode can run a plugin's init more than once per process, and can load a second installed copy from another config directory. The plugin keeps a process-global guard: a later instance takes over only after the earlier one's `dispose`, and a second copy stays inert. Doctor flags a second installed copy.
- A plugin reload loses the "created in this process" memory; the next lifecycle is then reported as `resume`, which is safe (C1 continuity by session id).
- Hook failure, timeout or daemon down: the plugin injects nothing and the turn proceeds; a lifecycle hook prints the shared `check-in unavailable` notice as the other harnesses do. A hook that throws can fail an opencode turn, so every plugin hook body is wrapped and never throws.

### D4. Delivery

One primitive: **append**. The plugin stores the framed page in the bound session with `client.session.prompt({noReply: true, parts: [{type: "text", text, synthetic: true}]})`.

- Framing is the existing one (`[herdr-threads] new messages:` then per-item header lines and indented bodies), produced in the plugin as `register.js` does, in the daemon's batch order. No new model-visible string.
- **Busy session:** the appended message is read by the next step of the running turn, and no wake is sent. A waking item appended while busy sets a **wake-pending** mark, cleared only by a model step whose message list held the append (its assistant message's `parentID` is the append or a later user message; [roast-pr-1-dispositions.md](roast-pr-1-dispositions.md) E1). If the session goes idle with the mark still set (the turn ended on its final text, an interrupt or a rejected permission), the plugin sends the wake. The plugin also re-reads the status after every append, because an append that lands just after the session went idle starts no turn (probe). This is the Claude mod's rule: the append is the delivery, the wake only guarantees it a turn.
- **Idle session, passive item** (not `wakes`, no ACK needed): appended, no turn. It is in context for the next turn. The session stays idle.
- **Idle session, waking item** (`wakes: true` or ACK-required): appended, then the plugin calls `client.session.promptAsync` with the content-free wake text `herdr-threads: wake up` (22 bytes, the existing `WAKE_UP`). The turn that starts sees the appended rows.
- The appended message is hidden in the TUI (a synthetic part shows no bubble, probe item 1); the user sees the model's reply, as with the Claude mod's hidden rows.
- **Ack:** after the append call returns the stored message (`data.info.id`), the plugin runs `watch ack --via append --row <msg id> <ids>`. The stored message id is the host evidence of entry that the `DeliveryPolicy` admission rule asks for, the same evidence class as the Claude mod's stored row. A truncated item is never acked. A failed append acks nothing and the item is offered again.
- The wake is reported as `via submit` with no ids, as the Claude mod does.
- The frame and batch-order code in the plugin is a third copy (after `src/delivery/frame.rs` and `register.js`); a parity golden pins it to the Rust framing. The plugin handles the whole watch-line protocol the mod does (attention lines, `presentation_epoch`, ack retry, restart on `binding_changed`, stop on `replaced` / `disabled`).
- **Channel not live** (stalled and in its cooldown, `mod_delivery` off, watch refused): the turn hook delivers pending items at each turn start, as for a hooks-only harness. There is no wake in that state.
- Idle gate for the wake: the bound session has no `busy` or `retry` status, no pending permission or question, no wake in flight, and the channel is live. Backoff after a failed submit follows the mod's rule.
- Child sessions never receive deliveries and are never woken.
- A woken turn can run in a session the TUI is not showing (D8), where a permission or question prompt blocks unseen. When the bound session raises `permission.asked` or `question.asked` during a turn the plugin started, the plugin shows a TUI toast naming the session. The toast is for the person; the model sees nothing.

Alternative considered and kept as the in-plugin fallback if the probe shows append is unreliable with a real model: inject at `chat.message` (turn start) and `tool.execute.after` (post-tool), acked `via context`.

### D5. Subagents

- A child session's shell calls carry both variables, so the CLI claims a read-only subagent binding of the parent (`cooperative_env_session`), exactly as agy children do. Writes are refused `subagent_read_only`.
- The child's first message gets the child restriction text (D3). No seat is allocated, moved or ended by a child.
- Nested children resolve to the root top-level session.

### D6. Setup, status, unsetup, doctor

`herdr-threads setup opencode` writes two owned files under the opencode config root, recorded in the setup manifest with digests, as the agy plugin directory is:

- `plugins/herdr-threads.js`: the plugin, embedded from `integrations/opencode/plugin/herdr-threads.js`, with a rendered launch line (absolute executable, `--state-dir`, `--host-endpoint`) as `register.js` has. It has no npm dependency, but configuring any plugin makes opencode wait for its own `@opencode-ai/plugin` install before loading plugins: about 5 s once online, about 70 s on every start without registry access until one install succeeds, with a blank TUI meanwhile (measured: [native-gate.md](native-gate.md), "Startup dependency install").
- `skills/herdr-threads/SKILL.md`: the shared skill plus an opencode section (its shell tool, the session variables). opencode dedupes same-named skills from `~/.agents/skills` and `~/.claude/skills` and picks the winner unpredictably (probe item 4). The copies share the common text, so the opencode-specific guidance also rides the lifecycle check-in (`OutputPolicy.extra_guidance`) and does not depend on which copy wins; doctor notes same-named copies.

Config root: `OPENCODE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`. `setup opencode` has no root flag; only the installer's internal `installer-integrations --config-root opencode=DIR` names another root. `opencode.json` is never edited, so JSONC is not a concern.

**Permission.** With consent (`--with-permissions`, asked interactively otherwise, as for agy), the rendered plugin adds `permission.bash["herdr-threads *"] = "allow"` and the same for `ht *` in its `config` hook. Without consent, opencode asks the user on the first call and its own "Allow always" covers the rest. The probe confirmed the mechanism. Rules are last-match-wins in key order, so the plugin removes and re-adds its two keys to place them last. A user rule that sets either exact key to `deny` is respected: the plugin adds nothing for it and doctor reports it.

Consent changes the rendered plugin, so the manifest records which variant was written and status does not report it as modified. `unsetup opencode` removes only the owned files. `status` and `doctor` report: plugin missing, modified or stale; skill missing; consent state. A plugin that failed to load is invisible to opencode's UI, so the plugin writes a small heartbeat record into the instance state dir at the end of its init, after its hooks are set up (code before a throw still runs, so an earlier write would prove nothing), and doctor reports "installed, never loaded".

No `installer_policy` in this epic: `install.sh` keeps handling Claude and Codex; opencode is set up by `herdr-threads setup opencode`, as agy and Hermes are.

### D7. Launch and handoff

`LaunchPolicy` for `--kind opencode`:

- Composes `opencode [caller args] --prompt <startup input>`; `--model M` maps to `-m M`; `HERDR_THREADS_OPENCODE_OPTS` for native options.
- Accepts `-m/--model`, `--agent`, `-c/--continue`, `-s/--session`, `--fork`.
- Refuses subcommands (`run`, `serve`, `attach`, `acp`, …), `--pure` (no plugin), `--auto` and its aliases `--yolo` / `--dangerously-skip-permissions`, `--port` / `--hostname` / `--mdns` (an unauthenticated server), `--mini`, and unknown flags.
- The startup input goes through `prepare_startup_input` (`before: ["--prompt"]`) and the model through `apply_model`; `compose_argv` leaves argv unchanged.
- Scope: an explicit `--profile-dir`, else the pane shell's `OPENCODE_CONFIG_DIR`, else its `$XDG_CONFIG_HOME/opencode`, else the launcher's root as setup resolves it, else `~/.config/opencode`; the readiness warning and the fingerprint read that root. An explicit `--profile-dir` is exported to the launched opencode as `OPENCODE_CONFIG_DIR`.
- Handoff uses the V2 bootstrap, as agy does. `--prompt` creates the session at startup, so a launched opencode enrolls at once.
- A `launch` with no startup input would leave a seat nothing can reach, because no session exists until a prompt. The policy then supplies the content-free prompt `herdr-threads: wake up`, so the launched session enrolls with one short turn.

### D8. What degrades

| Condition | Effect |
|---|---|
| Plugin not loaded (`--pure`, load error, not set up) | No enrollment, no delivery, no wake. CLI calls are refused `session_claim_required`. Doctor reports "never loaded". |
| opencode at the home screen with no session yet | Not enrolled until the first prompt. |
| Resumed by hand (`-c` / `-s`) | Bound again at its first prompt. Until then the seat has no live channel and a sender sees `wake=no_wake`. |
| `/new`, then idle at the home screen | The previous session stays bound and is still woken; its turn runs in a session the TUI is not showing, the home screen does not change, and Herdr reports the pane `idle` (its status is read from the screen). A permission prompt in that turn blocks unseen except for the plugin's toast. The next prompt rebinds to the new session. |
| `opencode serve` + `attach` | Plugins run in the server process, so its sessions are attributed to the server's pane, not the attaching client's. Existing accepted limit "foreground harness execution is user managed". |
| Channel stalled (10-minute cooldown), or `mod_delivery` off | Items arrive at the next turn start through the turn hook; no wake; the sender sees `wake=no_wake`. CLI calls show the `N pending` hint. |
| No permission consent | opencode asks the user per command until "Allow always". |
| A future opencode renames a hook or SDK call | Versionless: the plugin feature-detects each call. A missing `shell.env` loses CLI attribution only; a missing `noReply` falls back to hook injection; a missing `session.prompt` or `promptAsync` (detected before the first `watch`, or at the first wake) stops the channel, so the turn hook delivers and the daemon, seeing no channel, reports the seat `wake=no_wake`. Doctor reports the degraded capability from the heartbeat record. |

### D9. Delivery-contract check

1. Defaults unchanged (p1, T = 0).
2. The plugin wakes only on a line the daemon marked `wakes` or ACK-required.
3. Every pending item reaches context by append, with no `inbox` call.
4. A busy session is never prompted to wake; its items land in the running turn.
5. ACK requests and invitations are p0 and wake through the channel.
6. `IdleWake::Channel`. With no live channel there is no wake and the sender reads `wake=no_wake`.

Token rules: no new model-visible string except the opencode section of the skill and the launch/setup help fragments; sizes are stated in the commits that add them.

### D10. Core changes expected

The target is zero behaviour change in core. Known edits outside `src/harness/opencode/` and `integrations/opencode/`:

- `src/harness/mod.rs`, `registry.rs` `builtins()`: registration (allowlisted).
- `src/harness/unsupported.rs`: drop the `OPENCODE` marker.
- Tests and goldens that pin opencode as unsupported or pin the builtin list (listed in the file map).
- `src/host/native.rs:5765`: a test uses `opencode` as its unrecognized kind; it gets another name.
- opencode is the first `Channel` harness with no composer policy. `registry.rs` tests `idle_wake_on_agrees_with_has_idle_wake` and `idle_wake_on_spec_table`, and the builtin `match` in `adapter.rs` tests, need its row. Two behaviours must be checked and pinned by tests: the scheduler does not retry forever for a connected or dormant opencode binding with no channel, and the M12 line ("has no idle wake; threshold has no effect") is not printed for an opencode seat whose channel is merely not up yet. If either needs a core change, it is a neutral one keyed on `IdleWake::Channel` without a composer policy.
- The adapter declares `binding_turn_delivery = false` for bindings whose only evidence is a plain CLI call, so the `N pending` hint shows when neither hook nor channel serves the seat.
- Scripts: `check-harness-neutral` knobs, `wake-e2e.py`, `wake-e2e-herdr`, `validate-package.sh`, `herdr-release-smoke`.
- Docs: `README.md`, `integrations/skill/SKILL.md` session-id list, `integrations/opencode/README.md`, `docs/agent-usage.md`, `docs/compatibility/harnesses.md`, `AGENTS.md` (harness list, never-write list gains `~/.config/opencode` and `~/.opencode`), `CHANGELOG.md`, `TRUST-POLICY.md`.

Anything else found during implementation is reported to the coordinator before it is written, and to the `hermes-fix` coordinator when it touches `adapter.rs`, the registry, setup or launch plumbing.

Commits carry `Outside-adapter: <none | list>` naming every changed path outside `src/harness/opencode/` and `integrations/opencode/`.

### D11. TRUST-POLICY changes (same commits as the code)

No invariant is weakened and no provenance value is added.

- **Startup enrollment:** extend "Global hooks enroll supported top-level Herdr sessions" to the opencode plugin.
- **opencode session claim:** opencode exports no session variable; the herdr-threads plugin adds the session id to each shell-tool call from the hook's own `sessionID`. It is `cooperative_env_session` with a plugin as carrier. The child role comes from opencode's native `parentID`; an unknown role is refused, never read as top-level.
- **Push delivery evidence:** for opencode, `cooperative_mod_delivery` is claimed on the stored message the append returned, idle or queued into a running turn; the wake-pending rule guarantees a queued waking item a turn.
- **Accepted limits:** the D8 rows for `serve`/`attach`, the wake of a bound session the TUI is not showing, the hand-resumed session with no wake until its first prompt, and the consent-gated in-process permission grant.

### D12. Testing

1. **CI fixtures (default suite).** Adapter unit tests in `tests/harness/opencode_*.rs`: decode and encode, caller env and role witness, setup/status/unsetup with isolated roots, launch argv, doctor. Plugin logic tests run the plugin file under a fake `client` in the way `tests/harness/claude_mod.rs` drives `register.js`. Integration: watch registration and ack for `--harness opencode`.
2. **Native gate (opt-in, model-free).** `HT_OPENCODE_BIN` set: the real binary under isolated HOME and XDG dirs, a scripted OpenAI-compatible mock server grown from the research mock, a private tmux and an isolated daemon. Covers enroll, `/new`, session switch, subagent, resume, append while busy and while idle, the channel wake with a typed draft left intact, no wake while busy, unsetup. Not in the default suite.
3. **Wake e2e (opt-in, real model).** `scripts/wake-e2e.py --harness opencode`, a channel harness like Claude. Credential: `OPENROUTER_API_KEY` read from the Hermes env file at run time and passed only through a mode-0600 env file in the scratch root, removed at cleanup even with `--keep`; model `openrouter/deepseek/deepseek-v4-pro`. Never printed, logged, committed or kept in evidence. Not in the default release matrix.
4. **Herdr matrix.** `scripts/herdr-release-smoke` gains an opencode stand-in for `launch` and every handoff target, as agy has.

## Probe (done 2026-11-14, results in [probe-real-model.md](probe-real-model.md))

A real opencode under isolated dirs, the OpenRouter DeepSeek model and an isolated Herdr test session. It records evidence for:

1. A real model reads an appended `noReply` synthetic message, idle and mid-turn, and acts on it; how the message renders in the TUI.
2. `promptAsync` wake end to end, and into a session the TUI is not showing.
3. The `config`-hook permission grant.
4. Duplicate skill names (`~/.agents/skills` or `~/.claude/skills` beside the installed one).
5. The agent kind Herdr reports for an opencode pane, and its idle/working status.
6. Plugin init failure visibility and `--pure`.
7. An append during the final text step and during a rejected permission (the wake-pending cases).

Results: items 1-3 and 5-7 confirmed with `openrouter/deepseek/deepseek-v4-pro`; item 4 found the unpredictable skill winner. The findings are folded into D2-D8 above. No decision switched to its fallback.

## Decisions for the user

1. **Wake by plugin channel, with no composer typing** (D1). The original brief assumed a typed composer wake; the research shows the plugin wake is safer and simpler. Cost: the dormant cases in D8.
2. **Session claim through plugin-set variables** `HERDR_THREADS_OPENCODE_SESSION` / `_PARENT` (D2), since opencode exports none.
3. **Delivery by appending a stored message** (D4). It is hidden in the TUI, like the Claude mod's rows; the user sees the model's reply.
4. **Permission grant inside the plugin, with consent**, never by editing `opencode.json` (D6).
5. **Scope:** no typed fallback, no `install.sh` integration, no `serve`/`attach` support in this epic.

## Non-goals

A typed composer wake; `opencode serve` / `attach` / ACP / desktop and web clients; `install.sh` integration; a TUI plugin; macOS and Windows validation.
