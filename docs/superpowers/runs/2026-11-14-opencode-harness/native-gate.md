# opencode native gate (ht-vxdp.6, design D12 tier 2)

Date 2026-11-14. Test: `tests/opencode_native.rs` (own `[[test]]` target `opencode_native`, nextest group `named-seats`). It is opt-in: it runs only with `HT_OPENCODE_BIN=<opencode executable>`. Without the variable it prints `skipped: HT_OPENCODE_BIN not set` and passes.

```sh
HT_OPENCODE_BIN=$HOME/.opencode/bin/opencode nice cargo test --locked --all-features --test opencode_native -- --nocapture
```

## How it runs

- **Drive mode: `opencode serve --hostname 127.0.0.1 --port <free>`, prompted over its HTTP API** (`POST /session`, `POST /session/{id}/message`, `GET /session/status`).
  - Plugins load in the server process at the directory's first request. The heartbeat's `pid` is the serve process.
  - The process stays up and idle between turns, so the plugin's own wake can be seen.
  - `opencode run` exits after one turn, so it cannot show a wake. The TUI needs a terminal, and with `--port` it is reached through the same API anyway.
  - In serve the plugin's client goes over the server's own URL. In the TUI without `--port` it uses the in-process fetch. The plugin's code is the same in both.
- **Model: an in-process scripted OpenAI-compatible server**, `@ai-sdk/openai-compatible` on an ephemeral loopback port.
  - `RUNBASH: <cmd>` gets a `bash` tool call. `RUNTASK: <cmd>` gets a `task` tool call whose prompt is `RUNBASH: <cmd>`.
  - Every request body is kept, so the test asserts exactly what reached the model's context.
  - There is no credential.
- **No network.**
  - `NPM_CONFIG_REGISTRY` points at the mock, which answers 404. opencode's background `@opencode-ai/plugin` install then fails in under a second. Without the redirect it retries for about 70 s (observed).
  - When `unshare -rn` works, the test re-executes itself in a private network namespace with loopback only. `HT_OPENCODE_GATE_NETNS=0` opts out, and every proxy variable then points at a closed loopback port.
  - Both modes passed.
- **Isolation.**
  - The named-seats rig: its own daemon and state dir.
  - Scratch `HOME`, the four `XDG_*_HOME`, `TMPDIR` and `OPENCODE_CONFIG_DIR`.
  - `HERDR_PLUGIN_STATE_DIR` is set so the model's own `herdr-threads` commands reach the rig's state.
  - The opencode seat is a named seat (`HERDR_THREADS_SEAT_NAME=oc-gate`), so no Herdr pane or server is used.
  - The sender is a second named seat, a Claude stand-in. Person seats need a Herdr pane: `human send` from a named caller is refused with "named person seats are not supported".
  - Every child is spawned through `test_support::spawn`. The scratch root is removed at the end.

## Results (opencode `1.18.1` observed, not pinned; run inside the toolbox, gate ~8 s)

| # | Item | Result |
|---|---|---|
| 1 | `setup opencode --with-permissions` into the scratch config root installs `plugins/herdr-threads.js` and `skills/herdr-threads/SKILL.md`; opencode loads the plugin | shown: the heartbeat has `role: active`, `launch: true`, the serve pid, every capability true, and the permission grant applied; `doctor --json` reports the heartbeat `state: loaded` (`setup-status` reports the install, not the heartbeat); `unsetup` removes the plugin |
| 2 | First prompt → SessionStart check-in → top-level binding | shown: a binding with `harness='opencode'`, `native_session=ses_…` and `observation_provenance=cooperative_top_level`, the seat named `oc-gate`; `Your seat` text reached the model in the user message. Enrolled on turn 1 in the final runs, turn 2 in earlier ones (the first check-in can answer `check-in unavailable (CallerUnverified)` until the daemon's named-host pass has observed the new seat, as the named-seats rig documents; the plugin retries next turn, by design) |
| 3 | Shell env carries the session variables; a model command is attributed | shown: the bash tool saw `HERDR_THREADS_OPENCODE_SESSION=<ses>`, `_PARENT=` (empty), `_PID=<serve pid>`; `herdr-threads inbox --machine` and `send --seat alice` succeeded, and alice received the message from the opencode seat |
| 4 | Delivery into context; ACK provenance | shown: a p1 send was appended (`delivered via append` with the stored `msg_…` row in the ledger) and appeared in the next turn's request body; an ACK-requested send was appended and woken, and its receipt is `acked` with `action_provenance=cooperative_mod_delivery`, `via=append`, `row=msg_…` |
| 5 | Wake | shown: the ACK request (p0) started a plugin turn whose last user text starts with `herdr-threads: wake up` and whose context holds the item; a p1 send at T = 0 started no model call in 5 s and left the session idle |
| 5b | Non-default agent (review 2, item 1) | shown: with a configured primary agent `gatekeeper` (its prompt marks its model calls), a turn prompted with that agent took a mid-turn p1 append, and the step that read the append ran with the agent's prompt; a p0 then woke the idle session, and the woken turn ran with it too. The stored append, the wake prompt and their assistant steps carry `agent: gatekeeper`. With the plugin's agent fields removed the gate fails at the step after the append (checked once, 2026-11-14) |
| 6 | Subagent | shown: a scripted `task` child got the child restriction in its first message (SubagentStart); its shell saw `_PARENT=<root ses>`; its `herdr-threads send` was refused by the CLI ("subagents may read and summarize; only top-level agents check in, accept or ACK", code `unsupported`) and nothing was sent |

Not covered here, per D12 tier 2's longer list: `/new`, session switch and resume, both TUI-only (no event reaches a plugin). Also not covered: append while busy, the typed-draft wake (TUI composer), and wake while busy. The real-model wake e2e covers the TUI.

## Bugs found

None in the plugin or the adapter. Two harness-side facts, kept in the test:

- opencode's HTTP server keeps the connection open despite `connection: close`, so a client must read by `content-length` or chunked framing.
- The background dependency install blocks the instance bootstrap until it fails, which takes about 70 s against a refused proxy. The registry redirect avoids that wait.

## Startup dependency install (PR roast 1, F2)

Measured 2026-11-14 on opencode `1.18.1` (observed, not pinned), model-free, with the herdr-threads plugin as the only configured plugin.

**Method.**
- Driver: a throwaway script, not committed. Each run used its own scratch root: `HOME`, the four `XDG_*_HOME`, `TMPDIR`, and `OPENCODE_CONFIG_DIR` set to `$XDG_CONFIG_HOME/opencode`, so there is one config dir. There was no credential. The model was a custom provider on a dead loopback port, and `OPENCODE_DISABLE_AUTOUPDATE`, `_MODELS_FETCH`, `_LSP_DOWNLOAD` and `_SHARE` were set.
- The plugin was this branch's file with its launch line pointing at a stub `herdr-threads` that logs a timestamp per hook call. The stub prints nothing, so the session never binds and no `watch` runs. The heartbeat's `loaded_at` marks the end of the plugin's init.
- "Offline" means a private network namespace with loopback only (`unshare -rn`, as the gate does). "Online" means the host network with the real npm registry, a read-only fetch into the scratch dirs.
- Serve runs: `opencode serve` on loopback, then `POST /session`, then a `noReply` message, which fires `chat.message` and so the first hook. TUI runs: the TUI in a private tmux, 120x40, with the namespace entered inside the pane. `hi` + Enter was typed 1 s after the composer was drawn.

| Run | Plugin loaded (heartbeat) | First hook | Notes |
|---|---|---|---|
| serve, offline, no plugin configured | n/a | n/a | `POST /session` answered at 0.7 s: no wait without a plugin |
| serve, offline, first start (never installed) | 70.8 s | 70.8 s | `POST /session` blocked 70 s; the log has `background dependency install failed`; nothing written to the config dir |
| serve, offline, second start (still never installed) | 70.8 s | 70.9 s | the same wait on every start until one install succeeds |
| serve, online, first start | 4.8 s | 4.8 s | installs `@opencode-ai/plugin` `1.18.1` into the config dir: `package.json`, `package-lock.json`, `node_modules/` (62 MB) |
| serve, online, later start | 0.6 s | 0.7 s | installed dir reused |
| serve, offline, after a successful install | 0.6 s | 0.6 s | no network needed once installed |
| TUI, offline, first start | 71.8 s | 73.5 s | **the screen stays blank until 72.5 s**: no composer, and keys typed before it appears are lost (a `hi` typed at 3 s never arrived); the first prompt typed after it ran its hook at once |
| TUI, offline, after a successful install | 1.6 s | 3.3 s | composer at 2.2 s |

Side observation: copying only `node_modules/` and `package.json` (no `package-lock.json`) into a fresh offline root still waited 71.8 s. Only a complete install skips the wait.

**Source** (`anomalyco/opencode` v1.18.1):
- `config/config.ts` forks an `@opencode-ai/plugin` install for every config directory at every instance bootstrap, detached.
- `plugin/index.ts:180` joins those installs (`waitForDependencies`) before it loads external plugins, only `if (plugins.length)`.

So the wait comes from configuring any plugin, ours included. A user on a network sees it once, about 5 s, at the first start after setup. A user without registry access waits about 70 s, behind a blank TUI, on every start until one install succeeds. No cheap doctor signal was added: the wait joins the installs of every config directory opencode reads, project `.opencode/` directories included, so checking one root's `node_modules` would be a partial answer.

## Hygiene

- The real `~/.config/opencode`, `~/.local/share/opencode`, `~/.local/state/opencode`, `~/.cache/opencode` and `~/.opencode` kept their mtimes, and no file under them was newer than the session start.
- `scripts/check-no-leaked-processes --run-id` passed, both in the toolbox and on the host.
- One early `opencode --version` was run outside isolation. It wrote nothing (checked by mtime).
