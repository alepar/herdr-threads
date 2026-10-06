# Recording the three-pane README walkthrough

The video records an actual attached Herdr client: Alice (Claude Code) at upper
left, Bob (Codex) at upper right, and the human shell across the bottom third.
The agents' native tool activity and thread discussion are genuine. Font size
is 15. Commands are visibly typed at 90 ms per character (spaces take 1.3 times
that); only the long assignment text is typed at five times that speed. Opening
comments explain creation, invitation, communication, and observation.

## Prepare privately

Obtain the coordinator's serial slot before native startup, capture, rendering
or focused process tests. Use an isolated source worktree and an existing built
binary. Do not run the full suite or restart the shared Herdr server.

```sh
python3 scripts/demo-try-it-prepare.py --bin target/debug/herdr-threads
```

Retain the returned run directory and exact tab/human/Alice/Bob IDs. Preparation
creates a mode-0700 `/private/tmp/ht-try-it.*` directory, private HOME, ZDOTDIR,
CLAUDE_CONFIG_DIR, CODEX_HOME, XDG directories, TMPDIR and plugin state. Credentials
are copied, never linked to real profiles. Claude's wrapper uses the access token
from its private copy; Codex can refresh only its copied auth file.

The current profiles' preferences, permissions, hooks, skills, rules, agents,
custom prompts, plugin resources, status lines and startup/trust records are
copied. Source-profile resource paths are rebased to the private copies; Git
history and transient plugin clones are excluded. Walkthrough hooks use private
thread state. The copied Claude usage helper uses the copied token and private
cache, rather than reading the real keychain during capture. Real profiles are
never written. The private Herdr skill is replaced with the exact captured
binary's embedded guide, including after settings refresh, so copied older
instructions cannot invalidate the efficiency trial. Additional preferences can be refreshed before native startup:

```sh
python3 scripts/demo-try-it-prepare.py --copy-settings RUN_DIR
```

Trust records are copied honestly; new scratch-folder paths or rewritten hook
commands can require native review. Preflight both native CLIs in their owned
panes, reviewing only the generated scratch folder and hooks, then exit back to
their shells. Codex should receive native `-- --no-daemon`. Do not invent hook
trust hashes. Copied settings and completed private review persist between
preflight and the actual handoff, avoiding repeated startup confirmations.

## Attach the real camera

The user approved a separate attached client, including its initial inherited
focus/geometry behavior. This requires explicit coordination in a shared
session. It never starts or restarts the server. The client uses generated
private config and state with updates disabled and its sidebar hidden:

```sh
python3 scripts/demo-try-it-prepare.py --camera RUN_DIR
```

Keep that command running in the controller. Append literal key strings as JSON
lines to `RUN_DIR/control.jsonl`. Use the client-local navigator (`ctrl+b`, then
`g`), search for the unique returned tab label, select the owned pane, and focus
the human pane (`ctrl+b`, then `j`). Inspect a private preview to verify the exact
three-pane layout. Public `herdr tab focus` broadcasts to other clients; use local
camera navigation instead. Never record the initial inherited tab or navigation.

For example, append one JSON key string from a separate controller command:

```sh
python3 - RUN_DIR/control.jsonl <<'PY_KEYS'
import json, sys
with open(sys.argv[1], 'a') as keys:
    keys.write(json.dumps('\u0002g/UNIQUE_TAB_LABEL') + '\n')
PY_KEYS
```

Prepare/clear the owned shells. Run the typed-command recorder **inside the
returned human pane**, without redirecting stdout away from that pane:

```sh
python3 RUN_DIR/record.py --commands RUN_DIR/commands.json \
  --output RUN_DIR/live.cast --cwd RUN_DIR/project \
  --cols 157 --rows 15 --start-file RUN_DIR/typing-start \
  --finish-file RUN_DIR/finished --timeout 600
```

It clears its real shell and waits at the prompt. After verifying the camera
shows only the owned tab, create `RUN_DIR/camera-start`, then one second later
create `RUN_DIR/typing-start`. The camera drains private setup output, performs a
real client redraw at final geometry, and records actual PTY output at 160×62.
The separate human cast records every single-character input and its timestamp.
The full client can resize owned panes; align recorder dimensions with the
human pane's actual content size.

Observe the canonical thread and native panes. Require three substantive,
reciprocal Alice/Bob exchanges and an explicit shared conclusion accepted by
both. Only then create `RUN_DIR/finished`. The recorder holds the final scene,
interrupts follow and checks its actual exit status. Check recorder exit 0, then
create `RUN_DIR/camera-stop` and check camera exit 0. Preserve any failed attempt
privately; never replace or splice agent responses.

## Privacy export and render

Review the raw camera and canonical discussion before publication. The public
export replays the actual terminal with pyte 0.8.2, samples visible cells at
12 fps, retains styles/cursor state, crops only the single tab-bar row, and
replaces personal home-prefix cells with equal-width `$HOST_HOME___` placeholders.
An orphan directory name whose prefix scrolled out of view becomes `$USER_`.
These placeholders are privacy annotations, not reproduction commands. All other
pane cells come from the real client. No discussion text is supplied by the
exporter. Retain the unmodified raw stream outside the repository.

```sh
python3 -m pip install --target /private/tmp/try-it-render-deps pyte==0.8.2
PYTHONPATH=/private/tmp/try-it-render-deps python3 scripts/demo-try-it-export.py \
  RUN_DIR/camera.cast docs/media/try-it.cast --home-prefix "$HOME"
agg -q --theme dracula --font-size 15 --fps-cap 12 --speed 1 \
  --idle-time-limit 600 docs/media/try-it.cast /private/tmp/try-it-revision-full.gif
ffmpeg -y -i /private/tmp/try-it-revision-full.gif -ss 0.083333 -an \
  -c:v libx264 -crf 25 -vf 'fps=12,pad=ceil(iw/2)*2:ceil(ih/2)*2' \
  -pix_fmt yuv420p -movflags +faststart docs/media/try-it.mp4
```

Only the initial blank export frame is trimmed. The video keeps natural timing
and the renderer's final hold. The intermediate GIF is private; README uses a
GitHub-uploaded H.264 attachment so viewers can play, pause, seek and expand it.
Keep the MP4 in the repository as a downloadable copy. Verify rendered typing,
native action and final agreement frames at README size and full screen, plus
privacy, dimensions and duration before upload or merge.

The [human input cast](try-it-input.cast) separately preserves original input
characters and timing, with 18 output-only home-prefix redactions. The
[canonical message bodies](try-it-discussion.json) preserve all eight genuine
Alice/Bob posts, including both conclusions.

## Cleanup and verification

From the controller outside the demo tab:

```sh
python3 scripts/demo-try-it-prepare.py --cleanup RUN_DIR
scripts/check-no-leaked-processes --run-id RUN_ID --root WORKTREE_PATH
PYTHONPATH=/private/tmp/try-it-render-deps python3 -m unittest \
  scripts/tests/test_demo_try_it.py scripts/tests/test_demo_try_it_camera.py \
  scripts/tests/test_demo_try_it_export.py
```

Close only recorded owned topology; stop the private daemon and reap exact
run-tagged helpers. Verify recorded PIDs are absent. Cleanup removes all copied
profiles/credentials while retaining raw casts, canonical evidence and failed
attempts. Leave the task/source topology for coordinator cleanup. After rendering
and checks, report SAFE RELEASE; coordinate a separate main mutation window.

## Published take

The 2026-10-06 take used source base `97c98998`, Claude Code 2.1.290 with copied
Opus 5.5 preferences and Codex 0.160.1 with copied GPT-6.1-Sol medium preferences.
Private run: `c5e180fc-4c97-4e84-8740-ede77e08f67f`; canonical thread: `tfvkDs0mf`.
Alice/Bob address consistent rendering, accessibility, tab-aware enforcement,
formatter/CI discipline and ecosystem conventions before agreeing. Recorder and
camera both exited 0. The raw camera lasts 278.861236 seconds; the human cast
lasts 287.343417 seconds, including its pre-camera wait, and contains 1,468
single-character inputs. Measured normal/assignment medians are 96.135/51.181 ms
for Alice and 97.800/52.273 ms for Bob (requested 90/45 ms plus actual I/O).

The MP4 is 1496×1302 at 12 fps, 281.750000 seconds, 9,549,417 bytes. Its SHA-256 is
`3fd1409c8bfc32ef8f30dece069ec0830b1cbb289524f50865d0be61cfbc86be`.
The user approved publication to GitHub's public attachment service. The
[uploaded video](https://github.com/user-attachments/assets/3fbe26a8-a419-4456-85ad-51af3ab5b76b)
renders through GitHub's Markdown renderer as a video with playback controls;
its downloaded bytes match the MP4's SHA-256.
Raw camera SHA-256: `8872082f12770d13848d5c1fa53fba684c9d9a8d47744fa19321c41e794ef119`.
Raw human cast SHA-256: `05f853962c317b565fe6d0651a97e7b781a2b467208fad76c15d716a7992853b`.
Public camera cast SHA-256: `44fe0dfebe55b580f8d441313d9e80e6f6ac83349dbee3a44e0e1e9ad75d7a2b`.
Public input cast SHA-256: `0218205495d501c0a690753c7cb18b50b8892c0d09159483e4add672ef44604f`.
Canonical discussion SHA-256: `135fbe915a3569264c3a61ab5b551612c831ed5b12501aaf5360e281f31bb6e3`.

The private archive `/private/tmp/ht-try-it.4leew81y` retains raw/canonical evidence
and the failed socket-alias take. Herdr's host witness does not support those
aliases; final capture uses the real endpoint with visible-cell privacy export.
The earlier three-pane privacy trial is preserved at
`/private/tmp/ht-try-it.pkaf1d4b`. Both runs' owned processes are reaped and copied
profiles removed; both scoped leak checks pass. No full suite or release ran.

Historical `tea-party.gif`, `tea-party.mp4` and `scripts/demo-tea-party.sh` remain
byte-identical. The prior human-only GIF `try-it.gif` is retained as an earlier
artifact; README now uses the three-pane video.
