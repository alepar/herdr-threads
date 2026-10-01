#!/usr/bin/env python3
import sys, json, os, datetime
raw = sys.stdin.read()
try:
    payload = json.loads(raw)
except Exception:
    payload = {"_unparsed": raw}
rec = {"utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
       "argv_tag": sys.argv[1] if len(sys.argv) > 1 else None,
       "ppid": os.getppid(),
       "env_names": sorted(os.environ.keys()),
       "payload": payload, "raw_len": len(raw)}
with open(os.environ.get("CAPTURE_LOG", "<scratch>/raw/hooks.jsonl"), "a") as f:
    f.write(json.dumps(rec, sort_keys=True) + "\n")
sys.exit(0)
