"""Allowlisted Claude PreToolUse probe recorder for harmless Bash nonce calls."""

import datetime
import fcntl
import hashlib
import json
import os
import pathlib
import re
import secrets
import shlex
import subprocess
import sys
import time


PROBE_COMMAND = """printf 'HT4IS_CONTEXT=%s\\n' "$CLAUDE_PROBE_CONTEXT" """
PROBE_COMMAND = PROBE_COMMAND.rstrip()
ENV_FIELDS = (
    "HERDR_ENV", "HERDR_WORKSPACE_ID", "HERDR_TAB_ID", "HERDR_PANE_ID",
    "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_AGENT_ID",
    "CLAUDE_SESSION_ID", "CLAUDE_AGENT_ID",
)
INPUT_FIELDS = ("session_id", "tool_use_id", "agent_id", "agent_type")


def sanitize_host(pane_response, process_response, pane_id):
    """Keep only host identity fields; process command lines can contain secrets."""
    try:
        pane = pane_response["result"]["pane"]
        processes = process_response["result"]["process_info"]["foreground_processes"]
        if pane["pane_id"] != pane_id:
            return {"observer_status": "wrong_pane"}
        matches = [item["pid"] for item in processes if item.get("argv0") == "claude"
                   and type(item.get("pid")) is int]
        if len(matches) != 1:
            return {"observer_status": "ambiguous_process"}
        session_id = pane["agent_session"]["value"]
        if not isinstance(session_id, str) or not session_id:
            return {"observer_status": "missing_session"}
        return {"observer_status": "ok", "host_session_id": session_id,
                "host_pid": matches[0], "host_revision": pane.get("revision"),
                "host_agent_status": pane.get("agent_status")}
    except (KeyError, IndexError, TypeError, ValueError):
        return {"observer_status": "unknown_shape"}


def observe_host(pane_id):
    def stamped(result):
        return {**result, "observed_utc": datetime.datetime.now(
            datetime.timezone.utc).isoformat(), "observed_monotonic_ns": time.monotonic_ns()}

    if not isinstance(pane_id, str) or not pane_id:
        return stamped({"observer_status": "missing_pane"})
    try:
        pane = subprocess.run(["herdr", "pane", "get", pane_id], capture_output=True,
                              text=True, timeout=2, check=True)
        process = subprocess.run(["herdr", "pane", "process-info", "--pane", pane_id],
                                 capture_output=True, text=True, timeout=2, check=True)
        return stamped(sanitize_host(json.loads(pane.stdout), json.loads(process.stdout), pane_id))
    except (subprocess.SubprocessError, OSError, json.JSONDecodeError):
        return stamped({"observer_status": "unavailable"})


def verify_receipt(event, project_dir):
    """Check a probe tool result in its native transcript without emitting contents."""
    session = event.get("session_id")
    agent = event.get("agent_id")
    tool_id = event.get("tool_use_id")
    context = event.get("invocation_context")
    if not all(isinstance(value, str) and value for value in (session, tool_id, context)):
        return {"status": "missing_identity"}
    if not re.fullmatch(r"[a-zA-Z0-9-]+", session):
        return {"status": "unknown_identity"}
    if agent is not None and (not isinstance(agent, str) or
                              not re.fullmatch(r"[a-zA-Z0-9-]+", agent)):
        return {"status": "unknown_identity"}
    path = pathlib.Path(project_dir) / session
    if agent is None:
        path = path.with_suffix(".jsonl")
        kind = "root"
    else:
        path = path / "subagents" / ("agent-" + agent + ".jsonl")
        kind = "child"
    try:
        if path.stat().st_size > 10_000_000:
            return {"status": "oversize", "transcript_kind": kind}
        rows = [json.loads(line) for line in path.read_text().splitlines()]
    except (OSError, ValueError):
        return {"status": "unavailable", "transcript_kind": kind}
    use_found = False
    for row in rows:
        message = row.get("message")
        if not isinstance(message, dict) or not isinstance(message.get("content"), list):
            continue
        for item in message["content"]:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "tool_use" and item.get("id") == tool_id:
                use_found = item.get("name") == "Bash" and item.get("input", {}).get(
                    "command") == PROBE_COMMAND
            if item.get("type") == "tool_result" and item.get("tool_use_id") == tool_id:
                content = item.get("content")
                matched = use_found and isinstance(content, str) and content.strip(
                ) == "HT4IS_CONTEXT=" + context
                return {"status": "matched" if matched else "mismatch",
                        "transcript_kind": kind, "result_utc": row.get("timestamp")}
    return {"status": "missing_result", "transcript_kind": kind}


def recover_timeline(rollout_path, pane_id):
    """Recover only probe control and observer output from an original Codex rollout."""
    if not re.fullmatch(r"w[0-9]+:p[0-9]+", pane_id):
        raise ValueError("invalid pane ID")
    observe = f"python3 tests/native/provenance/claude/capture.py --observe {pane_id}"
    commands = {
        observe: "observe",
        f"herdr pane send-text {pane_id} /clear": "clear_text",
        f"herdr pane send-text {pane_id} /exit": "exit_text",
        f"herdr pane send-keys {pane_id} enter": "enter",
        f"herdr pane get {pane_id}": "pane_get",
    }
    recovered = []
    for number, raw_line in enumerate(pathlib.Path(rollout_path).read_text().splitlines(), 1):
        row = json.loads(raw_line)
        if row.get("type") != "event_msg":
            continue
        item = row.get("payload", {}).get("item", {})
        if item.get("type") != "CommandExecution" or item.get("exit_code") != 0:
            continue
        command = item.get("command")
        if not isinstance(command, list) or not command or not isinstance(command[-1], str):
            continue
        shell_command = command[-1]
        kind = commands.get(shell_command)
        if kind is None and shell_command.startswith("herdr agent start ") and (
                f"--kind claude --pane {pane_id} " in shell_command):
            kind = "agent_start"
        if kind is None:
            continue
        event = {
            "kind": kind, "source_line": number,
            "source_line_sha256": hashlib.sha256(raw_line.encode()).hexdigest(),
            "event_utc": row.get("timestamp"), "pane_id": pane_id,
        }
        if kind == "observe":
            observation = json.loads(item.get("stdout", ""))
            allowed = {
                "observer_status", "host_pid", "host_session_id", "host_revision",
                "host_agent_status", "observed_utc", "observed_monotonic_ns",
            }
            if set(observation) - allowed or observation.get("observer_status") != "ok":
                raise ValueError(f"unknown observer row at line {number}")
            event["observation"] = observation
        elif kind == "agent_start":
            start = json.loads(item.get("stdout", ""))["result"]["agent"]
            if start.get("pane_id") != pane_id:
                raise ValueError(f"wrong start pane at line {number}")
            event["agent_session_id"] = start["agent_session"]["value"]
            event["agent_status"] = start["agent_status"]
        elif kind == "pane_get":
            pane = json.loads(item.get("stdout", ""))["result"]["pane"]
            if pane.get("pane_id") != pane_id:
                raise ValueError(f"wrong pane at line {number}")
            event["agent_session_id"] = pane.get("agent_session", {}).get("value")
        recovered.append(event)
    return recovered


def verify_transition_sources(rollout_path, pane_id, expected):
    """Bind sanitized transition rows to exact original rollout lines."""
    recovered = {row["source_line"]: row for row in recover_timeline(rollout_path, pane_id)}
    allowed = {
        "kind", "source_line", "source_line_sha256", "event_utc", "pane_id",
        "session_sha256", "pid_sha256", "host_revision", "host_agent_status",
        "observed_utc", "observed_monotonic_ns", "observer_status", "agent_status",
    }
    seen = set()
    for fixture in expected:
        if not {"kind", "source_line", "source_line_sha256", "event_utc"} <= set(fixture):
            raise ValueError("transition source anchor missing")
        if set(fixture) - allowed:
            raise ValueError("unknown transition fixture field")
        line = fixture.get("source_line")
        if line in seen or line not in recovered:
            raise ValueError(f"missing or repeated transition line {line}")
        seen.add(line)
        actual = recovered[line]
        observation = actual.get("observation", {})
        session_id = observation.get("host_session_id", actual.get("agent_session_id"))
        pid = observation.get("host_pid")
        comparable = {
            **actual,
            "session_sha256": hashlib.sha256(session_id.encode()).hexdigest()
            if session_id is not None else None,
            "pid_sha256": hashlib.sha256(str(pid).encode()).hexdigest()
            if pid is not None else None,
            "host_revision": observation.get("host_revision"),
            "host_agent_status": observation.get("host_agent_status"),
            "observed_utc": observation.get("observed_utc"),
            "observed_monotonic_ns": observation.get("observed_monotonic_ns"),
            "observer_status": observation.get("observer_status"),
        }
        if any(comparable.get(key) != value for key, value in fixture.items()):
            raise ValueError(f"transition source mismatch at line {line}")
    return len(seen)


def record(envelope, env):
    """Return rewritten tool input only for the exact synthetic command."""
    if not isinstance(envelope, dict) or envelope.get("hook_event_name") != "PreToolUse":
        return None
    if envelope.get("tool_name") != "Bash":
        return None
    tool_input = envelope.get("tool_input")
    if not isinstance(tool_input, dict) or tool_input.get("command") != PROBE_COMMAND:
        return None
    if not env.get("CLAUDE_PROBE_LOG") or not env.get("CLAUDE_PROBE_NONCE"):
        return None
    context = "ctx-" + secrets.token_hex(16)
    row = {
        "harness": "claude",
        "version": env.get("CLAUDE_PROBE_VERSION", "unknown"),
        "event": "PreToolUse",
        "tool_name": "Bash",
        "nonce": env["CLAUDE_PROBE_NONCE"],
        "generation": env.get("CLAUDE_PROBE_GENERATION"),
        "invocation_context": context,
        "hook_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "hook_monotonic_ns": time.monotonic_ns(),
        "hook_pid": os.getpid(),
        "hook_parent_pid": os.getppid(),
        "transcript_hash": hashlib.sha256(str(envelope.get("transcript_path", "")).encode()).hexdigest()[:16],
    }
    row.update(observe_host(env.get("HERDR_PANE_ID")))
    row["host_observed_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    row["host_observed_monotonic_ns"] = time.monotonic_ns()
    for field in INPUT_FIELDS:
        if field in envelope:
            row[field] = envelope[field]
    for field in ENV_FIELDS:
        if field in env:
            row[field.lower()] = env[field]
    path = pathlib.Path(env["CLAUDE_PROBE_LOG"])
    with path.open("a", encoding="utf-8") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        stream.write(json.dumps(row, sort_keys=True) + "\n")
        stream.flush()
        fcntl.flock(stream, fcntl.LOCK_UN)
    updated = dict(tool_input)
    updated["command"] = "export CLAUDE_PROBE_CONTEXT=" + shlex.quote(context) + "; " + PROBE_COMMAND
    return {"hookSpecificOutput": {"hookEventName": "PreToolUse", "updatedInput": updated}}


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--observe":
        print(json.dumps(observe_host(sys.argv[2]), sort_keys=True))
        return
    if len(sys.argv) == 4 and sys.argv[1] == "--verify-receipts":
        project_dir = pathlib.Path(sys.argv[3])
        for number, line in enumerate(pathlib.Path(sys.argv[2]).read_text().splitlines(), 1):
            event = json.loads(line)
            print(json.dumps({"source_line": number, "tool_use_id": event.get("tool_use_id"),
                              **verify_receipt(event, project_dir)}, sort_keys=True))
        return
    if len(sys.argv) == 4 and sys.argv[1] == "--recover-timeline":
        for event in recover_timeline(sys.argv[2], sys.argv[3]):
            print(json.dumps(event, sort_keys=True))
        return
    if len(sys.argv) == 5 and sys.argv[1] == "--verify-transition-sources":
        fixtures = json.loads(pathlib.Path(sys.argv[3]).read_text())
        count = verify_transition_sources(sys.argv[2], sys.argv[4],
                                          fixtures["transition_sources"])
        print(f"validated {count} original rollout source rows")
        return
    envelope = json.load(sys.stdin)
    output = record(envelope, os.environ)
    if output is not None:
        print(json.dumps(output))


if __name__ == "__main__":
    main()
