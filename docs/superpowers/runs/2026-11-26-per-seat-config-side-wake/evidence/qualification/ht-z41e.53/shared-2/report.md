# wake e2e 154227-a60b

```json
{
  "run_id": "154227-a60b",
  "sha": "76fa45e016a41d30407364f41ff12ad9abe1de89",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.53/bin/herdr-threads",
  "bin_sha256": "3a039bd2b6fe76500eea100ae4e431ad2544aaeed3e5ad49016a409821bccae1",
  "root": "/tmp/ht-wake-e2e.5ebgynr4",
  "started_utc": "2026-11-27T23:42:27Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"s8MIaZBm4RIq\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "089f61d7fac70e6659ec8a2fb6b16e7c",
        "native_session": "01a21f63-9616-75f0-b432-19db17941deb",
        "execution": "aac0ae43-8d3f-5be6-96aa-aba8cb35e29b",
        "generation": 1,
        "process": {
          "pid": 158040,
          "start": 145066759
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21f63-a490-7f50-adea-203a85284fd9",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T23:46:45Z",
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
      "events": [],
      "null_events": [],
      "defect": false
    },
    "co-t3": {
      "events": [],
      "null_events": [],
      "defect": false
    }
  }
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| side-queue/group | 0 | codex | True | True (8.01) | {"bindings_after_side": [{"session": "01a21f63-9616-75f0-b432-19db17941deb", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 158040, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21f63-fe62-7fd2-a6ec-9cc30f44c0e4", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 158040, "disconnect": null, "concurrent": 1, "auxiliary": 1}], "root_session": "01a21f63-9616-75f0-b432-19db17941deb", "side_setup_ok": true, "side_typed": false, "side_view_changed": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the user approves). Standing", "\u203a Side of wake-e2e side-queue/group 154227-a60b:", "\u203a herdr-threads: wake up", "\u203a", "\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the user approves). Standing", "\u203a Side of wake-e2e side-queue/group 154227-a60b:", "\u203a herdr-threads: wake up", "\u203a", "\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "activity_evidence": {"attempt": "2ca65be5-5189-4204-94a5-b5addcdf6995", "native_session": "01a21f63-9616-75f0-b432-19db17941deb", "turn_id": "01a21f66-a164-7ff0-a9b2-64bd02594182", "observed_at_ms": 7772}, "delivery_evidence": {"passed": true, "complete": true, "input_ids": ["m8MIT3JgvqYD"], "output_ids": ["m8MIT3JgvqYD"], "started_utc": "2026-11-27T23:45:49.414Z", "completed_utc": "2026-11-27T23:45:51.637Z", "bytes_read": 358061, "files_examined": 2, "session": "01a21f63-9616-75f0-b432-19db17941deb", "turn_id": "01a21f66-a164-7ff0-a9b2-64bd02594182"}, "delivered_in_root": true, "side_bits": {"01a21f63-fe62-7fd2-a6ec-9cc30f44c0e4": {"auxiliary": 1, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "side_first_event": {"01a21f63-fe62-7fd2-a6ec-9cc30f44c0e4": {"event": "SessionStart", "source": "fork", "transcript_path": "present"}}, "settled_by_root": true, "wake_state_final": {"last_outcome": "submitted", "completed_at_utc": 1795823142146, "retry_step": 1}, "queue_transport_ok": true} | PASS |

1/1 expected rows passed (1 collected)
queue logical fulfillment + bounded cancellation observed: True

codex hook guidance: `{"rollouts": 2, "guidance_contexts": 2, "delivering_hooks": 2, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.5ebgynr4"}], "problems": []}`

exit code: 0
