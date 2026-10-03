#!/bin/sh
export DEADLINE=300 FOCUS_AT=100 RELEASE_AT=215 S_PANE_UP=1
D=REPO/docs/evidence/summary-smoke
exec $D/scripts/poke_probe.sh xcodex ycodex w2:p2 w2:p3 w2:p1 thread-DWKSdAn7 seat-RJ7yryH6 seat-5E7sEMcK $D/captures/codex-poke 300
