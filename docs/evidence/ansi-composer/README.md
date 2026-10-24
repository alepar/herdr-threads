# Styled composer captures (ht-ansi-composer)

Captured 2026-10-23 on Linux from panes created in an isolated Herdr 0.9.5 server (`scripts/lib/isolated-herdr.sh` layout), never from another session's pane. The files in `captures/` are byte-exact (`.gitattributes`: `-text -whitespace`).

## How

- **Claude Code 2.1.309** ran with a scratch `CLAUDE_CONFIG_DIR` (onboarding and folder trust seeded) against a local stand-in Messages API (`ANTHROPIC_BASE_URL`, a dummy key) that answers every request with the text `run the tests`, so no account or credential was used. Prompt suggestions sit behind a GrowthBook flag that is unreachable offline, so `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=1` turned them on. Claude makes no suggestion request before the second assistant turn (`early_conversation`), so two turns ran first. The fourth model request was the suggestion request, and its answer is the suggestion on screen.
- **Codex 0.163.0** ran with a scratch `CODEX_HOME` holding a mode-0600 copy of `auth.json`. It ran one real one-line turn, because `/side` refuses to open before the conversation has started.
- Claude's `recent-ansi`/`read-detection` pairs went through the socket API (`agent.read`), as the native host makes them. The Codex captures and the `placeholder-uncaptured` pair came through `herdr pane read`. `*.read-detection.txt` is `source: detection`. `*.recent-ansi.txt` is `source: recent, format: ansi`. `*.visible-ansi.txt` is `herdr pane read --source visible --format ansi`. The `placeholder-uncaptured` pair shows the launch line above Claude's banner; its dummy key prefix was replaced by the same-width `dummy-apikey-`.

## Findings

- Herdr's `detection` source ignores `format: ansi`: it returns plain text yet echoes `format: "ansi"` (Herdr `src/app/api_helpers.rs`, `(ReadFormat::Ansi, ReadSource::Detection) => detection_text()`). `visible` reads the viewport, so it follows the pane's scroll position. `recent` is bottom-anchored like `detection` (which is the last screen-height lines of `recent` text). The styled composer read therefore uses `source: recent, format: ansi`.
- **Claude**:
  - The prompt glyph `❯` is drawn unstyled and is followed by U+00A0.
  - The empty-composer placeholder and the prompt suggestion are both SGR 2 (dim) and nothing else: `❯\u{a0}\x1b[0m\x1b[2mrun the tests\x1b[0m`.
  - Typed text is unstyled.
  - Typing a prefix of the suggestion (`run`) hides the rest of it, so no row mixes styles.
  - The placeholder text varies per launch. `Try "fix lint errors"` (`placeholder-uncaptured`) is not the one captured placeholder, so the plain detection read finds it `Unreadable`.
- **Codex**:
  - The glyph `›` is bold. The placeholder `Ask Codex to do anything` and the `/side` placeholder `Ask a follow-up question` are dim. Typed text is unstyled.
  - The plain detection read classified the `/side` placeholder as typed `Text`, so every wake under `/side` was refused.
  - Codex's source (0.163 `bottom_pane/chat_composer.rs`) also draws `Viewing sub-agent — direct input is disabled` dim, with a bold glyph, while it blocks a typed submit. So "dim means empty" is not safe for Codex, and the `/side` placeholder is added to the known placeholders instead.

| File | Shows | Plain read | Styled read |
|---|---|---|---|
| `claude-2.1.309-suggestion` | dim suggestion `run the tests` | Unsafe (may be a suggestion) | Empty |
| `claude-2.1.309-placeholder` | dim `Try "how do I log an error?"` | Empty | Empty |
| `claude-2.1.309-placeholder-uncaptured` | dim `Try "fix lint errors"` | Unreadable | Empty |
| `claude-2.1.309-draft` | typed `hello draft` | Unsafe | Text |
| `claude-2.1.309-typed-prefix` | typed `run` over the suggestion | Unsafe | Text |
| `claude-2.1.309-multiline-draft` | typed two rows | Unsafe | Text |
| `codex-0.163.0-empty` / `-side` | dim placeholders | Empty | n/a |
| `codex-0.163.0-draft` / `-side-draft` | typed text | Text | n/a |
