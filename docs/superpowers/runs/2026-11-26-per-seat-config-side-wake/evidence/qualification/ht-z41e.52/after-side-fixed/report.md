# wake e2e 151511-51a7

```json
{
  "run_id": "151511-51a7",
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
  "root": "/tmp/ht-wake-e2e._wvb9f13",
  "started_utc": "2026-11-27T23:15:11Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-27T23:21:11Z",
  "exit_code": 1,
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
| side-native-defer/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21f4a-9d29-7bc3-8988-0577219129ac", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3712841, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f4b-11aa-7761-8173-5707296344ab", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3712841, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "side_setup_ok": true, "bindings_idle": true, "wake_state_before_send": {"last_outcome": "submitted", "completed_at_utc": 1795821335479, "retry_step": 0}, "refused_while_side": {"last_outcome": "unsafe", "completed_at_utc": 1795821507886, "retry_step": 0}, "wake_state_while_side": {"last_outcome": "unsafe", "completed_at_utc": 1795821507886, "retry_step": 0}, "side_typed": false, "submitted_while_side": false, "wake_state_after_close": {"last_outcome": "unsafe", "completed_at_utc": 1795821532136, "retry_step": 0}, "woke_before_watch": false, "delivered": false, "queue_transport_ok": true} | FAIL |
| resume-idle/group | 0 | codex | True | True (0.44) | {"unavailable_after_exit": true, "disconnect_reason": "liveness_process_exit", "unavailable_while_idle": true, "wakes_on_p1": 1, "delivered": false, "available_after": true, "queue_transport_ok": true} | FAIL |

0/2 expected rows passed (2 collected)

codex hook guidance: `{"rollouts": 2, "guidance_contexts": 3, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop, SessionStart/resume; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e._wvb9f13"}], "problems": []}`

exit code: 1
