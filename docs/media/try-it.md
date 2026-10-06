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
  --cols 157 --rows 18 --start-file RUN_DIR/typing-start \
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
characters and timing, with 14 output-only home-prefix redactions. The
[canonical message bodies](try-it-discussion.json) preserve nine genuine
Alice/Bob posts: Bob ready, three reciprocal rounds, and both conclusions.
The [Codex command evidence](try-it-codex-efficiency.json) ties every inbound
body and outbound post to the native session and canonical thread.

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

## Mailbox-first take

The 2026-10-06 take used source `128b3f84786c4a3e072121ad61151748cbc1367c`,
Claude Code 2.1.290 (Opus 5.5 medium) and Codex 0.160.1 (GPT-6.1-Sol medium),
with copied native preferences. Private run: `6975c164-ba43-4f06-bfc6-8dea3416d2c4`;
canonical thread: `t8HK3GC0u`. Alice/Bob discuss rendering, accessibility,
formatter enforcement, line limits, copy-paste and language conventions before
agreeing. Bob's ready message precedes Alice 1, so the debate needs no pre-join
history. That readiness change is separate from the shipped instruction fix.

Codex's native rollout proves 5 inbox calls, 1 acceptance and 5 sends: one ready
message, three debate replies and confirmation. It ran no `read`, `body`,
`follow`, manual `ack`, `pending-receipts` or polling commands. Each of Alice's
four complete bodies appeared in exactly one inbox response; all five Bob send
bodies match canonical messages. Five native turns completed while waiting for
hook notifications. No receipts remained pending. Two initial non-mail shell
calls discover the skill; the first attempted a missing unrelated skill.
This proves the observed session's behavior, rather than a universal efficiency
benchmark. Faster typing does not enter the command-count measurement.

The recorder and attached camera both exited 0. The raw camera lasts 304.543880
seconds; the human cast lasts 352.659269 seconds including the private camera
wait. All 1,553 input events contain one character and match the command plan.
Requested delays are 90 ms for ordinary input and 18 ms for assignments (5×).
Measured normal/assignment medians are 96.694/24.469 ms for Alice and
96.900/24.238 ms for Bob, including actual PTY/output overhead.

The H.264 MP4 is 1496×1302 at 12 fps, 307.416667 seconds, 9,482,756 bytes.
SHA-256: `0fe2626e2130e2b1bf0dc15d526c7d1aaf82303adc96a757d8bc32d2307247dd`.
The [new video attachment](https://github.com/user-attachments/assets/081ca706-6a44-4f03-b9ee-572a9b12ec71)
renders through GitHub’s Markdown service with playback controls, and its
downloaded bytes match the repository MP4’s SHA-256.
Only the initial blank export frame is trimmed; actual discussion timing and
the renderer's final hold remain. Public visible cells, styles and cursor state
are checked against the real client; changes are the tab-chrome crop and
same-width privacy placeholders. Raw evidence remains private.

The private archive `/private/tmp/ht-try-it.yh0qgnbj` retains the raw casts,
canonical messages, native rollout, command/output proof, input timing and
privacy checks. All eight tagged PIDs are absent after cleanup; the owned tab,
private daemon and copied profiles are removed. The scoped leak check passes.
The failed profile-copy attempt `/private/tmp/ht-try-it._wjogkzp` is retained,
with its processes reaped and credential copies removed.

The prior three-pane take remains in Git history and its
[original video attachment](https://github.com/user-attachments/assets/3fbe26a8-a419-4456-85ad-51af3ab5b76b),
with raw archive `/private/tmp/ht-try-it.4leew81y`. Historical `tea-party.gif`,
`tea-party.mp4`, `scripts/demo-tea-party.sh` and the earlier human-only `try-it.gif`
remain byte-identical. No full suite or release ran.
