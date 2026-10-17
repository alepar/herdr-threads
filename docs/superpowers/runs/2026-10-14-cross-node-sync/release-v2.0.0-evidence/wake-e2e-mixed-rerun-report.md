# wake e2e 093223-6bdc

```json
{
  "run_id": "093223-6bdc",
  "sha": "78d6ac8e32b8aa167a50e71d93c1b215866ea623",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 2.0.0",
    "claude": "2.1.301 (Claude Code)"
  },
  "harness_mode": "macos",
  "codex_args": "--approve-for-me",
  "bin": "/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-cross-node-sync/bin/herdr-threads",
  "bin_sha256": "3383e7e9703219252f17a452c05ed046e7d1696c4d63fb911e9abb2b421b1fed",
  "root": "/private/tmp/ht-wake-e2e.k6_hgpj8",
  "started_utc": "2026-10-16T16:32:23Z",
  "finished_utc": "2026-10-16T16:34:26Z",
  "exit_code": 0
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| mixed-order/group | 0 | claude | True | True (0.28) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mmTqmjuMxXZo", "mmTqzd7EDixB", "mmTqajgvwhGH", "mmTqUt3qj3Ak"], "order_pages": [["mmTqmjuMxXZo", "mmTqzd7EDixB", "mmTqajgvwhGH"], ["mmTqUt3qj3Ak"]], "order_expected": ["mmTqUt3qj3Ak", "mmTqajgvwhGH", "mmTqzd7EDixB", "mmTqmjuMxXZo"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | -1 | claude | True | True (0.29) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mmTqaFvZA7Ld", "mmTqcNy8auud", "mmTqO0g5IwSu", "mmTq8GcPAcrp"], "order_pages": [["mmTqaFvZA7Ld"], ["mmTqcNy8auud", "mmTqO0g5IwSu", "mmTq8GcPAcrp"]], "order_expected": ["mmTqcNy8auud", "mmTqO0g5IwSu", "mmTq8GcPAcrp", "mmTqaFvZA7Ld"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |
| mixed-order/group | 3 | claude | True | True (0.29) | {"delivered": true, "order": "unjudged", "order_judged": false, "order_shown": ["mmTqRYucmuoG", "mmTqYrHOBxs1", "mmTq1cvu7qbR", "mmTqgZUfayId"], "order_pages": [["mmTqRYucmuoG", "mmTqYrHOBxs1"], ["mmTq1cvu7qbR", "mmTqgZUfayId"]], "order_expected": ["mmTq1cvu7qbR", "mmTqgZUfayId", "mmTqYrHOBxs1", "mmTqRYucmuoG"], "markers": true, "ack": "acknowledged", "trailing_wake_s": null} | PASS |

3/3 expected rows passed (3 collected)

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}], "problems": []}`

exit code: 0
