#!/bin/sh
# usage: poke_probe.sh XAGENT YAGENT XPANE YPANE SPANE THREAD XSEAT YSEAT OUTDIR SECONDS
# Soft-deadline poke probe. X and Y are idle agents that were told not to ACK; S sends one require-ACK message to both
# with a 300 s deadline (soft point = 0.6, so about 180 s after the message). X is never focused: expect a poke at the soft
# point. Y's pane is focused from T+100 to T+215: expect no prompt in Y while focused (the poke is skipped) and a
# poke after focus moves back to S. Every 10 s the script records both panes' focus flag, the tail of both panes and
# the wake_work rows.
H=/private/tmp/ht-summary-smoke/h
HT=/private/tmp/ht-summary-smoke/ht
DB=/private/tmp/ht-summary-smoke/state/instances/9639634cd008ed0f1c61a379dd666732a771ab7f17494af1af87036b37bf06a6/threads.sqlite3
XA=$1; YA=$2; XP=$3; YP=$4; SP=$5; T=$6; XS=$7; YS=$8; OUT=$9; LEN=${10}
mkdir -p "$OUT"
START=$(date +%s)
$H pane run "$SP" "clear; $HT send $T --body 'Poke probe: please acknowledge this release note for ht-78.' --require-ack $XS $YS --deadline ${DEADLINE:-300} | tee $OUT/send.txt" > /dev/null
date -u +%H:%M:%S > "$OUT/t0.txt"
FOCUSED=0; RELEASED=0
while :; do
  NOW=$(( $(date +%s) - START ))
  [ "$NOW" -gt "$LEN" ] && break
  if [ "$NOW" -ge ${FOCUS_AT:-100} ] && [ "$FOCUSED" = 0 ]; then
    $H agent focus "$YA" > /dev/null; FOCUSED=1; echo "T+$NOW focus -> Y ($(date -u +%H:%M:%S))" >> "$OUT/events.txt"
  fi
  if [ "$NOW" -ge ${RELEASE_AT:-215} ] && [ "$RELEASED" = 0 ]; then
    if [ -n "${S_PANE_UP:-}" ]; then $H pane focus --pane "$YP" --direction up > /dev/null; else $H tab focus "${S_TAB:-w1:t1}" > /dev/null; fi; RELEASED=1; echo "T+$NOW focus -> S ($(date -u +%H:%M:%S))" >> "$OUT/events.txt"
  fi
  {
    echo "=== T+$NOW $(date -u +%H:%M:%S)"
    for P in "$XP" "$YP"; do
      echo "--- $P focused=$($H pane get "$P" | sed -n 's/.*"focused":\([a-z]*\).*/\1/p' | head -1) status=$($H pane get "$P" | sed -n 's/.*"agent_status":"\([a-z]*\)".*/\1/p' | head -1)"
      $H pane read "$P" --source recent | tail -6
    done
    sqlite3 -readonly "$DB" "select seat_id,retry_step,datetime(last_reserved_at_utc/1000,'unixepoch'),last_outcome from wake_work where seat_id in ('$XS','$YS')"
  } >> "$OUT/samples.txt"
  sleep 10
done
