#!/bin/zsh
# Redacted record: $S stands for the private scratch capture directory (local path removed).
H='[{hooks=[{type="command",command="python3 $S/live/inject.py",timeout=10}]}]'
HB='[{matcher="^Bash$",hooks=[{type="command",command="python3 $S/live/inject.py",timeout=10}]}]'
command aisw workspace check --tool codex || exit $?
cd $S/proj
HT_CAP_LOG=$S/live/hooks-run3.jsonl HERDR_AGENT=codex command codex --no-daemon exec -s read-only -C $S/proj resume --ignore-user-config --dangerously-bypass-hook-trust --json --skip-git-repo-check -m gpt-6-luna -c 'model_reasoning_effort="low"' \
  -c "hooks.SessionStart=$H" -c "hooks.PreToolUse=$HB" \
  01a0ef9e-76a2-7830-8c1f-431f5368a539 \
  "The read-only sandbox DOES allow read-only commands like echo; you must call your shell tool now with: echo capture-ok . After the tool returns, list verbatim the text of every developer-role message in your context that is shorter than 60 characters and contains the word marker, noting whether each appeared before or after the tool call. If none, say NONE."
