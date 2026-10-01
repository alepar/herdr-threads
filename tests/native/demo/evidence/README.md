Redacted live evidence replayed by `tests/native/demo/test_demo_verify.py` (offline; no model, daemon or Herdr).
Copied verbatim from the run evidence folder `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`,
archived in git tag `archive/herdr-threads-run-2026-09-26` (path unchanged there; `codex-158-live-hook-capture/`
is also kept in-tree under `docs/evidence/`):

- `claude-demo2-initial.stream.jsonl`: `native-claude-demo-2/run1-unhinted/claude-initial-aab6.stream.jsonl`
  (event 13 is `system/permission_denied` with a string `message`; the `result` event carries `permission_denials`).
- `claude-demo2-settings.local.json`: that run's setup output (only the dead export-prefix allow rule).
- `claude-demo2-hook-probe-SessionStart.json`: `native-claude-demo-2/dry-run-baseline/hook-probe-SessionStart.json`
  (the real `Ready commands` block).
- `codex-demo1-initial.events.jsonl`: `native-codex-demo-1/run1-default-sandbox/codex-initial-1356.events.jsonl`
  (`turn.failed` usage limit on aisw profile `default`).
- `codex-158-rollout-model-items.json`: `codex-158-live-hook-capture/payloads/run2-run3.rollout-model-items.json`
  (flat excerpt of rollout model items; developer messages are the delivered hook context).
- `claude-demo3-initial.stream.jsonl`: `native-claude-demo-3/run1-unhinted/claude-initial-a02f.stream.jsonl`
  (the model accepted and ACKed unaided; the `result` event's one permission denial is
  `alias herdr-threads; declare -F herdr-threads; ...; type herdr-threads`, not a herdr-threads command: D7).
- `claude-demo3-sqlite-initial.json`: that run's read-only SQLite extract after the initial phase (accepted
  invitation and acked receipt, both `cooperative_top_level`).
- `claude-demo3-hook-probe-SessionStart.json`: `native-claude-demo-3/dry-run-baseline/hook-probe-SessionStart.json`
  (ready commands that carry `--state-dir` themselves: D8).
- `codex-demo2-run1-initial.events.jsonl`: `native-codex-demo-2/run1-default-sandbox/codex-initial-007d.events.jsonl`
  (default `-s workspace-write`, no socket allowance: the sandbox refused the daemon socket and the pre-P2 CLI printed
  `daemon connection unavailable (host_unavailable)`; D2 `transport_denied`).
