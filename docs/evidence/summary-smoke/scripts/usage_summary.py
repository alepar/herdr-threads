#!/usr/bin/env python3
"""Sum model token usage of the smoke's Claude transcripts and Codex rollouts.

usage: usage_summary.py CLAUDE_PROJECT_DIR CODEX_SESSIONS_DIR

Claude: every assistant record (top level and subagents) under CLAUDE_PROJECT_DIR; cost at the published Haiku 4.5 list
prices ($1.00 / $5.00 per million input / output tokens, cache read $0.10, cache write 5 min $1.25). Codex: the last
token_count event of each rollout (cumulative total); no price is assumed for gpt-5.6-luna.
"""
import glob
import json
import os
import sys

claude_dir, codex_dir = sys.argv[1], sys.argv[2]
tot = {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0}
seen = set()
files = glob.glob(os.path.join(claude_dir, "**", "*.jsonl"), recursive=True)
for path in files:
    for line in open(path):
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        msg = rec.get("message", {})
        usage = msg.get("usage")
        if rec.get("type") != "assistant" or not usage:
            continue
        key = (msg.get("id"), rec.get("requestId"))
        if key in seen and key != (None, None):
            continue
        seen.add(key)
        tot["input"] += usage.get("input_tokens", 0)
        tot["output"] += usage.get("output_tokens", 0)
        tot["cache_read"] += usage.get("cache_read_input_tokens", 0)
        tot["cache_write"] += usage.get("cache_creation_input_tokens", 0)
cost = (tot["input"] * 1.0 + tot["output"] * 5.0 + tot["cache_read"] * 0.10 + tot["cache_write"] * 1.25) / 1e6
print(f"claude: {len(files)} transcript files, tokens {json.dumps(tot)}, estimated cost at Haiku 4.5 list price: ${cost:.2f}")

ctot = {"input_tokens": 0, "cached_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0}
rollouts = glob.glob(os.path.join(codex_dir, "**", "rollout-*.jsonl"), recursive=True)
for path in rollouts:
    last = None
    for line in open(path):
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        payload = rec.get("payload", {})
        if rec.get("type") == "event_msg" and payload.get("type") == "token_count" and payload.get("info"):
            last = payload["info"].get("total_token_usage")
    if last:
        for k in ctot:
            ctot[k] += last.get(k, 0)
print(f"codex: {len(rollouts)} rollouts, tokens {json.dumps(ctot)} (gpt-5.6-luna, no price assumed)")
