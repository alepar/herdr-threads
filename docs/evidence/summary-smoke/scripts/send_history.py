#!/usr/bin/env python3
"""Send the scripted long history of the summary smoke from the person pane S.

usage: send_history.py HT THREAD X_SEAT [COUNT]

HT is the herdr-threads wrapper that pins the private instance. The history is deterministic: a Release 7
rollout with decisions, asks, commitments, one human-authored instruction at message 12 and a final
require-ACK message to X. Messages are about 250 bytes so that chunk_bytes=2048 yields several level-0 jobs.
"""
import subprocess
import sys

ht, thread, xseat = sys.argv[1], sys.argv[2], sys.argv[3]
count = int(sys.argv[4]) if len(sys.argv) > 4 else 40

TOPICS = [
    ("decision", "We will ship Release 7 behind the flag rollout-7, enabled per region, starting with region eu-west."),
    ("ask", "Build seat: please produce the release candidate rc-7.1 from commit 9f3a2c1 and publish its checksum in this thread."),
    ("commitment", "Test seat commits to running the full regression suite against rc-7.1 and reporting failures with test names."),
    ("question", "Does the docs seat know whether the changelog for ht-71 needs the migration warning in the first paragraph?"),
    ("decision", "Changelog for ht-71 gets the migration warning in the first paragraph; docs seat owns the wording."),
    ("blocker", "Test seat is blocked on a flaky fixture in tests/export_roundtrip; ticket ht-72 tracks the fix."),
    ("ask", "Build seat: please quarantine export_roundtrip for rc-7.1 only and reopen it for rc-7.2 (ht-72)."),
    ("commitment", "Build seat commits to the quarantine of export_roundtrip within the hour and will cite commit ids here."),
    ("decision", "Region order is eu-west, us-east, ap-south; each region soaks for 30 minutes before the next one starts."),
    ("question", "Who owns the rollback runbook for the cache layer, and where does it live (ht-73)?"),
    ("decision", "The rollback runbook for the cache layer is owned by the build seat and lives in docs/runbooks/cache-rollback.md."),
    ("note", "Reminder: the dashboard for rollout-7 shows error rate per region; alert threshold stays at 2 percent."),
]

HUMAN_INSTRUCTION = (
    "Human instruction: always run the schema migration before the cache flush, never the other way round, "
    "and do not enable rollout-7 in us-east until I say so in this thread."
)


def send(body, *extra):
    result = subprocess.run([ht, "send", thread, "--body", body, *extra], capture_output=True, text=True)
    sys.stdout.write(result.stdout)
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        sys.exit(result.returncode)


for i in range(1, count + 1):
    kind, text = TOPICS[(i - 1) % len(TOPICS)]
    if i == 12:
        send(HUMAN_INSTRUCTION)
        continue
    filler = (
        f" Update {i}: status check recorded for region step {(i % 3) + 1}; the on-call notes mention queue depth "
        f"{100 + i * 7} and latency p95 {40 + i} ms, nothing else changed since the previous update."
    )
    send(f"[{kind} {i}] {text}{filler}")

send(
    "Please confirm you have read this thread: acknowledge the rollout checklist for ht-71 before you touch the "
    "cache flush. I need your receipt.",
    "--require-ack",
    xseat,
    "--deadline",
    "180",
)
