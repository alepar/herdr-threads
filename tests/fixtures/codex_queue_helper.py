#!/usr/bin/python3
"""Deterministic queue protocol peer; no model, auth or real harness config."""
import json, os, sys, time, tomllib, subprocess
from pathlib import Path
assert sys.argv[1] == "-c" and sys.argv[3:] == ["app-server", "--listen", "stdio://"]
root = Path(tomllib.loads(sys.argv[2])["sqlite_home"])
mode = (root / "mode").read_text()
opened = open(root / ("other.sqlite" if mode == "wrong_db" else "queue_1.sqlite"), "rb")
trace = (root / "trace").open("a", buffering=1)
for line in sys.stdin:
    row = json.loads(line)
    trace.write(json.dumps({"method": row["method"], "params": row.get("params"), "env_keys": sorted(os.environ)}) + "\n")
    if "id" not in row:
        continue
    method = row["method"]
    if method == "initialize" and mode == "init_error":
        print(json.dumps({"id": row["id"], "error": {"code": -32601, "message": "unsupported"}}), flush=True)
        continue
    if method == "initialize" and mode == "stderr_overflow":
        sys.stderr.write("x" * 9000)
        sys.stderr.flush()
    if method == "thread/queue/list" and mode == "missing_metadata":
        print(json.dumps({"id": row["id"], "error": {"code": -32600, "message": "thread not found"}}), flush=True)
        continue
    if method == "thread/queue/list" and mode == "malformed_preflight":
        print(json.dumps({"id": row["id"], "result": {"data": "invalid"}}), flush=True)
        continue
    if method == "thread/queue/list" and mode == "slow_preflight":
        time.sleep(0.70)
    if method == "thread/queue/add":
        if mode == "lost_reply":
            sys.exit(0)
        if mode == "oversize":
            print("x" * 300000, flush=True)
            continue
        if mode == "hang":
            time.sleep(10)
            continue
        result = {"queuedSubmission": {"id": "queue-owned-1", "clientUserMessageId": row["params"]["clientUserMessageId"], "input": row["params"]["input"]}}
    elif method == "thread/queue/list":
        cursor = row["params"].get("cursor")
        page = int(cursor or "0")
        result = {"data": [{"id": "owned-" + str(page), "clientUserMessageId": "owned-attempt-1", "input": [{"type": "text", "text": "herdr-threads: wake up", "text_elements": []}]}, {"id": "user-" + str(page), "clientUserMessageId": "another-input", "input": [{"type": "text", "text": "ordinary user text", "text_elements": []}]}], "nextCursor": str(page + 1) if mode == "pages" else None}
        result["data"] = result["data"][:row["params"]["limit"]]
    elif method == "thread/queue/delete":
        result = {"deleted": mode != "absent"}
    else:
        result = {}
    if method == "thread/queue/add" and mode == "descendant":
        child = subprocess.Popen(["/bin/sleep", "30"])
        (root / "descendant").write_text(str(child.pid))
    print(json.dumps({"id": row["id"], "result": result}), flush=True)
    if method == "thread/queue/add" and mode == "descendant":
        time.sleep(30)
