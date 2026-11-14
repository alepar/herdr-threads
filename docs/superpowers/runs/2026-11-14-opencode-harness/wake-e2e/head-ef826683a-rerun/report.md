# wake e2e 150936-c24d

```json
{
  "run_id": "150936-c24d",
  "sha": "ef826683a230391105ef72e59a8a91e98e84fa99",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.1.2",
    "opencode": "1.18.1"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "/var/home/alepar/AleCode/herdr-threads/.worktrees/opencode-harness/bin/herdr-threads",
  "bin_sha256": "49e10bb8a30eb5e9c74118b284b3b62eeb48687858ff2832ee7348785c5a9f7c",
  "root": "/tmp/ht-wake-e2e.pc0othc9",
  "started_utc": "2026-11-14T23:09:36Z",
  "finished_utc": "2026-11-14T23:13:39Z",
  "exit_code": 0
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| send-ack/group | 0 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 0 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |

6/6 expected rows passed (6 collected)

codex hook guidance: `null`

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e.pc0othc9"}], "problems": []}`

exit code: 0
