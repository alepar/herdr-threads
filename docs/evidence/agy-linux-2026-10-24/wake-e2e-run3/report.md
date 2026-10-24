# wake e2e 185319-6edd

```json
{
  "run_id": "185319-6edd",
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
  "root": "/tmp/ht-wake-e2e.pwr42hse",
  "started_utc": "2026-10-24T01:53:19Z",
  "finished_utc": "2026-10-24T01:55:47Z",
  "exit_code": 1
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| send-p1/group | 0 | agy | False | False | {"late_wake_s": null, "delivered": false, "trailing_wake_s": null} | FAIL |
| send-p1/group | -1 | agy | True | True (1.01) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 3 | agy | True | True (0.76) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 0 | agy | True | True (1.04) | {"delivered": false, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | FAIL |
| send-p0/group | -1 | agy | True | True (0.76) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 3 | agy | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null} | PASS |

4/6 expected rows passed (6 collected)

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e.pwr42hse"}], "problems": []}`

exit code: 1
