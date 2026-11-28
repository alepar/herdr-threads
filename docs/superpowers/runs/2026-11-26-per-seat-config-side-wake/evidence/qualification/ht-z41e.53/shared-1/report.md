# wake e2e 151948-e8d4

```json
{
  "run_id": "151948-e8d4",
  "sha": "76fa45e016a41d30407364f41ff12ad9abe1de89",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.53/bin/herdr-threads",
  "bin_sha256": "ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2",
  "root": "/tmp/ht-wake-e2e.9hyinm83",
  "started_utc": "2026-11-27T23:19:48Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sVXNUhNMSLS7\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "ba23e902201d126eade91c30a1b959ef",
        "native_session": "01a21f4e-d933-7d32-8788-2dc92d2aca67",
        "execution": "2de807e0-1632-5379-b0e8-38f522c5b949",
        "generation": 1,
        "process": {
          "pid": 3723053,
          "start": 144930810
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21f4e-e677-7302-aff9-ae4181f81599",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T23:22:07Z",
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
| side-queue/group | 0 | codex | True | True (6.51) | {"bindings_after_side": [{"session": "01a21f4e-d933-7d32-8788-2dc92d2aca67", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3723053, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f4f-4361-7253-865e-04814bb2dc68", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3723053, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "root_session": "01a21f4e-d933-7d32-8788-2dc92d2aca67", "side_setup_ok": true, "side_typed": false, "side_view_changed": true, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 5s \u2022 15:20", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-QUEUE-DONE"], "wake_markers": 1}, "activity_evidence": {"attempt": "14bc2c61-e4b3-4f4e-9470-70fb68d220b7", "native_session": "01a21f4e-d933-7d32-8788-2dc92d2aca67", "turn_id": "01a21f50-0df6-7961-8ead-e00cc244dd5f", "observed_at_ms": 6202}, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["mVXNL6bOcjoy"], "output_ids": ["mVXNL6bOcjoy"], "started_utc": "2026-11-27T23:21:09.881Z", "completed_utc": "2026-11-27T23:21:12.432Z", "bytes_read": 353608, "files_examined": 2, "session": "01a21f4e-d933-7d32-8788-2dc92d2aca67", "turn_id": "01a21f50-0df6-7961-8ead-e00cc244dd5f"}, "delivered_in_root": true, "side_bits": {"01a21f4f-4361-7253-865e-04814bb2dc68": {"auxiliary": 1, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "side_first_event": {"01a21f4f-4361-7253-865e-04814bb2dc68": {"event": "SessionStart", "source": "fork", "transcript_path": "present"}}, "settled_by_root": true, "wake_state_final": {"last_outcome": "submitted", "completed_at_utc": 1795821664323, "retry_step": 1}, "queue_transport_ok": true} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 2, "guidance_contexts": 2, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"skipped": "--keep", "workspace": "w2G", "daemon_attempted": true, "problems": []}`

exit code: 1
