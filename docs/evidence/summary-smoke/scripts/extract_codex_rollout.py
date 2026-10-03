#!/usr/bin/env python3
"""Extract the evidence-bearing records of one Codex rollout (session transcript).

usage: extract_codex_rollout.py ROLLOUT_JSONL OUT_FILE

Prints every message (role and text, which includes hook additionalContext), tool call and tool output, and the
compaction marker, with timestamps. Home paths and the user name are redacted.
"""
import json
import os
import re
import sys

src, out = sys.argv[1], sys.argv[2]
HOME = os.path.expanduser("~")


def clean(text):
    text = text.replace(HOME, "~")
    return re.sub(r"/Users/[A-Za-z0-9_.-]+", "/Users/USER", text)


def text_of(content):
    if isinstance(content, str):
        return content
    parts = []
    for item in content or []:
        if isinstance(item, dict):
            parts.append(item.get("text") or item.get("output") or json.dumps(item)[:400])
    return "\n".join(parts)


lines = []
for raw in open(src):
    try:
        rec = json.loads(raw)
    except ValueError:
        continue
    ts = rec.get("timestamp", "")
    kind = rec.get("type")
    payload = rec.get("payload", {}) if isinstance(rec.get("payload"), dict) else {}
    ptype = payload.get("type")
    if kind == "session_meta":
        meta = {k: payload.get(k) for k in ("id", "cli_version", "model_provider", "source")}
        lines.append(f"--- {ts} session_meta {json.dumps(meta)}")
    elif kind == "compacted":
        lines.append(f"--- {ts} COMPACTED")
    elif kind == "turn_context":
        lines.append(f"--- {ts} turn_context model={payload.get('model')} effort={payload.get('effort')}")
    elif kind == "response_item" and ptype == "message":
        lines.append(f"--- {ts} message[{payload.get('role')}]: {text_of(payload.get('content'))}")
    elif kind == "response_item" and ptype in ("function_call", "custom_tool_call"):
        args = payload.get("arguments") or payload.get("input") or ""
        lines.append(f"--- {ts} {ptype} {payload.get('name')}: {args}")
    elif kind == "response_item" and ptype in ("function_call_output", "custom_tool_call_output"):
        lines.append(f"--- {ts} tool output: {text_of(payload.get('output'))}")
    elif kind == "event_msg" and ptype in ("task_started", "task_complete", "agent_message", "user_message", "hook_started", "hook_completed"):
        lines.append(f"--- {ts} event {ptype}: {json.dumps(payload)[:600]}")

with open(out, "w") as fh:
    fh.write(clean("\n".join(lines)) + "\n")
print(f"{len(lines)} records -> {out}")
