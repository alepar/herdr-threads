# Spike: a plugin prompt queued behind a user turn, then Esc

Date: 2026-10-09. Claude Code **2.1.295**, interactive TUI, Opus 5.5 (medium effort), in a private
tmux server (`tmux -L racespike`). The signed-in profile was used in place as `CLAUDE_CONFIG_DIR`
with the real HOME (its settings.json was not modified). Settings came from
`claude --settings <file>` (allow `Bash(sleep:*)`), the plugin from `CLAUDE_CODE_PLUGIN_DIRS`,
and `CLAUDECODE` / `CLAUDE_CODE_ENTRYPOINT` were unset. Each trial ran in a fresh Claude session.

This settles escalation 1 of `code-final-review-11.md`, "Idle check vs submit not atomic" (spec D5).

## Question

The mod checks idle, then calls `$.prompt.submit(text)`. If the user presses Enter at the same
moment, the plugin prompt may end up queued behind the user's turn. If the user then presses Esc:

1. Is the queued plugin prompt still delivered to the model, or is it discarded?
2. What does the `submit` promise resolve to, and when?

The mod acks any resolution without `drop`. So the unsafe outcome is a prompt that is
discarded but still resolves as a success.

## Method

A throwaway mod, `racespike`, is a plugin dir with `hooks/hooks.json` and `hooks/register.js`, in the
same register/hook shape as the real mod. It logs JSONL with `Date.now()` timestamps for these events:
`session.start`; `prompt.submit` in and out (origin, turnId, drop); `turn.start` (turnId, text);
`turn.complete` (turnId, reason, isAborted); and each `$.prompt.submit` call with its resolve or
reject value. The mod calls `$.prompt.submit({ text: 'PLUGIN-MARKER-<n>: reply with the word PINEAPPLE' })`
with no idle check, using one of these triggers:

- `in-user-turn`: on the `turn.start` of the user's `SLEEPTEST` prompt. This forces the queued case.
- `same-instant-session$`: during the user's own `prompt.submit` dispatch, through a closure over
  `session.start`'s `$`. This is the closest model of "submit at the same instant as Enter".
- `same-instant-after1`: `$.clock.after(1)` from the user's `prompt.submit` dispatch, so the call
  lands just as the user's prompt enters.
- `same-instant` (RACETEST): the plugin's own `prompt.submit` hook's `$`. The real mod has no
  `prompt.submit` hook, so this is a negative control.

The driver (`drive.py`) types the user prompt "run the bash command `sleep 20` and then reply with
the single word DONE". It sends `Escape` at the time the mode calls for, waits about 45 s, and
captures the pane. In the second and third batches it then sends a FOLLOWUP prompt that asks the
model, with no tools, to quote the PLUGIN-MARKER id it saw, or say NONE. The followup proves whether
the plugin prompt reached the model's context. The driver's Esc timestamps and the mod's
timestamps use the same host clock.

## Results

Times are ms from the Esc keypress. "resolve @" is when the submit promise settled. Every
resolve value had exactly this shape:
`{ text: "PLUGIN-MARKER-…: reply with the word PINEAPPLE", origin: { kind: "plugin", name: "racespike" } }`.
It never had a `drop` key or a `context` key.

| Trial | Trigger | Esc? | Plugin prompt delivered? | Submit settled | Timing |
|---|---|---|---|---|---|
| t1 | in-user-turn | Esc during `sleep 20` | Yes, as its own turn; the model answered PINEAPPLE | resolve `{text, origin}` | resolve +77 ms, in the same ms as the plugin `turn.start`; the aborted user turn's `turn.complete` came 7 ms later |
| t3 | in-user-turn | Esc during sleep | Yes, PINEAPPLE | resolve `{text, origin}` | +78 ms, at plugin `turn.start`, before the aborted `turn.complete` (+90) |
| t9 | in-user-turn | Esc during sleep | Yes, PINEAPPLE; followup quoted the marker | resolve `{text, origin}` | +93 ms, at plugin `turn.start` |
| t10 | in-user-turn | Esc during sleep | Yes, PINEAPPLE; followup quoted the marker | resolve `{text, origin}` | +99 ms, at plugin `turn.start` |
| t5 | in-user-turn | Esc 0.3 s after Enter, before any response | Yes, PINEAPPLE | resolve `{text, origin}` | user turn aborted +64; resolve +78, at plugin `turn.start` |
| t15 | in-user-turn | Esc before any response | Yes, PINEAPPLE; the model quoted the marker later | resolve `{text, origin}` | +77 ms, at plugin `turn.start` |
| t4 | in-user-turn | **No Esc (control)** | Yes, after the user turn finished (DONE, then PINEAPPLE) | resolve `{text, origin}` | 24 ms after the user turn's `turn.complete`, at plugin `turn.start` |
| t2 | in-user-turn | Esc arrived too late, after both turns had finished | Yes; this is effectively a second control | resolve `{text, origin}` | 22 ms after the user turn's `turn.complete` |
| t8 | in-user-turn | Esc twice, 0.5 s apart | Entered, then the second Esc aborted the plugin's own turn; prompt stayed in the transcript, prompt box empty | resolve `{text, origin}` | +79 ms (first Esc); plugin turn aborted +60 ms after the second Esc |
| t14 | in-user-turn | Esc twice | Entered, its own turn aborted by the second Esc; **followup quoted the marker** | resolve `{text, origin}` | +73 ms (first Esc) |
| t7 | in-user-turn, short user turn (no tool) | Esc 13 ms **after the user turn had ended**, before the plugin turn began | Entered (plugin `turn.start` +3 ms after Esc), then this Esc aborted the plugin turn at +69 ms; prompt stayed in the transcript | resolve `{text, origin}` | +4 ms, at plugin `turn.start` |
| t13 | same as t7 | Esc 7 ms after the user turn ended, before the plugin turn began | Entered (+12 ms), plugin turn aborted at +63 ms; **followup quoted the marker** | resolve `{text, origin}` | +12 ms, at plugin `turn.start` |
| t16 | same-instant-session$ | Esc during sleep | Yes, PINEAPPLE; followup quoted the marker | resolve `{text, origin}` | +84 ms, at plugin `turn.start` |
| t18 | same-instant-session$ | Esc during sleep | **No** (followup: NONE) | **reject**: `prompt.submit: called from a prompt.submit hook, it would wait on the turn this hook is holding …` | rejected 22 ms after the call, before Esc |
| t17 | same-instant-after1 | Esc during sleep | Yes, PINEAPPLE; followup quoted the marker | resolve `{text, origin}` | +96 ms, at plugin `turn.start` |
| t19 | same-instant-after1 | Esc during sleep | Yes, PINEAPPLE; followup quoted the marker | resolve `{text, origin}` | +90 ms, at plugin `turn.start` |
| t11, t12 | same-instant from the plugin's own `prompt.submit` hook (negative control) | Esc during sleep | **No** (followup: NONE) | **reject**, same message | rejected about 20 ms after the call |
| t6 | — | — | Invalid: a bug in the spike mod meant submit was never called; fixed for t11 and later | — | — |

### Observations

- **A queued plugin prompt is never discarded by Esc.** The Esc trials were t1, t3, t5, t9, t10,
  t15, t16, t17 and t19, with Esc during the tool call or before the first response. In all nine,
  the plugin prompt started its own turn within about 75 to 100 ms of the Esc. Its `turn.start`
  fired before the aborted user turn's `turn.complete`, as in spike row 10. The model answered
  PINEAPPLE every time.
- **Esc right at the boundary.** In t7 and t13, Esc arrived after the user turn ended but before the
  plugin turn began. The plugin prompt still entered the conversation, and the same keypress then
  aborted the new plugin turn. The prompt box stayed empty: unlike a user prompt interrupted before
  its first response, a plugin prompt is not put back into the box. The rendered prompt stays in the
  transcript, and the next user turn's model quoted the marker (t13). A second Esc gives the same
  result (t8, t14). In these cases the message is in the model's context, but the model did not
  respond in the plugin's own turn. This is the same as a user prompt the user interrupted.
- **The submit promise settles only when the prompt enters.** It resolves in the same millisecond as
  the plugin turn's `turn.start` (and `prompt.submit.out` with `entered: true`). It never resolves at
  call time or while queued. The resolve value is the `PromptSubmitResult` success arm,
  `{ text, origin }`.
- **The only non-delivery seen was a rejection.** It happened only when the submit was attributed to
  an in-flight `prompt.submit` hook dispatch (t11, t12, t18). The engine's reason was "it would wait
  on the turn this hook is holding". That attribution was not deterministic: through the
  session-start closure, t16 was accepted and t18 rejected. The production mod maps a rejection
  to `{ drop: 'error:…' }`, refuses the batch and does not ack it. The production mod registers no
  `prompt.submit` hook, so this path should not occur for it in any case.
- No trial saw a submit that stayed pending, resolved with `drop`, or resolved without the prompt
  reaching the conversation.

## Raw log excerpts

t3, Esc during the user turn: the queued plugin prompt runs at once.

```
+2446  turn.start     75ce0f46… "SLEEPTEST: run the bash command `sleep 20` …"
+2446  submit.call    PLUGIN-MARKER-15191-1 (in-user-turn)
+6279  MARK esc
+6354  prompt.submit.in  origin {kind: plugin, name: racespike}
+6356  turn.start     c97f3e5e… "The racespike plugin sent a message:\nPLUGIN-MARKER-15191-1 …"
+6357  submit.resolve {"text":"PLUGIN-MARKER-15191-1: reply with the word PINEAPPLE","origin":{"kind":"plugin","name":"racespike"}}
+6369  turn.complete  75ce0f46… reason aborted, isAborted true
+8642  turn.complete  c97f3e5e… reason answer  "PINEAPPLE …"
```

t13, Esc after the user turn ended, before the plugin turn started:

```
+4003  turn.complete  909eb145… (user) reason answer "OK"
+4010  MARK esc
+4020  prompt.submit.in  origin plugin
+4022  turn.start     547b781d… plugin prompt
+4022  submit.resolve {"text":"PLUGIN-MARKER-41294-1: …","origin":{"kind":"plugin","name":"racespike"}}
+4073  turn.complete  547b781d… reason aborted, isAborted true
+54220 turn.complete  (FOLLOWUP) answer "PLUGIN-MARKER-41294-1"
```

t14 pane after a double Esc and the followup:

```
❯ SLEEPTEST: run the bash command sleep 20 and then reply with the single word DONE.
  Ran 1 shell command
  ⎿  Interrupted · What should Claude do instead?
› Prompt from the racespike plugin
❯ The racespike plugin sent a message:
  PLUGIN-MARKER-2102-1: reply with the word PINEAPPLE
  ⎿  Interrupted · What should Claude do instead?
❯ FOLLOWUP: without using any tools, quote the PLUGIN-MARKER id …
⏺ PLUGIN-MARKER-2102-1
```

t18, a submit attributed to the in-flight `prompt.submit` dispatch, which rejected:

```
+2747  prompt.submit.in  RACESESS … origin {kind: composer}
+2747  submit.call    PLUGIN-MARKER-16713-1 (same-instant-session$)
+2769  submit.reject  "racespike: prompt.submit: called from a prompt.submit hook, it would wait on the turn this hook is holding; answer { text } or next(e) instead, or submit from a later event (turn.complete…"
+58941 FOLLOWUP answer "NONE. …"
```

## Conclusion

**SAFE** on Claude Code 2.1.295.

- A plugin prompt queued behind a user turn is never discarded when the user presses Esc. It is
  delivered as its own turn right after the interrupt, about 75 to 100 ms later, before the aborted
  turn's `turn.complete`.
- If the Esc lands at the boundary, or a second Esc follows, the plugin turn itself is aborted. The
  prompt still stays in the conversation, and the next turn's model reads it.
- `$.prompt.submit` resolves only when the prompt enters, in the same millisecond as its own
  `turn.start`, with `{ text, origin: { kind: 'plugin', name } }` (no `drop`).
- The one non-delivery seen was a rejection (`prompt.submit: called from a prompt.submit hook …`),
  which the mod already treats as a drop and does not ack.
- Not one of the 18 valid trials had a prompt that was discarded but still resolved as a success.

Residual nuance, not a safety issue: an Esc at the boundary can abort the plugin's own turn before
the model answers it, so the message is in context but unanswered until the next turn. This
matches the spec's accepted interrupt behavior, and the post-abort hold covers later batches.

Not covered: a UserPromptSubmit settings hook that blocks the prompt (the `drop` arm; the stress
scenario `ups_block_drop` covers it), the desktop app, VS Code, and `claude -p`.

## Artifacts

The scratch directory
the spike's local scratch directory (not committed)
holds `mod/` (the spike mod), `drive.py`, and `out/` (per-trial `tN.jsonl`, `tN.pane.txt`,
`tN.marks.json`, plus batch output in `run2.txt`, `run3.txt` and `run4.txt`; t1 was printed to the console only).
