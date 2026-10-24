# Hermes 0.22.5 composer captures (ht-hermes-wake)

Captured 2026-10-23 on blacky (Linux 7.0.12, Herdr 0.9.5) for the Hermes `ComposerPolicy`
(`src/harness/hermes/composer.rs`, tests `tests/harness/hermes_composer.rs`,
`tests/multinode/hermes_composer.rs`).

## Recipe

- Install, scratch only: a local clone of `v0.22.5` (`bf0203a794a0`), then
  `HOME=<scratch>/home bash setup-hermes.sh` (Hermes' own installer). Model `openai-codex` /
  `gpt-6.1-sol`; credentials imported through `hermes model` ("Import these credentials?")
  from a mode-0600 copy of the Codex `auth.json` in the scratch HOME (`CODEX_HOME` pointed
  there). The copy was never refreshed.
- Bare `hermes` (default modern layout) with `HOME=<scratch>/home` in panes of an owned
  Herdr workspace, closed afterwards; its gateway was stopped (`hermes gateway stop`).
- Each state: `herdr agent read PANE --source detection` (`*.read-detection.txt`) and
  `herdr agent read PANE --source recent --format ansi` (`*.recent-ansi.txt`). Scratch paths
  read `<scratch>`, the session directory `<session>`; footer cwd fragments truncated by
  Hermes keep a few characters of that directory name.

## States

Pane 155 columns wide unless named `narrow-` (75 columns). Herdr reported `agent: hermes`,
`agent_status: idle` in every one of them, the busy ones included.

| Capture | Screen | Reads |
|---|---|---|
| `telemetry-modal` | first-run shared-metrics consent, no composer | Unreadable |
| `idle-empty`, `after-turn`, `narrow-idle-empty` | `❯ ` + catalog placeholder, italic (`ESC[3m` + 256-colour fg) | Empty (styled) |
| `draft`, `history-suggestion` | `❯ ` + unstyled typed text | Text |
| `multiline-draft` | typed first row + unindented second row (alt+enter) | Text |
| `busy`, `narrow-busy` | top rule `── ⠙ 💻 sleep 30 … ───`, composer `☤ ❯ type to redirect this turn …` | Unreadable |
| `busy-draft` | the same with `☤ ❯ redirect draft` | Unreadable |
| `palette` | command palette panel (`│ … │`) right above the top rule, footer blank | Unreadable |
| `model-picker`, `model-picker-loading` | `/model` picker open (and still loading), footer blank | Unreadable |

## Observations

- Herdr 0.9.5 reports a Hermes pane `idle` throughout a turn, so the screen is the busy guard:
  Hermes puts the live status into the composer's top rule while a turn or command runs.
- The empty-composer placeholder is a random entry of Hermes' localized catalog
  (`hermes_cli/tips.py`, `tips.placeholder.pNN`), chosen once per session, styled
  `placeholder: "{dim} italic"` (`hermes_cli/skin_engine.py`); typed text keeps the terminal's
  default rendition (`input-area: ""`).
- The footer's key hints (`/ commands · @ files · ! shell` while idle) are drawn only at 76
  columns or more and only while they fit beside the left side, which grows after the first
  turn: at 119 columns they were gone after one turn. The reader does not use them.
- The model picker and command palette blank the footer (`_get_status_bar_fragments`).
- `herdr agent prompt` submits in Hermes (text then Enter), while a `pane send-text` followed
  at once by Enter is taken as a pasted newline.
