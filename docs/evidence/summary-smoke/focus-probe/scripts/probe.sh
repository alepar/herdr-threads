#!/bin/sh
# Focused-pane poke probe on the REAL Herdr session (attached client) with a private herdr-threads instance.
R=/private/tmp/ht-focus-smoke; I=$R/state/instances/1ad20d92b9ac9faffa9264d0988d4eb82f05de97b3b81031cd3a415549c5f60d
DB=$I/threads.sqlite3; OUT=$R/out/probe2; mkdir -p $OUT
T=thread-vItUW9VM; XS=seat-btpASkNY; YS=seat-uvRw73el; XP=w4:pBM; YP=w4:pBN; HOME_TAB=w4:t66
: > $OUT/events.txt; : > $OUT/samples.txt
LOG0=$(wc -l < $I/daemon.log)
START=$(date +%s)
herdr tab focus $HOME_TAB >/dev/null 2>&1; herdr pane run w4:pBK "$R/ht send $T --body 'Focus probe 2: please acknowledge this release note.' --require-ack $XS --require-ack $YS --deadline 120 --json > $OUT/send.json 2>&1" >/dev/null; sleep 2
echo "T+0 send $(date -u +%H:%M:%S)" >> $OUT/events.txt
F=0; U=0
while :; do
  NOW=$(( $(date +%s) - START )); [ $NOW -gt 200 ] && break
  if [ $NOW -ge 60 ] && [ $F = 0 ]; then herdr agent focus yfocus >/dev/null 2>&1; F=1; echo "T+$NOW focus -> Y $(date -u +%H:%M:%S)" >> $OUT/events.txt; fi
  if [ $NOW -ge 110 ] && [ $U = 0 ]; then herdr tab focus $HOME_TAB >/dev/null 2>&1; U=1; echo "T+$NOW focus -> home tab $(date -u +%H:%M:%S)" >> $OUT/events.txt; fi
  {
    echo "=== T+$NOW $(date -u +%H:%M:%S)"
    for P in $XP $YP; do
      J=$(herdr pane get $P); echo "--- $P focused=$(echo "$J" | sed -n 's/.*"focused":\([a-z]*\).*/\1/p' | head -1) status=$(echo "$J" | sed -n 's/.*"agent_status":"\([a-z]*\)".*/\1/p' | head -1)"
      herdr pane read $P --source recent | grep -E "receipt due|attention pending" | tail -3
    done
  } >> $OUT/samples.txt
  sleep 5
done
sqlite3 -readonly $DB "select seat_id,retry_step,last_outcome from wake_work where seat_id in ('$XS','$YS')" > $OUT/wake_work.txt 2>&1
tail -n +$((LOG0+1)) $I/daemon.log > $OUT/daemon-log-window.txt
echo done > $OUT/finished
