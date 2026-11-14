# wake e2e 133205-6d3f

```json
{
  "run_id": "133205-6d3f",
  "sha": "67a5ad27dd64cdfc73ab6eb9a9973363522374fe",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.1.2",
    "opencode": "1.18.1"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "/var/home/alepar/AleCode/herdr-threads/.worktrees/opencode-harness/bin/herdr-threads",
  "bin_sha256": "c2d72e9b1fe08fd4057b9af430140540f6388adbfb00ab661037e2468940f8d7",
  "root": "/tmp/ht-wake-e2e.acwxpkqr",
  "started_utc": "2026-11-14T21:32:05Z",
  "finished_utc": "2026-11-14T21:52:30Z",
  "exit_code": 1
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| invite/group | 0 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| invite/group | -1 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| invite/group | 3 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | 0 | opencode | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | -1 | opencode | True | True (0.25) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | 3 | opencode | True | True (0.25) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 0 | opencode | True | True (0.52) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | 0 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | 3 | opencode | True | True (0.52) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 0 | opencode | True | True (0.36) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | 0 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mqLOJt3uDtKU", "mqLOlRykuiqK", "mqLOfzvGYNKt", "mqLOrNEJ1IzE"], "order_pages": [["mqLOJt3uDtKU", "mqLOlRykuiqK", "mqLOfzvGYNKt", "mqLOrNEJ1IzE"]], "order_expected": ["mqLOfzvGYNKt", "mqLOrNEJ1IzE", "mqLOlRykuiqK", "mqLOJt3uDtKU"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | -1 | opencode | True | True (0.26) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mqLOQSbs9F8f", "mqLOqadN7Idl", "mqLOprUGG9dy", "mqLOFxrAuP5y"], "order_pages": [["mqLOQSbs9F8f", "mqLOqadN7Idl", "mqLOprUGG9dy", "mqLOFxrAuP5y"]], "order_expected": ["mqLOFxrAuP5y", "mqLOqadN7Idl", "mqLOprUGG9dy", "mqLOQSbs9F8f"], "markers": false, "markers_reason": "mixed-order/group: no expected [herdr-threads] header on screen", "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | FAIL |
| mixed-order/group | 3 | opencode | True | True (0.26) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mqLOp3vCXhN9", "mqLOwddYsBLf", "mqLOP4GaX9QD", "mqLO3p8qwzpQ"], "order_pages": [["mqLOp3vCXhN9", "mqLOwddYsBLf", "mqLOP4GaX9QD", "mqLO3p8qwzpQ"]], "order_expected": ["mqLOP4GaX9QD", "mqLO3p8qwzpQ", "mqLOwddYsBLf", "mqLOp3vCXhN9"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-invite/group | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-invite/group | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-invite/group | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-invite/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-invite/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-invite/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/group | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/group | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/group | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/group | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/group | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/group | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/group | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/group | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/group | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": false, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | FAIL |
| busy-send-p0/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": false, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | FAIL |
| busy-send-ack/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": false, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | FAIL |

53/57 expected rows passed (57 collected)

codex hook guidance: `null`

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e.acwxpkqr"}], "problems": []}`

exit code: 1
