# wake e2e 071837-a96c

```json
{
  "run_id": "071837-a96c",
  "sha": "b93086b1bf0dfe4cce6ea4bd2060e4d0467c3e41",
  "dirty": false,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.4.0",
    "codex": "codex-cli 0.169.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "<worktree>/bin/herdr-threads",
  "bin_sha256": "01e1d19717151bb32d68fb256b3efc964adf1f5d9688588c50b7f55305360831",
  "root": "/tmp/ht-wake-e2e.1fvhgd8t",
  "started_utc": "2026-11-17T15:18:37Z",
  "queue_proof": false,
  "queue_observation": true,
  "finished_utc": "2026-11-17T15:21:38Z",
  "exit_code": 0,
  "codex_auth_refreshed_in_run": false
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| resume-idle/group | 0 | codex | True | True (0.89) | {"unavailable_after_exit": true, "disconnect_reason": "liveness_process_exit", "unavailable_while_idle": true, "wakes_on_p1": 0, "delivered": true, "available_after": true, "queue_transport_ok": true} | PASS |
| resume-quit/group | 0 | codex | True | True (17.39) | {"quit_exited": true, "unavailable_after_exit": true, "disconnect_reason": "liveness_end_event", "unavailable_while_idle": true, "wakes_on_p1": 0, "delivered": true, "available_after": true, "queue_transport_ok": true} | PASS |

2/2 expected rows passed (2 collected)

codex hook guidance: `{"rollouts": 1, "guidance_contexts": 3, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.1fvhgd8t"}], "problems": []}`

exit code: 0
