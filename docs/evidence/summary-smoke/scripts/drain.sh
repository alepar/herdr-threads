#!/bin/sh
# usage: drain.sh PANE ANSWER SECONDS   answer every permission prompt shown in a scratch pane for SECONDS (ANSWER = Enter or 2)
H=/private/tmp/ht-summary-smoke/h
END=$(( $(date +%s) + $3 )); n=0
while [ "$(date +%s)" -lt "$END" ]; do
  if $H pane read "$1" --source visible | grep -q "Do you want to proceed"; then
    $H pane send-keys "$1" "$2" >/dev/null
    n=$((n+1)); echo "$(date -u +%H:%M:%S) answered $2 ($n)"
    sleep 2
  else
    sleep 1
  fi
done
