# wake e2e 195218-ab4e

```json
{
  "run_id": "195218-ab4e",
  "sha": "744ca90a5e894ed551fee00fec69de6be68afa4f",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.1",
    "herdr-threads": "herdr-threads 1.0.0",
    "claude": "2.1.300 (Claude Code)",
    "codex": "codex-cli 0.162.1"
  },
  "harness_mode": "macos",
  "codex_args": "--approve-for-me",
  "bin": "/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/bin/herdr-threads",
  "bin_sha256": "332d9749c189b22a4a800fb688a59f8556b62e0b38833d49d46a9fdbc7065d6c",
  "root": "/private/tmp/ht-wake-e2e.b2wrcfh_",
  "started_utc": "2026-10-15T02:52:19Z",
  "finished_utc": "2026-10-15T03:15:35Z",
  "exit_code": 0,
  "codex_auth_refreshed_in_run": false
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| invite/group | 0 | claude | True | True (0.33) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | 0 | codex | True | True (0.56) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | -1 | claude | True | True (0.57) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | -1 | codex | True | True (0.82) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | 3 | claude | True | True (0.33) | {"joined": true, "trailing_wake_s": null} | PASS |
| invite/group | 3 | codex | True | True (0.63) | {"joined": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 0 | claude | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 0 | codex | False | False | {"joined": true, "late_wake_s": null, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | -1 | claude | True | True (0.58) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | -1 | codex | True | True (0.83) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 3 | claude | True | True (0.29) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| pair-open/1:1 | 3 | codex | True | True (0.82) | {"joined": true, "opening_message_delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | -1 | claude | True | True (0.4) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | -1 | codex | True | True (0.84) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 3 | claude | True | True (0.28) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/group | 3 | codex | True | True (0.83) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 0 | claude | True | True (0.29) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 0 | codex | True | True (0.55) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | -1 | claude | True | True (0.29) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | -1 | codex | True | True (0.84) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 3 | claude | True | True (0.29) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/group | 3 | codex | True | True (0.85) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | -1 | claude | True | True (0.28) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | -1 | codex | True | True (0.85) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 3 | claude | True | True (0.56) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/group | 3 | codex | True | True (0.83) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-ack/group | 0 | claude | True | True (0.29) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | 0 | codex | True | True (0.56) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | -1 | claude | True | True (0.3) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | -1 | codex | True | True (0.84) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | 3 | claude | True | True (0.32) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/group | 3 | codex | True | True (0.59) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | -1 | claude | True | True (0.29) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | -1 | codex | True | True (0.87) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 3 | claude | True | True (0.56) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p1/1:1 | 3 | codex | True | True (0.83) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 0 | claude | True | True (0.56) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 0 | codex | True | True (0.57) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | -1 | claude | True | True (0.29) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | -1 | codex | True | True (0.57) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 3 | claude | True | True (0.29) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p0/1:1 | 3 | codex | True | True (0.83) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 0 | claude | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 0 | codex | False | False | {"late_wake_s": null, "delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | -1 | claude | True | True (0.28) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | -1 | codex | True | True (0.57) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 3 | claude | True | True (0.28) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-p3/1:1 | 3 | codex | True | True (0.56) | {"delivered": true, "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 0 | claude | True | True (0.31) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 0 | codex | True | True (0.83) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | -1 | claude | True | True (0.29) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | -1 | codex | True | True (0.84) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 3 | claude | True | True (0.29) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| send-ack/1:1 | 3 | codex | True | True (0.83) | {"delivered": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 0 | claude | True | True (0.57) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["msjiBwkWG", "mhDmP1fMz", "mwZmctKVX", "mewlPOO3L"], "order_pages": [["msjiBwkWG", "mhDmP1fMz", "mwZmctKVX", "mewlPOO3L"]], "order_expected": ["mwZmctKVX", "mewlPOO3L", "mhDmP1fMz", "msjiBwkWG"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 0 | codex | True | True (0.57) | {"delivered": true, "order": true, "order_judged": true, "order_shown": ["mnAgRuWGS", "maTZbKCdz", "mMslzvEsZ", "m2ccI4XOs"], "order_pages": [["mnAgRuWGS", "maTZbKCdz", "mMslzvEsZ", "m2ccI4XOs"]], "order_expected": ["mnAgRuWGS", "maTZbKCdz", "mMslzvEsZ", "m2ccI4XOs"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | -1 | claude | True | True (0.29) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mEnyk9A7x", "m2TaPTcuh", "m2iP9vUpB", "mxkunwu02"], "order_pages": [["mEnyk9A7x"], ["m2TaPTcuh", "m2iP9vUpB", "mxkunwu02"]], "order_expected": ["m2TaPTcuh", "m2iP9vUpB", "mxkunwu02", "mEnyk9A7x"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | -1 | codex | True | True (0.57) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m8M4uezNK", "mLuEPMOmX", "mQt0pRStW", "m0jIWXwnp"], "order_pages": [["m8M4uezNK", "mLuEPMOmX", "mQt0pRStW", "m0jIWXwnp"]], "order_expected": ["m8M4uezNK", "mLuEPMOmX", "mQt0pRStW", "m0jIWXwnp"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 3 | claude | True | True (0.01) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m681mmd8v", "mhPP1krJw", "mIQ0IkVyg", "m6GFxoMIe"], "order_pages": [["m681mmd8v", "mhPP1krJw"], ["mIQ0IkVyg", "m6GFxoMIe"]], "order_expected": ["mIQ0IkVyg", "m6GFxoMIe", "mhPP1krJw", "m681mmd8v"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 3 | codex | True | True (0.84) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["m8ltfn2TC", "mPg37gbSq", "m8cTvhT3Z", "mh4pSHuCL"], "order_pages": [["m8ltfn2TC", "mPg37gbSq", "m8cTvhT3Z", "mh4pSHuCL"]], "order_expected": ["m8ltfn2TC", "mPg37gbSq", "m8cTvhT3Z", "mh4pSHuCL"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
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

114/114 expected rows passed (114 collected)

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}], "problems": []}`

exit code: 0
