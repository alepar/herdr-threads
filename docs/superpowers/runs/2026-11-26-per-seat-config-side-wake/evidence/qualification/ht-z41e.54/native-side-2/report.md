# wake e2e 154929-2b91

```json
{
  "run_id": "154929-2b91",
  "sha": "f9f5bd6a08d9885ee6ea21f82b191bd673e5e43c",
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
  "root": "/tmp/ht-wake-e2e.2kguwiuy",
  "started_utc": "2026-11-27T23:49:29Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-28T00:03:57Z",
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
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f6a-086e-7a61-a22e-8f769f74bf42", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 532129, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-68cf-7af2-a364-25271f156622", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 532129, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6c-7806-7963-b7a5-197c8aae1026", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 532129, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823633106, "retry_step": 0}, "overdue_warning": {"message": "eyQbb7WoqKCo", "event": {"classified_at": 1795823632962, "current_status": "pending", "deadline_at": 1795823630523, "detail": "last wake unsafe", "materialized_at": 1795823632962, "obligation": "receipt", "physical_status": "pending", "seat": "syQb72FxHtKD", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f6a-37e6-7760-8099-aefe87072068", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 544301, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-81c3-7fd2-ad40-082cc936c231", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 544301, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": false, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": false, "attempts": 3}}, {"side": 2, "new_binding": false, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823714552, "retry_step": 0}, "overdue_warning": {"message": "eyQbhNEQp5Uu", "event": {"classified_at": 1795823714429, "current_status": "pending", "deadline_at": 1795823714385, "detail": "last wake unsafe", "materialized_at": 1795823714429, "obligation": "receipt", "physical_status": "pending", "seat": "syQbIQ1sezZ1", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | FAIL |
| side-nudge/group | 0 | codex | True | True (1.28) | {"bindings_after_side": [{"session": "01a21f6a-086e-7a61-a22e-8f769f74bf42", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 532129, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-68cf-7af2-a364-25271f156622", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 532129, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6c-7806-7963-b7a5-197c8aae1026", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 532129, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6e-6240-79d1-9ab1-c7903899ae83", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 532129, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f70-6bfa-7ef0-a977-06be3f5fbf3e", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 532129, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795823842497, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["d00e1fe724e3d599d71703541d2b66a0"], "sibling_displayed_by": ["d00e1fe724e3d599d71703541d2b66a0"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 10.64, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824022169, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 0 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f6a-37e6-7760-8099-aefe87072068", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 544301, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-81c3-7fd2-ad40-082cc936c231", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 544301, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6f-ba0a-70b2-a80f-acefd3b4a0a7", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 544301, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f71-c670-7ff0-9bd0-b475ebb6dd9e", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 544301, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823714552, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 7.84, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824229850, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | -1 | codex | False | False | {"bindings_after_side": [{"session": "01a21f6a-1814-7cc2-9d8a-3faa15ebb72f", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 536086, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-7c03-7722-b814-1e443f5022b1", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 536086, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6a-c349-78f0-9649-132d2f20aca6", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 536086, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823638237, "retry_step": 0}, "overdue_warning": {"message": "eyQbVifi6VwV", "event": {"classified_at": 1795823637971, "current_status": "pending", "deadline_at": 1795823634792, "detail": "last wake unsafe", "materialized_at": 1795823637971, "obligation": "receipt", "physical_status": "pending", "seat": "syQbqosl4aIQ", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | -1 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f6a-1814-7cc2-9d8a-3faa15ebb72f", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 536086, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-7c03-7722-b814-1e443f5022b1", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 536086, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6a-c349-78f0-9649-132d2f20aca6", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 536086, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6e-718a-7260-af35-96b60c55b4a1", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 536086, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f70-86d9-7b42-8f19-d05680e011be", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 536086, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823638237, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["789cd02873052d0b223effa4dd401d85"], "sibling_displayed_by": ["789cd02873052d0b223effa4dd401d85"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 7.82, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795824119080, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 3 | codex | False | False | {"bindings_after_side": [{"session": "01a21f6a-2801-7541-92e0-252b68a1babf", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 541028, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-9000-7f40-adca-014e4942de00", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 541028, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6a-ce74-7c71-b84e-ef8f67b43292", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 541028, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795823638106, "retry_step": 0}, "overdue_warning": {"message": "eyQbITd6NVBD", "event": {"classified_at": 1795823637971, "current_status": "pending", "deadline_at": 1795823637911, "detail": "last wake unsafe", "materialized_at": 1795823637971, "obligation": "receipt", "physical_status": "pending", "seat": "syQbSRrlJyL6", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 3 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f6a-2801-7541-92e0-252b68a1babf", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 541028, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f6a-9000-7f40-adca-014e4942de00", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 541028, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6a-ce74-7c71-b84e-ef8f67b43292", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 541028, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f6e-852f-7d82-ad2e-e6cb8cc28f07", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 541028, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f70-96a2-7713-af7e-3edb8328ee48", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 541028, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795823971922, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["f9956fb16fa6267cd71befe0f4a712bd"], "sibling_displayed_by": ["f9956fb16fa6267cd71befe0f4a712bd"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 16.93, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824159007, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |

7/8 expected rows passed (8 collected)

codex hook guidance: `{"rollouts": 4, "guidance_contexts": 3, "delivering_hooks": 12, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.2kguwiuy"}], "problems": []}`

exit code: 1
