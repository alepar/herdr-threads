#!/bin/zsh
# Redacted record: $S stands for the private scratch capture directory (local path removed).
H='[{hooks=[{type="command",command="python3 $S/live/inject.py",timeout=10}]}]'
HB='[{matcher="^Bash$",hooks=[{type="command",command="python3 $S/live/inject.py",timeout=10}]}]'
command aisw workspace check --tool codex || exit $?
cd $S/proj
HT_CAP_LOG=$S/live/hooks-run2.jsonl HERDR_AGENT=codex command codex --no-daemon exec --ignore-user-config --dangerously-bypass-hook-trust --json --skip-git-repo-check -s read-only -C $S/proj -m gpt-6-luna -c 'model_reasoning_effort="low"' \
  -c "hooks.SessionStart=$H" -c "hooks.PreToolUse=$HB" \
  "First, before running anything, quote verbatim every string containing 'herdr-threads' that you can see anywhere in your context so far (or say NONE-BEFORE). Then run the shell command: echo capture-ok . After it finishes, quote verbatim every string containing 'herdr-threads' that is now visible in your context and say where it appeared (e.g. developer/system message, hook context, tool output), or say NONE-AFTER. Do not guess or invent."
