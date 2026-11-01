# screen_turn_running fixtures

Copies of existing captures (`ComposerPolicy::screen_turn_running`, spike T2), per harness:

- `claude/running.txt`: docs/evidence/claude-mod-delivery/live/pane-s-esc-interrupt-hold-esc_interrupt_hold-1.txt
- `claude/idle.txt`: docs/evidence/poke-spike/captures/claude-q1-empty.read-detection.txt
- `codex/running.txt`: docs/evidence/poke-spike/captures/codex-q5-poke-during-tool.read-detection.txt
- `codex/idle.txt`: docs/evidence/ansi-composer/captures/codex-0.163.0-empty.read-detection.txt
- `agy/running.txt`: tests/fixtures/agy/composer/busy.read-detection.txt
- `agy/idle.txt`: tests/fixtures/agy/composer/empty.read-detection.txt

Hermes has no running-turn capture, so it keeps the `None` default (a stale busy lasts until its next hook).
