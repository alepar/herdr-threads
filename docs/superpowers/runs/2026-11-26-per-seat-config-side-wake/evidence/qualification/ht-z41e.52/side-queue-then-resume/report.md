# wake e2e 152142-bcc7

```json
{
  "run_id": "152142-bcc7",
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
  "root": "/tmp/ht-wake-e2e.syxqywx9",
  "started_utc": "2026-11-27T23:21:42Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"s6tNUGXBzILc\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "9c278a903b4a568bc3dd08a3c3565091",
        "native_session": "01a21f50-93f7-7062-ad18-7c5e838f71f5",
        "execution": "8af6762b-3268-5e51-82d9-35df9fb3c612",
        "generation": 1,
        "process": {
          "pid": 3731341,
          "start": 144942166
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21f50-a19e-76c0-80c2-616e2ac10431",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T23:25:16Z",
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
| side-queue/group | 0 | codex | True | True (8.01) | {"bindings_after_side": [{"session": "01a21f50-93f7-7062-ad18-7c5e838f71f5", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3731341, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f50-fbae-7603-93fa-5cf5fcca712d", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3731341, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "root_session": "01a21f50-93f7-7062-ad18-7c5e838f71f5", "side_setup_ok": true, "side_typed": false, "side_view_changed": true, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 5s \u2022 15:22", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-QUEUE-DONE"], "wake_markers": 1}, "activity_evidence": {"attempt": "fce03879-d47b-48f0-b193-e10ce7deebc9", "native_session": "01a21f50-93f7-7062-ad18-7c5e838f71f5", "turn_id": "01a21f51-caa0-7d51-956c-cfe8fd4d6097", "observed_at_ms": 7582}, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["m6tNDSNQ8uom"], "output_ids": ["m6tNDSNQ8uom"], "started_utc": "2026-11-27T23:23:03.715Z", "completed_utc": "2026-11-27T23:23:07.032Z", "bytes_read": 359162, "files_examined": 2, "session": "01a21f50-93f7-7062-ad18-7c5e838f71f5", "turn_id": "01a21f51-caa0-7d51-956c-cfe8fd4d6097"}, "delivered_in_root": true, "side_bits": {"01a21f50-fbae-7603-93fa-5cf5fcca712d": {"auxiliary": 1, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "side_first_event": {"01a21f50-fbae-7603-93fa-5cf5fcca712d": {"event": "SessionStart", "source": "fork", "transcript_path": "present"}}, "settled_by_root": true, "wake_state_final": {"last_outcome": "submitted", "completed_at_utc": 1795821776703, "retry_step": 1}, "queue_transport_ok": true} | FAIL |
| resume-idle/group | 0 | codex | True | True (5.85) | {"unavailable_after_exit": true, "disconnect_reason": "liveness_process_exit", "unavailable_while_idle": true, "wakes_on_p1": 0, "delivered": true, "available_after": true, "queue_transport_ok": true} | PASS |

1/2 expected rows passed (2 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 2, "guidance_contexts": 3, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop, SessionStart/resume; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.syxqywx9"}], "problems": []}`

exit code: 1
