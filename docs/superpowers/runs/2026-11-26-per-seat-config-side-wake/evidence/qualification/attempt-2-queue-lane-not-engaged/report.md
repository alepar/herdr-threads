# wake e2e 111342-2224

```json
{
  "run_id": "111342-2224",
  "sha": "bf335ef3f99ee6a3b2d23a6c998db7c43e37f772",
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
  "root": "/tmp/ht-wake-e2e.iiwzkk4s",
  "started_utc": "2026-11-27T19:13:42Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sODMEFO2CPg2\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "d871a899333d9d8ab5d831de535cf65a",
        "native_session": "01a21e6d-8e5e-7c80-868b-a30971757cea",
        "execution": "ee641afb-d90c-595a-8b43-be18d6452aaf",
        "generation": 1,
        "process": {
          "pid": 3986846,
          "start": 143454325
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e6d-9add-7792-8151-30c74ffbcd0f",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:18:16Z",
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
| side-queue-focused/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21e6d-8e5e-7c80-868b-a30971757cea", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3986846, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21e6d-f82b-74c1-88cc-1a8d15da6c13", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3986846, "disconnect": null, "concurrent": 1, "auxiliary": 0}], "root_session": "01a21e6d-8e5e-7c80-868b-a30971757cea", "setup": {"side_setup_ok": true, "focused": true, "side_busy": true, "agent_status": "working"}, "screens": {"busy_side": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502", "                                                                               \u2502\u203a Side of wake-e2e side-queue-focused/", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  group 111342-2224: run the shell", "    + 216 lines (^t to expand)                                                 \u2502  command /tmp/ht-wake-e2e.iiwzkk4s/", "                                                                               \u2502  bin/herdr-threads inbox once, then", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  print SIDE-FOCUS-DONE. Do not act on", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502  anything it shows.", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.iiwzkk4s/state --host-endpoin\u2026\u2502\u2022 I\u2019ll run the inbox command once.", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502", "    + Show details                                                             \u2502\u2022 Ran /tmp/ht-wake-e2e.iiwzkk4s/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502    + Show details", "                                                                               \u2502", "  Worked for 10s \u2022 11:13                                                       \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "                                                                               \u2502  Worked for 5s \u2022 11:14", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  queue-focused/group 111342-2224: run", "                                                                               \u2502  the shell command sleep 100, then", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.iiwzkk4s/state accept tODMwze\u2026\u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "  \u2514 accepted                                                                   \u2502  run any herdr-threads command.", "    value: \"iODM94GoqE2v\"                                                      \u2502", "    + Show details                                                             \u2502", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502", "  Worked for 5s \u2022 11:14                                                        \u2502\u2022 Working (0s \u2022 esc to interrupt)", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run the wake e2e recipient                                         \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "after_send": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502\u203a Side of wake-e2e side-queue-focused/", "                                                                               \u2502  group 111342-2224: run the shell", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502  command /tmp/ht-wake-e2e.iiwzkk4s/", "    + 216 lines (^t to expand)                                                 \u2502  bin/herdr-threads inbox once, then", "                                                                               \u2502  print SIDE-FOCUS-DONE. Do not act on", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502  anything it shows.", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502", "    + 1 line (^t to expand)                                                    \u2502", "                                                                               \u2502\u2022 I\u2019ll run the inbox command once.", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.iiwzkk4s/state --host-endpoin\u2026\u2502", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502\u2022 Ran /tmp/ht-wake-e2e.iiwzkk4s/bin/he\u2026", "    + Show details                                                             \u2502  \u2514 empty", "                                                                               \u2502    + Show details", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "  Worked for 10s \u2022 11:13                                                       \u2502", "                                                                               \u2502  Worked for 5s \u2022 11:14", "                                                                               \u2502", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 111342-2224: run", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  the shell command sleep 100, then", "                                                                               \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.iiwzkk4s/state accept tODMwze\u2026\u2502  run any herdr-threads command.", "  \u2514 accepted                                                                   \u2502", "    value: \"iODM94GoqE2v\"                                                      \u2502", "    + Show details                                                             \u2502\u2022 I\u2019ll run sleep 100 and wait for it to", "                                                                               \u2502  finish.", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502\u2022 Waiting for background terminal (1m \u2026", "  Worked for 5s \u2022 11:14                                                        \u2502  \u2514 sleep 100", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run the wake e2e recipient                                         \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "side_done": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "  \u2514 Administrative recovery uses immediate argv1 `human`, with exact reported \u2026\u2502  print SIDE-FOCUS-DONE. Do not act on", "                                                                               \u2502  anything it shows.", "    Quiet channels can archive automatically after an uninterrupted hour of ve\u2026\u2502", "    + 216 lines (^t to expand)                                                 \u2502", "                                                                               \u2502\u2022 I\u2019ll run the inbox command once.", "\u2022 Exited (status 2) herdr-threads nudge-threshold 0                            \u2502", "  \u2514 herdr-threads: no resolved seat is mapped to pane w2:p3; run `herdr-thread\u2026\u2502\u2022 Ran /tmp/ht-wake-e2e.iiwzkk4s/bin/he\u2026", "    + 1 line (^t to expand)                                                    \u2502  \u2514 empty", "                                                                               \u2502    + Show details", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.iiwzkk4s/state --host-endpoin\u2026\u2502", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502\u2022 SIDE-FOCUS-DONE", "    + Show details                                                             \u2502", "                                                                               \u2502  Worked for 5s \u2022 11:14", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502", "                                                                               \u2502", "  Worked for 10s \u2022 11:13                                                       \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 111342-2224: run", "                                                                               \u2502  the shell command sleep 100, then", "\u203a herdr-threads: wake up                                                       \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "                                                                               \u2502  run any herdr-threads command.", "                                                                               \u2502", "\u2022 I\u2019ll accept the test invitation.                                             \u2502", "                                                                               \u2502\u2022 I\u2019ll run sleep 100 and wait for it to", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.iiwzkk4s/state accept tODMwze\u2026\u2502  finish.", "  \u2514 accepted                                                                   \u2502", "    value: \"iODM94GoqE2v\"                                                      \u2502\u2022 Ran sleep 100", "    + Show details                                                             \u2502  \u2514 (no output)", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502\u2022 SIDE-FOCUS-BUSY-DONE", "                                                                               \u2502", "  Worked for 5s \u2022 11:14                                                        \u2502  Worked for 1m 43s \u2022 11:16", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run the wake e2e recipient                                         \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"]}, "side_typed": false, "armed_s": null, "first_attempt": false, "root_busy_at_arm": false, "activity_evidence": null, "delivery_evidence": null, "delivered_in_root": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-FOCUS-DONE"], "wake_markers": 1}, "agent_status_at_send": "working", "side_phase_at_send": ["started"], "side_bits": {"01a21e6d-f82b-74c1-88cc-1a8d15da6c13": {"auxiliary": 0, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "settled_by_root": false, "wake_state_final": {"last_outcome": "unsafe", "completed_at_utc": 1795806956213, "retry_step": 0}, "close_side": {"archive_dialog": true, "archive_dialog_after": false}, "release_blocking": [], "queue_transport_ok": false} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: False

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.iiwzkk4s"}], "problems": []}`

exit code: 1
