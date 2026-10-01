#!/usr/bin/env python3
# Run-3 hook: logs stdin like hook.py, then RETURNS adapter-shaped output.
import sys, json, os, datetime
raw = sys.stdin.read()
payload = json.loads(raw)
tag = sys.argv[1]
M = "eb75e8"
out = {}
if tag == "SessionStart":
    out = {"hookSpecificOutput": {"hookEventName": "SessionStart",
           "additionalContext": "HT-SS-MARKER-" + M}}
elif tag == "PreToolUse" and payload.get("tool_name") == "Bash" and "agent_id" not in payload:
    ti = dict(payload["tool_input"])
    ti["command"] = "export HERDR_THREADS_CALLER_CONTEXT='ctx_probe-" + M + "';\n" + ti["command"]
    out = {"hookSpecificOutput": {"hookEventName": "PreToolUse", "updatedInput": ti,
           "additionalContext": "HT-PTU-MARKER-" + M}}
rec = {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "argv_tag": tag,
       "payload": payload, "returned": out}
with open("<scratch>/raw/hooks-run3.jsonl", "a") as f:
    f.write(json.dumps(rec, sort_keys=True) + "\n")
print(json.dumps(out))
sys.exit(0)
