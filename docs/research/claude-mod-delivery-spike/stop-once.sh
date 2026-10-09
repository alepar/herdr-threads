#!/bin/sh
# Spike Stop hook: block the first stop after a marker appears, forcing a continuation.
m="$HT_SPIKE_DIR/stop-armed"
if [ -f "$m" ]; then rm -f "$m"; echo '{"decision":"block","reason":"Stop-hook continuation: now say DELTA."}'; fi
exit 0
