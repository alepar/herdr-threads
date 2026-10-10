# agy native gate, final-SHA record

- Date (UTC): 2026-10-10
- Code SHA: c2f51b5121b7 (clean worktree, branch task-ht-poh.6.9 cut from super-auto/agy-harness-adapter 94e73c1a)
- Command: `scripts/agy-native-gate --report <this dir>/report.json` with every flow (no `--flow` filter; `--list` shows 14 flows, there is no `delivery-stop` flow in this tree)
- agy: official 1.3.2 darwin-arm, Google-signed, sha512 prefix 2657939f (full value in `report.json`)
- Herdr: 0.9.1 (isolated session, loopback sandbox, isolated HOME/XDG/TMPDIR)
- R17 compact framing and R18 wake run together on genuine agy in the `delivery` and `wake` flows.

## Per-flow status

| flow | status | seconds |
|---|---|---|
| allow-rewrite | PASS | 45.26 |
| boot | PASS | 14.53 |
| clear | PASS | 15.44 |
| delivery | PASS | 65.53 |
| downgrade | PASS | 67.96 |
| enroll | PASS | 14.51 |
| handoff | PASS | 29.56 |
| launch | PASS | 13.48 |
| permission | PASS | 20.66 |
| resume | PASS | 24.51 |
| setup | PASS | 4.13 |
| subagent | PASS | 14.47 |
| unsetup | PASS | 3.55 |
| wake | PASS | 215.54 |

All 14 flows PASS (`summary.txt`).

## Leak check

The gate's own leak check printed `LEAKS`, naming one process:
`LEAK rule=dead-owner pid=25162 ppid=1 cmd=/Users/alepar/.local/bin/herdr server`.
That server is not this run's: its environment is `HOME=/tmp/sp-ede3b852/ht-54r.4/ih/home`, i.e. it belongs to a task
of another epic (ht-54r.4, started 14:11 before this run) in another tab. The dead-owner rule matches it regardless of
run id. It was left untouched (shared-Herdr rule: never stop processes this run did not start). Everything under this
run's temp root (`/tmp/sp-525916de/ht-poh.6.9`) and worktree was checked with `stop-run-processes --check`: no match.
`scripts/check-no-leaked-processes --run-id <this run>` still lists the foreign pid for the same reason.

Follow-up after the live smoke: the foreign server had exited, and `scripts/check-no-leaked-processes --run-id <this run>`
printed `no leaked test processes`.
