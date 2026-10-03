#!/usr/bin/env python3
"""Extract the evidence-bearing records of one Claude session transcript (and its subagent transcripts).

usage: extract_claude_transcript.py SESSION_JSONL OUT_FILE

Prints, in order: hook attachments (SessionStart additionalContext), every Bash tool call with its result
(herdr-threads commands only), assistant text, and per subagent the model, the fetch/submit commands and results.
Home paths and the user name are redacted.
"""
import glob
import json
import os
import re
import sys

src, out = sys.argv[1], sys.argv[2]
HOME = os.path.expanduser("~")


def clean(text):
    text = text.replace(HOME, "~")
    return re.sub(r"/Users/[A-Za-z0-9_.-]+", "/Users/USER", text)


def blocks(path):
    for line in open(path):
        try:
            yield json.loads(line)
        except ValueError:
            continue


def tool_results(rec):
    content = rec.get("message", {}).get("content")
    if not isinstance(content, list):
        return
    for b in content:
        if b.get("type") == "tool_result":
            body = b.get("content")
            if isinstance(body, list):
                body = "\n".join(x.get("text", "") for x in body if isinstance(x, dict))
            yield b.get("tool_use_id"), str(body)


def walk(path, label, lines):
    pending = {}
    for rec in blocks(path):
        if rec.get("type") == "attachment":
            att = rec["attachment"]
            if att.get("type") == "hook_success" and att.get("stdout"):
                lines.append(f"--- {label} hook {att.get('hookName')} stdout")
                try:
                    ctx = json.loads(att["stdout"])["hookSpecificOutput"]["additionalContext"]
                    lines.append(ctx)
                except (ValueError, KeyError):
                    lines.append(att["stdout"])
        content = rec.get("message", {}).get("content")
        if rec.get("type") == "assistant" and isinstance(content, list):
            model = rec["message"].get("model")
            for b in content:
                if b.get("type") == "text" and b["text"].strip():
                    lines.append(f"--- {label} assistant ({model}): {b['text'].strip()}")
                elif b.get("type") == "tool_use":
                    inp = b["input"]
                    if b.get("name") == "Bash":
                        pending[b["id"]] = inp.get("command", "")
                        lines.append(f"--- {label} Bash ({model}): {inp.get('command', '')}")
                    elif b.get("name") in ("Agent", "Task"):
                        lines.append(f"--- {label} AGENT SPAWN ({model}): model={inp.get('model')} description={inp.get('description')}")
                    else:
                        lines.append(f"--- {label} {b.get('name')} ({model}): {json.dumps(inp)[:300]}")
        for tid, body in tool_results(rec):
            lines.append(f"--- {label} result: {body}")
        if rec.get("type") == "user" and isinstance(content, str) and content.strip():
            lines.append(f"--- {label} user prompt: {content.strip()}")


lines = []
walk(src, "top", lines)
base = src[:-6]
for sub in sorted(glob.glob(os.path.join(base, "subagents", "*.jsonl")), key=os.path.getmtime):
    meta = json.load(open(sub[:-6] + ".meta.json"))
    lines.append(f"=== subagent {os.path.basename(sub)} {meta.get('description')}")
    walk(sub, "sub", lines)

with open(out, "w") as fh:
    fh.write(clean("\n".join(lines)) + "\n")
print(f"{len(lines)} records -> {out}")
