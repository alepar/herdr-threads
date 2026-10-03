#!/usr/bin/env python3
# Compact-capture hook: logs stdin, then RETURNS adapter-shaped output.
# SessionStart source=compact gets a distinct marker so delivery after compaction is attributable.
import sys, json, os, datetime
raw = sys.stdin.read()
payload = json.loads(raw)
tag = sys.argv[1]
M = "c287cp"
out = {}
if tag == "SessionStart":
    if payload.get("source") == "compact":
        out = {"hookSpecificOutput": {"hookEventName": "SessionStart",
               "additionalContext": "HT-COMPACT-MARKER-" + M}}
    else:
        out = {"hookSpecificOutput": {"hookEventName": "SessionStart",
               "additionalContext": "HT-SS-MARKER-" + M}}
rec = {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "argv_tag": tag,
       "payload": payload, "returned": out}
with open("<scratch>/raw/hooks-out.jsonl", "a") as f:
    f.write(json.dumps(rec, sort_keys=True) + "\n")
print(json.dumps(out))
sys.exit(0)
