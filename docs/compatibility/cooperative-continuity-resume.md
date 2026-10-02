# Cooperative continuity: what a resumed session looks like (ht-rzi.2)

Evidence behind TRUST-POLICY C1: a top-level `SessionStart` with source `resume` whose harness session id uniquely matches an unresolved seat's last binding reattaches that seat. The gate depends on three facts, each recorded below with its source and what was and was not observed:

1. A resume delivers the **original** session id (the id the seat's last binding recorded).
2. A resume delivers **one** `SessionStart` event, with no `startup` event carrying a new id before or after it (Claude Code issue 24265 reported startup(new id) followed by resume(original id)).
3. `/clear` (Claude) and `/new` (Codex) deliver a **new** id under another source, so they can never match.

Nothing here weakens the policy: the match is cooperative and structural, and Herdr's `agent get` output is recorded as a diagnostic only.

## Environment of the 2026-10-01 capture

| Tool | Version |
| --- | --- |
| Claude Code | 2.1.287 |
| Codex CLI | codex-cli 0.159.3 (installed; not run, see below) |
| Herdr | 0.9.1 |

`~/.claude`, `~/.codex` and the shared Herdr server were not edited. The capture used a throwaway hook script passed through `claude --settings <scratch>/settings.json` only; the scratch directory sat under the session scratchpad. Claude Code itself still writes its own session transcript under `~/.claude/projects/` for any run (that is the harness's normal state, not an edit of its configuration).

## Capture 1 (this task): Claude Code 2.1.287, print mode

Hook (capture only, appends the SessionStart stdin and a timestamp to a scratch file and exits 0):

```text
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"<scratch>/cap.sh"}]}]}}
```

Commands, run from a scratch working directory:

```text
claude -p "say ok"  --settings <scratch>/settings.json --output-format json
claude -p "say ok2" --resume <orig> --settings <scratch>/settings.json --output-format json
```

Hook stdin, session ids replaced by tokens and paths redacted:

| Run | Events delivered | Payload |
| --- | --- | --- |
| first launch | 1 | `{"session_id":"<orig>","transcript_path":"~/.claude/projects/<proj>/<orig>.jsonl","cwd":"<scratch>","hook_event_name":"SessionStart","source":"startup"}` |
| `--resume <orig>` | **1** | `{"session_id":"<orig>","transcript_path":"~/.claude/projects/<proj>/<orig>.jsonl","cwd":"<scratch>","hook_event_name":"SessionStart","source":"resume","seconds_since_last_response":12,"context_tokens":25235,"prompt_cache_likely_expired":false,"estimated_cache_write_usd":0.2019}` |

The resumed run's own result reported `session_id` `<orig>` as well. SessionStart events per resume: **1**, source `resume`, id equal to the original. No `startup` event (and no second id) appeared, before or after it.

Limits of this capture: print mode only. The pre-existing interactive evidence below covers the interactive launcher.

## Capture 2 (bead ht-zp0): interactive Claude 2.1.287 and Codex 0.159.3/0.160.0 in a Herdr pane

Taken 2026-10-01 in a scratch pane of the shared Herdr 0.9.1 server (pane split from the orchestrating tab, closed afterwards; the server itself was not restarted). Claude ran with the same capture hook passed through `claude --settings <scratch>/settings.json` (SessionStart stdin appended to a scratch file). No harness configuration was edited. `herdr agent get` was read from outside after each action.

| Action | SessionStart events | `source` | `session_id` | Herdr `agent_session` after the action |
| --- | --- | --- | --- | --- |
| Claude: fresh interactive launch | 1 | `startup` | `<orig>` | `<orig>` |
| Claude: `/exit`, then `claude --resume <orig>` | **1** | `resume` | `<orig>` | `<orig>` |
| Claude: `/exit`, then `claude --continue` | **1** | `resume` | `<orig>` | `<orig>` |
| Claude: `/clear` | 1 | `clear` | `<new>` | `<new>` (updated promptly) |
| Codex 0.159.3: fresh interactive launch | (no capture hook) | — | `<c-orig>` (rollout `rollout-…-<c-orig>.jsonl`) | absent until the first turn completed, then `<c-orig>` |
| Codex: `/quit`, then `codex resume <c-orig>` (CLI self-updated to 0.160.0 on this launch) | (no capture hook) | — | `<c-orig>` (same rollout file, one `session_meta`) | `<c-orig>` immediately after the resumed launch, before any turn |

Findings:
- Claude 2.1.287 interactive: one `resume` event per resume or continue, with the original id; no `startup` + `resume` pair (Claude Code issue 24265 not reproduced).
- Codex: resume keeps the original session id. Herdr's integration reported it right after the resumed launch. On a fresh launch Herdr's report appeared only after the first turn, so a fresh Codex pane can show no `agent_session` for a while (explains the earlier "Codex panes show none" observation).
- The Codex SessionStart payload itself (event count and `source` on resume) was not captured interactively: Codex hooks come only from `CODEX_HOME`, which was not edited, and the account hit its weekly usage limit before a resumed turn could run. The 0.158.0 `codex exec resume` capture above remains the evidence for the payload.

## Existing captures reused (not repeated here)

- Claude Code 2.1.283, interactive, private Herdr server and pane (`docs/compatibility/claude-lifecycle-probe.md`, attempt 2):

  | Native action | Source | Session |
  | --- | --- | --- |
  | fresh launch | `startup` | `<orig>` |
  | `/clear` | `clear` | `<new>` (a new id) |
  | fresh process `--resume` of the original | `resume` | `<orig>` (the original id) |

  One event per action. The resume event carries the original id and was the only SessionStart of that process.
- Claude Code 2.1.286 resume payload shape: `tests/fixtures/claude-2.1.286/03-sessionstart-resume.json` (same keys as capture 1).
- Codex 0.158.0 live capture, `codex exec` then `codex exec resume <id>` (`docs/evidence/codex-158-live-hook-capture/`, fixtures `tests/fixtures/codex-0.158.0-live/run2.session-start.startup.json` and `run3.session-start.resume.json`): startup delivers one event; the resume delivers **one** event with `source="resume"` and the **same** `session_id` as the original. Keys: `session_id, transcript_path, cwd, hook_event_name, model, permission_mode, source`.

## Not captured by this task

| Item | Why |
| --- | --- |
| Interactive `claude --resume` and `/clear` on 2.1.287 | Captured in capture 2. |
| Codex `codex resume` and `/new` on 0.159.3 | Codex needs the user's credentials; a temporary `CODEX_HOME` would have required copying them, and running against `~/.codex` would write sessions there. Nothing was faked: the 0.158.0 live capture above is the evidence. Count on 0.159.3 not measured. |
| `herdr agent get <pane>` from inside a hook during a resume, for either harness (capture 2 read it from outside, after each action) | Needs a scratch pane in a private named Herdr session. The shared Herdr server was not touched and no private session was started. The hook running here would have reported this orchestrating pane, not the scratch harness. |

What is known about Herdr session reporting (bead ht-rzi.2 comment, from the integration scripts embedded in Herdr 0.9.1, observed 2026-10-01): claude, codex and other integrations call `pane.report_agent_session` with `agent_session_id`, usually with `session_start_source` (startup/resume/new/clear). Claude panes show `agent_session` in `herdr agent list`; three live Codex panes showed none although the Herdr state hook was installed (cause undetermined). The relation between the reported value and the SessionStart `session_id` on a resume is therefore **not established** here. That is why the daemon records the comparison (`match`, `mismatch`, `absent`, `read_error`) per reattachment in `allocation_decisions.continuity_diagnostic`, shows it in `seat inspect`, and never lets it decide.

## Dedupe rule: none

A dedupe rule (a `startup` check-in on a seat reattached moments earlier must not overwrite its `native_session`) is only needed if a resume is followed or preceded by a `startup` carrying a new id. Every capture above shows a single `resume` event with the original id, so **no dedupe rule was added**. If a later Claude or Codex version is observed to deliver startup(new id) + resume(orig id), the consequence is bounded: the resume event reattaches (the match uses the original id), and a later `startup` for the same pane is the ordinary lifecycle check-in of the seat, which replaces its binding (TRUST-POLICY A4); revisit then.

## What C1 relies on, per harness

| | Claude | Codex |
| --- | --- | --- |
| Resume id equals the original | observed (2.1.283 interactive, 2.1.287 print and interactive) | observed (0.158.0 `codex exec resume`; 0.159.3→0.160.0 interactive `codex resume`) |
| One SessionStart per resume | observed | observed |
| `/clear` or `/new` gives a new id | `/clear` observed (2.1.283) | not captured; the gate excludes `clear` and `compact` sources regardless |
| Launch form | `claude --resume` | `codex resume` by hand reattaches; the managed launch form stays refused by ht-rzi.4 |

Neither harness is excluded from C1. The unmeasured rows above are limits of this evidence, not evidence of a different behavior.
