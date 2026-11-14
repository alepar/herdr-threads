# PR roast 1: escalation dispositions

Date 2026-11-14. Escalations E1-E4 of [the roast report](2026-11-14-opencode-harness-roast-pr-1.md), judged against opencode `anomalyco/opencode` at the commit the research read (`v1.18.1`, `99f638d8293f6985726ba509da602296c4963497`; paths relative to `packages/opencode/src/`). Evidence only; nothing is pinned to that version.

## E1. The wake-pending mark cleared by a step that did not read the append: real, fixed

`session/prompt.ts` `runLoop`, per step:

1. line 1092 reads the message list (`filterCompactedEffect`);
2. line 1096 takes `lastUser` = the user message with the greatest id in that list (`MessageV2.latest`, `session/message-v2.ts:585`);
3. lines 1141-1184 resolve the model and agent and apply reminders;
4. line 1187 creates the step's assistant message, `id: MessageID.ascending()`, `parentID: lastUser.id`, and line 1201 stores it (`message.updated`);
5. line 1262 builds the model context from the list read in step 1.

An append stored between 1 and 4 gets an id below the assistant's, but is not in that step's context. The loop then exits once the step ends on its text, because its last assistant is newer than every user message (line 1115, `lastUser.id < lastAssistant.id`). The plugin's old rule (any assistant id greater than the append's) cleared the mark in exactly that case, so the waking item got no turn.

Fix: the mark is cleared only by an assistant message whose `parentID` is the append's id or a later one, which means the step's list held the append. A step whose list did not hold it leaves the mark, and the wake is sent when the session goes idle. Residual: a user message created after the append but stored before it (two concurrent prompts) would clear the mark too; the plugin serializes its own appends, so only a concurrent TUI or HTTP prompt could do this. Test: `E1: a step created after the append from a message list read before it still owes the wake`.

## E2. `refreshStatus()` overwriting the local busy mark: real, fixed

`prompt()` (`session/prompt.ts:1052`) runs `createUserMessage`, which fires `chat.message` (line 1000), and only then `loop()` → `runLoop`, whose first act is `status.set(busy)` (line 1089). So while the plugin's own `chat.message` hook runs (up to 8 s), `session.status()` does not list the session yet, and the post-append status read set it to idle. A waking append in that window then got a wake into a session about to start the user's turn (rule 4).

Fix: a top-level user message marks its session as starting a turn until that session's next status event, or 18 s (the hook bound plus the settle time). While the mark holds, a status read that says idle does not overwrite the local busy state. Test: `E2: a user message whose turn has not started keeps the session busy against a status read`.

## E3. No timeout on `client.session.*` calls: real hazard, fixed

The plugin's client calls the in-process server (`plugin/index.ts:140-146`, `Server.Default().app.fetch` with no port), and the SDK sets no timeout. Whether a call stalls was not observed, but nothing bounds it, and a stalled append or wake left `appendInflight` or `wakeInflight` set for the process's life. A stalled `session.get` blocked the user's turn in `chat.message`.

Fix: every SDK call the plugin awaits is bounded: `prompt` (append) and `promptAsync` (wake) at 15 s, `status` and `get` at 5 s. A timeout takes the call's existing failure path:

- an append: no ACK, 30 s backoff, offered again;
- a wake: the wake stays pending, 30 s backoff;
- a status read: the local status stays;
- `session.get`: the role is unknown, so `chat.message` runs no hook (retried next message) and `shell.env` sets the parent to `?` (D2).

A timed-out append or wake keeps its own-message mark for 10 minutes, so a late-stored message is still recognised as the plugin's own and is not taken for a user turn. Tests: the four "hung" tests in `plugin.test.mjs`.

## E4. A failed append retried although appends are not idempotent: real, documented as an accepted limit

`prompt()` stores the message in `createUserMessage` (line 1057) and then runs `sessions.touch` (line 1058) before returning, so an error after the store is possible. A timeout (E3) is a second way to get "failed" for a stored message. No cheap check exists: the plugin would have to list the session's messages and match the framed text, and a late-landing call could still store after that check. Delivery is at-least-once, as with the Claude mod. A repeated page costs context; nothing is lost and no receipt settles twice, because a failed append claims nothing. The accepted limit is in TRUST-POLICY.md, next to the opencode limits, and in `integrations/opencode/README.md` under Limits.

## Review 2, item 1. Plugin-started messages ran with the default agent: real, fixed

Read in the same source (`v1.18.1`, `99f638d8293f6985726ba509da602296c4963497`, paths relative to `packages/opencode/src/`; the SDK under `packages/sdk/js/src/`).

- Both routes, `POST /session/{id}/message` (`client.session.prompt`) and `POST /session/{id}/prompt_async` (`client.session.promptAsync`), pass the body to `SessionPrompt.prompt` (`server/routes/instance/httpapi/handlers/session.ts:300-326`). The body is `PromptInput` without `sessionID` (`server/routes/instance/httpapi/groups/session.ts:70`), which takes `agent`, `model: {providerID, modelID}` and `variant` (`session/prompt.ts:1494-1519`).
- `createUserMessage` (`session/prompt.ts:635-654`): a body with no `agent` gets `agents.defaultInfo()`, which is `default_agent` from config or else the first visible primary agent (`agent/agent.ts:328-340`), so `build` by default. With no `model` it takes that agent's own model, else the session's current model (`currentModel`, `session/prompt.ts:614-633`). With no `variant` it takes that agent's variant, else none.
- The stored message then records that agent and model (`session/prompt.ts:656-670`). When they differ from the session's, `sessions.setAgentModel` makes them the session's current agent and model (`session/prompt.ts:672-690`), so a later message with no fields inherits them as well.
- `runLoop` takes each step's agent and model from the newest user message in its list (`lastUser`, `session/prompt.ts:1096`, used at 1141, 1170 and 1187-1192). So a `noReply` append with no fields, stored while a turn runs, makes the next step of that turn run as the default agent. A `promptAsync` wake with no fields runs the whole woken turn as the default agent, and the step's assistant message records `agent: build`.
- `chat.message` gets the request fields as input and the stored message (resolved `agent` and `model {providerID, modelID, variant}`) as `output.message` (`session/prompt.ts:998-1008`).
- The v1 SDK's body types list `agent` and `model` but not `variant` (`gen/types.gen.ts:2587-2611`). The client sends the body through `JSON.stringify` unfiltered (`gen/core/bodySerializer.gen.ts:52-55`), and the server schema accepts `variant`.

Fix: the plugin records `agent`, `model {providerID, modelID}` and `variant` from each real top-level `chat.message`. It takes them from the stored `output.message` and falls back to the input fields. The append, the compact re-entry append and the wake all send the recorded fields. The plugin's own appends and wake prompts never update the record (an append returns before recording; a wake mark skips it). The record is per session, capped at 64 sessions.

Before any real message of a session: no append and no wake for it. Any of the three would run as the default agent. This cannot happen today, because a session gets a channel only when a real top-level message checks it in, and that same message records the fields first (a launch without `--prompt` sends its content-free first prompt from the TUI, a real message carrying the TUI's agent). If it ever happened, the items would wait for the turn hook of the next real message.

Tests: `append and wake carry the agent, model and variant of the newest real user message` and `the agent record is per session` (`plugin.test.mjs`). The native gate's step 5b uses a non-default primary agent with a marked prompt. A mid-turn append's next step and a plugin wake turn both run with that agent's prompt (mock requests), and the stored append, the wake prompt and their assistant steps carry `agent: gatekeeper`.

## Review 2, item 7. Per-agent bash permissions are not covered by the grant: confirmed, documented

`agent/agent.ts` builds every built-in agent's ruleset as `Permission.merge(defaults, <agent defaults>, user)`, where `user` is the global `permission` (lines 138-277), so the grant the plugin adds there applies to them. A configured agent's own `permission` is then merged after that (`item.permission = Permission.merge(item.permission, Permission.fromConfig(value.permission ?? {}))`, line 293). Rules are evaluated last match wins (`permission/index.ts:28-32`, `findLast`), so an agent-level `permission.bash` rule matching a herdr-threads command decides it for that agent, ahead of the grant. The plugin does not edit `agent.*`, and doctor reads only the plugin's heartbeat, which records the global grant. One line in `integrations/opencode/README.md` (Permissions) says so.
