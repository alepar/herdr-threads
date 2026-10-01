#!/usr/bin/env python3
import sys, json, os, time
raw = sys.stdin.buffer.read()
try: p = json.loads(raw)
except Exception: p = {"_unparsed": raw.decode("utf-8","replace")}
ev = p.get("hook_event_name")
marker = {"PreToolUse": "herdr-threads-capture-marker", "SessionStart": "herdr-threads-session-marker"}.get(ev)
out = json.dumps({"hookSpecificOutput": {"hookEventName": ev, "additionalContext": marker}}) if marker else ""
with open(os.environ["HT_CAP_LOG"], "a") as f:
    f.write(json.dumps({"t": time.time(), "env_names": sorted(os.environ), "payload": p, "stdout": out}) + "\n")
if out: sys.stdout.write(out)
sys.exit(0)
