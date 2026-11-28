# wake e2e 160514-dfbe

```json
{
  "run_id": "160514-dfbe",
  "sha": "d1c26b022fb5758837405a000961488ec412637f",
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
  "root": "/tmp/ht-wake-e2e.7r7giwnk",
  "started_utc": "2026-11-28T00:05:14Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-28T00:18:06Z",
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
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f78-7239-7431-acde-48caedc6e4cc", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 870752, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-f585-7e51-8cdf-4f62edb3ced2", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 870752, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f79-3312-7b91-b105-e9a88fae8657", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 870752, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795824580569, "retry_step": 0}, "overdue_warning": {"message": "eIiwvWwEUpPi", "event": {"classified_at": 1795824580239, "current_status": "pending", "deadline_at": 1795824580191, "detail": "last wake unsafe", "materialized_at": 1795824580239, "obligation": "receipt", "physical_status": "pending", "seat": "sIiwNjOv3a1i", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f78-a152-7722-99ef-a0b56548d610", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-eb81-7f80-a56a-025d5277a897", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f7a-f61a-71c2-b962-30b4644f3d37", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795824697771, "retry_step": 0}, "overdue_warning": {"message": "eIiwhMQyjAKF", "event": {"classified_at": 1795824697749, "current_status": "pending", "deadline_at": 1795824697688, "detail": "last wake unsafe", "materialized_at": 1795824697749, "obligation": "receipt", "physical_status": "pending", "seat": "sIiwLprTj67T", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 0 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f78-a152-7722-99ef-a0b56548d610", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-eb81-7f80-a56a-025d5277a897", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f7a-f61a-71c2-b962-30b4644f3d37", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f7e-b2e3-7d80-a121-96d825b63d14", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f7e-ed4d-7002-a6ed-07ed85b96d58", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 877808, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795824697771, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 5.4, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824862676, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 0 | codex | True | True (1.01) | {"bindings_after_side": [{"session": "01a21f78-7239-7431-acde-48caedc6e4cc", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 870752, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-f585-7e51-8cdf-4f62edb3ced2", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 870752, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f79-3312-7b91-b105-e9a88fae8657", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 870752, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7c-df11-7ad1-93bc-86a420fa7014", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 870752, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7e-ecad-7e71-ad02-0d13c9d479ac", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 870752, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824912665, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["6b073cf4e86adb341ac1fbdbcfd9b110"], "sibling_displayed_by": ["6b073cf4e86adb341ac1fbdbcfd9b110"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 3.78, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824980004, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | -1 | codex | False | False | {"bindings_after_side": [{"session": "01a21f78-81a3-7cc2-90eb-6be054ead96e", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 874283, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-dbec-7b73-8ba4-8ada1938f5f7", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 874283, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7a-dfcb-7390-8baa-af76e37de71e", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 874283, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795824692973, "retry_step": 0}, "overdue_warning": {"message": "eIiwrvs1gX6I", "event": {"classified_at": 1795824692745, "current_status": "pending", "deadline_at": 1795824690711, "detail": "last wake unsafe", "materialized_at": 1795824692745, "obligation": "receipt", "physical_status": "pending", "seat": "sIiwalZNghgq", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | -1 | codex | True | True (1.01) | {"bindings_after_side": [{"session": "01a21f78-81a3-7cc2-90eb-6be054ead96e", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 874283, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-dbec-7b73-8ba4-8ada1938f5f7", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 874283, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7a-dfcb-7390-8baa-af76e37de71e", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 874283, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7e-8d7f-7ed2-ad26-2023758d1200", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 874283, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f80-9f19-7a32-bfd9-0d3a13e62a16", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 874283, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824904205, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["d4acd40d3b04bff200d3ac9c89d2c879"], "sibling_displayed_by": ["d4acd40d3b04bff200d3ac9c89d2c879"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 8.58, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795825081771, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 3 | codex | False | False | {"bindings_after_side": [{"session": "01a21f78-9183-7873-8699-e3a4c7055b19", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 875915, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-edd5-7ca0-bedb-3b5b125a69fc", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 875915, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f79-2517-7231-9e63-32db2bf633fa", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 875915, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795824459680, "retry_step": 0}, "overdue_warning": {"message": "eIiwOUhLgwOo", "event": {"classified_at": 1795824459437, "current_status": "pending", "deadline_at": 1795824459389, "detail": "last wake unsafe", "materialized_at": 1795824459437, "obligation": "receipt", "physical_status": "pending", "seat": "sIiwVsxrkj5l", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 3 | codex | True | True (1.01) | {"bindings_after_side": [{"session": "01a21f78-9183-7873-8699-e3a4c7055b19", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 875915, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f78-edd5-7ca0-bedb-3b5b125a69fc", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 875915, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f79-2517-7231-9e63-32db2bf633fa", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 875915, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7b-06f3-7172-a25f-f0437924fa33", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 875915, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f7d-14d4-7383-9a1a-c75eb804ce49", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 875915, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824672528, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["d2f7bc53688bf730d1bc93fa5295f445"], "sibling_displayed_by": ["d2f7bc53688bf730d1bc93fa5295f445"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 12.38, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795824854663, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |

8/8 expected rows passed (8 collected)

codex hook guidance: `{"rollouts": 4, "guidance_contexts": 3, "delivering_hooks": 12, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.7r7giwnk"}], "problems": []}`

exit code: 0
