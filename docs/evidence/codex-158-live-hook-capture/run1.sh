#!/bin/zsh
# Redacted record: $S stands for the private scratch capture directory (local path removed).
H='[{hooks=[{type="command",command="python3 $S/capture.py",timeout=10}]}]'
HB='[{matcher="^Bash$",hooks=[{type="command",command="python3 $S/capture.py",timeout=10}]}]'
command aisw workspace check --tool codex || exit $?
cd $S/proj
HT_CAP_LOG=$S/live/hooks-run1.jsonl HERDR_AGENT=codex command codex --no-daemon exec --ephemeral --ignore-user-config --dangerously-bypass-hook-trust --json --skip-git-repo-check -s read-only -C $S/proj -m gpt-6-luna -c 'model_reasoning_effort="low"' \
  -c "hooks.SessionStart=$H" -c "hooks.SubagentStart=$H" -c "hooks.PreToolUse=$HB" \
  "Step 1: run the shell command: echo capture-ok . Step 2: spawn exactly one subagent whose only task is to run the shell command: echo child-ok . Wait for it, then reply DONE. Keep it minimal."
