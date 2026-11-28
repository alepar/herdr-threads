# wake e2e 113419-81c0

```json
{
  "run_id": "113419-81c0",
  "sha": "fc3612e3fa2fc165a6dbc0af1de36e626101083f",
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
  "root": "/tmp/ht-wake-e2e.w1jqpklj",
  "started_utc": "2026-11-27T19:34:19Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sS027PiUtEnv\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "066257489e49f9920b9fa8189ed41843",
        "native_session": "01a21e80-6b25-70e1-8977-f6d77f24e9d1",
        "execution": "f6180505-8bda-5098-8695-80672825bc6f",
        "generation": 1,
        "process": {
          "pid": 219036,
          "start": 143577930
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e80-772a-76d2-8f4d-c65125cb5fd7",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:36:57Z",
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
| root-draft-queue/group | 0 | codex | True | True (34.52) | {"root_session": "01a21e80-6b25-70e1-8977-f6d77f24e9d1", "draft_typed": true, "intents_while_draft": [{"ordinal": 2, "attempt": "9d899a92-b9e6-4362-a08c-6c05a662f34f", "phase": "prepared", "logical_status": "active"}, {"ordinal": 2, "attempt": "9d899a92-b9e6-4362-a08c-6c05a662f34f", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 2, "attempt": "9d899a92-b9e6-4362-a08c-6c05a662f34f", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "8d5dcdfc-5779-4c5c-95da-e8040f9ec961", "phase": "prepared", "logical_status": "active"}, {"ordinal": 3, "attempt": "8d5dcdfc-5779-4c5c-95da-e8040f9ec961", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "8d5dcdfc-5779-4c5c-95da-e8040f9ec961", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "8d5dcdfc-5779-4c5c-95da-e8040f9ec961", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "dc4aa836-a1e7-42b2-869b-b23a3c6ac869", "phase": "prepared", "logical_status": "active"}, {"ordinal": 4, "attempt": "dc4aa836-a1e7-42b2-869b-b23a3c6ac869", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "dc4aa836-a1e7-42b2-869b-b23a3c6ac869", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "dc4aa836-a1e7-42b2-869b-b23a3c6ac869", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "c26e3631-c6f6-44c5-b42c-8e644cbee6e3", "phase": "prepared", "logical_status": "active"}, {"ordinal": 5, "attempt": "c26e3631-c6f6-44c5-b42c-8e644cbee6e3", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "c26e3631-c6f6-44c5-b42c-8e644cbee6e3", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "c26e3631-c6f6-44c5-b42c-8e644cbee6e3", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "2f20a731-65dc-4e50-8398-aff4f0ecc191", "phase": "prepared", "logical_status": "active"}, {"ordinal": 6, "attempt": "2f20a731-65dc-4e50-8398-aff4f0ecc191", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "2f20a731-65dc-4e50-8398-aff4f0ecc191", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "2f20a731-65dc-4e50-8398-aff4f0ecc191", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "ed44b2ce-0fbc-448e-848a-777e5dcf5408", "phase": "prepared", "logical_status": "active"}, {"ordinal": 7, "attempt": "ed44b2ce-0fbc-448e-848a-777e5dcf5408", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "ed44b2ce-0fbc-448e-848a-777e5dcf5408", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "ed44b2ce-0fbc-448e-848a-777e5dcf5408", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "b5cf6008-4765-4e4f-9a6d-051e8b0646b4", "phase": "prepared", "logical_status": "active"}, {"ordinal": 8, "attempt": "b5cf6008-4765-4e4f-9a6d-051e8b0646b4", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "b5cf6008-4765-4e4f-9a6d-051e8b0646b4", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 9, "attempt": "a8cae124-90da-46b8-b9af-227d92827bd9", "phase": "prepared", "logical_status": "active"}, {"ordinal": 9, "attempt": "a8cae124-90da-46b8-b9af-227d92827bd9", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 9, "attempt": "a8cae124-90da-46b8-b9af-227d92827bd9", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 10, "attempt": "e9ecbddc-3bc3-43c3-bb95-93badc554569", "phase": "prepared", "logical_status": "active"}, {"ordinal": 10, "attempt": "e9ecbddc-3bc3-43c3-bb95-93badc554569", "phase": "no_effect", "logical_status": "active"}], "no_wake_while_draft": true, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "wake_state_while_draft": {"last_outcome": "unsafe", "completed_at_utc": 1795808175628, "retry_step": 0}, "draft_intact": true, "draft_submitted": false, "draft_cleared": true, "queue_after_clear": true, "root_busy_at_arm": false, "delivered": true, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["mS02Aglbymf9"], "output_ids": ["mS02Aglbymf9"], "started_utc": "2026-11-27T19:36:51.361Z", "completed_utc": "2026-11-27T19:36:55.011Z", "bytes_read": 359234, "files_examined": 1, "session": "01a21e80-6b25-70e1-8977-f6d77f24e9d1", "turn_id": "01a21e82-b19e-79c1-9de9-554fd9f55ff5"}, "activity_evidence": {"attempt": "739e1dbb-a74e-440d-9cbf-bf5c341a5708", "native_session": "01a21e80-6b25-70e1-8977-f6d77f24e9d1", "turn_id": "01a21e82-b19e-79c1-9de9-554fd9f55ff5", "observed_at_ms": 86956}, "release_blocking": [], "queue_transport_ok": true} | PASS |

1/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.w1jqpklj"}], "problems": []}`

exit code: 0
