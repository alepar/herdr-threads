# wake e2e 124546-26f3

```json
{
  "run_id": "124546-26f3",
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
  "root": "/tmp/ht-wake-e2e.pasjoyft",
  "started_utc": "2026-11-27T20:45:46Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"szB62eYla0cF\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "4f6a3f33665b3b757b389c7bb83f2f0e",
        "native_session": "01a21ec1-d49e-71b3-81f1-f8d9e5f753a5",
        "execution": "b90e4746-9b93-54df-8ea9-ed22bd1a22ec",
        "generation": 1,
        "process": {
          "pid": 740512,
          "start": 144006622
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ec1-e0f4-7812-a23a-f53092e7643f",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T20:50:08Z",
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
| side-queue-focused/group | 0 | codex | True | True (6.0) | {"bindings_after_side": [{"session": "01a21ec1-d49e-71b3-81f1-f8d9e5f753a5", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 740512, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21ec2-3e2d-7c13-b23d-4f26f0db57ef", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 740512, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "root_session": "01a21ec1-d49e-71b3-81f1-f8d9e5f753a5", "setup": {"side_setup_ok": true, "focused": true, "side_busy": true, "agent_status": "working"}, "screens": {"busy_side": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502", "                                                                               \u2502\u203a Side of wake-e2e side-queue-focused/", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  group 124546-26f3: run the shell", "    + 216 lines (^t to expand)                                                 \u2502  command /tmp/ht-wake-e2e.pasjoyft/", "                                                                               \u2502  bin/herdr-threads inbox once, then", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  print SIDE-FOCUS-DONE. Do not act on", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502  anything it shows.", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.pasjoyft/state --host-endpoin\u2026\u2502\u2022 I\u2019ll run the requested inbox command", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502  once.", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 Ran /tmp/ht-wake-e2e.pasjoyft/bin/he\u2026", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502  \u2514 empty", "                                                                               \u2502    + Show details", "  Worked for 10s \u2022 12:46                                                       \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 6s \u2022 12:46", "                                                                               \u2502", "                                                                               \u2502", "\u2022 I\u2019ll accept the test invitation.                                             \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 124546-26f3: run", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.pasjoyft/state accept tzB64iS\u2026\u2502  the shell command sleep 100, then", "  \u2514 accepted                                                                   \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "    value: \"izB6ewoHYVdS\"                                                      \u2502  run any herdr-threads command.", "    + Show details                                                             \u2502", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502", "  Worked for 4s \u2022 12:46                                                        \u2502\u2022 Working (0s \u2022 esc to interrupt)", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run wake e2e recipient                                             \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "after_send": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "                                                                               \u2502", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502\u203a Side of wake-e2e side-queue-focused/", "    + 216 lines (^t to expand)                                                 \u2502  group 124546-26f3: run the shell", "                                                                               \u2502  command /tmp/ht-wake-e2e.pasjoyft/", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  bin/herdr-threads inbox once, then", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502  print SIDE-FOCUS-DONE. Do not act on", "    + 1 line (^t to expand)                                                    \u2502  anything it shows.", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.pasjoyft/state --host-endpoin\u2026\u2502", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502\u2022 I\u2019ll run the requested inbox command", "    + Show details                                                             \u2502  once.", "                                                                               \u2502", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502\u2022 Ran /tmp/ht-wake-e2e.pasjoyft/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "  Worked for 10s \u2022 12:46                                                       \u2502    + Show details", "                                                                               \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502  Worked for 6s \u2022 12:46", "                                                                               \u2502", "\u2022 I\u2019ll accept the test invitation.                                             \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.pasjoyft/state accept tzB64iS\u2026\u2502  queue-focused/group 124546-26f3: run", "  \u2514 accepted                                                                   \u2502  the shell command sleep 100, then", "    value: \"izB6ewoHYVdS\"                                                      \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "    + Show details                                                             \u2502  run any herdr-threads command.", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502\u2022 I\u2019ll run sleep 100.", "  Worked for 4s \u2022 12:46                                                        \u2502", "\u2022 Working (0s)                                                                 \u2502\u2022 Working (6s \u2022 esc to interrupt) \u00b7 1 \u2026", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run wake e2e recipient                                             \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "side_done": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  print SIDE-FOCUS-DONE. Do not act on", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502  anything it shows.", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.pasjoyft/state --host-endpoin\u2026\u2502\u2022 I\u2019ll run the requested inbox command", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502  once.", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 Ran /tmp/ht-wake-e2e.pasjoyft/bin/he\u2026", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502  \u2514 empty", "                                                                               \u2502    + Show details", "  Worked for 10s \u2022 12:46                                                       \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 6s \u2022 12:46", "                                                                               \u2502", "                                                                               \u2502", "\u2022 I\u2019ll accept the test invitation.                                             \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 124546-26f3: run", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.pasjoyft/state accept tzB64iS\u2026\u2502  the shell command sleep 100, then", "  \u2514 accepted                                                                   \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "    value: \"izB6ewoHYVdS\"                                                      \u2502  run any herdr-threads command.", "    + Show details                                                             \u2502", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502\u2022 I\u2019ll run sleep 100.", "                                                                               \u2502", "  Worked for 4s \u2022 12:46                                                        \u2502\u2022 Ran sleep 100", "                                                                               \u2502  \u2514 (no output)", "\u2022 RECEIVED [herdr-threads] message mzB6yMyBMVVA in tzB64iSo4aQ0 from           \u2502", "  szB62IlGaZWd [p0] [human]: side-queue-focused/group 124546-26f3              \u2502\u2022 SIDE-FOCUS-BUSY-DONE", "                                                                               \u2502", "  Worked for 2s \u2022 12:47                                                        \u2502  Worked for 1m 44s \u2022 12:48", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run wake e2e recipient                                             \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"]}, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "side_typed": false, "armed_s": 0.49, "first_attempt": true, "root_busy_at_arm": false, "activity_evidence": {"attempt": "414b0c61-33a8-40c4-a29a-969f5a7e01b8", "native_session": "01a21ec1-d49e-71b3-81f1-f8d9e5f753a5", "turn_id": "01a21ec3-09ff-7542-9a7f-474973774201", "observed_at_ms": 5724}, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["mzB6yMyBMVVA"], "output_ids": ["mzB6yMyBMVVA"], "started_utc": "2026-11-27T20:47:08.289Z", "completed_utc": "2026-11-27T20:47:10.860Z", "bytes_read": 353505, "files_examined": 2, "session": "01a21ec1-d49e-71b3-81f1-f8d9e5f753a5", "turn_id": "01a21ec3-09ff-7542-9a7f-474973774201"}, "delivered_in_root": true, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 6s \u2022 12:46", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-FOCUS-DONE"], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-FOCUS-DONE"], "wake_markers": 1}, "agent_status_at_send": "working", "side_phase_at_send": ["started"], "side_bits": {"01a21ec2-3e2d-7c13-b23d-4f26f0db57ef": {"auxiliary": 1, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "settled_by_root": true, "wake_state_final": {"last_outcome": "submitted", "completed_at_utc": 1795812423152, "retry_step": 1}, "close_side": {"archive_dialog": true, "archive_dialog_after": false}, "release_blocking": [], "queue_transport_ok": true} | PASS |

1/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.pasjoyft"}], "problems": []}`

exit code: 0
