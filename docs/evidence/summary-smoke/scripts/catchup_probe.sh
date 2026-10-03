#!/bin/sh
# usage: catchup_probe.sh XPANE YPANE THREAD XSEAT OUTDIR
# Catch-up hold probe, timed to fit inside one catch-up row (extension_until = entered + p99_cold_ms = 120 s here):
# X opens the row (summary -> Work), the agent seat Y sends X a require-ACK message (an ordinary agent message above the
# frontier: human and relays-user messages are priority and never held), X is asked to run `inbox` (its PreToolUse
# hook prints the attention digest), and the pending-receipts view is saved. The hook output is read afterwards from
# X's transcript. The first message of the hold must NOT be offered; after the row ends it is.
H=/private/tmp/ht-summary-smoke/h
HT=/private/tmp/ht-summary-smoke/ht
XP=$1; YP=$2; T=$3; XS=$4; OUT=$5
mkdir -p "$OUT"
date -u +%H:%M:%S > "$OUT/t0.txt"
$H agent prompt "$XP" "Run herdr-threads summary $T once and stop immediately after it prints. Do not spawn workers, do not run job commands, do nothing else." > /dev/null
sleep 8
$H agent prompt "$YP" "Run exactly: herdr-threads send $T --body 'Catch-up probe from the build seat: please acknowledge this status check for ht-77.' --require-ack $XS --deadline 180   Then reply with one line. Do not ACK anything." > /dev/null
sleep 15
date -u +%H:%M:%S > "$OUT/t-probe.txt"
$H agent prompt "$XP" "Run herdr-threads inbox once, report its output in one line, and stop. Do not ACK anything." > /dev/null
sleep 12
$HT pending-receipts --seat "$XS" --thread "$T" --limit 50 > "$OUT/pending-receipts-X.txt" 2>&1
python3 "$(dirname "$0")/pending_rows.py" "$HT" "$XS" > "$OUT/deferred-rows.txt"
date -u +%H:%M:%S > "$OUT/t-end.txt"
