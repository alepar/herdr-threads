#!/bin/sh
# Codex half of the smoke: a second workspace with three panes (S = person pane, X, Y), shells prepared with the scratch
# CODEX_HOME, then both Codex agents launched with `herdr-threads launch --kind codex`.
R=/private/tmp/ht-summary-smoke
H=$R/h
export PATH=$R/bin:$PATH
export CODEX_HOME=$R/codex-home
cd $R
WS=$($H workspace create --cwd $R/proj-codex --label smoke-codex)
SP=$(echo "$WS" | grep -o '"root_pane":{[^}]*"pane_id":"[^"]*"' | grep -o '"pane_id":"[^"]*"' | cut -d'"' -f4)
echo "S pane: $SP"
WSID=$(echo "$WS" | grep -o '"workspace_id":"[^"]*"' | head -1 | cut -d'"' -f4)
XP=$($H pane split "$SP" --direction right | grep -o '"pane_id":"[^"]*"' | cut -d'"' -f4)
YP=$($H pane split "$SP" --direction down | grep -o '"pane_id":"[^"]*"' | cut -d'"' -f4)
echo "X pane: $XP  Y pane: $YP"
for p in $SP $XP $YP; do
  $H pane run "$p" 'export PATH=/private/tmp/ht-summary-smoke/bin:$PATH CODEX_HOME=/private/tmp/ht-summary-smoke/codex-home; clear' > /dev/null
done
sleep 1
$H pane run "$SP" "$R/ht me init"
for pair in "$XP:xcodex" "$YP:ycodex"; do
  P=${pair%%:*}; N=${pair##*:}
  $R/ht launch --pane "$P" --kind codex --name "$N" -- -m gpt-5.6-luna -c 'model_reasoning_effort="low"' 2>&1 | grep -E "outcome|^seat|error|refus"
done
