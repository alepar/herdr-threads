#!/usr/bin/env python3
"""Send N fresh status messages from the person pane S, so that a recovering seat has a new tail to summarize.

usage: send_tail.py HT THREAD N
"""
import subprocess
import sys

ht, thread, count = sys.argv[1], sys.argv[2], int(sys.argv[3])
for i in range(1, count + 1):
    body = (
        f"[tail {i}] Region eu-west soak is at step {i}; the on-call notes mention queue depth {400 + i * 5} and "
        f"latency p95 {80 + i} ms. Decision: keep rollout-7 at 10 percent in eu-west until the next status check; "
        f"the build seat will report the rc-7.1 checksum for ht-75 when it exists."
    )
    result = subprocess.run([ht, "send", thread, "--body", body], capture_output=True, text=True)
    sys.stdout.write(result.stdout)
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        sys.exit(result.returncode)
