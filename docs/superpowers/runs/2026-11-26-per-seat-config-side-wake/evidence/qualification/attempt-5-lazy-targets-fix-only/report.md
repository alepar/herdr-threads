# wake e2e 120929-dd69

```json
{
  "run_id": "120929-dd69",
  "sha": "a7d1d92e0b2af33d65d31026a60a5873ac12c9b3",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.43/bin/herdr-threads",
  "bin_sha256": "fff0ee74c8b3aac9adab52224fa096f18e4d9b9c81943f08f047d950005172d0",
  "root": "/tmp/ht-wake-e2e.vhwkrgkr",
  "started_utc": "2026-11-27T20:09:29Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sG3xnaAQxk4u\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "89a08ebaa9957dfde8dc4811c7ff8834",
        "native_session": "01a21ea0-9b28-7633-9a6b-3f5dfba8492b",
        "execution": "0ffe0244-2d20-5295-8b09-b0d47bc7004f",
        "generation": 1,
        "process": {
          "pid": 422952,
          "start": 143788903
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ea0-a8ac-7973-9e33-4e73e814e09b",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T20:16:03Z",
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
| side-queue-focused/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21ea0-9b28-7633-9a6b-3f5dfba8492b", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 422952, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21ea1-03da-7bc1-9404-cdb96b13ad61", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 422952, "disconnect": null, "concurrent": 1, "auxiliary": 0}], "root_session": "01a21ea0-9b28-7633-9a6b-3f5dfba8492b", "setup": {"side_setup_ok": true, "focused": true, "side_busy": true, "agent_status": "working"}, "screens": {"busy_side": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "                                                                               \u2502", "\u2022 Ran herdr-threads skill                                                      \u2502\u203a Side of wake-e2e side-queue-focused/", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502  group 120929-dd69: run the shell", "                                                                               \u2502  command /tmp/ht-wake-e2e.vhwkrgkr/", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  bin/herdr-threads inbox once, then", "    + 216 lines (^t to expand)                                                 \u2502  print SIDE-FOCUS-DONE. Do not act on", "                                                                               \u2502  anything it shows.", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502", "    + 1 line (^t to expand)                                                    \u2502\u2022 I\u2019ll run the inbox command once.", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vhwkrgkr/state --host-endpoin\u2026\u2502\u2022 Ran /tmp/ht-wake-e2e.vhwkrgkr/bin/he\u2026", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502  \u2514 empty", "    + Show details                                                             \u2502    + Show details", "                                                                               \u2502", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "  Worked for 10s \u2022 12:09                                                       \u2502  Worked for 5s \u2022 12:10", "                                                                               \u2502", "                                                                               \u2502", "\u203a herdr-threads: wake up                                                       \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 120929-dd69: run", "                                                                               \u2502  the shell command sleep 100, then", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "                                                                               \u2502  run any herdr-threads command.", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vhwkrgkr/state accept tG3xbiD\u2026\u2502", "  \u2514 accepted                                                                   \u2502", "    value: \"iG3xybVpivOc\"                                                      \u2502", "    + Show details                                                             \u2502", "                                                                               \u2502", "  Worked for 4s \u2022 12:09                                                        \u2502\u2022 Working (0s \u2022 esc to interrupt)", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Accept herdr-threads invitations                                   \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "after_send": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "                                                                               \u2502\u203a Side of wake-e2e side-queue-focused/", "\u2022 Ran herdr-threads skill                                                      \u2502  group 120929-dd69: run the shell", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502  command /tmp/ht-wake-e2e.vhwkrgkr/", "                                                                               \u2502  bin/herdr-threads inbox once, then", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  print SIDE-FOCUS-DONE. Do not act on", "    + 216 lines (^t to expand)                                                 \u2502  anything it shows.", "                                                                               \u2502", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502\u2022 I\u2019ll run the inbox command once.", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502\u2022 Ran /tmp/ht-wake-e2e.vhwkrgkr/bin/he\u2026", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vhwkrgkr/state --host-endpoin\u2026\u2502  \u2514 empty", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502    + Show details", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502", "                                                                               \u2502  Worked for 5s \u2022 12:10", "  Worked for 10s \u2022 12:09                                                       \u2502", "                                                                               \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "\u203a herdr-threads: wake up                                                       \u2502  queue-focused/group 120929-dd69: run", "                                                                               \u2502  the shell command sleep 100, then", "                                                                               \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  run any herdr-threads command.", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vhwkrgkr/state accept tG3xbiD\u2026\u2502", "  \u2514 accepted                                                                   \u2502\u2022 I\u2019ll run sleep 100 in this side", "    value: \"iG3xybVpivOc\"                                                      \u2502  conversation.", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 Waiting for background terminal (1m \u2026", "  Worked for 4s \u2022 12:09                                                        \u2502  \u2514 sleep 100", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Accept herdr-threads invitations                                   \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "side_done": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "                                                                               \u2502  print SIDE-FOCUS-DONE. Do not act on", "\u2022 Ran herdr-threads skill                                                      \u2502  anything it shows.", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502", "                                                                               \u2502", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502\u2022 I\u2019ll run the inbox command once.", "    + 216 lines (^t to expand)                                                 \u2502", "                                                                               \u2502\u2022 Ran /tmp/ht-wake-e2e.vhwkrgkr/bin/he\u2026", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  \u2514 empty", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502    + Show details", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vhwkrgkr/state --host-endpoin\u2026\u2502", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502  Worked for 5s \u2022 12:10", "    + Show details                                                             \u2502", "                                                                               \u2502", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 120929-dd69: run", "  Worked for 10s \u2022 12:09                                                       \u2502  the shell command sleep 100, then", "                                                                               \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "                                                                               \u2502  run any herdr-threads command.", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502", "                                                                               \u2502\u2022 I\u2019ll run sleep 100 in this side", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  conversation.", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.vhwkrgkr/state accept tG3xbiD\u2026\u2502\u2022 Ran sleep 100", "  \u2514 accepted                                                                   \u2502  \u2514 (no output)", "    value: \"iG3xybVpivOc\"                                                      \u2502", "    + Show details                                                             \u2502\u2022 SIDE-FOCUS-BUSY-DONE", "                                                                               \u2502", "  Worked for 4s \u2022 12:09                                                        \u2502  Worked for 1m 44s \u2022 12:14", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Accept herdr-threads invitations                                   \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"]}, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "side_typed": false, "armed_s": null, "first_attempt": false, "root_busy_at_arm": false, "activity_evidence": null, "delivery_evidence": null, "delivered_in_root": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502\u203a Side focus test of wake-e2e side-", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502  queue-focused/group 120929-dd69: run", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-FOCUS-DONE"], "wake_markers": 1}, "agent_status_at_send": "working", "side_phase_at_send": ["started"], "side_bits": {"01a21ea1-03da-7bc1-9404-cdb96b13ad61": {"auxiliary": 0, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "settled_by_root": false, "wake_state_final": {"last_outcome": "unsafe", "completed_at_utc": 1795810436077, "retry_step": 0}, "close_side": {"archive_dialog": true, "archive_dialog_after": false}, "release_blocking": [], "queue_transport_ok": false} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: False

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.vhwkrgkr"}], "problems": []}`

exit code: 1
