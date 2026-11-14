# wake e2e 042019-15b5

```json
{
  "run_id": "042019-15b5",
  "sha": "8eb398aabfb0d21f2c9969046b2bffd0dbb536fc",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.1.1",
    "opencode": "1.18.1"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "/var/home/alepar/AleCode/herdr-threads/.worktrees/opencode-e2e/bin/herdr-threads",
  "bin_sha256": "103c96a1f4168a51198c82ebfdc39c058838e29f82387d4715c75b10d9f0d655",
  "root": "/tmp/ht-wake-e2e.d75fndfa",
  "started_utc": "2026-11-14T12:20:19Z",
  "finished_utc": "2026-11-14T12:40:00Z",
  "exit_code": 0
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| invite/group | 0 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| invite/group | -1 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| invite/group | 3 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | 0 | opencode | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | -1 | opencode | True | True (0.25) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | 3 | opencode | True | True (0.51) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | 0 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | -1 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | -1 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | -1 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | -1 | opencode | True | True (0.35) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | -1 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | 0 | opencode | True | True (0.26) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m6Vmj3PvH4hb", "m6Vm53Y79J7r", "m6VmEiF8eLpk", "m6Vm8H2lCyYv"], "order_pages": [["m6Vmj3PvH4hb", "m6Vm53Y79J7r", "m6VmEiF8eLpk"], ["m6Vm8H2lCyYv"]], "order_expected": ["m6VmEiF8eLpk", "m6Vm8H2lCyYv", "m6Vm53Y79J7r", "m6Vmj3PvH4hb"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | -1 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m6Vmr3d0Nw0U", "m6Vm0ovZcuYu", "m6VmMpcaLnkE", "m6VmbmGZ4rHF"], "order_pages": [["m6Vmr3d0Nw0U", "m6Vm0ovZcuYu", "m6VmMpcaLnkE", "m6VmbmGZ4rHF"]], "order_expected": ["m6VmMpcaLnkE", "m6VmbmGZ4rHF", "m6Vm0ovZcuYu", "m6Vmr3d0Nw0U"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | 3 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m6VmzA82V4xT", "m6VmehjG2Xsv", "m6VmybO023CZ", "m6VmZKZwQTpb"], "order_pages": [["m6VmzA82V4xT", "m6VmehjG2Xsv", "m6VmybO023CZ", "m6VmZKZwQTpb"]], "order_expected": ["m6VmybO023CZ", "m6VmZKZwQTpb", "m6VmehjG2Xsv", "m6VmzA82V4xT"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
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
| busy-send-p1/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |

57/57 expected rows passed (57 collected)

codex hook guidance: `null`

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e.d75fndfa"}], "problems": []}`

exit code: 0
