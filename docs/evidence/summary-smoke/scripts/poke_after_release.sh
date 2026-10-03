#!/bin/sh
# usage: poke_after_release.sh XAGENT YAGENT XPANE YPANE SPANE THREAD OUTDIR
# Soft poke versus focus, timed around the catch-up release (a poke candidate is not eligible while its seat has an
# active catch-up row, and the wake spacing otherwise starves pokes, so the release is the one moment both seats are
# eligible together). T+0: S sends 8 messages (default 300 s receipts, soft point about T+180). T+75: X and Y each run
# `summary` (Work: both open a catch-up row of about 120 s). T+90: Y's pane is focused. The rows stall at about T+195 and
# release: X (unfocused) should be poked, Y (focused) should be skipped. T+250: focus moves back to S; Y should then be
# poked. The pane tails, focus flags, catch_up rows and wake_work rows are sampled every 10 s.
H=/private/tmp/ht-summary-smoke/h
HT=/private/tmp/ht-summary-smoke/ht
DB=/private/tmp/ht-summary-smoke/state/instances/9639634cd008ed0f1c61a379dd666732a771ab7f17494af1af87036b37bf06a6/threads.sqlite3
XA=$1; YA=$2; XP=$3; YP=$4; SP=$5; T=$6; OUT=$7
mkdir -p "$OUT"
START=$(date +%s)
$H pane run "$SP" "clear; python3 $(dirname "$0")/send_tail.py $HT $T 8 > $OUT/tail-send.txt 2>&1" > /dev/null
date -u +%H:%M:%S > "$OUT/t0.txt"
SUMMARY=0; FOCUSED=0; RELEASED=0
while :; do
  NOW=$(( $(date +%s) - START ))
  [ "$NOW" -gt 330 ] && break
  if [ "$NOW" -ge 75 ] && [ "$SUMMARY" = 0 ]; then
    for A in "$XA" "$YA"; do
      $H agent prompt "$A" "Run herdr-threads summary $T once and stop immediately after it prints. Do not spawn workers, do not run job commands, do nothing else." > /dev/null
    done
    SUMMARY=1; echo "T+$NOW summary prompts to X and Y ($(date -u +%H:%M:%S))" >> "$OUT/events.txt"
  fi
  if [ "$NOW" -ge 90 ] && [ "$FOCUSED" = 0 ]; then
    $H agent focus "$YA" > /dev/null; FOCUSED=1; echo "T+$NOW focus -> Y ($(date -u +%H:%M:%S))" >> "$OUT/events.txt"
  fi
  if [ "$NOW" -ge 250 ] && [ "$RELEASED" = 0 ]; then
    $H pane focus --pane "$YP" --direction up > /dev/null; RELEASED=1; echo "T+$NOW focus -> S ($(date -u +%H:%M:%S))" >> "$OUT/events.txt"
  fi
  {
    echo "=== T+$NOW $(date -u +%H:%M:%S)"
    for P in "$XP" "$YP"; do
      echo "--- $P focused=$($H pane get "$P" | sed -n 's/.*"focused":\([a-z]*\).*/\1/p' | head -1) status=$($H pane get "$P" | sed -n 's/.*"agent_status":"\([a-z]*\)".*/\1/p' | head -1)"
      $H pane read "$P" --source recent | grep -v "^$" | tail -4
    done
    sqlite3 -readonly "$DB" "select seat_id,state,ifnull(end_reason,''),datetime(ended_at/1000,'unixepoch') from catch_up where thread_id='$T'"
    sqlite3 -readonly "$DB" "select seat_id,retry_step,datetime(last_reserved_at_utc/1000,'unixepoch'),last_outcome from wake_work where seat_id in (select seat_id from catch_up where thread_id='$T')"
  } >> "$OUT/samples.txt"
  sleep 10
done
