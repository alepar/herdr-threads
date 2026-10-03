#!/bin/sh
# usage: bringup.sh claude|codex
# Starts the private instance daemon (settings first), creates one workspace with three panes
# (S = person pane p1, X = p2, Y = p3), prepares their shells and launches the two agents.
KIND=$1
R=/private/tmp/ht-summary-smoke
H=$R/h
cd $R
export PATH=$R/bin:$PATH
export CLAUDE_CONFIG_DIR=$R/claude-config
INST=$R/state/instances/9639634cd008ed0f1c61a379dd666732a771ab7f17494af1af87036b37bf06a6
$R/ht daemon ensure >/dev/null 2>&1
$R/ht daemon stop >/dev/null 2>&1
sleep 3
printf '%s' '{"summary":{"chunk_bytes":2048,"display_bytes":8192,"p99_cold_ms":20000,"exit_grace_ms":15000}}' > $INST/settings.json
chmod 600 $INST/settings.json
$R/ht daemon ensure >/dev/null 2>&1
sleep 2
$H workspace create --cwd $R/proj-$KIND --label smoke-$KIND | grep -o '"pane_id":"[^"]*"' | head -1
$H pane split w1:p1 --direction right | grep -o '"pane_id":"[^"]*"'
$H pane split w1:p1 --direction down | grep -o '"pane_id":"[^"]*"'
$R/prep-pane.sh w1:p1 w1:p2 w1:p3
sleep 1
$H pane run w1:p1 "$R/ht me init"
if [ "$KIND" = claude ]; then
  $R/launch-claude.sh w1:p2 x$KIND | grep -E "outcome|^seat"
  $R/launch-claude.sh w1:p3 y$KIND | grep -E "outcome|^seat"
fi
