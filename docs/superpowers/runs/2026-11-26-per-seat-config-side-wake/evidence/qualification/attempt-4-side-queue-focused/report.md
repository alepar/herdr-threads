# wake e2e 112205-9d65

```json
{
  "run_id": "112205-9d65",
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
  "root": "/tmp/ht-wake-e2e.vlvsubbm",
  "started_utc": "2026-11-27T19:22:05Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sgfzQBexQmb9\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "8f3ecb099a88bd9c15885af12a80bee4",
        "native_session": "01a21e75-36c3-79c3-a116-82fae912f07f",
        "execution": "10db2c19-5dcf-592f-95b2-8077bf3107a8",
        "generation": 1,
        "process": {
          "pid": 114220,
          "start": 143504523
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e75-43ba-7ab2-b4b7-a93a29aa7f6f",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:26:39Z",
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
  }
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| side-queue-focused/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21e75-36c3-79c3-a116-82fae912f07f", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 114220, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21e75-9f75-78c0-946a-3a3ac202295f", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 114220, "disconnect": null, "concurrent": 1, "auxiliary": 0}], "root_session": "01a21e75-36c3-79c3-a116-82fae912f07f", "setup": {"side_setup_ok": true, "focused": true, "side_busy": true, "agent_status": "working"}, "screens": {"busy_side": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502", "                                                                               \u2502\u203a Side of wake-e2e side-queue-focused/", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  group 112205-9d65: run the shell", "    + 216 lines (^t to expand)                                                 \u2502  command /tmp/ht-wake-e2e.vlvsubbm/", "                                                                               \u2502  bin/herdr-threads inbox once, then", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  print SIDE-FOCUS-DONE. Do not act on", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502  anything it shows.", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vlvsubbm/state --host-endpoin\u2026\u2502\u2022 I\u2019ll run the inbox command once.", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502", "    + Show details                                                             \u2502\u2022 Ran /tmp/ht-wake-e2e.vlvsubbm/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502    + Show details", "                                                                               \u2502", "  Worked for 10s \u2022 11:22                                                       \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "                                                                               \u2502  Worked for 5s \u2022 11:22", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  queue-focused/group 112205-9d65: run", "                                                                               \u2502  the shell command sleep 100, then", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vlvsubbm/state accept tgfzkbs\u2026\u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "  \u2514 accepted                                                                   \u2502  run any herdr-threads command.", "    value: \"igfzx7WqEFSg\"                                                      \u2502", "    + Show details                                                             \u2502", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502", "  Worked for 4s \u2022 11:22                                                        \u2502\u2022 Working (0s \u2022 esc to interrupt)", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run herdr-threads wake e2e                                         \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "after_send": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502", "                                                                               \u2502\u203a Side of wake-e2e side-queue-focused/", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  group 112205-9d65: run the shell", "    + 216 lines (^t to expand)                                                 \u2502  command /tmp/ht-wake-e2e.vlvsubbm/", "                                                                               \u2502  bin/herdr-threads inbox once, then", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  print SIDE-FOCUS-DONE. Do not act on", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502  anything it shows.", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vlvsubbm/state --host-endpoin\u2026\u2502\u2022 I\u2019ll run the inbox command once.", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502", "    + Show details                                                             \u2502\u2022 Ran /tmp/ht-wake-e2e.vlvsubbm/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502    + Show details", "                                                                               \u2502", "  Worked for 10s \u2022 11:22                                                       \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "                                                                               \u2502  Worked for 5s \u2022 11:22", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  queue-focused/group 112205-9d65: run", "                                                                               \u2502  the shell command sleep 100, then", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vlvsubbm/state accept tgfzkbs\u2026\u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "  \u2514 accepted                                                                   \u2502  run any herdr-threads command.", "    value: \"igfzx7WqEFSg\"                                                      \u2502", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 I\u2019ll run the 100-second sleep.", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502\u2022 Waiting for background terminal (1m \u2026", "  Worked for 4s \u2022 11:22                                                        \u2502  \u2514 sleep 100", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run herdr-threads wake e2e                                         \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "side_done": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502  bin/herdr-threads inbox once, then", "                                                                               \u2502  print SIDE-FOCUS-DONE. Do not act on", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  anything it shows.", "    + 216 lines (^t to expand)                                                 \u2502", "                                                                               \u2502", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502\u2022 I\u2019ll run the inbox command once.", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502", "    + 1 line (^t to expand)                                                    \u2502\u2022 Ran /tmp/ht-wake-e2e.vlvsubbm/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vlvsubbm/state --host-endpoin\u2026\u2502    + Show details", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502", "    + Show details                                                             \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502  Worked for 5s \u2022 11:22", "                                                                               \u2502", "  Worked for 10s \u2022 11:22                                                       \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 112205-9d65: run", "\u203a herdr-threads: wake up                                                       \u2502  the shell command sleep 100, then", "                                                                               \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "                                                                               \u2502  run any herdr-threads command.", "\u2022 I\u2019ll accept the test invitation.                                             \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vlvsubbm/state accept tgfzkbs\u2026\u2502\u2022 I\u2019ll run the 100-second sleep.", "  \u2514 accepted                                                                   \u2502", "    value: \"igfzx7WqEFSg\"                                                      \u2502\u2022 Ran sleep 100", "    + Show details                                                             \u2502  \u2514 (no output)", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502\u2022 SIDE-FOCUS-BUSY-DONE", "                                                                               \u2502", "  Worked for 4s \u2022 11:22                                                        \u2502  Worked for 1m 44s \u2022 11:25", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run herdr-threads wake e2e                                         \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"]}, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "side_typed": false, "armed_s": null, "first_attempt": false, "root_busy_at_arm": false, "activity_evidence": null, "delivery_evidence": null, "delivered_in_root": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "agent_status_at_send": "working", "side_phase_at_send": ["started"], "side_bits": {"01a21e75-9f75-78c0-946a-3a3ac202295f": {"auxiliary": 0, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "settled_by_root": false, "wake_state_final": {"last_outcome": "unsafe", "completed_at_utc": 1795807455877, "retry_step": 0}, "close_side": {"archive_dialog": true, "archive_dialog_after": false}, "release_blocking": [], "queue_transport_ok": false} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: False

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.vlvsubbm"}], "problems": []}`

exit code: 1
