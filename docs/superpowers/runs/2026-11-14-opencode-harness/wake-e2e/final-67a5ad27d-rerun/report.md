# wake e2e 135319-6882

```json
{
  "run_id": "135319-6882",
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
  "root": "/tmp/ht-wake-e2e._t42jw7t",
  "started_utc": "2026-11-14T21:53:19Z",
  "finished_utc": "2026-11-14T21:59:50Z",
  "exit_code": 0
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| mixed-order/group | 0 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mq8fFZZ8amh1", "mq8f0bA6WI3f", "mq8fO9QKCG79", "mq8fEd6ZEVpm"], "order_pages": [["mq8fFZZ8amh1", "mq8f0bA6WI3f", "mq8fO9QKCG79", "mq8fEd6ZEVpm"]], "order_expected": ["mq8fO9QKCG79", "mq8fEd6ZEVpm", "mq8f0bA6WI3f", "mq8fFZZ8amh1"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | -1 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mq8fnwZdUNjl", "mq8f4L12AUBT", "mq8fC4ODEKSb", "mq8frfTgvetz"], "order_pages": [["mq8fnwZdUNjl", "mq8f4L12AUBT", "mq8fC4ODEKSb", "mq8frfTgvetz"]], "order_expected": ["mq8frfTgvetz", "mq8f4L12AUBT", "mq8fC4ODEKSb", "mq8fnwZdUNjl"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| mixed-order/group | 3 | opencode | True | True (0.25) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mq8fnumtaSAU", "mq8f8lWryUi3", "mq8fS7JlnWdH", "mq8fMbZLFvLY"], "order_pages": [["mq8fnumtaSAU", "mq8f8lWryUi3", "mq8fS7JlnWdH", "mq8fMbZLFvLY"]], "order_expected": ["mq8fMbZLFvLY", "mq8f8lWryUi3", "mq8fS7JlnWdH", "mq8fnumtaSAU"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null, "appended": true, "plugin_wakes": 1, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p1/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-p0/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | 0 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | -1 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |
| busy-send-ack/1:1 | 3 | opencode | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged", "appended": true, "plugin_wakes": 0, "plugin_wake_ok": true, "composer_empty": true} | PASS |

12/12 expected rows passed (12 collected)

codex hook guidance: `null`

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"root_removed": "/tmp/ht-wake-e2e._t42jw7t"}], "problems": []}`

exit code: 0
