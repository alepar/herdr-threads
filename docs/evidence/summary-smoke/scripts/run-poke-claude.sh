#!/bin/sh
export DEADLINE=400 FOCUS_AT=200 RELEASE_AT=330
D=REPO/docs/evidence/summary-smoke
exec $D/scripts/poke_probe.sh pclaude qclaude w1:p4 w1:p5 w1:p1 thread-oqQfpCN3 seat-dI6CZeba seat-KqDe0evQ $D/captures/claude-poke 430
