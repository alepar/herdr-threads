# agy native gate, final-SHA record (re-run after ht-poh.53–58)

- Date (UTC): 2026-10-10
- Code SHA: 9af7ae9f3985 (clean worktree, super-auto/agy-harness-adapter)
- Command: `HT_AGY_BIN=.superpowers/agy-bin/1.3.2/agy scripts/agy-native-gate --report <this dir>/report.json`, all 14 flows
- agy: official 1.3.2 darwin-arm (same binary as the c2f51b5121b7 record); Herdr 0.9.1 isolated session
- Result: all 14 flows PASS; gate leak check clean (`summary.txt`).
- Supersedes `2026-10-10-c2f51b5121b7` as the final-SHA record: covers ht-poh.53 notice blocks, the
  `confirmable` step/probation choice and ht-poh.57's wake gate on genuine agy.
