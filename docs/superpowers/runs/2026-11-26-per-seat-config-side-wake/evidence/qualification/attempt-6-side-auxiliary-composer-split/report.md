# wake e2e 122744-3847

```json
{
  "run_id": "122744-3847",
  "sha": "0f6082f726c01c0d4a24c24df1b7b55baea8abf0",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.43/bin/herdr-threads",
  "bin_sha256": "80b1f2370e9c8257d63a99fe7774cb12609a505594d55616ad5e2b83a65de6f7",
  "root": "/tmp/ht-wake-e2e.cci56cmz",
  "started_utc": "2026-11-27T20:27:44Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"syCbJYXSj2Nh\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "ed9be6d38af4abbcc3b13899a7939920",
        "native_session": "01a21eb1-4eb4-7d92-ba87-be11f09331af",
        "execution": "6060db7b-1f72-54c3-9574-143ffc90ae75",
        "generation": 1,
        "process": {
          "pid": 582862,
          "start": 143898372
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21eb1-5c6b-7ae1-869b-feffee8f6d8f",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T20:32:28Z",
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
| side-queue-focused/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21eb1-4eb4-7d92-ba87-be11f09331af", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 582862, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21eb1-e559-7be1-a783-7cc382386e4d", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 582862, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "root_session": "01a21eb1-4eb4-7d92-ba87-be11f09331af", "setup": {"side_setup_ok": true, "focused": true, "side_busy": true, "agent_status": "working"}, "screens": {"busy_side": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "    For more information, try '--help'. (invalid_request)                      \u2502", "    + 2 lines (^t to expand)                                                   \u2502\u203a Side of wake-e2e side-queue-focused/", "                                                                               \u2502  group 122744-3847: run the shell", "\u2022 Ran herdr-threads --help                                                     \u2502  command /tmp/ht-wake-e2e.cci56cmz/", "  \u2514   5  outcome unknown; inspect `herdr-threads pending-ops` and `retry` the \u2026\u2502  bin/herdr-threads inbox once, then", "                                                                               \u2502  print SIDE-FOCUS-DONE. Do not act on", "    Are you an AI agent? Before using threads, run `herdr-threads skill` for c\u2026\u2502  anything it shows.", "    + 95 lines (^t to expand)                                                  \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.cci56cmz/state --host-endpoin\u2026\u2502\u2022 I\u2019ll run the requested inbox command", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502  once.", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 Ran /tmp/ht-wake-e2e.cci56cmz/bin/he\u2026", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502  \u2514 empty", "                                                                               \u2502    + Show details", "  Worked for 16s \u2022 12:28                                                       \u2502", "                                                                               \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 6s \u2022 12:28", "                                                                               \u2502", "                                                                               \u2502", "\u2022 I\u2019ll accept the test invitation.                                             \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 122744-3847: run", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.cci56cmz/state accept tyCbG2U\u2026\u2502  the shell command sleep 100, then", "  \u2514 accepted                                                                   \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "    value: \"iyCblxKeaaAJ\"                                                      \u2502  run any herdr-threads command.", "    + Show details                                                             \u2502", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502", "  Worked for 6s \u2022 12:28                                                        \u2502\u2022 Working (0s \u2022 esc to interrupt)", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run wake e2e test recipient                                        \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "after_send": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "    For more information, try '--help'. (invalid_request)                      \u2502\u203a Side of wake-e2e side-queue-focused/", "    + 2 lines (^t to expand)                                                   \u2502  group 122744-3847: run the shell", "                                                                               \u2502  command /tmp/ht-wake-e2e.cci56cmz/", "\u2022 Ran herdr-threads --help                                                     \u2502  bin/herdr-threads inbox once, then", "  \u2514   5  outcome unknown; inspect `herdr-threads pending-ops` and `retry` the \u2026\u2502  print SIDE-FOCUS-DONE. Do not act on", "                                                                               \u2502  anything it shows.", "    Are you an AI agent? Before using threads, run `herdr-threads skill` for c\u2026\u2502", "    + 95 lines (^t to expand)                                                  \u2502", "                                                                               \u2502\u2022 I\u2019ll run the requested inbox command", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.cci56cmz/state --host-endpoin\u2026\u2502  once.", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502", "    + Show details                                                             \u2502\u2022 Ran /tmp/ht-wake-e2e.cci56cmz/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502    + Show details", "                                                                               \u2502", "  Worked for 16s \u2022 12:28                                                       \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "                                                                               \u2502  Worked for 6s \u2022 12:28", "\u203a herdr-threads: wake up                                                       \u2502", "                                                                               \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "\u2022 I\u2019ll accept the test invitation.                                             \u2502  queue-focused/group 122744-3847: run", "                                                                               \u2502  the shell command sleep 100, then", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.cci56cmz/state accept tyCbG2U\u2026\u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "  \u2514 accepted                                                                   \u2502  run any herdr-threads command.", "    value: \"iyCblxKeaaAJ\"                                                      \u2502", "    + Show details                                                             \u2502", "                                                                               \u2502\u2022 I\u2019ll run sleep 100 now.", "\u2022 Accepted.                                                                    \u2502", "                                                                               \u2502\u2022 Waiting for background terminal (1m \u2026", "  Worked for 6s \u2022 12:28                                                        \u2502  \u2514 sleep 100", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run wake e2e test recipient                                        \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"], "side_done": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "    For more information, try '--help'. (invalid_request)                      \u2502  print SIDE-FOCUS-DONE. Do not act on", "    + 2 lines (^t to expand)                                                   \u2502  anything it shows.", "                                                                               \u2502", "\u2022 Ran herdr-threads --help                                                     \u2502", "  \u2514   5  outcome unknown; inspect `herdr-threads pending-ops` and `retry` the \u2026\u2502\u2022 I\u2019ll run the requested inbox command", "                                                                               \u2502  once.", "    Are you an AI agent? Before using threads, run `herdr-threads skill` for c\u2026\u2502", "    + 95 lines (^t to expand)                                                  \u2502\u2022 Ran /tmp/ht-wake-e2e.cci56cmz/bin/he\u2026", "                                                                               \u2502  \u2514 empty", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.cci56cmz/state --host-endpoin\u2026\u2502    + Show details", "  \u2514 nudge_threshold: 0 (unchanged)                                             \u2502", "    + Show details                                                             \u2502\u2022 SIDE-FOCUS-DONE", "                                                                               \u2502", "\u2022 SETUP-DONE nudge_threshold=0                                                 \u2502  Worked for 6s \u2022 12:28", "                                                                               \u2502", "  Worked for 16s \u2022 12:28                                                       \u2502", "                                                                               \u2502\u203a Side focus test of wake-e2e side-", "                                                                               \u2502  queue-focused/group 122744-3847: run", "\u203a herdr-threads: wake up                                                       \u2502  the shell command sleep 100, then", "                                                                               \u2502  print SIDE-FOCUS-BUSY-DONE. Do not", "                                                                               \u2502  run any herdr-threads command.", "\u2022 I\u2019ll accept the test invitation.                                             \u2502", "                                                                               \u2502", "\u2022 Ran herdr-threads --state-dir /tmp/ht-wake-e2e.cci56cmz/state accept tyCbG2U\u2026\u2502\u2022 I\u2019ll run sleep 100 now.", "  \u2514 accepted                                                                   \u2502", "    value: \"iyCblxKeaaAJ\"                                                      \u2502\u2022 Ran sleep 100", "    + Show details                                                             \u2502  \u2514 (no output)", "                                                                               \u2502", "\u2022 Accepted.                                                                    \u2502\u2022 SIDE-FOCUS-BUSY-DONE", "                                                                               \u2502", "  Worked for 6s \u2022 12:28                                                        \u2502  Worked for 1m 44s \u2022 12:30", "                                                                               \u2502", "                                                                               \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question", "                                                                               \u2502", "  GPT-6.2-Sol default \u00b7 ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-\u2502  GPT-6.2-Sol default \u00b7 ~/AleCode/", "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-   \u2502  herdr-threads/.worktrees/super-auto-", "  z41e.43 \u00b7 Run wake e2e test recipient                                        \u2502  per-seat-config-side-wake/.worktrees\u2026", "                                                                               \u2502  \u2190 for agents \u00b7 ? for shortcuts"]}, "daemon_view": {"doctor_wake": null, "diagnostics": "{\"version\":9,\"result\":{\"kind\":\"diagnostics\",\"data\":{\"items\":[],\"next_cursor\":null,\"next_argv\":null,\"high_water_ordinal\":1,\"scope_revision\":null,\"has_more\":false,\"stop_reason\":\"complete\",\"consistency\":\"bounded_live\"}}}"}, "side_typed": false, "armed_s": null, "first_attempt": false, "root_busy_at_arm": false, "activity_evidence": null, "delivery_evidence": null, "delivered_in_root": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502  Worked for 6s \u2022 12:28", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": ["                                                                               \u2502\u2022 SIDE-FOCUS-DONE"], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side-\u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "agent_status_at_send": "working", "side_phase_at_send": ["started"], "side_bits": {"01a21eb1-e559-7be1-a783-7cc382386e4d": {"auxiliary": 1, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "settled_by_root": false, "wake_state_final": {"last_outcome": "unsafe", "completed_at_utc": 1795811407414, "retry_step": 0}, "close_side": {"archive_dialog": true, "archive_dialog_after": false}, "release_blocking": [], "queue_transport_ok": false} | FAIL |

0/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: False

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 1, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"skipped": "--keep", "workspace": "w2", "daemon_attempted": true, "problems": []}`

exit code: 1
