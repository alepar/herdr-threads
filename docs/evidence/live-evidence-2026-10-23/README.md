# Live evidence where we had none (ht-live-evidence, 2026-10-23)

Idea 20 of `docs/research/2026-10-23-integration-alternatives/README.md`: live, recorded evidence for Hermes 0.22.x,
a real-model agy case in the wake e2e, and three Claude mod behaviours. Host blacky (Linux 7.0.12 x86_64), Herdr
0.9.5, herdr-threads 2.3.1 built from `integration/ht-live-evidence` (main `afdb1a605` plus the driver changes in
this branch). Every agent ran in panes or private tmux servers this task created, against private herdr-threads
state; nothing was written to a real harness config.

## Summary

| Question | Result | Evidence |
|---|---|---|
| Hermes 0.22.5: setup, enable, recognition | setup and enable work with Hermes' official launcher only; Herdr recognizes a bare Hermes on Linux (`agent: hermes`) | [hermes/](hermes/) |
| Hermes 0.22.5: identity (research F2, open question 6) | **broken**: the auto-spawned gateway keeps the first pane's `HERDR_PANE_ID`; both panes' sessions bind to that pane's seat | `hermes/gateway-env-herdr.txt`, `hermes/seat-inspect-*.txt` |
| Hermes 0.22.5: delivery | `pre_llm_call` context reaches the model on a classic CLI turn through the gateway, but **into the wrong pane's session** | `hermes/pane-b-wM-p3.txt` |
| Hermes 0.22.5: idle wake (research F1) | **never fires**: a p0 `--nudge` to an idle seat records `wake_work.last_outcome = 'unsafe'`, nothing typed | `hermes/wake-work.txt`, `hermes/delivery-inspect-p0.txt` |
| agy real-model wake e2e | option added (`--harness agy`, opt-in); **not run**: no agy and no Gemini credential on this host | [agy.md](agy.md) |
| Claude: does a failing Bash result set `isError`? | **yes**, identical to a user reject at the permission dialog | [claude/iserror-ht-mod-busy/](claude/iserror-ht-mod-busy/) |
| Claude: effect on our mod | an ordinary failing last tool holds the next waking item **119.3 s** (`post_abort`) | [claude/failing-tool-hold/](claude/failing-tool-hold/) |
| Claude: `prompt.submit` against a typed draft, 120 s expiry | see [Claude draft hold](#claude-draft-hold) | [claude/draft-hold/](claude/draft-hold/) |

## Defects (contradictions with our code or docs; not fixed here, for routing)

1. **Hermes ≥ 0.22.1: every Hermes session binds to the seat of the pane that started the gateway** (high). Two bare
   `hermes` panes, `wM:p2` (started first) and `wM:p3`. The gateway process (`hermes_cli.main --profile default
   gateway run`) inherited `HERDR_PANE_ID=wM:p2`, `HERDR_TAB_ID=wM:t2`. After one turn in each pane, seat
   `syHlbs78cWym` (target `wM:p2`) holds two Hermes bindings: native session `local-9a590d67…` (pane A, ordinal 2,
   now disconnected) and `local-31787204…` (pane B, ordinal 3, connected); pane `wM:p3` has no seat. A p1 sent to
   pane A's seat (`myHlHi4sYBhV`) was put into **pane B's** model context on its next turn (the model printed
   `RECEIVED myHlHi4sYBhV …`). This is research flaw F2, now observed: `integrations/hermes/__init__.py` passes no
   `env=` to the hook child and `src/cli/hook.rs` reads `HERDR_PANE_ID` from the process env. It is a misdelivery
   and a seat binding taken on the wrong pane's evidence, so it touches TRUST-POLICY (A2, never bind on heuristic
   evidence).
2. **Hermes idle wake never fires; the docs still call a Hermes seat wakeable** (high). `send --seat syHlbs78cWym
   --nudge` to the idle seat: neither pane left `idle` in 40 s, `delivery inspect` shows `attempted: null`, and the
   daemon's `wake_work` row for the seat ends `last_outcome = 'unsafe'` (no Hermes `ComposerPolicy`, so the composer
   read is never `Empty`). Research flaw F1; `integrations/hermes/README.md` ("the way to get a wakeable Hermes").
   Also relevant to any future composer policy: Hermes 0.22.5 draws a dim placeholder in the empty composer
   (`❯ What changed in this repo recently?`), and Herdr reports the pane `idle` while the first-run telemetry
   consent modal is up.
3. **Claude mod: an ordinary failing tool arms the post-abort hold** (medium). Claude Code reports a failing Bash
   (`false`, `exit 3`), a failed Read and a PreToolUse deny as `{isError: true}` with no `deny` field, exactly like an
   Esc at the permission dialog (ht-mod-busy's probe, 2.1.309). `userRejected()` (`register.js:121`) therefore treats
   a turn whose last tool failed as a user reject: live on 2.1.310, a require-ACK message sent right after such a
   turn was held `post_abort` and delivered 119.3 s later by the `HOLD_IDLE_MS` expiry. Research flaw F6. The probe
   shows a discriminator: a failing tool's result is followed by a main `turn.step` about 23 ms later; a user
   reject is followed by `turn.complete` (`isAborted: false`, `reason: "answer"`) with no `turn.step` between.
4. **`setup hermes` fails on any install without Hermes' published launcher, with an opaque error** (low).
   `herdr-threads setup hermes` probes `hermes --print-runtime-command --module trace …`, which only the launchers
   that `setup-hermes.sh` / the official installer publish implement. With a plain environment install (here `uv
   sync` of the v0.22.5 tag) setup refuses `Hermes profile inspection unavailable`: `inspect_profile`
   (`src/harness/hermes.rs:1384`) drops the underlying error (`hermes: 'trace' is not a hermes command`). Reinstalling
   through `setup-hermes.sh` fixed it. Not a version gate, but the message should name the missing launcher
   capability.

Overlap: ht-mod-busy owns the busy-path consequence of the `isError` shape (a waking item is attached only to a
non-error main result; context on an `isError` result does reach the model, 5/5 in their probe). The fix for
defect 3 belongs with whoever owns `register.js` holds.

## Hermes 0.22.5

Install (scratch only): `git clone --depth 1 --branch v0.22.5 https://github.com/NousResearch/hermes-agent`
(`bf0203a794a0`, 2026-10-22), then `HOME=<scratch>/home bash setup-hermes.sh` (Hermes' own pm-managed install into
the scratch HOME: tools under `~/.hermes/tools`, launchers in `~/.local/bin`). Model: `openai-codex` provider,
`gpt-6.1-sol`, with credentials imported from a mode-0600 copy of the Codex `auth.json` placed in the scratch HOME
(Hermes' own "import Codex CLI credentials" path). The copy was never refreshed, the source was unchanged, and the
copy and Hermes' auth store were deleted after the run. A first `uv sync` install (non-official entry point) is
what exposed defect 4.

Run, all against a private daemon (`--state-dir <scratch>/ht-state`, discovery off) and one owned Herdr workspace
`wM` on the shared server (closed afterwards):

1. `herdr-threads setup hermes --profile default` with `HOME=<scratch>/home` (`setup-hermes.out`): installed,
   `bridge_current: true`. `hermes --profile default plugins enable herdr-threads` (`plugins-enable.out`):
   "Gateway reloaded plugins — active in the running gateway now: hooks."
2. Coordinator: a person seat in `wM:p1` (`human me init`).
3. `wM:p2`: `PATH=<scratch>/bin:$PATH HOME=<scratch>/home hermes` (bare classic CLI). It auto-spawned the gateway
   (`gateway run --quiet --idle-exit`), and showed the telemetry consent modal first (answered "No thanks").
   `herdr agent get` reported `agent: hermes, idle`. `wM:p3`: the same command; it attached to the same gateway.
4. One neutral turn in each pane (`Reply with the single word ALPHA` / `BRAVO`), then `seat list`: one Hermes seat,
   target `wM:p2`, two bindings (defect 1).
5. `send --seat syHlbs78cWym` (p1, `myHlHi4sYBhV`), then a turn in `wM:p3` asking whether its context held a
   herdr-threads message: `RECEIVED myHlHi4sYBhV HERMES-DELIVERY-1: …` (delivery works; wrong pane).
6. `send --seat syHlbs78cWym --nudge` (p0, `myHlbvtmOs74`) to the idle seat; both panes polled each second for
   40 s: no status change, no typed text (defect 2).

Files: `versions.txt`, `setup-hermes.out`, `plugins-enable.out`, `gateway-env-herdr.txt`, `seat-list.txt`,
`seat-inspect-syHlbs78cWym.txt`, `delivery-inspect-p1.txt`, `delivery-inspect-p0.txt`, `wake-work.txt`,
`pane-a-wM-p2.txt`, `pane-b-wM-p3.txt` (scratch paths shown as `<scratch>`, the home as `<home>`).

Answers to research open questions: 6 (an auto-spawned gateway keeps the first pane's `HERDR_PANE_ID`): **yes**.
2, first half (`platform` on a gateway-hosted classic CLI turn): the bridge's platform filter admitted the turn (the
context arrived), so the classic CLI reports `cli` or empty; the Ink TUI (`hermes --tui`) was not run. Not
exercised: TUI, delegated children, `/new` reset, relaunch, `herdr-threads launch --kind hermes`.

## agy

See [agy.md](agy.md).

## Claude draft hold

`python3 tests/native/claude_mod/stress.py --profile <signed-in claude-1 profile, in place> --scenarios draft_hold
--iterations 1 --bin bin/herdr-threads` (Claude Code 2.1.310, Opus 5.5; files in `claude/draft-hold/`). The
session was idle with no open turn (hook log); the driver typed `DRAFTKEEP-0-a7a74d please do not send` into the
composer without Enter and sent a require-ACK message:

| ms after send | Ledger / hook log |
|---|---|
| +46 | `received`, `held` reason `draft` (`$.prompt.read()` returned the typed text) |
| +120,610 | `delivered` via `append`, then `submit` `wake up` (`DRAFT_WAIT_MS` expired) |
| +120,663 | `acked` `settled` |

- **The 120 s expiry works as coded**: one hold, then the wake anyway (`register.js:541-546`).
- **`$.prompt.submit` does not touch a typed draft**: the plugin's turn ran with the prompt `wake up` alone (the
  `UserPromptSubmit` hook saw exactly that), the model answered it, and afterwards the composer still held the
  draft, unchanged and unsubmitted (`pane-…-typed.txt`, `-held.txt`, `-after.txt`). The research's INFERRED claim
  "the draft text survives" is now observed. A person still mid-sentence after 120 s sees a plugin turn run above
  their draft; nothing is lost.
- Not covered: the person pressing Enter while the plugin turn runs (the earlier queued-prompt spike,
  `docs/evidence/claude-mod-delivery/submit-race-spike.md`, covers the reverse order), and prompt suggestions: setup
  ran with `--keep-prompt-suggestions`, but no suggestion was on screen when the draft was typed, so whether
  `$.prompt.read()` returns a dim suggestion as text was not observed.

## Claude `isError` (ht-mod-busy's probe, reused)

Captured by the ht-mod-busy tab on Claude Code 2.1.309 with a scratch logging mod (`claude/iserror-ht-mod-busy/mod/`,
logs `log-run{1,2,3}.jsonl`, panes, the PreToolUse deny `settings.json`); they shared it with this task at the
coordinator's request, and it is copied here rather than measured twice. Ordinary failing Bash, a failed Read and a hook deny all reach the mod's
`tool.call` as `{ref, result, text, isError: true}` with no `deny`; `result` is the error text (`"Error: Exit code
1"`). An Esc at the permission dialog gives the same shape (`"Error: The user doesn't want to proceed with this tool
use..."`), then `turn.complete` 29 ms later with `isAborted: false`, `reason: "answer"`. Esc during generation gives
`turn.complete` `isAborted: true`, `reason: "aborted"`.

## Claude failing-tool hold

`python3 tests/native/claude_mod/stress.py --profile <signed-in claude-1 profile, in place> --scenarios
draft_hold,failing_tool_hold --iterations 1 --bin bin/herdr-threads` (Claude Code 2.1.310, Opus 5.5, stand-in Herdr
endpoint, private daemon; files in `claude/failing-tool-hold/`). `failing_tool_hold`: the model ran `false` once and
replied `done` (`Stop` at …314861); a require-ACK message sent 1.5 s later was `held` `post_abort` at …316434 and
delivered by append plus `wake up` at …435643, **119.3 s** after the send (defect 3). That run's `draft_hold`
iteration raced the session's start-up invitation wake turn, so its draft was typed into a busy session; it is kept
as an observation (the message rode a tool-call result as `context` in 2.3 s; the typed text stayed in the composer
and was not submitted) and the scenario now waits for no open turn before typing (re-run below).

Claude Code 2.1.310 frames a plugin-submitted prompt for the model with its own line after the text: "This is how
Claude Code surfaces a prompt a plugin submits between turns — it starts this turn in the user's place. Address the
message above." (harness text, not ours; it costs every mod wake about 40 tokens).
