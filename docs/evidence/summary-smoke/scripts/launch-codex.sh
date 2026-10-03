#!/bin/sh
# usage: launch-codex.sh PANE NAME
export PATH=/private/tmp/ht-summary-smoke/bin:$PATH
export CODEX_HOME=/private/tmp/ht-summary-smoke/codex-home
exec /private/tmp/ht-summary-smoke/ht launch --pane "$1" --kind codex --name "$2" -- -m gpt-5.6-luna -c 'model_reasoning_effort="low"'
