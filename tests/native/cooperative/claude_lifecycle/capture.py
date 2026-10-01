"""Passive, allowlisted lifecycle capture; no hook output or permission decisions."""
import fcntl
import json
import os
from pathlib import Path
import sys
import time

FIELDS = ("hook_event_name", "source", "session_id", "agent_id", "agent_type")
CONTEXT = ("HERDR_ENV", "HERDR_WORKSPACE_ID", "HERDR_TAB_ID", "HERDR_PANE_ID")


def record(payload, role):
    row = {key: payload[key] for key in FIELDS if key in payload}
    row.update(role=role, observed_ns=time.time_ns(), caller_pid=os.getppid(),
               native_version=os.environ["CLAUDE_LIFECYCLE_VERSION"],
               caller_context={key: os.environ.get(key) for key in CONTEXT},
               phase=os.environ.get("CLAUDE_LIFECYCLE_PHASE", "interactive"))
    # Presence of an identity is observable even if its value is not in our allowlist.
    row["native_event_identity_present"] = any(key in payload for key in ("event_id", "hook_event_id"))
    path = Path(os.environ["CLAUDE_LIFECYCLE_LOG"])
    with path.open("a", encoding="utf-8") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        stream.write(json.dumps(row, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


if __name__ == "__main__":
    record(json.load(sys.stdin), sys.argv[1] if len(sys.argv) > 1 else "capture")
