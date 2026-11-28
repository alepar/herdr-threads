# wake e2e 150653-5376

```json
{
  "run_id": "150653-5376",
  "sha": "76fa45e016a41d30407364f41ff12ad9abe1de89",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.52/bin/herdr-threads",
  "bin_sha256": "ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2",
  "root": "/tmp/ht-wake-e2e.myfek5ug",
  "started_utc": "2026-11-27T23:06:53Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-27T23:08:33Z",
  "exit_code": 0,
  "codex_auth_refreshed_in_run": false,
  "root_transcript": {
    "co-t0": {
      "events": [
        [
          "SessionStart",
          "startup"
        ],
        [
          "UserPromptSubmit",
          null
        ],
        [
          "PreToolUse",
          null
        ],
        [
          "Stop",
          null
        ],
        [
          "SessionStart",
          "resume"
        ]
      ],
      "null_events": [],
      "defect": false
    },
    "co-t-1": {
      "events": [],
      "null_events": [],
      "defect": false
    },
    "co-t3": {
      "events": [],
      "null_events": [],
      "defect": false
    }
  }
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| resume-idle/group | 0 | codex | True | True (5.83) | {"unavailable_after_exit": true, "disconnect_reason": "liveness_process_exit", "unavailable_while_idle": true, "wakes_on_p1": 0, "delivered": true, "available_after": true, "queue_transport_ok": true} | PASS |

1/1 expected rows passed (1 collected)

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 2, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop, SessionStart/resume; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.myfek5ug"}], "problems": []}`

exit code: 0
