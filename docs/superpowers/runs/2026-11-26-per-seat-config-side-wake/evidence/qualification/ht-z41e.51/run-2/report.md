# wake e2e 151348-e8ec

```json
{
  "run_id": "151348-e8ec",
  "sha": "76fa45e016a41d30407364f41ff12ad9abe1de89",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.51/bin/herdr-threads",
  "bin_sha256": "ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2",
  "root": "/tmp/ht-wake-e2e.sabk2be5",
  "started_utc": "2026-11-27T23:13:48Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-27T23:15:14Z",
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
| side-native-defer/group | 0 | codex | True | True (1.52) | {"bindings_after_side": [{"session": "01a21f49-5bcf-7360-b3d1-861be8b444f2", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3707697, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f49-bbc1-78e3-8b20-8a6061826b42", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3707697, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "side_setup_ok": true, "bindings_idle": true, "wake_state_before_send": {"last_outcome": "submitted", "completed_at_utc": 1795821247430, "retry_step": 0}, "refused_while_side": {"last_outcome": "unsafe", "completed_at_utc": 1795821303886, "retry_step": 0}, "wake_state_while_side": {"last_outcome": "unsafe", "completed_at_utc": 1795821303886, "retry_step": 0}, "side_typed": false, "submitted_while_side": false, "close_side": {"archive_dialog": true, "archive_dialog_after": false}, "wake_state_after_close": {"last_outcome": "unsafe", "completed_at_utc": 1795821305932, "retry_step": 1}, "woke_before_watch": false, "delivered": true, "queue_transport_ok": true} | PASS |

1/1 expected rows passed (1 collected)

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.sabk2be5"}], "problems": []}`

exit code: 0
