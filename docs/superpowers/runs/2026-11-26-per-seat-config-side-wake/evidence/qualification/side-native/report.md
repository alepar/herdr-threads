# wake e2e 135352-e441

```json
{
  "run_id": "135352-e441",
  "sha": "715d3a8ce25877e3b5617b2564c191fb0c12127c",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.31/bin/herdr-threads",
  "bin_sha256": "ce0daa6ebd684e61b4b7724ef957d31b7f3c61a499be9169dec185e5403d11d2",
  "root": "/tmp/ht-wake-e2e.5ab0i15d",
  "started_utc": "2026-11-27T21:53:52Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-27T22:15:39Z",
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
        ],
        [
          "SessionStart",
          "fork"
        ],
        [
          "SessionStart",
          "resume"
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
| side-native-defer/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21f00-2b6d-7f31-a972-99dc536c1b80", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2253409, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-85cd-7102-a955-997beec9ec6c", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2253409, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "side_setup_ok": true, "bindings_idle": true, "wake_state_before_send": {"last_outcome": "submitted", "completed_at_utc": 1795816450582, "retry_step": 0}, "refused_while_side": {"last_outcome": "unsafe", "completed_at_utc": 1795816505397, "retry_step": 0}, "wake_state_while_side": {"last_outcome": "unsafe", "completed_at_utc": 1795816505397, "retry_step": 0}, "side_typed": false, "submitted_while_side": false, "wake_state_after_close": {"last_outcome": "unsafe", "completed_at_utc": 1795816533967, "retry_step": 0}, "woke_before_watch": false, "delivered": false, "queue_transport_ok": true} | FAIL |
| side-nudge/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21f00-5a03-77b3-a7ad-78ccbf3089e8", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 2258865, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-a758-7823-8f97-b93cd9474c9b", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 2258865, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": false, "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795816779544, "retry_step": 0}, "delivered": false, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_wake_s": null, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795817019517, "retry_step": 0}, "sibling_woke_before_watch": false, "sibling_hello_sent": true, "sibling_delivered": false, "queue_transport_ok": true} | FAIL |
| side-nudge/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21f00-2b6d-7f31-a972-99dc536c1b80", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2253409, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-85cd-7102-a955-997beec9ec6c", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2253409, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": false, "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795816881871, "retry_step": 0}, "delivered": false, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_wake_s": null, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795817123499, "retry_step": 0}, "sibling_woke_before_watch": false, "sibling_hello_sent": true, "sibling_delivered": false, "queue_transport_ok": true} | FAIL |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f00-5a03-77b3-a7ad-78ccbf3089e8", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 2258865, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-a758-7823-8f97-b93cd9474c9b", "provenance": "cooperative_env_session", "liveness": "connected", "pid": 2258865, "disconnect": null, "concurrent": 0, "auxiliary": 0}], "connected_same_pid": true, "side_setup_ok": false, "composer_at_end": "\u203a                                                                              \u2502", "draft_intact": false, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795817396723, "retry_step": 0}, "overdue_warning": {"message": "eQNy4t86eQpr", "event": {"classified_at": 1795817396600, "current_status": "pending", "deadline_at": 1795817396562, "detail": "last wake unsafe", "materialized_at": 1795817396600, "obligation": "receipt", "physical_status": "pending", "seat": "sQNy55iI4CP0", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": false, "queue_transport_ok": true} | FAIL |
| side-draft/group | 0 | codex | False | False | {"bindings_after_side": [{"session": "01a21f00-2b6d-7f31-a972-99dc536c1b80", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2253409, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-85cd-7102-a955-997beec9ec6c", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2253409, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": false, "composer_at_end": "\u203a                                                                              \u2502", "draft_intact": false, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795817512011, "retry_step": 0}, "overdue_warning": {"message": "eQNyAqbw6s0m", "event": {"classified_at": 1795817511884, "current_status": "pending", "deadline_at": 1795817511849, "detail": "last wake unsafe", "materialized_at": 1795817511884, "obligation": "receipt", "physical_status": "pending", "seat": "sQNy9I3M72mv", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": false, "queue_transport_ok": true} | FAIL |
| config-prose/group | 0 | codex | False | False | {"done_marker": true, "calls": ["--state-dir /tmp/ht-wake-e2e.5ab0i15d/state config seat list", "human --state-dir /tmp/ht-wake-e2e.5ab0i15d/state --host-endpoint $HOME/.config/herdr/herdr.sock config seat set --seat sQNy9I3M72mv programmatic_wake on"], "config_value": "on", "verdict": {"passed": true, "reason": null}, "approval_bypass": "--approve-for-me", "ask_bypassed": true, "queue_transport_ok": false} | FAIL |
| resume-idle/group | 0 | codex | True | True (5.96) | {"unavailable_after_exit": true, "disconnect_reason": "liveness_process_exit", "unavailable_while_idle": true, "wakes_on_p1": 0, "delivered": false, "available_after": true, "queue_transport_ok": true} | FAIL |
| side-nudge/group | -1 | codex | True | False | {"bindings_after_side": [{"session": "01a21f00-3acb-7482-a77d-75f2fc369d3e", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2254875, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-a685-7e62-8a48-267c25fb3a0a", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2254875, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": false, "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795816660319, "retry_step": 0}, "delivered": false, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_wake_s": null, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795816899158, "retry_step": 0}, "sibling_woke_before_watch": false, "sibling_hello_sent": true, "sibling_delivered": false, "queue_transport_ok": true} | FAIL |
| side-draft/group | -1 | codex | False | False | {"bindings_after_side": [{"session": "01a21f00-3acb-7482-a77d-75f2fc369d3e", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2254875, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-a685-7e62-8a48-267c25fb3a0a", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2254875, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": false, "composer_at_end": "\u203a                                                                              \u2502", "draft_intact": false, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795817276613, "retry_step": 0}, "overdue_warning": {"message": "eQNyWkgzMMOr", "event": {"classified_at": 1795817276498, "current_status": "pending", "deadline_at": 1795817276448, "detail": "last wake unsafe", "materialized_at": 1795817276498, "obligation": "receipt", "physical_status": "pending", "seat": "sQNyGL7Gx8th", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": false, "queue_transport_ok": true} | FAIL |
| side-nudge/group | 3 | codex | True | False | {"bindings_after_side": [{"session": "01a21f00-4b11-7331-9f07-612abeab7754", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2256643, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-a17e-72a2-92b4-cb8688fd67fe", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2256643, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": false, "late_wake_s": null, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795816775940, "retry_step": 0}, "delivered": false, "sibling_side_busy": true, "sibling_side_bindings": [], "sibling_displayed_by": [], "sibling_displayed": false, "sibling_wake_s": null, "sibling_wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795816999772, "retry_step": 0}, "sibling_woke_before_watch": false, "sibling_hello_sent": true, "sibling_delivered": true, "queue_transport_ok": true} | FAIL |
| side-draft/group | 3 | codex | False | False | {"bindings_after_side": [{"session": "01a21f00-4b11-7331-9f07-612abeab7754", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2256643, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f00-a17e-72a2-92b4-cb8688fd67fe", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 2256643, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "connected_same_pid": true, "side_setup_ok": false, "composer_at_end": "\u203a                                                                              \u2502", "draft_intact": false, "wake_state": {"last_outcome": "unsafe", "completed_at_utc": 1795817371792, "retry_step": 0}, "overdue_warning": {"message": "eQNyIt5rXbiK", "event": {"classified_at": 1795817371576, "current_status": "pending", "deadline_at": 1795817368741, "detail": "last wake unsafe", "materialized_at": 1795817371576, "obligation": "receipt", "physical_status": "pending", "seat": "sQNyzQ634oEu", "wake": "unsafe"}}, "warning_wake": "unsafe", "warning_detail": "last wake unsafe", "delivered": false, "queue_transport_ok": true} | FAIL |

0/11 expected rows passed (11 collected)

codex hook guidance: `{"rollouts": 9, "guidance_contexts": 7, "delivering_hooks": 11, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop, SessionStart/fork, SessionStart/resume; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.5ab0i15d"}], "problems": []}`

exit code: 1
