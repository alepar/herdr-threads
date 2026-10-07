# Wrapper, TUI and server pane attribution

Research date: 2026-10-06. This records compatibility evidence and remaining limits; it does not establish a general client-session-to-pane locator. The launch change removes automatic daemon-mode arguments, but the combined seat-detection feature is not ready for main integration.

## Native Codex and wrapper argument handling

The installed executable `/Users/alepar/.local/bin/codex` resolves to
`/Users/alepar/.codex/packages/standalone/current/bin/codex`. Its isolated version probe reports `codex-cli 0.160.1`.
The user reports that `codex` on another machine is an alias to a wrapper script and that `codex --no-daemon` is rejected there. That wrapper is not available in this checkout, so its parser and exact rejection have not been independently inspected. Native support does not imply wrapper support.

A help/parser-only probe used an absolute native executable, a fresh temporary working directory, and an environment containing only PATH, isolated HOME, CODEX_HOME, CLAUDE_CONFIG_DIR and XDG_CONFIG_HOME. It started no server, made no model call, and removed its temporary directories:

| Native argv | Exit | Observed result |
|---|---:|---|
| `--no-daemon --help` | 0 | Help lists `--no-daemon`: run without the shared background server, even if it is already running |
| `exec --no-daemon --help` | 2 | `error: unexpected argument '--no-daemon' found` |
| `--no-daemon --no-daemon --help` | 2 | `error: the argument '--no-daemon' cannot be used multiple times` |

Thus unsupported placement after `exec`, duplication, and a wrapper's rejection are distinct cases. These results do not reproduce the unavailable wrapper itself.

A reproducible isolated parser probe (no shell profiles) is:

```python
import os, subprocess, tempfile
with tempfile.TemporaryDirectory(prefix="ht-parser-probe-", dir="/private/tmp") as root:
    for name in ("codex", "claude", "xdg"):
        os.mkdir(root + "/" + name)
    env = {"PATH": "/usr/bin:/bin", "HOME": root,
           "CODEX_HOME": root + "/codex",
           "CLAUDE_CONFIG_DIR": root + "/claude",
           "XDG_CONFIG_HOME": root + "/xdg"}
    executable = "/Users/alepar/.local/bin/codex"
    for args in (("--no-daemon", "--help"),
                 ("exec", "--no-daemon", "--help"),
                 ("--no-daemon", "--no-daemon", "--help")):
        result = subprocess.run([executable, *args], env=env, cwd=root,
                                capture_output=True, text=True, timeout=15)
        print(args, result.returncode, result.stdout, result.stderr)
```

## Codex TUI versus server

Pinned upstream source for the installed release distinguishes the shared server from its terminal clients. The daemon backend constructs `app-server --listen unix://`, optionally with `--remote-control`; its updater uses `app-server daemon pid-update-loop`. These are server modes, not TUI execution evidence. See [backend/pid.rs, command_args](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/app-server-daemon/src/backend/pid.rs#L347-L377).

The [TUI launch target selector](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/tui/src/lib.rs#L969-L998) chooses an explicit remote endpoint, a reusable local daemon, or an embedded server. A foreground native Codex process can therefore be the TUI while its hooks and tools execute in a server started from another pane. Excluding server argv helps classify processes; it does not correlate a hook's session to a particular terminal client.

Tool shell environments originate in the executing process: [shell_environment.rs](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/protocol/src/shell_environment.rs#L48-L52) uses `std::env::vars()`. The same module applies configured `shell_environment_policy.set` overrides and injects the actual thread id as `CODEX_THREAD_ID` ([lines 126–140](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/protocol/src/shell_environment.rs#L126-L140)). An explicit per-thread policy can set a tool's pane claim; the thread id alone supplies no TUI pane.

Hooks use a separate environment snapshot. [Hooks::new](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/hooks/src/registry.rs#L68-L76) captures `std::env::vars_os()`; [command_runner](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/hooks/src/engine/command_runner.rs#L397-L408) replays it plus internal hook-source overrides. Hook configuration does not consume the tool shell environment policy. Consequently `-c shell_environment_policy.set.HERDR_PANE_ID=...` does not establish the same pane for hooks.

There is no public per-handler environment field in this release's [HookHandlerConfig::Command](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/config/src/hook_config.rs#L151-L175). Internal hook-source environment overrides supply plugin path variables ([discovery.rs](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/hooks/src/engine/discovery.rs#L247-L255)). Changing the literal hook command to include a pane changes its normalized trust identity ([hook_hash](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/hooks/src/engine/discovery.rs#L733-L758)); it must not silently reuse trust for a different command.

The public [initialize client metadata](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/app-server-protocol/src/protocol/v1.rs#L23-L36) contains name, title and version, without a client PID or pane. The inspected [thread/start parameters](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L59-L163) have no arbitrary client environment or pane field. Daemon PID records identify the server, not its TUI clients. No supported persisted TUI-PID/current-thread map or client-to-pane locator was established by this research. Open files, cached session fields and temporal proximity have not been validated as attribution evidence.

## Live native TUI shell observation

The retained [probe script](../evidence/wrapper-seat-detection/native_tui_env_probe.py) and
[results](../evidence/wrapper-seat-detection/native-tui-env-results.json) use native Codex 0.160.1,
an owned private app-server socket and isolated configuration directories. The server and TUI
have distinct pane and environment markers. The TUI submits its `!` shell shortcut without a
model call or account credentials. All owned processes exited and scratch directories were removed.

| Mode | TUI pane marker | Shell pane marker | Shell parent |
|---|---|---|---|
| Private app-server | `probe-client:p2` | `probe-server:p1` | app-server PID 81532 |
| Same server, explicit `shell_environment_policy.set.HERDR_PANE_ID` | `probe-client:p2` | `probe-client:p2` | app-server PID 81532 |
| Embedded `--no-daemon` control | `probe-client:p2` | `probe-client:p2` | TUI PID 81670 |

The policy-override case retains the server's other environment marker. This confirms that the
override changes one tool variable rather than forwarding the client environment. These observations
concern the `!` shortcut; they do not by themselves qualify the model-selected Bash/exec tool or hooks.

The subsequent [model-tool probe](../evidence/wrapper-seat-detection/native_model_tool_env_probe.py)
uses a local scripted Responses SSE fixture to select the native `exec_command` tool from a TUI
prompt. The fixture follows [Codex's pinned function-call test helpers](https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/core/tests/common/responses.rs#L900-L989).
The [retained results](../evidence/wrapper-seat-detection/native-model-tool-env-results.json)
include the emitted call, the native function-call output returned to the fixture, and SessionStart
and PreToolUse captures. PreToolUse identifies the tool as `Bash`; its session matches the tool's
`CODEX_THREAD_ID`.

| Mode | Native exec tool pane | SessionStart and PreToolUse pane |
|---|---|---|
| Private app-server, client `probe-client:p2` | `probe-server:p1` | `probe-server:p1` |
| Same server, explicit tool pane policy | `probe-client:p2` | `probe-server:p1` |
| Embedded `--no-daemon` control | `probe-client:p2` | `probe-client:p2` |

Thus actual Bash-tool execution also uses the server environment. A tool policy override supplies
an explicit tool claim but cannot correct the hook claim. The probe uses scratch-only hook-trust
bypass and full-access execution to measure environment inheritance; it does not qualify native
hook trust or sandbox behavior. It uses no external model or account, modifies no real configuration,
and leaves no owned processes or scratch directory. These are measured Codex 0.160.1 observations;
there is no corresponding live Claude daemon-tool capture in this evidence.

## Installed Claude evidence

The installed native executable is `/Users/alepar/.local/share/claude/versions/2.1.290`, reached through `~/.local/bin/claude`. SHA-256:
`b8412a3826b2dc8ecb1c0605970c28dea28355de5faa740407dd881acdd40237`.
Its embedded build metadata reports version 2.1.290, build time `2026-10-05T16:12:37Z`, git SHA `3897a65075994f4efddcf34b4a32ea7c7c5ebc13`.

The native binary embeds readable JavaScript alongside bytecode. Targeted inspection found the following byte-offset anchors (these are build-specific offsets, not source line numbers):

| Offset | Embedded behavior |
|---:|---|
| 181581890 | Daemon argument classifier recognizes `daemon` after leading permission flags |
| 181593702 | `--daemon-worker` dispatches to `runDaemonWorker` |
| 183792375 | Noninteractive classifier recognizes `-p`, `--print`, `--init-only`, `--sdk-url`, or non-TTY stdout |
| 184034540 | `us()` constructs child environment from `process.env` plus host settings and proxy environment |
| 193238193 | `xBr()` composes shell environment from `us()`, explicit overrides and session runtime variables |
| 195748925 | Background startup constructs `daemon run --origin transient --spawned-by ...` |
| 196628367 | Supervisor spawns `--daemon-worker KIND` |
| 196492218 | Background PTY helper uses `--bg-pty-host ... -- CHILD`; Unix argv0 is `claude bg-pty-host` |

These distinguish known server/worker/helper modes from normal interactive execution. They do not prove that every foreground Claude-shaped argv is a TUI, nor establish that a session running through a background service inherits the attaching TUI's pane. No explicit HERDR propagation or supported client-session-to-pane map was found. `CLAUDE_CLIENT_PRESENCE_FILE` only checks a marker for cloud presence pulses in the inspected implementation; it is not established pane evidence. No real user configuration, auth data or session transcript was printed or changed.

## Attribution boundary and practical options

`src/cli/hook.rs` currently takes its pane from ambient `HERDR_PANE_ID`; `src/cli/mod.rs` does likewise for commands. In a shared-server topology this may describe the server's launch pane. Detecting that the ancestor is a server can establish that ambient attribution is unreliable; it cannot supply the missing client pane.

For managed launches, the target selected by the caller is explicit information. Possible designs are a client-side registration handed to the daemon and subsequently correlated with the actual hook session, or explicit per-invocation hook claims with appropriate trust handling. A static trusted hook helper consulting an explicit launch registration avoids changing its command text each launch, but its correlation protocol and provenance still require design and validation. Setting tool environment policy alone is insufficient for hooks. Top-level native `--no-daemon` avoids shared-server environment reuse when supported by the selected entrypoint; it does not solve compatibility with wrappers that reject it.

For an existing manual wrapper launch with no explicit registration, this research does not establish enough evidence to recover the correct TUI pane automatically. Report attribution unavailable rather than guess a pane. An explicit cooperative selector remains possible under policy A1.

[TRUST-POLICY.md](../../TRUST-POLICY.md) governs any implementation: C1 prohibits allocating or moving seats using Herdr's agent field or saved hints; C5 rejects invented current-execution evidence. A1 treats pane/session attribution as a cooperative claim, never focused-pane inference. A2 requires the daemon's canonical deciding view; a client registry is a hint until validated in that view. Neither foreground argv classification nor best-effort `agent_session` observations authorize a seat transition. This document changes no invariant or accepted limit.
