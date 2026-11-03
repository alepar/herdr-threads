# Run profile — ht-kqrt

```
Profile: context — agents 16 (measured 16) · peak≥80% 0 · compacted 0 agents, 0 compactions · coordinator 702K/1000K (70%), 0 compactions · max coordinator 70%
Profile: context agent — coordinator peak 702K/1000K (70%) · first 261K · compactions 0
Profile: unmeasured — Claude context windows from the model table: claude-opus-5-5 1000K, claude-sonnet-5-5 1000K
Profile: unavailable — no timed task dispatch in the runs given (transcripts missing)
```

## Context

- Profile: context — agents 16 (measured 16) · peak≥80% 0 · compacted 0 agents, 0 compactions · coordinator 702K/1000K (70%), 0 compactions · max coordinator 70%
- Profile: context agent — coordinator peak 702K/1000K (70%) · first 261K · compactions 0

| agent | role | model | tasks | requests | first | peak | window | source | peak% | compactions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| coordinator | coordinator | claude-opus-5-5 | 31 | 209 | 261K | 702K | 1000K | table | 70% | 0 |
| final-review | agent | claude-opus-5-5 | 1 | 31 | 44K | 154K | 1000K | table | 15% | 0 |
| plan | agent | claude-opus-5-5 | 1 | 11 | 40K | 94K | 1000K | table | 9% | 0 |
| plan-rest | agent | claude-opus-5-5 | 1 | 13 | 40K | 89K | 1000K | table | 9% | 0 |
| read-ledger | agent | claude-sonnet-5-5 | 1 | 3 | 39K | 66K | 1000K | table | 7% | 0 |
| read-ledger:finish | agent | claude-sonnet-5-5 | 1 | 2 | 39K | 66K | 1000K | table | 7% | 0 |
| read-ledger:retry | agent | claude-sonnet-5-5 | 1 | 2 | 39K | 64K | 1000K | table | 6% | 0 |
| assemble:final | agent | claude-sonnet-5-5 | 1 | 7 | 40K | 49K | 1000K | table | 5% | 0 |
| worktree-sweep | agent | claude-sonnet-5-5 | 1 | 3 | 39K | 50K | 1000K | table | 5% | 0 |
| bd-ready | agent | claude-sonnet-5-5 | 1 | 2 | 39K | 41K | 1000K | table | 4% | 0 |
| close-epics | agent | claude-sonnet-5-5 | 1 | 2 | 38K | 40K | 1000K | table | 4% | 0 |
| ledger-append:detector | agent | claude-sonnet-5-5 | 1 | 2 | 38K | 41K | 1000K | table | 4% | 0 |
| ledger-append:launch | agent | claude-sonnet-5-5 | 1 | 1 | 39K | 39K | 1000K | table | 4% | 0 |
| ledger-append:metrics | agent | claude-sonnet-5-5 | 1 | 2 | 40K | 45K | 1000K | table | 4% | 0 |
| ledger-append:recovery:deferred:ht-kqrt:sweep:environments-named-seats-herdr-app-20261031T201115 | agent | claude-sonnet-5-5 | 1 | 2 | 38K | 41K | 1000K | table | 4% | 0 |
| ledger-append:surface:1 | agent | claude-sonnet-5-5 | 1 | 2 | 41K | 43K | 1000K | table | 4% | 0 |
