#!/usr/bin/env python3
"""Summarise a Codex rollout jsonl: task_started/task_complete, user/assistant messages, function calls
and their outputs, in order. Home paths redacted. usage: codex_events.py <rollout.jsonl> [since-ISO]"""
import json, sys, os, re
home = os.path.expanduser("~")
since = sys.argv[2] if len(sys.argv) > 2 else ""
def clip(s, n=120):
    return re.sub(r"\s+", " ", str(s)).replace(home, "~")[:n]
for l in open(sys.argv[1]):
    d = json.loads(l); ts = d.get("timestamp", "")
    if ts < since or d.get("type") not in ("event_msg", "response_item"):
        continue
    p = d["payload"]; t = p.get("type")
    if t in ("task_started", "task_complete"):
        print(ts, t.upper(), p.get("turn_id", "")[-6:])
    elif t == "message" and p.get("role") in ("user", "assistant"):
        txt = " ".join(c.get("text", "") for c in p.get("content", []))
        if txt.startswith("<"):
            continue
        print(ts, "MSG", p["role"], clip(txt))
    elif t == "function_call":
        print(ts, "CALL", p.get("name"), clip(p.get("arguments")))
    elif t == "function_call_output":
        print(ts, "OUTPUT", clip(p.get("output"), 80))
