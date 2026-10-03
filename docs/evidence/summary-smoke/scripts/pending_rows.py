#!/usr/bin/env python3
"""Print selected pending-receipt rows of a seat (all pages), as JSON lines.

usage: pending_rows.py HT SEAT [MESSAGE_ID ...]   (no MESSAGE_ID: only rows with a deferral or an extension)
"""
import json
import subprocess
import sys

ht, seat, wanted = sys.argv[1], sys.argv[2], set(sys.argv[3:])
cursor, rows = None, []
for _ in range(20):
    cmd = [ht, "--json", "pending-receipts", "--seat", seat, "--limit", "50"] + (["--cursor", cursor] if cursor else [])
    data = json.loads(subprocess.run(cmd, capture_output=True, text=True).stdout)["result"]["data"]
    rows += data["items"]
    cursor = data.get("next_cursor") or data.get("cursor")
    if not data.get("has_more"):
        break
print(f"total pending rows: {len(rows)}")
shown = 0
for row in rows:
    if (wanted and row["message"] in wanted) or (not wanted and "deferred_until" in row):
        print(json.dumps(row, sort_keys=True))
        shown += 1
        if not wanted and shown >= 3:
            break
