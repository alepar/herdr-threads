# Poke spike: composer stash, poke during a turn, worker spawn (ht-1ip.12)

Native spike for spec §10 and §12 of the [thread-summaries design](../../design/herdr-threads/thread-summaries/2026-10-02-thread-summaries-compaction-survival-design.md).
It decides whether the recipe capabilities `composer_stash` and `poke_during_turn` may be declared, and records how each harness spawns summary workers.

Versions: herdr 0.9.1, claude 2.1.287 (haiku-4-5), codex 0.160.0.

## Codex runs used a mock provider

The Codex account was out of quota: gpt-5-mini is rejected for ChatGPT accounts, and gpt-5.6-luna was over its usage limit until 2026-10-04. For Q5 and Q6, Codex therefore ran the real 0.160.0 client against a local mock Responses provider (`scripts/mock_responses.py`). Those results show client-side queue and steer behaviour only.

## Verdicts

| Harness | composer_stash | poke_during_turn | worker spawn |
|---|---|---|---|
| claude 2.1.287 | yes | yes, steered into the running turn at the next tool boundary (not a separate user turn) | yes: Agent tool with model "haiku", parallel |
| codex 0.160.0 | yes | yes, same behaviour (mock provider) | yes: `multi_agent_v1.spawn_agent` with `model`; real overlap unmeasured |

## Findings, with captures

**Q1. Is there a native "human input" status?** No. `agent_status` stays idle on both harnesses while a draft is typed.
- On Claude, `agent explain` evidence carries the composer text: compare `claude-q1-q2-single.explain.txt` with `claude-q1-empty.explain.txt`.
- On Codex it does not (`codex-q-cycle-1-draft.explain.txt`).
- The daemon must therefore read the composer itself.

**Q2. Can the composer be read?**

`agent read --source detection` returns the composer rows on both harnesses:

| Harness | Composer rows | Empty composer |
|---|---|---|
| Claude | between two `─` rules: `❯ `, then a 2-space indent | `❯`, or the placeholder `Try "how do I log an error?"` |
| Codex | `› `, then a 2-space indent until a blank row and the footer | `› Ask Codex to do anything` |

- Both a real newline in `pane send-text` and `shift+enter` produce multi-line text without submitting (`*-q2-multiline-*`).
- A soft wrap cannot be told apart from a hard newline (`*-q2-wrap`).
- An image pasted with ctrl+v shows as the placeholder `[Image #1]` (`*-q2-image-placeholder`). Codex puts it before the typed text.

**Q3. Can it be cleared?** Yes: send `ctrl+u` repeatedly until a read shows the composer empty.
- Claude clears per visual row; Codex clears per logical line, plus one per newline.
- `ctrl+a ctrl+k` does not clear on Claude.

**Q4. Can it be restored?** Yes. `send-text` with real newlines restores the text exactly and does not submit (compare `*-q-cycle-1-draft` with `*-q-cycle-2-restored`).
- A retyped `[Image #1]` comes back as literal text only (`claude-q4-image-retyped.transcript.txt`, `codex-q4-image-retyped.rollout-events.txt`), so stashing is unsafe when an image placeholder is present.
- On Codex, `send-text` followed immediately by `enter` is swallowed. A 2 s pause or a second Enter works.

**Q5. What happens to input sent during an active turn?** `agent prompt`, and `send-text` plus Enter, both queue the input and inject it into the *current* turn at the next tool boundary.
- Claude records a `queue-operation` and a `queued_command` attachment, no user record, and the model says "Also received".
- Codex adds a user message inside the same task, and the UI shows "Messages to be submitted after next tool call".
- If the composer already holds text, the two merge as `DRAFT-TEXT-xyzPOKE-TEST-...` with no separator, on both harnesses (`*-q5-composer-text-then-poke*`, `codex-q5-mock-requests-composer-merge.jsonl`).
- Further captures: `claude-q5-agentprompt.transcript.txt`, `claude-q5-sendtext.transcript.txt`, `codex-q5-mock-requests-*.jsonl`, `codex-session-events-mock.txt`.

**Q6. How are workers spawned?**
- **Claude:** the `Agent` tool with `{description, prompt, model: "haiku"}`. Two async calls in one message overlapped, and both ran haiku-4-5 (`claude-q6-workers.transcript.txt`).
- **Codex:** the `multi_agent_v1` namespace (`codex-0.160.0-tool-schemas-exec_command-multi_agent_v1.json`, `codex-q6-mock-requests-workers.jsonl`).
  - Tools: `spawn_agent{message, items, model, reasoning_effort, fork_context}`, `wait_agent`, `send_input`, `close_agent`, `resume_agent`.
  - Model overrides listed: gpt-6.1-sol, gpt-6-astra, gpt-6-sol, gpt-6-luna, gpt-5.6-sol.
  - Codex spawns only when the user explicitly asks.

## Recommendations for ht-1ip.14

Declare both capabilities with these rules:
- **Read** the composer with `agent read --source detection`.
- **Clear** it with a `ctrl+u` loop until a read shows it empty.
- **Retype** with `send-text` using real newlines, and never Enter.
- **Skip the stash** when `[Image #` or `[Pasted text` is present, or when any composer row is within 12 columns of the pane width (a soft wrap cannot be told from a newline).
- **poke_during_turn** steers into the current turn and merges with any composer text, so HumanInput during an ActiveTurn must stash first.
- **On Codex**, use `agent prompt`, not `send-text` plus Enter.

## Side effects of the spike

- The scratch tab and panes were closed, the mock was stopped, and the scratch dirs were removed.
- Global state left behind:
  - one inert project entry for the scratch dir in `~/.claude.json`, from accepting the trust prompt;
  - the clipboard, overwritten with the test PNG;
  - harness session history under `~/.claude/projects/-private-tmp-ht-poke-spike-claude/` and `~/.codex/sessions/2026/10/02/`.

This note was written by the coordinating session from the spike agent's report (`.superpowers/sdd/ht-1ip-plan/task-3-report.md`), because the harness refused the spike subagent's own write of this file.
