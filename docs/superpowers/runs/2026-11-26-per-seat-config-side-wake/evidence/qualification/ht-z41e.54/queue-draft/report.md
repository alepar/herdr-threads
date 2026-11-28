# wake e2e 163700-cc2d

```json
{
  "run_id": "163700-cc2d",
  "sha": "9e35a4c2d8edab1073e3331e6a0eaa6d6132c743",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.54/bin/herdr-threads",
  "bin_sha256": "ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2",
  "root": "/tmp/ht-wake-e2e.5q271mrd",
  "started_utc": "2026-11-28T00:37:00Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-plain": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sc1y141kIFLZ\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sc1ycrL8Fziq\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t3": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sc1y22MPiuOg\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t-1": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sc1yDzbkc0wr\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "9c9324945121cf0974001d405f3b1f8b",
        "native_session": "01a21f95-84de-7c23-9c7b-f58ef5ad36e7",
        "execution": "103bd601-af7d-5760-b29a-33c96e477fd4",
        "generation": 1,
        "process": {
          "pid": 1119823,
          "start": 145393962
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21f95-91e5-7d32-87a0-1df54de9d60b",
      "ordinal": 2
    },
    "co-t3": {
      "adapter": "codex",
      "scope": {
        "binding": "b2eb7f3015985f700e7565b1157aa974",
        "native_session": "01a21f95-a3bd-7410-bc58-53ed3b87f95f",
        "execution": "11a14adf-8f41-55c5-959f-ae60166cbedb",
        "generation": 1,
        "process": {
          "pid": 1120284,
          "start": 145394766
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21f95-b154-74e0-9d84-6f8847cffba8",
      "ordinal": 2
    },
    "co-t-1": {
      "adapter": "codex",
      "scope": {
        "binding": "9e4fff30352efad1bc59051e80a39fd6",
        "native_session": "01a21f95-b64a-7703-a18c-3e228e01ff02",
        "execution": "b6acd57f-b00d-5d77-81f4-09e119b878e6",
        "generation": 1,
        "process": {
          "pid": 1120646,
          "start": 145395169
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21f95-c10b-72c2-9b0c-0a4758f7f6da",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-28T00:43:44Z",
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
    "co-t3": {
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
    }
  }
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f95-84de-7c23-9c7b-f58ef5ad36e7", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1119823, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f95-edfe-7420-af94-5f7170393a9a", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1119823, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f96-2a61-71a2-bdc4-7b2c9517b2b9", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1119823, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795826363200, "retry_step": 0}, "overdue_warning": {"message": "ec1yzAnPxwMi", "event": {"classified_at": 1795826362809, "current_status": "pending", "deadline_at": 1795826362636, "detail": "last wake unsafe", "materialized_at": 1795826362809, "obligation": "receipt", "physical_status": "pending", "seat": "sc1ycrL8Fziq", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f95-9411-7d22-95eb-372d38330d05", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1120036, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f95-de19-7110-8e35-a2387a429e5c", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1120036, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f97-e83d-7801-8033-8b1f07fb4e7a", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1120036, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795826594724, "retry_step": 0}, "overdue_warning": {"message": "ec1yqzGzEQsX", "event": {"classified_at": 1795826594599, "current_status": "pending", "deadline_at": 1795826594566, "detail": "last wake unsafe", "materialized_at": 1795826594599, "obligation": "receipt", "physical_status": "pending", "seat": "sc1y141kIFLZ", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | -1 | codex | False | False | {"bindings_after_side": [{"session": "01a21f95-b64a-7703-a18c-3e228e01ff02", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1120646, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f96-1beb-7d71-b8bb-9f378dbb7eec", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1120646, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f96-577c-7852-816b-126069d79f07", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1120646, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795826373210, "retry_step": 0}, "overdue_warning": {"message": "ec1yDX4cYwuT", "event": {"classified_at": 1795826372823, "current_status": "pending", "deadline_at": 1795826372780, "detail": "last wake unsafe", "materialized_at": 1795826372823, "obligation": "receipt", "physical_status": "pending", "seat": "sc1yDzbkc0wr", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 3 | codex | False | False | {"bindings_after_side": [{"session": "01a21f95-a3bd-7410-bc58-53ed3b87f95f", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1120284, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f96-1245-7d73-be55-b89e798799f3", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1120284, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f96-53c5-7e51-8c87-b28a30ea8399", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1120284, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795826489903, "retry_step": 0}, "overdue_warning": {"message": "ec1yinlCZFFK", "event": {"classified_at": 1795826489415, "current_status": "pending", "deadline_at": 1795826489373, "detail": "last wake unsafe", "materialized_at": 1795826489415, "obligation": "receipt", "physical_status": "pending", "seat": "sc1y22MPiuOg", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |

4/4 expected rows passed (4 collected)

codex hook guidance: `{"rollouts": 4, "guidance_contexts": 3, "delivering_hooks": 6, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.5q271mrd"}], "problems": []}`

exit code: 0
