# wake e2e 125011-d806

```json
{
  "run_id": "125011-d806",
  "sha": "d8e87ea03eebe01f742d33ed52f466b29dd8b4ac",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.43/bin/herdr-threads",
  "bin_sha256": "ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2",
  "root": "/tmp/ht-wake-e2e.9ceimbct",
  "started_utc": "2026-11-27T20:50:11Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"s2aiwdw33CaV\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "ab30415e6568da54ce747c4694009a4b",
        "native_session": "01a21ec5-e147-7fd3-a1d4-1d1d801450fe",
        "execution": "cec73c92-5ff8-5daf-8bf8-d9433a6db37d",
        "generation": 1,
        "process": {
          "pid": 819848,
          "start": 144033193
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ec5-eeda-7511-a4c0-f3dc3ac8249d",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T20:52:19Z",
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
| root-draft-queue/group | 0 | codex | True | True (4.0) | {"root_session": "01a21ec5-e147-7fd3-a1d4-1d1d801450fe", "draft_typed": true, "intents_while_draft": [{"ordinal": 2, "attempt": "ade07361-a325-4713-b553-7f4a4d028c1e", "phase": "prepared", "logical_status": "active"}, {"ordinal": 2, "attempt": "ade07361-a325-4713-b553-7f4a4d028c1e", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 2, "attempt": "ade07361-a325-4713-b553-7f4a4d028c1e", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 2, "attempt": "ade07361-a325-4713-b553-7f4a4d028c1e", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "dfd0cffd-a2cc-4ae5-8347-d0b6a492a425", "phase": "prepared", "logical_status": "active"}, {"ordinal": 3, "attempt": "dfd0cffd-a2cc-4ae5-8347-d0b6a492a425", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "dfd0cffd-a2cc-4ae5-8347-d0b6a492a425", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 3, "attempt": "dfd0cffd-a2cc-4ae5-8347-d0b6a492a425", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "d0943fb1-029c-4a8c-8580-24130ff2e511", "phase": "prepared", "logical_status": "active"}, {"ordinal": 4, "attempt": "d0943fb1-029c-4a8c-8580-24130ff2e511", "phase": "prepared", "logical_status": "active"}, {"ordinal": 4, "attempt": "d0943fb1-029c-4a8c-8580-24130ff2e511", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "d0943fb1-029c-4a8c-8580-24130ff2e511", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 4, "attempt": "d0943fb1-029c-4a8c-8580-24130ff2e511", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "af9768ea-08a8-4bdc-b62b-81b48ad6ddc2", "phase": "prepared", "logical_status": "active"}, {"ordinal": 5, "attempt": "af9768ea-08a8-4bdc-b62b-81b48ad6ddc2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "af9768ea-08a8-4bdc-b62b-81b48ad6ddc2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 5, "attempt": "af9768ea-08a8-4bdc-b62b-81b48ad6ddc2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "039a95b5-78bf-429e-83a5-394bc454284e", "phase": "prepared", "logical_status": "active"}, {"ordinal": 6, "attempt": "039a95b5-78bf-429e-83a5-394bc454284e", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 6, "attempt": "039a95b5-78bf-429e-83a5-394bc454284e", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "119876ea-1d9f-4ac8-8f5e-8425fcbdb0e2", "phase": "prepared", "logical_status": "active"}, {"ordinal": 7, "attempt": "119876ea-1d9f-4ac8-8f5e-8425fcbdb0e2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 7, "attempt": "119876ea-1d9f-4ac8-8f5e-8425fcbdb0e2", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "39e31a9a-8a61-49c5-9bc2-e02755f615cc", "phase": "prepared", "logical_status": "active"}, {"ordinal": 8, "attempt": "39e31a9a-8a61-49c5-9bc2-e02755f615cc", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "39e31a9a-8a61-49c5-9bc2-e02755f615cc", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 8, "attempt": "39e31a9a-8a61-49c5-9bc2-e02755f615cc", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 9, "attempt": "d1638a01-8232-4384-8b7d-9aef098c2576", "phase": "prepared", "logical_status": "active"}, {"ordinal": 9, "attempt": "d1638a01-8232-4384-8b7d-9aef098c2576", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 9, "attempt": "d1638a01-8232-4384-8b7d-9aef098c2576", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 9, "attempt": "d1638a01-8232-4384-8b7d-9aef098c2576", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 10, "attempt": "e472c614-49f0-4770-92af-d3818ae823b4", "phase": "prepared", "logical_status": "active"}, {"ordinal": 10, "attempt": "e472c614-49f0-4770-92af-d3818ae823b4", "phase": "no_effect", "logical_status": "active"}, {"ordinal": 10, "attempt": "e472c614-49f0-4770-92af-d3818ae823b4", "phase": "no_effect", "logical_status": "active"}], "no_wake_while_draft": true, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "wake_state_while_draft": {"last_outcome": "unsafe", "completed_at_utc": 1795812706791, "retry_step": 0}, "draft_intact": true, "draft_submitted": false, "draft_cleared": true, "queue_after_clear": true, "root_busy_at_arm": false, "delivered": true, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["m2aifZdrISut"], "output_ids": ["m2aifZdrISut"], "started_utc": "2026-11-27T20:52:13.981Z", "completed_utc": "2026-11-27T20:52:17.523Z", "bytes_read": 353551, "files_examined": 1, "session": "01a21ec5-e147-7fd3-a1d4-1d1d801450fe", "turn_id": "01a21ec7-b41a-77f3-8015-c2cca02007a3"}, "activity_evidence": {"attempt": "73861cba-305a-48ee-9245-47c8a66ed2ba", "native_session": "01a21ec5-e147-7fd3-a1d4-1d1d801450fe", "turn_id": "01a21ec7-b41a-77f3-8015-c2cca02007a3", "observed_at_ms": 56848}, "release_blocking": [], "queue_transport_ok": true} | PASS |

1/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.9ceimbct"}], "problems": []}`

exit code: 0
