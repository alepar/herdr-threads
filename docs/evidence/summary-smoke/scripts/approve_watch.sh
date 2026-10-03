#!/bin/sh
# usage: approve_watch.sh PANE SECONDS LOG
# Scratch smoke only. Polls one agent pane; when Claude shows a "Do you want to proceed?" permission prompt whose
# command is a heredoc write of the worker submission file or a herdr-threads command, it presses Enter (Yes) and
# logs the first lines of the prompt. Any other prompt is logged and left unanswered.
H=${H:-/private/tmp/ht-summary-smoke/h}
PANE=$1; END=$(( $(date +%s) + $2 )); LOG=$3
while [ "$(date +%s)" -lt "$END" ]; do
  V=$("$H" pane read "$PANE" --source visible 2>/dev/null)
  case "$V" in
    *"Do you want to proceed?"*)
      if printf '%s' "$V" | grep -q -e "submission" -e "herdr-threads" -e "scratchpad"; then
        printf '%s approved: %s\n' "$(date -u +%H:%M:%S)" "$(printf '%s' "$V" | grep -m1 -e 'cat >' -e 'herdr-threads' -e 'submission')" >> "$LOG"
        "$H" pane send-keys "$PANE" Enter >/dev/null 2>&1
        sleep 3
      else
        printf '%s NOT approved (unrecognised prompt)\n' "$(date -u +%H:%M:%S)" >> "$LOG"
        sleep 5
      fi
      ;;
    *) sleep 2 ;;
  esac
done
