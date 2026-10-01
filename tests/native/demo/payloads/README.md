Captured native hook input shapes used only by `scripts/validate-native-demo.py --dry-run`
to stand in for the real harness hook calls. The driver rebinds `session_id`, `cwd` and
`transcript_path` to the run before use.

- Claude: the driver now uses the committed Claude 2.1.286 fixtures
  (`tests/fixtures/claude-2.1.286/01-sessionstart-startup.json`, `02-pretooluse-bash-root.json`), matching the
  installed version the recipe admits. The `claude-*` files here are the earlier 2.1.284 capture, kept for reference
  (`docs/evidence/claude-284-hook-capture/payloads/01`, `02`).
- `codex-*`: Codex 0.158.0 capture (`docs/evidence/codex-158-live-hook-capture/payloads/run2.session-start.startup.json`,
  `run3.pre-tool-use.root.json`); paths there are already redacted placeholders.
