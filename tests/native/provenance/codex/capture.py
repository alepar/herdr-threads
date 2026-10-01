"""Allowlist-only native Codex PreToolUse probe. Set HT_CODEX_PROBE_LOG privately."""

import datetime
import json
import os
import re
import subprocess
import sys
from pathlib import Path

from classify import sanitize

NONCE = re.compile(r"HT_NONCE_[A-Za-z0-9_-]{4,64}")


def transcript_metadata(path):
    """Read only native identity/turn headers. Missing or partial files stay unknown."""
    result = {}
    if not isinstance(path, str):
        return result
    try:
        with Path(path).open(encoding="utf-8") as transcript:
            for line in transcript:
                if '"session_meta"' not in line and '"turn_context"' not in line:
                    continue
                row = json.loads(line)
                payload = row.get("payload", {})
                if row.get("type") == "session_meta" and "execution_id" not in result:
                    source = payload.get("source")
                    result["thread_source"] = "subagent" if isinstance(source, dict) and "subagent" in source else source
                    result["execution_id"] = payload.get("id")
                elif row.get("type") == "turn_context":
                    result["transcript_turn_id"] = payload.get("turn_id")
                    result["root_turn_id"] = payload.get("root_turn_id")
    except (OSError, ValueError, TypeError):
        return {}
    return sanitize(result)


def host_session(pane_id):
    if not isinstance(pane_id, str) or not re.fullmatch(r"w[0-9]+:p[0-9]+", pane_id):
        return {}
    try:
        process = subprocess.run(["herdr", "agent", "get", pane_id], capture_output=True,
                                 text=True, timeout=3, check=True)
        value = json.loads(process.stdout)["result"]["agent"].get("agent_session", {}).get("value")
        return {"host_session_id": value,
                "host_observed_utc": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        return {}


def process_identity(response):
    try:
        processes = response["result"]["process_info"]["foreground_processes"]
        codex = [p["pid"] for p in processes if p.get("name") == "codex"
                 and isinstance(p.get("pid"), int)]
        return codex[0] if len(codex) == 1 else None
    except (KeyError, TypeError):
        return None


def hook_codex_pid():
    pid = os.getppid()
    for _ in range(12):
        try:
            output = subprocess.run(["ps", "-p", str(pid), "-o", "ppid=", "-o", "comm="],
                                    capture_output=True, text=True, timeout=2, check=True).stdout.strip()
            parent, command = output.split(maxsplit=1)
            if Path(command).name == "codex":
                return pid
            pid = int(parent)
        except (OSError, subprocess.SubprocessError, ValueError):
            return None
    return None


def host_process(pane_id):
    if not isinstance(pane_id, str) or not re.fullmatch(r"w[0-9]+:p[0-9]+", pane_id):
        return {}
    try:
        process = subprocess.run(["herdr", "pane", "process-info", "--pane", pane_id],
                                 capture_output=True, text=True, timeout=3, check=True)
        pid = process_identity(json.loads(process.stdout))
        return {"host_codex_pid": pid,
                "process_observed_utc": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    except (OSError, subprocess.SubprocessError, ValueError):
        return {}


def codex_version():
    try:
        process = subprocess.run(["codex", "--version"], capture_output=True,
                                 text=True, timeout=3, check=True)
        return process.stdout.strip().removeprefix("codex-cli ")
    except (OSError, subprocess.SubprocessError):
        return "unknown"


def record(envelope, path):
    safe = sanitize(envelope)
    with Path(path).open("a", encoding="utf-8") as output:
        output.write(json.dumps(safe, sort_keys=True) + "\n")
    return safe


def hook():
    payload = json.load(sys.stdin)
    command = payload.get("tool_input", {}).get("command")
    match = NONCE.search(command) if isinstance(command, str) else None
    envelope = {
        "harness": "codex", "version": codex_version(),
        "event": payload.get("hook_event_name"),
        "source": payload.get("source"),
        "session_id": payload.get("session_id"),
        "turn_id": payload.get("turn_id"),
        "tool_use_id": payload.get("tool_use_id"),
        "agent_id": payload.get("agent_id"),
        "agent_type": payload.get("agent_type"),
        "nonce": match.group() if match else None,
        "hook_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "herdr_env": os.getenv("HERDR_ENV"),
        "workspace_id": os.getenv("HERDR_WORKSPACE_ID"),
        "tab_id": os.getenv("HERDR_TAB_ID"),
        "pane_id": os.getenv("HERDR_PANE_ID"),
    }
    interested = bool(match) or payload.get("hook_event_name") == "SessionStart"
    if interested:
        envelope.update(transcript_metadata(payload.get("transcript_path")))
        envelope.update(host_session(os.getenv("HERDR_PANE_ID")))
        envelope["hook_codex_pid"] = hook_codex_pid()
        envelope.update(host_process(os.getenv("HERDR_PANE_ID")))
    # Native hook metadata has no documented execution_id; do not synthesize it.
    log = os.getenv("HT_CODEX_PROBE_LOG")
    if log and interested:
        record(envelope, log)
    if match and payload.get("hook_event_name") == "PreToolUse" and payload.get("tool_name") == "Bash":
        token = payload.get("tool_use_id", "")
        if isinstance(token, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,128}", token):
            # Only the nonce probe is rewritten. The live test checks this command's semantics.
            rewritten = f"HT_PROBE_CONTEXT={token} {command}"
            print(json.dumps({"hookSpecificOutput": {
                "hookEventName": "PreToolUse", "permissionDecision": "allow",
                "updatedInput": {"command": rewritten},
            }}))


if __name__ == "__main__":
    hook()
