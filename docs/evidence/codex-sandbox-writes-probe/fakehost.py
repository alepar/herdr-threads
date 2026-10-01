#!/usr/bin/env python3
"""Private stand-in Herdr endpoint (ping, session.snapshot, pane.get), one request per
connection, as in tests/integration/sweep.rs. Usage: fakehost.py SOCKET"""
import json, os, socket, sys
path = sys.argv[1]
panes = [
    {"pane_id": p, "terminal_id": t, "workspace_id": "w1", "tab_id": "w1:t1",
     "focused": False, "agent_status": "idle", "revision": 3}
    for p, t in (("w1:p1", "term-a"), ("w1:p2", "term-b"))
]
try:
    os.unlink(path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX); s.bind(path); s.listen(16)
while True:
    c, _ = s.accept()
    try:
        line = c.makefile().readline()
        req = json.loads(line)
        rid, m = req.get("id"), req.get("method")
        if m == "ping":
            rep = {"id": rid, "result": {"type": "pong", "version": "0.9.1", "protocol": 22}}
        elif m == "session.snapshot":
            rep = {"id": rid, "result": {"type": "session_snapshot", "snapshot": {
                "version": "0.9.1", "protocol": 22, "panes": panes, "agents": [],
                "tabs": [], "workspaces": [], "layouts": []}}}
        elif m == "pane.get":
            hit = [p for p in panes if p["pane_id"] == req.get("params", {}).get("pane_id")]
            rep = ({"id": rid, "result": {"type": "pane_info", "pane": hit[0]}} if hit else
                   {"id": rid, "error": {"code": "pane_not_found", "message": "pane not found"}})
        else:
            rep = {"id": rid, "error": {"code": "agent_not_found", "message": "no agent in pane"}}
        c.sendall((json.dumps(rep) + "\n").encode())
    except Exception:
        pass
    finally:
        c.close()
