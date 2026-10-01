# Codex native caller probe (0.157.1, macOS)

**Capture passed; default accountable CLI transport is unsupported on this tested configuration.** The hook distinguished native root and child calls, including two children, and a scoped escalated command reached a private local socket. Ordinary Codex shell commands could not connect to either that socket or the Herdr socket (`Operation not permitted`). The documented exact Unix-socket permission profile also failed on installed 0.157.1; the standalone `codex sandbox --allow-unix-socket` flag connected, but native interactive/exec help exposes no equivalent flag. This probe does not authorize product ACKs.

The installed native Codex process ran with `--no-daemon` in one owned Herdr pane. A non-login shell there reported `HERDR_ENV=1`, workspace `w4`, tab `w4:t1`, pane `w4:p2`. Its `HCOM_DIR` was a private directory under `/private/tmp` with `HCOM_AUTO_APPROVE=0` and `HCOM_AUTO_TRUST_WORKSPACE=0`. Existing user hooks and configuration were preserved. The exact session-flags hook was trusted only through a `-c hooks.state={...}` override: `hooks/list` reported it and the existing user hook as trusted. No `/hooks` trust action or blanket bypass was used.

## Measured identity and timing

| Native scenario | Observation at `PreToolUse` | Result |
| --- | --- | --- |
| Root, child, root | `HT_NONCE_BEFORE`, `HT_NONCE_CHILD1` and `HT_NONCE_AFTER1` shared parent `session_id`/pane; the child had its own `agent_id` and `turn_id`, while the root turn before and after matched. These early hook rows reported `version=unknown` and did not contain transcript execution or PID fields. | Hook-level child distinction was observed; complete execution attribution was unavailable for these three calls. |
| Later complete root and child calls | `HT_NONCE_PIDROOT` reported transcript source `cli`, execution ID equal to host session, matching turn and hook/host PID. `HT_NONCE_PIDCHILD` reported `subagent`, execution ID equal to child `agent_id`, distinct child/root turns, and matching PID. | Complete hook-time fields were observed on these later, separate calls. |
| Two concurrent children | Both calls had different `agent_id`, execution IDs, `turn_id` and `tool_use_id`; hook-time transcript reads reported `subagent`. Both inherited the same Codex PID and pane. | Child identity cannot come from PID or pane alone. |
| `/new` in one pane | Immediately after `/new`, `herdr agent get` still named the predecessor session and foreground Codex PID was unchanged. No SessionStart hook had fired. On the first new turn, the all-SessionStart hook fired with `source=startup` and the new session; a `matcher=^clear$` hook did not fire. | An old context can still match cached session **and** PID before the first new turn. Current-execution freshness is unsupported in that window. |
| Exit and resume same conversation | Session ID stayed the same, while Codex foreground PID changed from the old process to a new process. Hook ancestry and `herdr pane process-info` matched the new PID. | Session ID alone cannot fence a predecessor. |
| Code-mode nested exec | Transcript recorded a `custom_tool_call` named `exec` invoking `tools.exec_command`. Its nested Bash `PreToolUse` hook ran and rewrote the synthetic command. | Observed for this path. |
| No probe hook | A new native `--no-daemon` Codex session ran the synthetic receiver; its context field was empty. | Missing context must fail closed. |

For the complete hook-time root/child rows, the transcript header and current turn were readable synchronously. One root hook sampled host session 6 ms and process 14 ms after hook entry; one child sampled them 6 ms and 15 ms after hook entry. These are observations, not latency bounds. [Official Codex hooks documentation](https://learn.chatgpt.com/docs/hooks) says child hooks use the parent `session_id` and warns that transcript format is unstable. The transcript parser in this probe is version-pinned and rejects missing/unknown shapes.

## Invocation transport

The probe hook used supported `PreToolUse.updatedInput` to prepend `HT_PROBE_CONTEXT=<tool_use_id>` to a nonce-tagged Bash command. A direct `printf ... "$HT_PROBE_CONTEXT"` printed an empty value because the shell expanded arguments before applying that inline assignment. A nested shell and a direct external Python receiver saw the exact hook `tool_use_id`. This establishes transport only for the tested external executable form. The receiver also saw the same token through code-mode nested exec.

The private socket experiment sent the token to an observer that read Herdr after receiving the request. Default root and child tool commands were denied at `connect()`. An explicit **single-command** `sandbox_permissions="require_escalated"` call succeeded; no interactive blocked UI appeared, and the native command completed through automatic review. Its socket observer recorded a matching context token, host session and foreground Codex PID after request arrival. No persistent approval or global config change was observed. This is a separately tested configuration, not default transport support.

The installed `codex sandbox -P` command with a session-only profile containing `network.unix_sockets` for the exact private socket still returned `PermissionError: [Errno 1] Operation not permitted`. Adding `--allow-unix-socket` connected in the standalone sandbox positive control. [Official configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents the narrow profile key, but this result does not establish native interactive support. Do not use broad network permission as a substitute.

## Replay and limits

### Native utility replay

The committed [receiver](../../tests/native/provenance/codex/receiver.py) and [server](../../tests/native/provenance/codex/server.py) replace the private `receiver.py`/`receiver3.py` and `server.py` used in the historical run. Their output is allowlisted, but a new run is new evidence; the rows above came from the private scripts and logs named in the Task 2 report. These commands assume an owned Herdr pane and a non-login shell. Substitute the pane returned by `herdr pane split` for `w4:p2` if it differs. Keep the socket and JSONL under a private directory.

```sh
PROBE_DIR="$PWD/tests/native/provenance/codex"
PROBE_SCRATCH="$(mktemp -d /private/tmp/ht-codex-replay.XXXXXX)"
chmod 700 "$PROBE_SCRATCH"
mkdir -m 700 "$PROBE_SCRATCH/hcom" "$PROBE_SCRATCH/evidence"
herdr pane split --pane w4:p1 --direction right --cwd "$PWD" --env "HCOM_DIR=$PROBE_SCRATCH/hcom" --env HCOM_AUTO_APPROVE=0 --env HCOM_AUTO_TRUST_WORKSPACE=0 --env "HT_CODEX_PROBE_LOG=$PROBE_SCRATCH/evidence/hooks.jsonl" --no-focus
python3 "$PROBE_DIR/server.py" --socket "$PROBE_SCRATCH/probe.sock" --evidence "$PROBE_SCRATCH/evidence/receiver.jsonl" --pane w4:p2 &
PROBE_SERVER_PID=$!
```

Wait for the server's `READY` output before submitting a receiver command. The historical Codex launch below used the exact Task 2 checkout path and a `hooks/list` hash for that *session-flags command*. For a different checkout path or hook command, obtain that command's key/hash from `codex app-server hooks/list` and replace only the exact `trusted_hash` session override; do not persist trust in user config. The historical native launch was:

```sh
herdr agent start ht_codex_probe --kind codex --pane w4:p2 --timeout 120000 -- --no-daemon -c 'hooks.PreToolUse=[{matcher="^Bash$",hooks=[{type="command",command="python3 ~/code/herdr-threads/.worktrees/herdr-native-mailbox-thread-plugin/.worktrees/super-auto-herdr-native-mailbox-thread-plugin--task-ht-4is.2.1/tests/native/provenance/codex/capture.py"}]}]' -c 'hooks.state={"/<session-flags>/config.toml:pre_tool_use:0:0"={trusted_hash="sha256:2c25e51ddb4db54b739b9c7b6d2d6195bbe632475cce7ce58ddc22f2db4b0cc4"}}'
```

The historical scoped socket prompt instructed the native subject to invoke `tools.exec_command` in Code Mode with `login:false`, `sandbox_permissions:"require_escalated"` and `python3 /private/tmp/ht-codex-probe-2c2e381/receiver3.py HT_NONCE_SOCKETESC`. The committed, parameterized equivalent is this *single* synthetic external command:

```sh
python3 "$PROBE_DIR/receiver.py" --socket "$PROBE_SCRATCH/probe.sock" HT_NONCE_SOCKETESC
```

From the same controller shell, this submits the expanded absolute command to the owned native subject:

```sh
herdr agent prompt ht_codex_probe "Use Code Mode tools.exec_command with login:false and sandbox_permissions:require_escalated to run only python3 $PROBE_DIR/receiver.py --socket $PROBE_SCRATCH/probe.sock HT_NONCE_SOCKETESC. Report the stdout." --wait --timeout 120000
```

For a default-sandbox negative control, use the same command without `sandbox_permissions`. For transport without a socket, run `python3 "$PROBE_DIR/receiver.py" HT_NONCE_CODEMODE`; it prints only nonce, context, `HERDR_ENV`, and pane. In the historical run, the token in `hooks.jsonl` for `HT_NONCE_SOCKETESC` equaled the receiver log's context, and the server's host read happened after request arrival. The new server adds `received_utc`, validates the exact pane and nonce, and never accepts a second pane. Stop only the owned subject and `kill "$PROBE_SERVER_PID"` after the run; keep the private evidence out of commits. Session flags, `--no-daemon`, and private hcom isolation are required for the measured configuration. The scoped escalation remains approval-mediated and does not establish unattended transport.

Run `python3 -m unittest discover -s tests/native/provenance/codex -p 'test_*.py'` to replay the sanitized cases. The manifest separates capture PASS from authorization UNSUPPORTED. Early root-child-root and code-mode rows retain their native `version=unknown` and missing transcript/PID fields; the no-hook case has no fabricated hook row. The classifier is a probe artifact; its generation, permit issue/expiry/decision times, hook-return/evidence times, `metadata_current` and post-request fields are synthetic inputs to a future adapter, not trusted client assertions. Its explicit expired-context and post-hook-evidence cases deny. It also rejects missing metadata, unknown versions, child execution, stale session/PID/generation, absent synchronous turn data and missing transport. Its single positive fixture is a conditional scoped-escalation contract replay, not native current-execution proof.

The product still needs a tested way for ordinary accountable CLI calls to reach its daemon without one approval per operation. It also needs a supported invalidation source for the `/new` interval before the first new turn: a fresh host RPC, matching session ID and matching PID all accepted predecessor values there. The fixture's `metadata_current` value is an explicit evidence annotation, not a native signal established for that interval. Transaction-time invalidation checks remain necessary. The host/native observation and database decision cannot be atomic; replacement after the observation remains a documented residual race. No receipt, ACK or SQLite mutation was performed in this probe.
