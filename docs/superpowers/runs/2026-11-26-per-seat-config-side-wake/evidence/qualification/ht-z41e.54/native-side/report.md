# wake e2e 153345-0351

```json
{
  "run_id": "153345-0351",
  "sha": "0c0899c2e7b1adee300163663fa364352fc1e99d",
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
  "root": "/tmp/ht-wake-e2e._fz9zaqd",
  "started_utc": "2026-11-27T23:33:45Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-27T23:48:56Z",
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
| side-nudge/group | 0 | codex | True | True (1.01) | {"bindings_after_side": [{"session": "01a21f5b-9e85-7f01-9950-6049b6231ef6", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3966206, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5b-ff87-7f22-9249-b6aa0bda7575", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3966206, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5c-3fb6-7760-ae87-769e11f038a7", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3966206, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795822521013, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["c941478132e1214273dfb45116e92ba3"], "sibling_displayed_by": ["c941478132e1214273dfb45116e92ba3"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "archive_dialog_after": false}, "sibling_wake_s": 9.44, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795822700624, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 0 | codex | True | True (1.26) | {"bindings_after_side": [{"session": "01a21f5b-cdaa-78b1-b831-8a6e132b868a", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5c-2c96-7090-a93b-29d4d4b47e16", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5e-3da9-7603-88a5-0253ff0ebe9c", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795822770165, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["dcaf5c9ff68775cd2ccc9ee9487557ad"], "sibling_displayed_by": ["dcaf5c9ff68775cd2ccc9ee9487557ad"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "archive_dialog_after": false}, "sibling_wake_s": 7.57, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795822926032, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f5b-9e85-7f01-9950-6049b6231ef6", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3966206, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5b-ff87-7f22-9249-b6aa0bda7575", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3966206, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5c-3fb6-7760-ae87-769e11f038a7", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3966206, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5d-aad3-76d2-bde3-b02b758cb739", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3966206, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5f-dbe6-7303-b823-8daa2451b123", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3966206, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f61-e7fb-72f3-b575-69076c78d624", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3966206, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": false}}], "composer_at_end": "\u203a                                                                              \u2502\u203a Ask a follow-up question", "draft_intact": false, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823056078, "retry_step": 2}, "overdue_warning": {"message": "ezZ5C8uMrsLy", "event": {"classified_at": 1795823055843, "current_status": "pending", "deadline_at": 1795823054307, "detail": "last wake unsafe", "materialized_at": 1795823055843, "obligation": "receipt", "physical_status": "pending", "seat": "szZ5S2Yj8pTN", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": false, "queue_transport_ok": true} | FAIL |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f5b-cdaa-78b1-b831-8a6e132b868a", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5c-2c96-7090-a93b-29d4d4b47e16", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5e-3da9-7603-88a5-0253ff0ebe9c", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f61-8232-7600-b694-a6858cec902f", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f63-b87c-7bb1-8d6d-324797a21df6", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f65-c453-7a03-9f37-abbf7a0493a7", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 3974829, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823308401, "retry_step": 2}, "overdue_warning": {"message": "ezZ54xImAegm", "event": {"classified_at": 1795823308257, "current_status": "pending", "deadline_at": 1795823308145, "detail": "last wake unsafe", "materialized_at": 1795823308257, "obligation": "receipt", "physical_status": "pending", "seat": "szZ5LMUbVrdT", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | -1 | codex | True | True (1.27) | {"bindings_after_side": [{"session": "01a21f5b-bdb4-7080-a336-83f97da1fe54", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3970822, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5c-1bcc-7040-9637-3b2c0572ad21", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5e-2768-76f3-be32-3ebf40e62e4d", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795822645444, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["e2daf2b8311d9efb0e1dc723ef6e548a"], "sibling_displayed_by": ["e2daf2b8311d9efb0e1dc723ef6e548a"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "archive_dialog_after": false}, "sibling_wake_s": 11.15, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795822827613, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | -1 | codex | False | False | {"bindings_after_side": [{"session": "01a21f5b-bdb4-7080-a336-83f97da1fe54", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3970822, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5c-1bcc-7040-9637-3b2c0572ad21", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5e-2768-76f3-be32-3ebf40e62e4d", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5f-9120-7680-b54b-8c987954f8b0", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f61-cc57-7e13-a6df-550659d95d6e", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f62-0c09-72c3-a10e-502da5877e18", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3970822, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795822946627, "retry_step": 2}, "overdue_warning": {"message": "ezZ5vDYTPDiw", "event": {"classified_at": 1795822946299, "current_status": "pending", "deadline_at": 1795822946245, "detail": "last wake unsafe", "materialized_at": 1795822946299, "obligation": "receipt", "physical_status": "pending", "seat": "szZ5SZD1r4kQ", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 3 | codex | True | True (1.02) | {"bindings_after_side": [{"session": "01a21f5b-ade3-7c42-b5d8-e1e2e2261f8a", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3968559, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5c-0c25-7292-85b0-4d7e5736cd31", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5e-1cdd-7f71-b915-7e1ba170c913", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795822642727, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["6fd34bf0aedf6741d0357456ce4a5592"], "sibling_displayed_by": ["6fd34bf0aedf6741d0357456ce4a5592"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "archive_dialog_after": false}, "sibling_wake_s": 13.69, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795822795437, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 3 | codex | False | False | {"bindings_after_side": [{"session": "01a21f5b-ade3-7c42-b5d8-e1e2e2261f8a", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3968559, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f5c-0c25-7292-85b0-4d7e5736cd31", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5e-1cdd-7f71-b915-7e1ba170c913", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f5f-81fd-7da1-9b74-e31b51b2fece", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f61-c7f4-7d32-b290-7c99d509ca69", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f63-cf24-7ce1-b0e2-dc6411b1b445", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 3968559, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823061219, "retry_step": 2}, "overdue_warning": {"message": "ezZ5ixr2sqlk", "event": {"classified_at": 1795823060853, "current_status": "pending", "deadline_at": 1795823060804, "detail": "last wake unsafe", "materialized_at": 1795823060853, "obligation": "receipt", "physical_status": "pending", "seat": "szZ5q6ezUOxx", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |

7/8 expected rows passed (8 collected)

codex hook guidance: `{"rollouts": 5, "guidance_contexts": 4, "delivering_hooks": 11, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e._fz9zaqd"}], "problems": []}`

exit code: 1
