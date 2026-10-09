# Spike: delivering herdr-threads messages through a Claude Code mod

Date: 2026-10-09. Claude Code 2.1.294, interactive TUI sessions in a private tmux server
(`tmux -L htspike`), with an isolated `CLAUDE_CONFIG_DIR` (a copy of the user's settings without
hooks, status line or plugins, signed in separately). The sessions had no Herdr or
cross-session messaging environment, so nothing reached the real herdr-threads daemon.

Background research: `~/Documents/Claude_Code_Mods_Delivery_Research_20261008/report.md`.

## What was built

- `mod/`: a plugin whose hooks module follows a feed file through a session-long
  `$.process.spawn(['tail', '-F', feed])` child and delivers each JSON line in one of three ways:
  - `submit`: `$.prompt.submit`, a new turn once the session is idle
  - `context`: attached to the next main-conversation tool result from a `tool.call` hook
  - `append`: `$.session.append` of a user-role row, which starts no turn
- `drive.py`: starts sessions, feeds messages, sends keys, and prints the mod's event ledger.
- `evidence/`: the event ledgers per session, plus the debug-log lines from the crash test. The
  first load's rows for `s1` were overwritten before the ledger moved to one file per load; the
  console output from those steps is summarised below.

## Results

| # | Scenario | Result |
|---|---|---|
| 1 | Idle wake (`submit`) | The turn started about 30 ms after the mod read the line. The feed child saw the line in under 1 ms. The submit call resolved at the moment the turn started. |
| 2 | User draft in the prompt box, no guard | The plugin turn ran, and the draft `half-typed user draft XYZ` stayed in the box. It was neither submitted nor lost. |
| 3 | Permission dialog open | A forced submit did not disturb the dialog. It resolved only after the user approved and the turn finished, then started its own turn. |
| 3 | `context` while the dialog was open | Attached after the approved Bash result. Claude used it in the same turn (`CTX-3`). The transcript stores it as a `hook_additional_context` attachment from PostToolUse. It is not shown in the TUI. |
| 4 | `append` mid-turn (during `sleep 8`) | Claude read it in the same turn (`APP-4`). Stored as an `isMeta` user row with origin `{kind: plugin}`. Not shown in the TUI. |
| 5 | `append` while idle | No turn started. Claude read it on the next user turn (`KIWI-5`), and it was still there after `/resume`. |
| 6 | `/clear` | `session.end` fired with reason `clear`. The feed child survived, the session id changed, and delivery continued. |
| 7 | `/resume <id>` | `session.end` fired with reason `resume`. The id switched back, the feed child survived, and delivery continued. |
| 8 | Hot reload (save) | `session.start` ran again. The old child was killed and exactly one new one started. Module state reset. |
| 9 | Bypass permissions mode | Submit delivered immediately. Unlike the inbox socket, it is not held for approval. |
| 9 | Auto mode | Submit delivered immediately. |
| 10 | User presses Esc with a submit queued | **The queued plugin turn started right after the interrupt.** The new turn's `turn.start` also arrived before the aborted turn's `turn.complete`. |
| 11 | Worker wedged (busy loop) | Nothing was detected until a hook event arrived. Then a 5 s heartbeat timeout unloaded the mod, with a visible transcript line (`ht-spike was unloaded: it crashed the hooks worker`). The session kept working. The feed child was killed, so no orphan was left, and messages fed while unloaded were lost from this feed. `/reload-plugins` restored delivery. |
| 12 | `asUser: true` | Rendered as plain text under a `› Prompt from the ht-spike plugin` header, without the "The ht-spike plugin sent a message" frame. |
| 13 | Loaded via `CLAUDE_CODE_PLUGIN_DIRS` instead of `--plugin-dir` | Works the same. |
| - | Pane identity | `HERDR_PANE_ID` from the pane's environment was readable in every session. |

How a framed submit renders (both the TUI and what Claude reads):

```
› Prompt from the ht-spike plugin
❯ The ht-spike plugin sent a message:
  [herdr-threads] message m1 from alice: Please reply with exactly the word PONG-1 and nothing else.
  This is how Claude Code surfaces a prompt a plugin submits between turns — it starts this turn in the user's place. Address the message above.
```

## Design consequences for a real herdr-threads mod

1. **A complete Claude-side hot path in one mod:** `context` covers delivery between tool calls,
   `submit` wakes an idle session, and `append` gives lazy delivery that rides along with the next turn. The
   PostToolUse/Stop hook processes and send-keys are not needed while the mod is live.
2. **Never queue a submit while busy, and hold after an interrupt.** A queued submit takes over the
   session right after Esc. After an aborted turn, wait for the user's next turn to finish, or for an
   idle period with an empty prompt box.
3. **Track busy state per turn id, not with one flag.** Events from the next turn can arrive before
   the aborted turn's `turn.complete`.
4. **Context and append are invisible to the user.** Pair each one with a `$.ui.log` line, or an
   `asUser`-free framed submit when visibility matters.
5. **The watch stream is the liveness signal.** Unload, crash, reload and kill switch all end the child
   process. The daemon should treat a disconnected watch as "mod gone": fall back to hooks plus send-keys,
   and resume from the unacked cursor when the mod reconnects. Messages must not be dropped in the gap.
6. **Rebind on `session.end`.** After `clear` or `resume`, read `$.session.id()` again and re-check the
   seat in. The child keeps running.
7. **Receipts:** submit resolution, context attachment and append resolution are each a point where
   the message has entered the conversation. Ack from there.
8. **Installation without a marketplace:** `CLAUDE_CODE_PLUGIN_DIRS` works. A marketplace install
   is the documented route.

## Not covered

- The real remote kill switch, simulated only by reasoning: it stops the mod from loading, which
  looks the same to the daemon as no watch connection.
- A subagent's tool calls (the hook skips calls with `e.agentId`; not exercised).
- Slash-prefixed submit text (framing always starts with `[herdr-threads]`; the prior-art spike
  reports a refusal).
- Desktop app, VS Code, `claude -p`.
