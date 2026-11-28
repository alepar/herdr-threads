# wake e2e 164511-dcc6

```json
{
  "run_id": "164511-dcc6",
  "sha": "c16d92ee1ba45b81b65c1cb1792198a67196513a",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.54/bin/herdr-threads",
  "bin_sha256": "3a039bd2b6fe76500eea100ae4e431ad2544aaeed3e5ad49016a409821bccae1",
  "root": "/tmp/ht-wake-e2e.odk6gt5t",
  "started_utc": "2026-11-28T00:45:11Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-28T00:55:53Z",
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
| side-nudge/group | 0 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f9d-0860-7510-bb95-bf701027994b", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1145457, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-6138-7dc1-bed8-796e2bc2fc5d", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9d-9b3b-7d22-a0d3-97c43eca15c0", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795826803519, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["888d64488cdcc4c7365a90852e078831"], "sibling_displayed_by": ["888d64488cdcc4c7365a90852e078831"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 14.7, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795826987283, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 0 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f9d-367d-7590-8211-02d7f3e5e990", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-853c-75e2-b6c5-bc0c8ad8b6dc", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9e-a42e-7e90-9a55-b84133f92e52", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795826991644, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 10.88, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795827051307, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f9d-367d-7590-8211-02d7f3e5e990", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-853c-75e2-b6c5-bc0c8ad8b6dc", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9e-a42e-7e90-9a55-b84133f92e52", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21fa2-9672-7723-8fed-e52948894da5", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21fa2-cbc2-7ab1-8322-0604e50a5d72", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 1146428, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795827187991, "retry_step": 2}, "overdue_warning": {"message": "ev2otyf7l6Bb", "event": {"classified_at": 1795827187750, "current_status": "pending", "deadline_at": 1795827187698, "detail": "last wake unsafe", "materialized_at": 1795827187750, "obligation": "receipt", "physical_status": "pending", "seat": "sv2osripApqC", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f9d-0860-7510-bb95-bf701027994b", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1145457, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-6138-7dc1-bed8-796e2bc2fc5d", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9d-9b3b-7d22-a0d3-97c43eca15c0", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9e-ff49-75b0-95b2-59b9f846e12b", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa1-4616-7d21-a331-5b7bcf1d9bc7", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa3-1227-7293-a7f3-2991dd4c6b9b", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145457, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795827327229, "retry_step": 2}, "overdue_warning": {"message": "ev2oEfcmlAMn", "event": {"classified_at": 1795827327086, "current_status": "pending", "deadline_at": 1795827327041, "detail": "last wake unsafe", "materialized_at": 1795827327086, "obligation": "receipt", "physical_status": "pending", "seat": "sv2oZsR1SHob", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | -1 | codex | True | True (1.01) | {"bindings_after_side": [{"session": "01a21f9d-16c8-7833-a0be-de2d2e0f2ba9", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1145663, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-8184-70c0-94c8-b0489f106b89", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9d-c364-7c62-8250-ff3857c4302a", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795826934089, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["82b76362498823ccc689ac90146f8847"], "sibling_displayed_by": ["82b76362498823ccc689ac90146f8847"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 13.15, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795827117468, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | -1 | codex | False | False | {"bindings_after_side": [{"session": "01a21f9d-16c8-7833-a0be-de2d2e0f2ba9", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1145663, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-8184-70c0-94c8-b0489f106b89", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9d-c364-7c62-8250-ff3857c4302a", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa1-016b-7303-abca-f186813cdf0f", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa3-44e3-7ce1-95e5-1f434e59607d", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa3-8012-70d2-bc67-d7c84ba4cc08", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1145663, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795827237193, "retry_step": 2}, "overdue_warning": {"message": "ev2ov5tXy7yf", "event": {"classified_at": 1795827237060, "current_status": "pending", "deadline_at": 1795827235863, "detail": "last wake unsafe", "materialized_at": 1795827237060, "obligation": "receipt", "physical_status": "pending", "seat": "sv2ol6l8icIj", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |
| side-nudge/group | 3 | codex | True | True (0.76) | {"bindings_after_side": [{"session": "01a21f9d-27c6-7941-ada0-7e506634cae3", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1146080, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-8437-76a0-b69b-97e837020404", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9d-c103-7430-b696-7ac9dec3f885", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "late_wake_s": null, "wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795826739401, "retry_step": 1}, "delivered": true, "sibling_side_busy": true, "sibling_side_bindings": ["8781f26094cc09004245c7b016c2b3be"], "sibling_displayed_by": ["8781f26094cc09004245c7b016c2b3be"], "sibling_displayed": true, "sibling_close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}, "sibling_wake_s": 5.82, "sibling_wake_state": {"last_outcome": "submitted", "completed_at_utc": 1795826989236, "retry_step": 2}, "sibling_woke_before_watch": false, "sibling_delivered": true, "queue_transport_ok": true} | PASS |
| side-draft/group | 3 | codex | False | False | {"bindings_after_side": [{"session": "01a21f9d-27c6-7941-ada0-7e506634cae3", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 1146080, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f9d-8437-76a0-b69b-97e837020404", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9d-c103-7430-b696-7ac9dec3f885", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21f9f-2791-7480-9bc3-3db28084ebd7", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa1-4bf5-7dd0-9b77-fce2efff259e", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}, {"session": "01a21fa1-8aa5-77b1-827e-5de3ead98568", "provenance": "cooperative_top_level", "liveness": "disconnected", "pid": 1146080, "disconnect": "liveness_end_event", "concurrent": 1, "auxiliary": 1}], "connected_same_pid": false, "side_setup_ok": true, "side_setup": [{"side": 1, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}, {"side": 2, "new_binding": true, "close": {"archive_dialog": true, "attempts": 1, "archive_dialog_after": false}}], "composer_at_end": "\u203a draft-kept-by-wake-e2e", "draft_intact": true, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795827237324, "retry_step": 2}, "overdue_warning": {"message": "ev2oqzkTzCGD", "event": {"classified_at": 1795827237060, "current_status": "pending", "deadline_at": 1795827232872, "detail": "last wake unsafe", "materialized_at": 1795827237060, "obligation": "receipt", "physical_status": "pending", "seat": "sv2oI9OiBa7i", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": true, "queue_transport_ok": true} | PASS |

8/8 expected rows passed (8 collected)

codex hook guidance: `{"rollouts": 4, "guidance_contexts": 3, "delivering_hooks": 12, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.odk6gt5t"}], "problems": []}`

exit code: 0
