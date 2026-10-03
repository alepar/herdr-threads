#!/bin/sh
# usage: restart-codex.sh PANE...   quit Codex in each pane and start it again with the smoke flags
H=/private/tmp/ht-summary-smoke/h
for p in "$@"; do $H pane send-text "$p" '/quit'; sleep 1; $H pane send-keys "$p" Enter; done
sleep 4
for p in "$@"; do $H pane run "$p" 'clear; codex -m gpt-5.6-luna -c model_reasoning_effort=\"low\"' > /dev/null; done
sleep 10
