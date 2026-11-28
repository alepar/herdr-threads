# wake e2e 132831-ac72

```json
{
  "run_id": "132831-ac72",
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
  "root": "/tmp/ht-wake-e2e.15os8j8w",
  "started_utc": "2026-11-27T21:28:31Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t-1": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sJTdEW9lAsxz\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sJTdBgtIsbaH\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t3": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sJTdvqJEgsUd\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t-1": {
      "adapter": "codex",
      "scope": {
        "binding": "1f808c4f72b3965cbae078006a86f9ce",
        "native_session": "01a21ee8-f77e-7cf3-a6b6-d3222bb25bea",
        "execution": "567045b1-5866-5e4c-8336-f63aff775260",
        "generation": 1,
        "process": {
          "pid": 1422461,
          "start": 144263152
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ee9-05a1-75e0-acee-ed342eb4c079",
      "ordinal": 2
    },
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "36fe10e1f05b6d7f388e0051bc808906",
        "native_session": "01a21ee9-06da-7d31-afde-04e008a47978",
        "execution": "346cf99f-604f-5ba9-8142-efe10cfc69ee",
        "generation": 1,
        "process": {
          "pid": 1424447,
          "start": 144263554
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ee9-1568-79a3-8a58-245d8fe24b68",
      "ordinal": 2
    },
    "co-t3": {
      "adapter": "codex",
      "scope": {
        "binding": "c83490c322fb3c2d09081dff87aa3ae8",
        "native_session": "01a21ee9-1674-7a33-b555-34ea7c13b69f",
        "execution": "6c74c8f0-e3bf-5291-907e-fcef57378ec9",
        "generation": 1,
        "process": {
          "pid": 1425375,
          "start": 144263958
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ee9-2521-7c40-9960-ecf1bc0a1098",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T21:30:37Z",
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
| busy-invite/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "queue_transport_ok": true} | PASS |
| busy-invite/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "queue_transport_ok": true} | PASS |
| busy-invite/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "queue_transport_ok": true} | PASS |

3/3 expected rows passed (3 collected)

codex hook guidance: `{"rollouts": 3, "guidance_contexts": 3, "delivering_hooks": 6, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.15os8j8w"}], "problems": []}`

exit code: 0
