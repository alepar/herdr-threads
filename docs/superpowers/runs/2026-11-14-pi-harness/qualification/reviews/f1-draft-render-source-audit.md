# Native draft failure: source-backed hypothesis

Status: leading hypothesis, not a demonstrated near-submit screen state. No fix or new accepted limitation is approved by this audit. Read-only source/retained-trace inspection; no runtime or build performed.

## Frozen references

Failure artifacts: `native-path-corrected/{manifest.json,commands.json,captures.json,native.jsonl}` in this directory. Manifest source HEAD is `a424ed20432d325a3f3ed8516733aa066cb4b501`; candidate binary SHA256 is `a8aef2df56e6954fe3c12b7d02841f521712ceb403d13f9b93fabdabe2224289`.

The inspected runtime is the local research package under `docs/superpowers/runs/2026-11-14-pi-harness/research/candidates/probe/node_modules/@earendil-works/`. Both `pi-coding-agent/package.json` and `pi-tui/package.json` report version `1.5.2`. These are inspected installed-source references; package metadata alone does not prove which bundled instructions executed in the failed process.

| File | SHA256 |
| --- | --- |
| pi-coding-agent/dist/modes/interactive/interactive-mode.js | a316502682c68544e1950b80e3b19d584c7a43331cfb7029765b5c04d18e4465 |
| pi-tui/dist/components/editor.js | a58560dea1ca206670183007d2eb683db11dd1bd25d99349d7e009683472a7c2 |
| pi-tui/dist/tui.js | 45116c628fdad126ff6e89ebd1ba5439f25c245f13f3b052f63e2817ad9ff9b5 |
| scripts/pi_tui_fixtures/provider.mjs | 1bbb2f67a76a015beebeb055fb915389fb6aa50178c46d1e7c17e95d7d2380fa |
| scripts/pi-tui-qualify.py | ce7567bb9bbba0b6f56a0eee2e883a64955c9590dfba111734eca0241d83b5f2 |

The provider hash matches the failed-run manifest's `fixture_sha256`.

## Observed sequence and missing witness

The controller's capture at `1794741149623` shows draft `OWNED NOTICE QUALIFICATION DRAFT` and an inverse cursor after its text. That capture precedes the private slash-command preparation at `1794741149658` and controller Enter at `1794741149854`.

The command handler restores the draft programmatically. Native state at `1794741150159` reports idle and the exact draft in the editor API. No subsequent terminal screen capture before the wake is retained. At `1794741160524`, terminal input records bracketed-paste `herdr-threads: wake up`; Enter follows at `1794741160824`. `before_agent_start` at `1794741160863` contains the concatenated draft plus wake text. The controller's intended draft-clear operation was not reached.

Thus the trace proves an internal draft was submitted with the wake. It does not prove the draft remained visible on the terminal at the daemon's composer read.

## Causal source path

1. The provider command calls `ctx.ui.setEditorText(data.hold)` before sending the public notice cause (`scripts/pi_tui_fixtures/provider.mjs:67`).
2. Pi's UI API implements `setEditorText` solely as `this.editor.setText(text)` (`interactive-mode.js:2194`). `Editor.setText` delegates to `setTextInternal` (`editor.js:928`), which updates editor memory/cursor and invokes `onChange`, without requesting a render (`editor.js:375`). The installed interactive `onChange` only updates the editor border when entering/leaving bash mode (`interactive-mode.js:2575`). Neither the slash command nor this draft begins with `!`.
3. Enter clears the editor synchronously before invoking `onSubmit` (`editor.js:1145`). Terminal input then schedules an immediate render (`tui.js:780`); that scheduler uses `process.nextTick` (`tui.js:669`).
4. Normal submission resolves the input promise (`interactive-mode.js:2835`, `:3576`). The asynchronous main loop then calls `session.prompt(userInput)` (`interactive-mode.js:949`), which executes the fixture command. The main loop requests no render after that call (`:951`).

This supports a concrete explanation: Enter paints the cleared editor, then the command restores the draft only in memory. A subsequent screen-based composer check can see an empty editor while the next pasted text appends to the internal draft. Actual ordering and the near-submit screen remain unobserved in the retained failure.

## Alternative path check

Notification dispatch calls `HostPort::submit_prompt_if_owed` (`src/notification/dispatch.rs:669`). Native submission variants funnel through `submit_prompt_mode`; its composer check is mandatory (`src/host/native.rs:2208`). Nonempty Pi detection returns `Unsafe`, and does not request a styled override (`src/harness/pi/composer.rs:63`, `:75`; `src/harness/composer.rs:105`). The lower-level public `NativeCli::prompt_with_limit` has no external production callsite in the inspected tree. No alternate wake bypass was found.

## Discriminating observation

A diagnostic-only capture after the completed `notice_cause` handler should retain actual detection text, styled terminal text, and fresh editor API state. The diagnostic must not request a repaint, alter editor text, press a key, change wake eligibility, or otherwise repair the condition being observed. If internal draft and visible empty editor coincide, that supports this hypothesis; if both show the draft, the daemon's actual read and candidate process identity remain necessary to explain the gate result. An earlier post-cause capture is still not an exact near-submit witness.
