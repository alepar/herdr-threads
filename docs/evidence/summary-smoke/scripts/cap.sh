#!/bin/sh
# usage: cap.sh NAME PANE   save a pane's recent-unwrapped scrollback, plus `agent get` JSON, under captures/NAME.*
# H is the herdr client wrapper that points at the private smoke server.
H=${H:-/private/tmp/ht-summary-smoke/h}
OUT=${OUT:-$(dirname "$0")/../captures}
mkdir -p "$OUT"
"$H" pane read "$2" --source recent-unwrapped > "$OUT/$1.pane.txt" 2>&1
"$H" pane get "$2" > "$OUT/$1.get.json" 2>&1
