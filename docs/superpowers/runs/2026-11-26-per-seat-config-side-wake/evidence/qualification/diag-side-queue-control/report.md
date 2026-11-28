# wake e2e 111857-56c6

```json
{
  "run_id": "111857-56c6",
  "sha": "bf335ef3f99ee6a3b2d23a6c998db7c43e37f772",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.43/bin/herdr-threads",
  "bin_sha256": "2a096d766a6fe65b06a8e33f9c55b339d938283604360c751865742eb88d3ae1",
  "root": "/tmp/ht-wake-e2e.99g000u8",
  "started_utc": "2026-11-27T19:18:58Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sWhHn68RSmH1\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "4697269b4aecbc8c76f70afccdf44181",
        "native_session": "01a21e72-5b36-7521-9093-f1f75ad52f05",
        "execution": "0c565d85-2599-54db-9860-0d173e129077",
        "generation": 1,
        "process": {
          "pid": 47274,
          "start": 143485812
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e72-68cb-7c41-8f65-45d61cc80d79",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:21:16Z",
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
| side-queue/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21e72-5b36-7521-9093-f1f75ad52f05", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 47274, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21e72-c542-7ca3-a572-ef52c9d275d1", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 47274, "disconnect": null, "concurrent": 1, "auxiliary": 0}], "root_session": "01a21e72-5b36-7521-9093-f1f75ad52f05", "side_setup_ok": true, "side_typed": false, "side_view_changed": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "activity_evidence": null, "delivered_in_root": false, "side_bits": {"01a21e72-c542-7ca3-a572-ef52c9d275d1": {"auxiliary": 0, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "side_first_event": {"01a21e72-c542-7ca3-a572-ef52c9d275d1": {"event": "SessionStart", "source": "fork", "transcript_path": "present"}}, "settled_by_root": false, "wake_state_final": {"last_outcome": "unsafe", "completed_at_utc": 1795807267061, "retry_step": 0}, "queue_transport_ok": false} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: False

codex hook guidance: `{"rollouts": 2, "guidance_contexts": 2, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.99g000u8"}], "problems": []}`

exit code: 1
