#!/bin/bash
# usage: run.sh <scratch> <settings-file> <tag> <prompt> [extra args...]
S="$1"; SET="$2"; TAG="$3"; PROMPT="$4"; shift 4
cp "$S/$SET" "$S/proj/.claude/settings.local.json"
cd "$S/proj"
unset_args=()
while IFS= read -r n; do unset_args+=(-u "$n"); done < <(env | grep -oE '^(CLAUDE|HERDR)[A-Za-z0-9_]*')
env "${unset_args[@]}" /bin/sh -c 'env | grep -E "^(CLAUDE|HERDR)" | cut -d= -f1' > "$S/raw/$TAG.parent-leak.txt"
env "${unset_args[@]}" ~/.local/share/claude/versions/2.1.286 -p "$PROMPT" "$@" \
  --model claude-haiku-4-5-20251001 --max-budget-usd 0.10 --setting-sources project,local \
  --output-format json < /dev/null > "$S/raw/$TAG.json" 2> "$S/raw/$TAG.stderr"
echo "exit=$?"
