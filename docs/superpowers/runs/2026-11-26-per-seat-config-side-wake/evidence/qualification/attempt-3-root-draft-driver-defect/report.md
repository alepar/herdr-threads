# wake e2e 112928-eac8

```json
{
  "run_id": "112928-eac8",
  "sha": "144b17d8544aa82f91eaa73f9c63101f414a6a2b",
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
  "root": "/tmp/ht-wake-e2e.2gh58hi9",
  "started_utc": "2026-11-27T19:29:28Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"smzDKU2OPCBV\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "dce5fc7ef7c6a56f382caf6b296116e1",
        "native_session": "01a21e7b-f76c-7a03-ad2b-61b18ca0e2ce",
        "execution": "e267751b-f505-5f1b-9195-70044484b878",
        "generation": 1,
        "process": {
          "pid": 180538,
          "start": 143548796
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e7c-051c-7273-939e-b86c4fe72c3a",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:32:05Z",
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
  },
  "release_blocking_defects": "release-blocking defects: co-t0 root-draft-queue/group: root_draft_lost"
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| root-draft-queue/group | 0 | codex | True | True (34.52) | {"root_session": "01a21e7b-f76c-7a03-ad2b-61b18ca0e2ce", "draft_typed": true, "intents_while_draft": [{"ordinal": 2, "attempt": "43885d44-87e6-4f81-b332-2f4c7ad86748", "phase": "prepared", "logical_status": "active"}, {"ordinal": 2, "attempt": "43885d44-87e6-4f81-b332-2f4c7ad86748", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 2, "attempt": "43885d44-87e6-4f81-b332-2f4c7ad86748", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "64848762-6e2b-43d0-b378-22d6142a44e2", "phase": "prepared", "logical_status": "active"}, {"ordinal": 3, "attempt": "64848762-6e2b-43d0-b378-22d6142a44e2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "64848762-6e2b-43d0-b378-22d6142a44e2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "64fcf4e9-8642-4d4e-b8ae-a3686e67da7f", "phase": "prepared", "logical_status": "active"}, {"ordinal": 4, "attempt": "64fcf4e9-8642-4d4e-b8ae-a3686e67da7f", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "64fcf4e9-8642-4d4e-b8ae-a3686e67da7f", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "64fcf4e9-8642-4d4e-b8ae-a3686e67da7f", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "6f4d3d6b-d8f2-4066-b46d-3347ee00f685", "phase": "prepared", "logical_status": "active"}, {"ordinal": 5, "attempt": "6f4d3d6b-d8f2-4066-b46d-3347ee00f685", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "6f4d3d6b-d8f2-4066-b46d-3347ee00f685", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "6f4d3d6b-d8f2-4066-b46d-3347ee00f685", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "eef83c83-3fe8-4ec9-bc6e-b4cfe37b54b1", "phase": "prepared", "logical_status": "active"}, {"ordinal": 6, "attempt": "eef83c83-3fe8-4ec9-bc6e-b4cfe37b54b1", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "eef83c83-3fe8-4ec9-bc6e-b4cfe37b54b1", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "eef83c83-3fe8-4ec9-bc6e-b4cfe37b54b1", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "c71acead-8e3e-47ab-ba8d-444ebbe2ca22", "phase": "prepared", "logical_status": "active"}, {"ordinal": 7, "attempt": "c71acead-8e3e-47ab-ba8d-444ebbe2ca22", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "c71acead-8e3e-47ab-ba8d-444ebbe2ca22", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "e3e854b9-b55a-4520-a7aa-87d2c27ecfcd", "phase": "prepared", "logical_status": "active"}, {"ordinal": 8, "attempt": "e3e854b9-b55a-4520-a7aa-87d2c27ecfcd", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "e3e854b9-b55a-4520-a7aa-87d2c27ecfcd", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "e3e854b9-b55a-4520-a7aa-87d2c27ecfcd", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 9, "attempt": "9f7f92b0-05a6-48c4-81bf-572585db4728", "phase": "prepared", "logical_status": "active"}, {"ordinal": 9, "attempt": "9f7f92b0-05a6-48c4-81bf-572585db4728", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 10, "attempt": "3019b033-1f3d-4c4c-a60d-afd4b43903d7", "phase": "prepared", "logical_status": "active"}, {"ordinal": 10, "attempt": "3019b033-1f3d-4c4c-a60d-afd4b43903d7", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 10, "attempt": "3019b033-1f3d-4c4c-a60d-afd4b43903d7", "phase": "no_effect", "logical_status": "active"}], "no_wake_while_draft": true, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "wake_state_while_draft": {"last_outcome": "unsafe", "completed_at_utc": 1795807884765, "retry_step": 0}, "draft_intact": false, "draft_submitted": false, "draft_cleared": true, "queue_after_clear": true, "root_busy_at_arm": false, "delivered": true, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["mmzDvasqzMTh"], "output_ids": ["mmzDvasqzMTh"], "started_utc": "2026-11-27T19:32:00.010Z", "completed_utc": "2026-11-27T19:32:02.908Z", "bytes_read": 363018, "files_examined": 1, "session": "01a21e7b-f76c-7a03-ad2b-61b18ca0e2ce", "turn_id": "01a21e7e-3f87-7ba1-895a-ca0426193d61"}, "activity_evidence": {"attempt": "e6f20f26-572f-49dd-a55a-5d62004c90b0", "native_session": "01a21e7b-f76c-7a03-ad2b-61b18ca0e2ce", "turn_id": "01a21e7e-3f87-7ba1-895a-ca0426193d61", "observed_at_ms": 87077}, "release_blocking": ["root_draft_lost"], "queue_transport_ok": true} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

release-blocking defects: co-t0 root-draft-queue/group: root_draft_lost

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.2gh58hi9"}], "problems": []}`

exit code: 1
