# agy composer captures

`<state>.read-detection.txt` is the output of `herdr agent read <pane> --source detection` for a
genuine agy, with the isolated root rewritten to `<root>`. `agent-list.txt` is `herdr agent list`
for the same pane (Herdr reports agent kind `agy`). Read by `tests/harness/agy_composer.rs`.

- agy: Antigravity CLI 1.3.2 (official darwin-arm build, the `HT_AGY_BIN` binary)
- Herdr: 0.9.1
- Captured: 2026-10-10, pane 119 columns by 40 rows
- Model-free: every model request goes to `harness-fake_server.py` (the frozen research probe
  fake Gemini server, copied unchanged) on loopback, answering from a scripted queue.

## Recipe

```
HT_AGY_BIN=/path/to/agy tests/fixtures/agy/composer/capture.sh
```

`capture.sh` makes a private root (`scripts/lib/isolated-herdr.sh`: its own Herdr session, HOME and
XDG dirs), seeds `antigravity-cli/settings.json` (`altScreenMode: never`, `modelProvider: gemini`,
trusted workspace, `permissions.allow: ["command(sleep)"]`), starts the fake server, and runs agy in a
Herdr pane under `sandbox-exec` with `scripts/agy-sandbox/loopback.sb.in` (loopback only, writes
confined to the root). The first run walks agy's onboarding screens with keys. It tears everything down
on exit; check with `scripts/check-no-leaked-processes`.

| Capture | How |
|---|---|
| `empty` | idle composer |
| `help-overlay` | `?` typed in an empty composer opens the shortcuts overlay |
| `draft` | `pane send-text "hello draft"`, no Enter |
| `multiline` | `first row`, `shift+enter`, `second row` |
| `multiline-after-ctrl-u-1`, `-2` | `ctrl+u` once and twice after `multiline` |
| `busy` | the queue holds the reply for 40 s |
| `permission-prompt` | the scripted reply is `run_command echo …`, which is not in the allow list |
| `subagent-generating` | scripted `invoke_subagent`; the parent finishes at once, the child's reply is held |
| `subagent-tool-running` | as above, the child's reply is `run_command sleep 90` (allowed) |
| `old-agent-row` | the scripted reply quotes `● Agent(DeepCoder)  Generating · 12m31s`, so the row sits in the transcript above the composer |

The parent and the child race for the next queue entry, so `capture.sh` retries a subagent state in a
fresh pane until the screen shows the `● Agent(` row.

## Grammar the captures show

- Composer: two full-width `─` rules; between them the first row starts `>` (`>` alone is empty), later
  rows are indented two spaces. A footer line follows the bottom rule: `? for shortcuts …` when idle and
  empty, blank-led (`   … Gemini 3.1 Pro · low`) with a draft, `esc to cancel …` while a turn runs.
- Transcript separators above the composer are 60 `─` wide, not pane width.
- A subagent adds a status region: rule, `  ● Agent(<name>)  <Working|tool action> · <elapsed>`, rule, and
  the footer gains ` · N subagent(s)`. Herdr reports `idle` for this screen (issue #2354).
- The permission prompt and the help overlay have a single full-width rule followed by many lines.
- `ctrl+u` deletes back to the start of the line and, at a line start, joins the row to the one above, so
  one press clears a single-row draft and repeated presses clear every row. `ctrl+c` does not clear (it
  prints `press ctrl+c again to exit`).

## Correction to the provisional design rule

Design D8 said "the rows between the last two `─` rules". The captures need more: with a subagent the last
two rules enclose the agent row, and a permission prompt's last two rules (the 60-wide transcript separator
and the pane-wide one) enclose transcript text starting `>`. `AgyComposer` therefore uses only
pane-width rules, requires exactly one idle footer line after the bottom rule, and treats any
`Agent(<name>)` row from the third-from-last pane-width rule down as `Unreadable`.

The old-row capture stands in for a stale row in scrollback with a model reply that quotes a row; agy
itself printed no such row after the child finished.
