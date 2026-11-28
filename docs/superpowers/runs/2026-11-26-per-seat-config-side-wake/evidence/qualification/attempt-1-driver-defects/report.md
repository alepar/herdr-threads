# wake e2e 110233-c0b8

```json
{
  "run_id": "110233-c0b8",
  "sha": "52ca846950a94938022936b24ba3d28c84b88c06",
  "dirty": true,
  "versions": {
    "herdr": "herdr 0.9.5",
    "herdr-threads": "herdr-threads 3.6.0",
    "codex": "codex-cli 0.172.0"
  },
  "harness_mode": "linux",
  "codex_args": "--approve-for-me",
  "bin": "$HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/.worktrees/super-auto-per-seat-config-side-wake--task-ht-z41e.43/bin/herdr-threads",
  "bin_sha256": "2a096d766a6fe65b06a8e33f9c55b339d938283604360c751865742eb88d3ae1",
  "root": "/tmp/ht-wake-e2e.o7mfmkit",
  "started_utc": "2026-11-27T19:02:33Z",
  "queue_proof": true,
  "queue_observation": true,
  "queue_seats": {
    "co-t0": "{\"version\":9,\"result\":{\"kind\":\"seat_config_written\",\"data\":{\"seat\":\"sMgMqsqzbDtx\",\"key\":\"programmatic_wake\",\"value\":\"on\",\"is_default\":false,\"changed\":true}}}"
  },
  "queue_bootstrap": {
    "co-t0": {
      "adapter": "codex",
      "scope": {
        "binding": "fca44795c02a266c8dd767c6a80b40b1",
        "native_session": "01a21e63-55d9-7bb2-93c7-6ed03d1a674d",
        "execution": "31ed46c5-65bc-5f25-8e9e-677006843d17",
        "generation": 1,
        "process": {
          "pid": 3886760,
          "start": 143387363
        },
        "kernel_boot": "6d757418-e8a4-456d-a680-21953e8fa0a3"
      },
      "turn_id": "01a21e63-6320-7532-a34e-4a53ef9fe94e",
      "ordinal": 2
    }
  },
  "finished_utc": "2026-11-27T19:09:58Z",
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
      "events": [],
      "null_events": [],
      "defect": false
    },
    "co-t3": {
      "events": [],
      "null_events": [],
      "defect": false
    }
  },
  "release_blocking_defects": "release-blocking defects: co-t0 root-draft-queue/group: root_draft_lost"
}
```

| case | T | harness | expect wake | woke (s) | checks | result |
|---|---|---|---|---|---|---|
| side-queue-focused/group | 0 | codex | True | False | {"bindings_after_side": [{"session": "01a21e63-55d9-7bb2-93c7-6ed03d1a674d", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3886760, "disconnect": null, "concurrent": 0, "auxiliary": 0}, {"session": "01a21e63-c327-7392-8728-7eba675fcc4b", "provenance": "cooperative_top_level", "liveness": "connected", "pid": 3886760, "disconnect": null, "concurrent": 1, "auxiliary": 0}], "root_session": "01a21e63-55d9-7bb2-93c7-6ed03d1a674d", "setup": {"side_setup_ok": true, "focused": true, "side_busy": true, "agent_status": "working", "focused_at_send": true}, "side_typed": false, "armed_s": null, "first_attempt": false, "root_busy_at_arm": false, "activity_evidence": null, "delivery_evidence": null, "delivered_in_root": false, "side_view_before": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "side_view_after": {"prompts": ["\u203a You are a test recipient in an automated herdr-threads wake e2e run (the us\u2026 \u2502 Run side \u2026\u00d7                        +", "\u203a herdr-threads: wake up                                                       \u2502    herdr-threads inbox rows. Peer data", "\u203a                                                                              \u2502\u203a Ask a follow-up question"], "answers": [], "wake_markers": 1}, "agent_status_at_send": "working", "side_phase_at_send": ["ended"], "side_bits": {"01a21e63-c327-7392-8728-7eba675fcc4b": {"auxiliary": 0, "concurrent": 1}}, "root_bits": {"auxiliary": 0, "concurrent": 0}, "settled_by_root": false, "wake_state_final": {"last_outcome": "unsafe", "completed_at_utc": 1795806284888, "retry_step": 0}, "release_blocking": [], "queue_transport_ok": false} | FAIL |
| root-draft-queue/group | 0 | codex | True | False | {"root_session": "01a21e63-55d9-7bb2-93c7-6ed03d1a674d", "draft_typed": false, "intents_while_draft": [], "no_wake_while_draft": true, "wake_state_while_draft": {"last_outcome": "unsafe", "completed_at_utc": 1795806490740, "retry_step": 0}, "draft_intact": false, "draft_submitted": false, "draft_cleared": true, "queue_after_clear": false, "root_busy_at_arm": false, "delivered": false, "delivery_evidence": null, "activity_evidence": null, "release_blocking": ["root_draft_lost"], "queue_transport_ok": false} | FAIL |

0/2 expected rows passed (2 collected)
queue logical fulfillment + bounded cancellation observed: False

codex hook guidance: `{"rollouts": 2, "guidance_contexts": 2, "delivering_hooks": 1, "over_allowance": 0, "pass": true}`

root transcript co-t0: events SessionStart/startup, UserPromptSubmit, PreToolUse, Stop; root null transcript_path: none

root transcript co-t-1: events none; root null transcript_path: none

root transcript co-t3: events none; root null transcript_path: none

release-blocking defects: co-t0 root-draft-queue/group: root_draft_lost

cleanup: `{"steps": [{"workspace_close_rc": 0}, {"daemon_stop_rc": 0}, {"codex_auth_copy": "deleted (unchanged)"}, {"root_removed": "/tmp/ht-wake-e2e.o7mfmkit"}], "problems": []}`

exit code: 1
