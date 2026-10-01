#!/usr/bin/env python3
import sys, json, os, time
raw = sys.stdin.buffer.read()
log = os.environ["HT_CAP_LOG"]  # mandatory; was a local scratch path (redacted)
try:
    payload = json.loads(raw)
except Exception:
    payload = {"_unparsed": raw.decode("utf-8", "replace")}
rec = {"t": time.time(), "argv": sys.argv[1:], "pid": os.getpid(), "ppid": os.getppid(),
       "env_names": sorted(os.environ.keys()), "payload": payload}
with open(log, "a") as f:
    f.write(json.dumps(rec) + "\n")
sys.exit(0)
