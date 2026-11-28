# wake e2e 130537-96d0

```json
{
  "run_id": "130537-96d0",
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
  "root": "/tmp/ht-wake-e2e.q9a909kf",
  "started_utc": "2026-11-27T21:05:37Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"svSWbDiF121F\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t-1": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"svSW8Fbwb1Tz\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}",
    "co-t3": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"svSWulMBe3fV\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "a01e3cef4c15856034989e635f329b44",
        "native_session": "01a21ed3-fd6b-75f0-8d05-c93324320dfd",
        "execution": "74c71a03-fdfb-53ab-a0d3-285b235fc848",
        "generation": 1,
        "process": {
          "pid": 921915,
          "start": 144125693
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ed4-0c2e-7b23-bac6-82048ba3a109",
      "ordinal": 2
    },
    "co-t-1": {
      "adapter": "codex",
      "scope": {
        "binding": "0978f45da52e25640674f4b31fb82bad",
        "native_session": "01a21ed4-0d27-7fa3-a2e3-6b740de64797",
        "execution": "5042b56c-abe3-51af-95fe-d6b36e8c48dc",
        "generation": 1,
        "process": {
          "pid": 922913,
          "start": 144126096
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ed4-1be1-7b72-94ed-9c73a4f6309a",
      "ordinal": 2
    },
    "co-t3": {
      "adapter": "codex",
      "scope": {
        "binding": "5a55773085e2f557dc7976f9f1ad7fc7",
        "native_session": "01a21ed4-1cd7-7751-8a54-94e9a10dbaa9",
        "execution": "e0e0b333-962f-59dc-88f1-f8cb011d54cf",
        "generation": 1,
        "process": {
          "pid": 923614,
          "start": 144126498
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21ed4-2b97-7c50-aea6-84e7843bdb76",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T21:28:18Z",
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
| invite/group | 0 | codex | True | True (6.07) | {"joined": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| invite/group | -1 | codex | True | True (7.94) | {"joined": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| invite/group | 3 | codex | True | True (5.82) | {"joined": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| pair-open/1:1 | 0 | codex | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| pair-open/1:1 | -1 | codex | True | True (8.1) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| pair-open/1:1 | 3 | codex | True | True (7.08) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p1/group | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p1/group | -1 | codex | True | True (10.12) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p1/group | 3 | codex | True | True (10.36) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/group | 0 | codex | True | True (3.03) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/group | -1 | codex | True | True (10.35) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/group | 3 | codex | True | True (10.13) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p3/group | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p3/group | -1 | codex | True | True (9.6) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p3/group | 3 | codex | True | True (0.76) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-ack/group | 0 | codex | True | True (3.03) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-ack/group | -1 | codex | True | True (10.1) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-ack/group | 3 | codex | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p1/1:1 | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p1/1:1 | -1 | codex | True | True (10.1) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p1/1:1 | 3 | codex | True | True (10.11) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/1:1 | 0 | codex | True | True (2.8) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/1:1 | -1 | codex | True | True (10.36) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p0/1:1 | 3 | codex | True | True (10.13) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p3/1:1 | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p3/1:1 | -1 | codex | True | True (9.62) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-p3/1:1 | 3 | codex | True | True (10.12) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-ack/1:1 | 0 | codex | True | True (2.27) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-ack/1:1 | -1 | codex | True | True (9.61) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| send-ack/1:1 | 3 | codex | True | True (10.36) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| mixed-order/group | 0 | codex | True | True (10.12) | {"delivered": true, "order": true, "order_judged": true, "order_shown": ["mvSW4bBh8kNa", "mvSWejvVTjEc", "mvSWVg1OYl37", "mvSWcjI49E47"], "order_pages": [["mvSW4bBh8kNa", "mvSWejvVTjEc", "mvSWVg1OYl37", "mvSWcjI49E47"]], "order_expected": ["mvSW4bBh8kNa", "mvSWejvVTjEc", "mvSWVg1OYl37", "mvSWcjI49E47"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| mixed-order/group | -1 | codex | True | True (0.76) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mvSWda2wWUlU", "mvSWwrWUJ9pB", "mvSWt0z6QC36", "mvSW72unKvoi"], "order_pages": [["mvSWda2wWUlU", "mvSWwrWUJ9pB", "mvSWt0z6QC36", "mvSW72unKvoi"]], "order_expected": ["mvSWda2wWUlU", "mvSWwrWUJ9pB", "mvSWt0z6QC36", "mvSW72unKvoi"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| mixed-order/group | 3 | codex | True | True (9.96) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mvSWXYBqO4vX", "mvSWxjtobCwX", "mvSWOZhkpZlP", "mvSWTqtgmO1d"], "order_pages": [["mvSWXYBqO4vX", "mvSWxjtobCwX", "mvSWOZhkpZlP", "mvSWTqtgmO1d"]], "order_expected": ["mvSWXYBqO4vX", "mvSWxjtobCwX", "mvSWOZhkpZlP", "mvSWTqtgmO1d"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "queue_transport_ok": true} | PASS |
| busy-invite/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": "busy prompt not on screen", "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "queue_transport_ok": true} | FAIL |
| busy-invite/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "queue_transport_ok": true} | PASS |
| busy-invite/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "queue_transport_ok": true} | PASS |
| busy-invite/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-invite/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-invite/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p1/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p1/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p1/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p0/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p0/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p0/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-ack/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "queue_transport_ok": true} | PASS |
| busy-send-ack/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "queue_transport_ok": true} | PASS |
| busy-send-ack/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "queue_transport_ok": true} | PASS |
| busy-send-p1/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p1/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p1/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p0/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p0/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-p0/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "queue_transport_ok": true} | PASS |
| busy-send-ack/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "queue_transport_ok": true} | PASS |
| busy-send-ack/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "queue_transport_ok": true} | PASS |
| busy-send-ack/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "queue_transport_ok": true} | PASS |
| busy-tool-p1/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "tool_ran": true, "queue_transport_ok": true} | PASS |
| busy-tool-p1/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "tool_ran": true, "queue_transport_ok": true} | PASS |
| busy-tool-p1/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "tool_ran": true, "queue_transport_ok": true} | PASS |

59/60 expected rows passed (60 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 3, "guidance_contexts": 3, "delivering_hooks": 60, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t3: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.q9a909kf"}], "problems": []}`

exit code: 1
