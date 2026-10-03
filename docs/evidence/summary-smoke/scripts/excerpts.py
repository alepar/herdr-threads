#!/usr/bin/env python3
"""Cut the per-claim excerpt files out of the extracted transcripts (captures/*-transcript.txt).

usage: excerpts.py CAPTURES_DIR

Per harness (claude, codex) and seat X / Y it writes:
  NN-hook-after-recovery.txt   the hook text delivered at the recovery event (instruction and hot thread)
  NN-work-output.txt           every `summary` output that printed Work (jobs with fetch / submit commands)
  NN-worker-spawns.txt         one line per worker spawn (model) and every submit result seen by a worker
  NN-ready.txt                 every `summary` output that printed Ready (blocks, ledger)
"""
import os
import re
import sys

cap = sys.argv[1]


def records(path):
    cur, out = [], []
    for line in open(path):
        if re.match(r"^(--- |=== )", line):
            if cur:
                out.append("".join(cur))
            cur = [line]
        else:
            cur.append(line)
    if cur:
        out.append("".join(cur))
    return out


def write(name, items):
    with open(os.path.join(cap, name), "w") as fh:
        fh.write("\n".join(items) if items else "(none found)\n")


for harness, seats in (("claude", ("x", "y")), ("codex", ("x", "y"))):
    for seat in seats:
        path = os.path.join(cap, f"{harness}-{seat}-transcript.txt")
        if not os.path.exists(path):
            continue
        recs = records(path)
        base = f"{harness}-{seat}"
        hook = [r for r in recs if "Context was reset" in r and ("hook SessionStart:compact" in r or "message[developer]" in r)]
        write(f"{base}-01-hook-after-recovery.txt", hook[:2])
        work = [r for r in recs if re.search(r"summary thread-\S+ frontier #\d+: work \(", r)]
        write(f"{base}-02-work-output.txt", work)
        spawns = [r.splitlines()[0][:400] for r in recs if "AGENT SPAWN" in r or "spawn_agent" in r.splitlines()[0]]
        results = [r.strip()[:300] for r in recs if re.search(r"result: (stored|rejected|Stored)|^rejected:|status\":\"stored", r)]
        write(f"{base}-03-worker-spawns-and-submits.txt", spawns + ["--- submit results seen in the transcript"] + results)
        ready = [r for r in recs if re.search(r"summary thread-\S+ frontier #\d+ \(peer-derived data below", r)]
        write(f"{base}-04-ready.txt", ready)
print("ok")
