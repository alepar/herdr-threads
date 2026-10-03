#!/usr/bin/env python3
"""Summarise a Claude Code transcript jsonl as an ordered event list (user text, queue ops,
queued_command attachments, assistant text/tool_use, tool_result). Home paths redacted.
usage: claude_events.py <session.jsonl> [since-ISO-timestamp]"""
import json, sys, os, re
f = sys.argv[1]
since = sys.argv[2] if len(sys.argv) > 2 else ""
home = os.path.expanduser("~")
def clip(s, n=140):
    s = re.sub(r"\s+", " ", str(s)).replace(home, "~")
    return s[:n]
for l in open(f):
    d = json.loads(l)
    t = d.get("type")
    ts = d.get("timestamp", "")
    if ts < since:
        continue
    if t == "queue-operation":
        print(ts, "QUEUE", d.get("operation"), clip(d.get("content", ""), 60))
    elif t == "attachment":
        a = d["attachment"]
        if a.get("type") == "queued_command":
            print(ts, "ATTACH queued_command", clip(a.get("prompt")), "|", clip({k: v for k, v in a.items() if k not in ("prompt", "type")}, 100))
    elif t == "user":
        c = d["message"]["content"]
        if isinstance(c, str):
            print(ts, "USER", clip(c))
        else:
            for x in c:
                if x.get("type") == "tool_result":
                    print(ts, "TOOL_RESULT", clip(x.get("content")))
                else:
                    print(ts, "USER-" + str(x.get("type")), clip(x.get("text", "")))
    elif t == "assistant":
        for x in d["message"]["content"]:
            if x.get("type") == "text":
                print(ts, "ASSISTANT text", clip(x["text"]))
            elif x.get("type") == "tool_use":
                print(ts, "ASSISTANT tool_use", x.get("name"), clip(json.dumps(x.get("input")), 160))
