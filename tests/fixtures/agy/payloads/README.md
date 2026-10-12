# agy hook payload fixtures

One recorded hook `stdin` object per file; absolute paths are `<root>/...`. Sources are the research probe runs (`line` is 1-based). The hook argv, not the payload, names the event.

- `session-start-top.json`: research B `exp/runs/tui-turn1.jsonl` line 1 (`SessionStart`)
- `session-start-child.json`: research B `exp/runs/tui-subagent.jsonl` line 4 (`SessionStart`)
- `session-start-after-clear.json`: research B `exp/runs/tui-after-clear.jsonl` line 1 (`SessionStart`)
- `pre-invocation-inv0-fresh.json`: research B `exp/runs/tui-turn1.jsonl` line 2 (`PreInvocation`)
- `pre-invocation-inv0-resume.json`: research B `exp/runs/19-conversation-id.jsonl` line 1 (`PreInvocation`)
- `pre-invocation-inv1.json`: research B `exp/runs/tui-turn1.jsonl` line 6 (`PreInvocation`)
- `stop-no-tool-call.json`: research B `exp/runs/tui-turn1.jsonl` line 8 (`Stop`)
- `stop-inject-error.json`: probe-toolcall `runs/tui-04-hooks.jsonl` line 3 (`Stop`)
