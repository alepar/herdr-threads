#!/bin/sh
# usage: launch-claude.sh PANE NAME
# launch's preflight reads the hooks from ITS CLAUDE_CONFIG_DIR (scratch, set up by `setup claude`); the agent itself keeps
# the real config dir (login) and gets the scratch settings through --settings, with the user-level sources excluded.
export CLAUDE_CONFIG_DIR=/private/tmp/ht-summary-smoke/claude-config
export PATH=/private/tmp/ht-summary-smoke/bin:$PATH
exec /private/tmp/ht-summary-smoke/ht launch --pane "$1" --kind claude --name "$2" --harness-binary /private/tmp/ht-summary-smoke/bin/claude -- --setting-sources project,local --settings /private/tmp/ht-summary-smoke/claude-config/settings.json --model claude-haiku-4-5-20251001
