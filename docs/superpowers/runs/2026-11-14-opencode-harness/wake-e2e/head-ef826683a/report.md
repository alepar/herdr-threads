# wake e2e 144844-fa89

```json
{
  "run_id": "144844-fa89",
  "sha": "ef826683a230391105ef72e59a8a91e98e84fa99",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.1.2",
    "opencode": "1.18.1"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "/var/home/alepar/AleCode/herdr-threads/.worktrees/opencode-harness/bin/herdr-threads",
  "bin_sha256": "49e10bb8a30eb5e9c74118b284b3b62eeb48687858ff2832ee7348785c5a9f7c",
  "root": "/tmp/ht-wake-e2e.nbikgmnl",
  "started_utc": "2026-11-14T22:48:44Z",
  "finished_utc": "2026-11-14T23:08:58Z",
  "exit_code": 1
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| invite/group | 0 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| invite/group | -1 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| invite/group | 3 | opencode | True | True (0.25) | {"joined": true, "trailing_wake_s": null, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | 0 | opencode | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | -1 | opencode | True | True (0.51) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| pair-open/1:1 | 3 | opencode | True | True (0.51) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/group | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/group | 3 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | -1 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/group | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p1/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | 0 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | -1 | opencode | True | True (0.51) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p0/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | 0 | opencode | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | -1 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-p3/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | 0 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| send-ack/1:1 | -1 | opencode | True | True (0.25) | {"delivered": false, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | FAIL |
| send-ack/1:1 | 3 | opencode | True | True (0.25) | {"delivered": true, "inbox_calls": 0, "no_extra_inbox": true, "ack": "acknowledged", "admission_row": true, "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | 0 | opencode | True | True (0.51) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mFrg2WvRLfvw", "mFrgecOTKIEb", "mFrgT7FuFmv4", "mFrg5Sm4wjRa"], "order_pages": [["mFrg2WvRLfvw", "mFrgecOTKIEb", "mFrgT7FuFmv4", "mFrg5Sm4wjRa"]], "order_expected": ["mFrgT7FuFmv4", "mFrg5Sm4wjRa", "mFrgecOTKIEb", "mFrg2WvRLfvw"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | -1 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mFrgDITPdayC", "mFrgRr2D7ypA", "mFrgCj3GH7zB", "mFrgLh4zrC71"], "order_pages": [["mFrgDITPdayC", "mFrgRr2D7ypA", "mFrgCj3GH7zB", "mFrgLh4zrC71"]], "order_expected": ["mFrgCj3GH7zB", "mFrgLh4zrC71", "mFrgRr2D7ypA", "mFrgDITPdayC"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | 3 | opencode | True | True (0.51) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mFrgKPc3PX0c", "mFrgLEjHUeVR", "mFrgtAJvLRSw", "mFrgHz7UI5in"], "order_pages": [["mFrgKPc3PX0c", "mFrgLEjHUeVR", "mFrgtAJvLRSw", "mFrgHz7UI5in"]], "order_expected": ["mFrgtAJvLRSw", "mFrgHz7UI5in", "mFrgLEjHUeVR", "mFrgKPc3PX0c"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
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

56/57 expected rows passed (57 collected)

codex hook guidance: `null`

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e.nbikgmnl"}], "problems": []}`

exit code: 1
