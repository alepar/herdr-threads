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
| 6 | Subagent | shown: a scripted `task` child got the child restriction in its first message (SubagentStart); its shell saw `_PARENT=<root ses>`; its `herdr-threads send` was refused by the CLI ("subagents may read and summarize; only top-level agents check in, accept or ACK", code `unsupported`) and nothing was sent |

Not covered here, per D12 tier 2's longer list: `/new`, session switch and resume, both TUI-only (no event reaches a plugin). Also not covered: append while busy, the typed-draft wake (TUI composer), and wake while busy. The real-model wake e2e covers the TUI.

## Bugs found

None in the plugin or the adapter. Two harness-side facts, kept in the test:

- opencode's HTTP server keeps the connection open despite `connection: close`, so a client must read by `content-length` or chunked framing.
- The background dependency install blocks the instance bootstrap until it fails, which takes about 70 s against a refused proxy. The registry redirect avoids that wait.

## Hygiene

- The real `~/.config/opencode`, `~/.local/share/opencode`, `~/.local/state/opencode`, `~/.cache/opencode` and `~/.opencode` kept their mtimes, and no file under them was newer than the session start.
- `scripts/check-no-leaked-processes --run-id` passed, both in the toolbox and on the host.
- One early `opencode --version` was run outside isolation. It wrote nothing (checked by mtime).
