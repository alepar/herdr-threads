#!/bin/sh
# usage: launch-claude.sh PANE NAME — launch preflight reads hooks from the scratch CLAUDE_CONFIG_DIR; the agent keeps the
# real config dir (login) and gets the scratch settings (hooks + promptSuggestionEnabled:false) via --settings.
export CLAUDE_CONFIG_DIR=/private/tmp/ht-focus-smoke/claude-config
export PATH=/private/tmp/ht-focus-smoke/bin:$PATH
exec /private/tmp/ht-focus-smoke/ht launch --pane "$1" --kind claude --name "$2" --harness-binary /private/tmp/ht-focus-smoke/bin/claude -- --setting-sources project,local --settings /private/tmp/ht-focus-smoke/claude-config/settings.json --model claude-haiku-4-5-20251001
