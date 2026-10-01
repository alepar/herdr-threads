## Host-recovery validation (ht-4is.11.5) on fcb37c8: 12 of 12 scenarios pass

The full isolated suite passed: the script exited 0, every scenario passed, and all 124 checks passed. There were no failures, so there are no causes to report.

- **Source:** `fcb37c8db067f8569f7e96c15ddf9ab02c2014d3` ("Park wave-5 review minors"), detached worktree `/private/tmp/ht-hr-tip`
- **Build:** `cargo build --release --locked` succeeded in 48.8s.
- **Binary sha256:** `d61ec472a1b859dd6fafd57ca13e1aba65d3aaf35bf07917d4a9425d71513731`. `report.md` records the same hash.
- **Herdr:** 0.9.1, pinned=True, on a private server only.
- **Platform:** macOS 26.6.2 arm64
- **Duration:** about 93s. Run dir `/private/tmp/htr-lmo5f76j`.

| Scenario | Result | Checks | Description |
|---|---|---|---|
| R01 | PASS | 13/13 | private Herdr host and test daemon |
| R02 | PASS | 11/11 | rename and reorder keep the seat |
| R03 | PASS | 6/6 | CLI exit retains the seat |
| R04 | PASS | 15/15 | socket denial keeps unresolved state, then structural reconfirmation |
| R05 | PASS | 14/14 | socket unavailability keeps unresolved state, then reconfirmation |
| R06 | PASS | 14/14 | daemon SIGKILL: deadlines never reset, one warning |
| R07 | PASS | 3/3 | missed events across daemon downtime recover by snapshot |
| R08 | PASS | 6/6 | observed move of a registered seat keeps the seat |
| R09 | PASS | 8/8 | pane close retires |
| R10 | PASS | 24/24 | host stop/restore: no auto-claim, repair labelled repair |
| R11 | PASS | 4/4 | independent instance namespaces |
| R12 | PASS | 6/6 | shared server and foreign daemons untouched |

The runner executed the scenarios in the order R01, R02, R03, R06, R07, R04, R05, R08–R12.

**Evidence:** `/private/tmp/ht-hr-tip-evidence/htr-lmo5f76j/`, which holds `results.json`, `report.md`, `commands.jsonl`, `owned-resources-ledger.jsonl` and `evidence/`. The full console log is at `<scratch>/hr-tip.log`.

**Hygiene:**
- **Shared server:** the shared Herdr server was not touched. `~/.local/bin/herdr server` is still pid 80553, started Sun Sep 27 22:31:23 2026, with the same pid and start time before and after the run. R12 also passed.
- **Worktree:** I removed the detached worktree `/private/tmp/ht-hr-tip` with `worktree remove --force`, and the path no longer exists. The evidence directory was kept.
- **Writes:** I made no code changes or commits and did not write to the integration worktree. Nothing was pushed and no model was launched.
