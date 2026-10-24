# wake e2e 184835-502c

```json
{
  "run_id": "184835-502c",
  "sha": "096e18a36f690bd8880c004728fb5535b71866cb",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 2.3.1",
    "agy": "1.4.6"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "<home>/AleCode/herdr-threads/.worktrees/ht-agy-linux/bin/herdr-threads",
  "bin_sha256": "176b08637245b0cd7b6777c6539c6c9d8004c5671d477ade9beac08d1cc87160",
  "root": "/tmp/ht-wake-e2e.svzj7fkj",
  "started_utc": "2026-10-24T01:48:35Z",
  "finished_utc": "2026-10-24T01:50:59Z",
  "exit_code": 1
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| send-p1/group | 0 | agy | None | None | {"error": "ag-t0 is blocked at a dialog: . Yes, and always allow in this conversation for commands that start with 'echo' | 3. Yes, and always allow for commands that start with 'echo' (Persist to settings.json) | 4. No, cancel | \u2191/\u2193 Navigate \u00b7 tab Amend \u00b7 ctrl+g edit/expand command | esc to cancel                                                                                                                "} | FAIL |
| send-p1/group | -1 | agy | True | True (0.76) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 3 | agy | True | True (0.76) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 0 | agy | None | None | {"error": "ag-t0 is blocked at a dialog: . Yes, and always allow in this conversation for commands that start with 'echo' | 3. Yes, and always allow for commands that start with 'echo' (Persist to settings.json) | 4. No, cancel | \u2191/\u2193 Navigate \u00b7 tab Amend \u00b7 ctrl+g edit/expand command | esc to cancel                                                                                                                "} | FAIL |
| send-p0/group | -1 | agy | True | True (0.76) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 3 | agy | True | True (1.02) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |

4/6 expected rows passed (6 collected)

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e.svzj7fkj"}], "problems": []}`

exit code: 1
