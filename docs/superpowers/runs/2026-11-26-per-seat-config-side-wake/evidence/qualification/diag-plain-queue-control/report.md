# wake e2e 112703-ac32

```json
{
  "run_id": "112703-ac32",
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
  "root": "/tmp/ht-wake-e2e.geilnho5",
  "started_utc": "2026-11-27T19:27:03Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"s0BhH87wOgZD\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t-1": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"s0Bhfril26Go\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t3": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"s0BhfSCWXJ1t\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "303f383881fea0507945dd1094c84fe2",
        "native_session": "01a21e79-c457-7283-aee6-155799f8d867",
        "execution": "26b2d54f-91d3-5aed-9a70-6ea541fe359d",
        "generation": 1,
        "process": {
          "pid": 157619,
          "start": 143534368
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e79-d184-7ab2-8b02-f7e14262aa24",
      "ordinal": 2
    },
    "co-t-1": {
      "adapter": "codex",
      "scope": {
        "binding": "bd1c2e7d2e60ff94f0e35edc04a0ae6a",
        "native_session": "01a21e79-d3fe-71c2-8c80-b85b15d60368",
        "execution": "d82246a8-2697-59c1-ab64-d2ec1dd5c822",
        "generation": 1,
        "process": {
          "pid": 158701,
          "start": 143534770
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e79-e139-7c52-ae9c-7dcde259ae06",
      "ordinal": 2
    },
    "co-t3": {
      "adapter": "codex",
      "scope": {
        "binding": "09d5473b8fcb759980ad28e95e3fb9b0",
        "native_session": "01a21e79-e3d0-7540-b6fe-59f1c0e5c189",
        "execution": "2673bee9-30a4-5c3a-853a-59cd9e6be1b7",
        "generation": 1,
        "process": {
          "pid": 159602,
          "start": 143535172
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e79-f0ee-7a12-abbf-48848b8a5483",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:29:02Z",
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
| send-p0/group | 0 | codex | True | True (7.33) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/group | -1 | codex | True | True (8.43) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/group | 3 | codex | True | True (8.08) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |

3/3 expected rows passed (3 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 3, "guidance_contexts": 3, "delivering_hooks": 6, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.geilnho5"}], "problems": []}`

exit code: 0
