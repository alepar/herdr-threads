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
