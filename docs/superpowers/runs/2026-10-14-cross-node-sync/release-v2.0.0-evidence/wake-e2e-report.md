# wake e2e 090601-197a

```json
{
  "run_id": "090601-197a",
  "sha": "8ee92612799c2978e090a0c57bdb33e2a5a923cf",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 2.0.0",
    "claude": "2.1.301 (Claude Code)",
    "codex": "codex-cli 0.163.0"
  },
  "harness_mode": "macos",
  "codex_args": "--approve-for-me",
  "bin": "/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-cross-node-sync/bin/herdr-threads",
  "bin_sha256": "3383e7e9703219252f17a452c05ed046e7d1696c4d63fb911e9abb2b421b1fed",
  "root": "/private/tmp/ht-wake-e2e.74sx4ecz",
  "started_utc": "2026-10-16T16:06:01Z",
  "finished_utc": "2026-10-16T16:29:17Z",
  "exit_code": 1,
  "codex_auth_refreshed_in_run": false
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| invite/group | 0 | claude | True | True (0.27) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | 0 | codex | True | True (0.79) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | -1 | claude | True | True (0.28) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | -1 | codex | True | True (0.79) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | 3 | claude | True | True (0.27) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | 3 | codex | True | True (0.53) | {"joined": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 0 | claude | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 0 | codex | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | -1 | claude | True | True (0.27) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | -1 | codex | True | True (0.53) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 3 | claude | True | True (0.26) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 3 | codex | True | True (0.54) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | -1 | claude | True | True (0.28) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | -1 | codex | True | True (0.8) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 3 | claude | True | True (0.55) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 3 | codex | True | True (0.55) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 0 | claude | True | True (0.82) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 0 | codex | True | True (0.75) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | -1 | claude | True | True (0.28) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | -1 | codex | True | True (0.62) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 3 | claude | True | True (0.27) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 3 | codex | True | True (0.78) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | -1 | claude | True | True (0.42) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | -1 | codex | True | True (0.78) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 3 | claude | True | True (0.41) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 3 | codex | True | True (0.76) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-ack/group | 0 | claude | True | True (0.82) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | 0 | codex | True | True (1.26) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | -1 | claude | True | True (0.88) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | -1 | codex | True | True (1.13) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | 3 | claude | True | True (0.93) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | 3 | codex | True | True (1.25) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | -1 | claude | True | True (0.76) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | -1 | codex | True | True (0.91) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 3 | claude | True | True (0.41) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 3 | codex | True | True (0.76) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 0 | claude | True | True (0.4) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 0 | codex | True | True (1.52) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | -1 | claude | True | True (0.42) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | -1 | codex | True | True (0.78) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 3 | claude | True | True (0.41) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 3 | codex | True | True (0.78) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | -1 | claude | True | True (0.41) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | -1 | codex | True | True (0.84) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 3 | claude | True | True (0.47) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 3 | codex | True | True (0.73) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 0 | claude | True | True (0.27) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 0 | codex | True | True (1.07) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | -1 | claude | True | True (0.49) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | -1 | codex | True | True (0.54) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 3 | claude | True | True (0.27) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 3 | codex | True | True (0.82) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 0 | claude | True | True (0.01) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m9rVx81DNulo", "m9rVKVUh5xy8", "m9rVLuNn20WK", "m9rV9ukUB8xL"], "order_pages": [["m9rVx81DNulo", "m9rVKVUh5xy8", "m9rVLuNn20WK"], ["m9rV9ukUB8xL"]], "order_expected": ["m9rV9ukUB8xL", "m9rVLuNn20WK", "m9rVKVUh5xy8", "m9rVx81DNulo"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 0 | codex | True | True (0.8) | {"delivered": true, "order": true, "order_judged": true, "order_shown": ["m9rVGbUUiPrN", "m9rVWn7Rftlm", "m9rV3cUJFK6u", "m9rVshms2PeK"], "order_pages": [["m9rVGbUUiPrN", "m9rVWn7Rftlm", "m9rV3cUJFK6u", "m9rVshms2PeK"]], "order_expected": ["m9rVGbUUiPrN", "m9rVWn7Rftlm", "m9rV3cUJFK6u", "m9rVshms2PeK"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | -1 | claude | True | True (0.01) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m9rV9Y78nkyD", "m9rVYc2W5UqY", "m9rV3Dob1lXP", "m9rVzDu2HakO"], "order_pages": [["m9rV9Y78nkyD"], ["m9rVYc2W5UqY", "m9rV3Dob1lXP", "m9rVzDu2HakO"]], "order_expected": ["m9rVYc2W5UqY", "m9rV3Dob1lXP", "m9rVzDu2HakO", "m9rV9Y78nkyD"], "markers": false, "markers_reason": "mixed-order/group: m9rV9Y78nkyD shown p1 expected p5; m9rVYc2W5UqY shown p1 expected p0; m9rV3Dob1lXP shown p1 expected p0", "ack": "acknowledged", "trailing_wake_s": null} | FAIL |
| mixed-order/group | -1 | codex | True | True (0.81) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m9rVV8WxkVEB", "m9rVJjnj4SHK", "m9rVfLIec6U8", "m9rV8SK5SicU"], "order_pages": [["m9rVV8WxkVEB", "m9rVJjnj4SHK", "m9rVfLIec6U8", "m9rV8SK5SicU"]], "order_expected": ["m9rVV8WxkVEB", "m9rVJjnj4SHK", "m9rVfLIec6U8", "m9rV8SK5SicU"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 3 | claude | True | True (0.28) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m9rVNjQ7pTxD", "m9rVaV1fLCFk", "m9rV1D4HFOPK", "m9rVWlfq5gWH"], "order_pages": [["m9rVNjQ7pTxD", "m9rVaV1fLCFk"], ["m9rV1D4HFOPK", "m9rVWlfq5gWH"]], "order_expected": ["m9rV1D4HFOPK", "m9rVWlfq5gWH", "m9rVaV1fLCFk", "m9rVNjQ7pTxD"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 3 | codex | True | True (0.81) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m9rVxNrghg06", "m9rVHr5JltHt", "m9rVHmXVIqut", "m9rVKREXBpO9"], "order_pages": [["m9rVxNrghg06", "m9rVHr5JltHt", "m9rVHmXVIqut", "m9rVKREXBpO9"]], "order_expected": ["m9rVxNrghg06", "m9rVHr5JltHt", "m9rVHmXVIqut", "m9rVKREXBpO9"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| busy-invite/group | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true} | PASS |
| busy-invite/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true} | PASS |
| busy-invite/group | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true} | PASS |
| busy-invite/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true} | PASS |
| busy-invite/group | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true} | PASS |
| busy-invite/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true} | PASS |
| busy-invite/1:1 | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true} | PASS |
| busy-invite/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true} | PASS |
| busy-invite/1:1 | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true} | PASS |
| busy-invite/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true} | PASS |
| busy-invite/1:1 | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true} | PASS |
| busy-invite/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "joined": true, "opening_message_delivered_in_turn": true} | PASS |
| busy-send-p1/group | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/group | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/group | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/group | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/group | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/group | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-ack/group | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/group | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/group | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/group | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/group | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/group | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-p1/1:1 | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/1:1 | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/1:1 | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p1/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/1:1 | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/1:1 | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/1:1 | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-p0/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true} | PASS |
| busy-send-ack/1:1 | 0 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/1:1 | 0 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/1:1 | -1 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/1:1 | -1 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/1:1 | 3 | claude | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |
| busy-send-ack/1:1 | 3 | codex | False | False | {"busy_at_send": "working", "self_started_turn_after_s": null, "busy_turn_end_seen": true, "wake_prompts_in_busy_turn": 0, "wake_prompts_after_busy_turn": 0, "delivered_in_turn": true, "ack": "acknowledged"} | PASS |

113/114 expected rows passed (114 collected)

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}], "problems": []}`

exit code: 1
