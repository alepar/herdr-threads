# agent-talk x Goose (Block / AAIF)

- Alternative: agent-talk, <https://github.com/xhluca/agent-talk>, shallow clone at `8a7fc4f1` ("release: 0.2.4"). Paths `A:<path>:<line>` are relative to that clone root. No code from it was executed.
- Harness: Goose. Paths `G136:` = `goose@1.36` (tag `v1.36.0`, `f13f369a`, matches the installed binary) and `Gmain:` = `goose@main` (`c3bfc5f7`, 1.56.0), relative to the clone roots described in `primers/goose.md`.
- Tags: VERIFIED = read in source or docs at the cited place. INFERRED = my reading, not confirmed by source or a run. "unknown" = not determined.

## Support status

**Not supported.** agent-talk has no Goose integration of any kind. VERIFIED:

- `grep -rniE 'goose|aaif'` over the clone (excluding `.git`) returns 0 hits.
- The README names its supported agents, and Goose is not among them: "Six: **Claude Code, OpenAI Codex, Google Antigravity, pi, opencode, and GitHub Copilot.**" (`A:README.md:361-366`). The auto-receive table lists the same six (`A:README.md:215-229`).
- The `init` skill's per-agent install notes cover codex, Antigravity, pi, opencode and Copilot CLI only (`A:skills/init/SKILL.md:55-71`). Its session-registry step 4 is "Claude Code only" (`A:skills/init/SKILL.md:541-545`), and the push steps 4b, 4c and 4d are pi, opencode and Codex (`A:skills/init/SKILL.md:578-679`). Step 4d repeats the registry write for Codex and says "Any unique id works" (`A:skills/init/SKILL.md:632-647`).
- No plugin manifest Goose can install. Goose's Open Plugins installer looks for `.goose-plugin/plugin.json`, `.plugin/plugin.json` or `plugin.json` at the repository root, or a `hooks/hooks.json`, `commands` or `agents` marker (`G136:crates/goose/src/plugins/formats/open_plugins.rs:14-21,45-50`). The Gemini path needs a `gemini-extension.json`. agent-talk ships `.claude-plugin/plugin.json`, `.codex-plugin/plugin.json` and a pi `package.json` (`A:.claude-plugin/plugin.json`, `A:.codex-plugin/plugin.json`, `A:package.json:20-27`), so `goose plugin install https://github.com/xhluca/agent-talk` would fail with "No supported plugin format found" (`G136:crates/goose/src/plugins/mod.rs:266-286`). INFERRED from the detection code; not run.

The rest of this report asks whether agent-talk's **generic** mechanism (prose skills that drive the `retalk` CLI, a detached follower, per-session NDJSON spools) would work on Goose anyway, and how well.

## What would carry over: the generic pull path

agent-talk's harness-neutral core is three pieces. VERIFIED from `overview.md` and the sources cited there:

1. **Skills** (`A:skills/*/SKILL.md`, 15 files, 116,746 bytes in total by `wc -c`) that tell the model which `retalk` commands to run.
2. **The `retalk` CLI** (one-shot `retalk receive --peer <fp> --save --dir <user>/identity ...`, NDJSON `{"id","from","name","text"}` "auto-acked") (`A:skills/receive/SKILL.md:100-104`).
3. **A detached follower** `bin/follow.sh start <user> <peer>...`, which runs `retalk receive --follow --interval 60 --quiet | spool-writer.py` under `setsid nohup` (`A:bin/follow.sh:93-115`) and writes into per-session spools plus, by default, a legacy per-user `<user>/inbox.ndjson` (`A:bin/spool-writer.py:28-30,176-199`).

On Goose:

- **Skills load if the user places them by hand.** Goose discovers `SKILL.md` files under `<wd>/.agents/skills`, `<wd>/.goose/skills`, `<wd>/.claude/skills`, `~/.agents/skills`, `<goose config dir>/skills`, `~/.claude/skills`, `~/.config/agents/skills` and installed plugins' skill directories (`G136:crates/goose/src/skills/mod.rs:288-317`; `Gmain:crates/goose/src/skills/mod.rs:303-332` keeps the same project directories). The default-enabled `skills` platform extension lists each skill as `• <name> - <description>` in the system instructions and loads one on demand through its `load_skill` tool (`G136:crates/goose/src/skills/client.rs:61-67,224-244`, `G136:crates/goose/src/agents/platform_extensions/mod.rs:192-202`; `Gmain:crates/goose/src/skills/client.rs:84-85`). VERIFIED. So the Copilot recipe `ln -sfn "$PWD/agent-talk/skills" <dir>` (`A:README.md:196-202`) pointed at `~/.agents/skills` would expose the same skills to Goose. INFERRED: an agent-talk installed as a **Claude plugin** lives in Claude's plugin cache, not in `~/.claude/skills`, so Goose would not pick it up by accident.
- **The skills' Claude-isms need translating, and the skill already says so.** `AskUserQuestion` becomes plain text, `${CLAUDE_PLUGIN_ROOT}` becomes the clone path, and the monitor and session map are skipped "on other agents" (`A:skills/init/SKILL.md:55-71`). VERIFIED text; INFERRED that a Goose model would apply it, since Goose is not named.
- **Commands run.** Goose's default `GOOSE_MODE` is `auto`, so the developer shell tool runs `retalk ...` and `follow.sh ...` without approval prompts (`G136:crates/goose/src/config/goose_mode.rs:24-34`, primer section 9). VERIFIED default. The follower detaches with `setsid` precisely so a tool-call process group that ends does not kill it (`A:bin/follow.sh:95-104`, comment about `codex exec`). INFERRED that this also survives Goose's shell tool; not checked.

So a Goose session would get agent-talk's **"generic"** tier, the one Antigravity and Copilot get: manual or model-initiated pull, no push, no wake.

## Injection mechanism

There is no agent-talk injection mechanism for Goose. What a Goose model would get is the pull path, plus a spool it could read:

| Aspect | Behaviour on Goose | Tag |
| --- | --- | --- |
| Hook / tool / API | None of agent-talk's own. The model runs `retalk receive --peer <fp> --save ...` through Goose's developer shell tool, as the `receive` skill tells it (`A:skills/receive/SKILL.md:100-116`). Or it reads `<user>/inbox.ndjson`, the legacy spool that the follower keeps writing by default (`A:bin/spool-writer.py:195-196`). | VERIFIED (agent-talk side); INFERRED (that Goose runs it) |
| Per-session spool | **Not written unless the model improvises a registration.** The spool writer fans out only to session ids registered in `~/.agent-talk/by-session/<sid>` (`A:bin/spool-writer.py:107-125,187-194`). The skill registers only on Claude (step 4) and Codex (step 4d), and step 4d says "Any unique id works" because the spool is just a file (`A:skills/init/SKILL.md:541-561,632-647`). So a Goose model could copy step 4d with any unique id; it does not need its goose session id. Nothing tells it to, and nothing consumes that spool on Goose except the model's own reads. | VERIFIED code; INFERRED outcome |
| When it fires | Only when the model decides to run `receive`, either because the user asks or because the skill says that in `auto` mode "the agent reads [the spool] each turn and relays anything new" (`A:skills/receive/SKILL.md:169-174`). Nothing enforces this per-turn read. It is a prose instruction the model may drop after a few turns or after compaction. | VERIFIED text; INFERRED reliability |
| Payload | Raw NDJSON lines `{"id","from","name","text"[,"group","group_id"],"ts"}` from retalk or from the spool (`A:skills/receive/SKILL.md:103`, `A:bin/spool-writer.py:164-172`). The model then re-renders them as a markdown chat transcript with `📥`/`📤`, quoted bodies and 1-3 turns of context (`A:skills/receive/SKILL.md:33-57`). | VERIFIED |
| Size bounds | None in agent-talk. A one-shot `retalk receive` returns everything pending. The spool rotates at 8 MiB (`A:bin/spool-writer.py:69,128-145`), which bounds disk use but not context. Goose's own shell-tool output limits would apply (not examined). | VERIFIED absence; Goose limit unknown |
| Framing | No header. The record is raw JSON, and the only framing is the model's own transcript rendering. | VERIFIED |
| Dedupe / ACK | Transport only. The relay deletes on delivery ("auto-acked"), so a second `retalk receive` does not repeat a message. A model reading `inbox.ndjson` has no cursor and can re-read and re-relay old lines. There is no reader ACK and no receipt to the sender (`overview.md`, "Delivery guarantees"). | VERIFIED (transport); INFERRED (spool re-read) |
| Mid-turn | Only if the model happens to call `receive` mid-turn. Messages that arrive during a long turn wait until the model asks. | INFERRED |

## Wake mechanism

**None.** An idle Goose session waiting at the `> ` prompt never learns that mail arrived. VERIFIED by absence: agent-talk has no Goose wake path, and its only wake implementations are Codex app-server `turn/start` (`A:bin/codex_wake.py`), pi `triggerTurn` and opencode `promptAsync` (`A:extensions/inbox-monitor.ts:76-78`, `A:extensions/opencode/inbox-monitor.ts:82`). None of them applies to Goose.

Would any of agent-talk's generic wake recipes work on Goose?

| agent-talk recipe | On Goose | Tag |
| --- | --- | --- |
| Claude plugin monitor `retalk-inbox` (`A:monitors/monitors.json`, `A:bin/inbox-monitor.sh`) | No equivalent. Goose plugins are files plus shell hooks, with no long-running monitor component (primer section 4). | VERIFIED (Goose primer) |
| `Monitor` tool tailing the spool (`A:skills/receive/SKILL.md:222-246`) | No `Monitor`-style tool found in Goose (INFERRED by absence). MCP server notifications reach only the UI, and only during a tool call of that server (`G136:crates/goose/src/agents/extension_manager.rs:1663`, primer section 6). | VERIFIED |
| "Fallback: interval polling (no Monitor tool)" with a scheduled wake-up or loop (`A:skills/receive/SKILL.md:247-252`) No in-session wake-up timer found. The model-visible `platform__manage_schedule` tool (offered when a scheduler service is configured) and `goose schedule add --cron` both schedule **recipe** runs (`G136:crates/goose/src/agents/platform_tools.rs:4-21`, `G136:crates/goose/src/agents/agent.rs:1268-1270`), and a scheduled run is a **new** session (primer section 2), so neither can wake the pane's session. INFERRED: the model could only "poll" by staying busy in a `sleep; receive` loop, which costs a provider call per tick and blocks the human. | VERIFIED (Goose); INFERRED (outcome) |
| Keystroke injection | agent-talk explicitly rejects typing into a TUI as "not a supported interface … can collide with whatever the user is doing" (`A:docs/antigravity-auto-receive.md:76-82`). It would not do this for Goose either. | VERIFIED (stance) |

Idle/busy detection, draft safety, latency: not applicable, because there is no wake. Pull latency is the relay poll (default 60 s, `A:bin/follow.sh:15-16`) for the follower's spool, plus however long until the human or model next asks. Failure mode: **silent non-delivery**. The relay has already deleted the message on the follower's read, and the per-session spool is never written for an unregistered Goose session. The message then exists only in the legacy `inbox.ndjson` and in retalk's sealed `--save` history (`A:bin/spool-writer.py:176-199`, `overview.md` "Gap between relay ACK and model"). INFERRED.

## Setup / installation footprint on Goose

agent-talk writes **nothing** Goose-specific. If a user adapted the Copilot recipe, the footprint would be:

- one symlink: `~/.agents/skills` (or `<project>/.agents/skills`) pointing at the clone's `skills/` (INFERRED, modelled on `A:README.md:196-202`);
- agent-talk's own state, which is the same on every harness: `~/.agent-talk/users/<name>/` (identity, contacts, `check-mode`, `receive-from`, pid files, `follow.err`, spools) (`A:docs/README.md:9-14`, per `overview.md`);
- a detached follower process per user and peer set (`A:bin/follow.sh:93-115`);
- the `retalk` binary, installed by `uv` or a fallback (`A:skills/init/SKILL.md:96-110`).

Nothing is written to `~/.config/goose`. Goose hooks, extensions and `GOOSE_MOIM_*` are not touched. VERIFIED by absence.

## Token cost

- **Per session (static):** the skills load on demand through `load_skill`. The first use loads `init` (47,368 bytes, roughly 12k tokens at about 4 bytes per token), then `receive` (14,617 bytes, roughly 3.7k tokens) and `send` (6,609 bytes). Goose lists every skill's name and description in the system instructions on every session (`G136:crates/goose/src/skills/client.rs:224-244`); the 15 agent-talk `description:` lines total 5,045 bytes (roughly 1.3k tokens). VERIFIED (`wc -c`, `awk` over the frontmatter); token estimates INFERRED.
- **Per message:** the raw NDJSON line or lines in the shell tool result, plus a model-written transcript that repeats the body and 1-3 prior turns of context. That is about two to four times the body, because the skill requires the full text and never a summary (`A:skills/receive/SKILL.md:33-57`). INFERRED multiplier.
- **Per quiet turn:** zero if the model ignores the "read each turn" instruction. If it obeys, each turn costs one shell tool call plus a model round trip, about 100-300 tokens plus a full context re-read for the extra provider call. INFERRED.

## Limitations and bugs visible for Goose

1. **Not supported, and not even listed as "generic".** The README promises nothing for Goose, and `goose plugin install` does not recognise the repository's layout (Support status). VERIFIED.
2. **No push, no wake.** A Goose peer is reachable only when someone asks. This matches agent-talk's Antigravity and Copilot tier, but those two are at least documented. VERIFIED absence.
3. **The per-session spool model breaks without a registry step.** Fan-out happens only to `by-session/<sid>`. The skill gives no Goose registry step (the Codex step 4d would work with any unique id, but Goose is not pointed at it), and the spool writer's own comment assumes "Only Claude Code writes the session map" (`A:bin/spool-writer.py:78-82`, stale against step 4d). The design therefore falls back to the per-user legacy file, which is shared across sessions and has no cursor, exactly what the per-session spools were built to avoid (`A:skills/init/SKILL.md:565-568`). With `--no-legacy` and no registration, message records are written nowhere but retalk's `--save` history (`A:bin/spool-writer.py:195-199`). VERIFIED code; INFERRED consequence.
4. **The "read the spool each turn" contract is model memory**, not a harness hook. Goose 1.36 hooks cannot add context anyway (primer section 3.4), so agent-talk would have nothing to enforce it with even if it shipped a Goose plugin. VERIFIED (Goose); INFERRED (drift).
5. **Identity also depends on model memory.** Every command names `--dir <user>/identity` explicitly (`overview.md`, "Identity and session model"). On Goose, after `/compact` or a resume, the model has to rediscover which user it is. INFERRED.

## Clever ideas worth stealing

Few are Goose-specific, because agent-talk built nothing for Goose. Three apply to a herdr-threads Goose adapter:

1. **A detached, harness-agnostic follower writing per-session files**, with harness-specific consumers layered on top (`A:bin/follow.sh`, `A:bin/spool-writer.py`). On Goose the natural consumer is the `tom` MOIM file: launch `goose session` with `GOOSE_MOIM_MESSAGE_FILE=<per-session file>`. Goose then re-reads that file before every provider call (turn start and after every tool batch) and injects it with no tool call (`G136:crates/goose/src/agents/platform_extensions/tom.rs:63-110`, primer section 5). One caveat applies if anyone copies agent-talk literally: `read_bounded` keeps the **first** 64 KiB (`tom.rs:88-105`). Pointing it at an append-only spool such as agent-talk's would freeze the injected view on the oldest messages once the file passed 64 KiB, and it would re-inject every old message on every provider call (1.36; on main a new turn-context block is added only when the text changes, primer section 5). The file has to be a daemon-rewritten "currently pending" view that is cleared on display, not a log. VERIFIED (truncation code); INFERRED (design).
2. **Body-free nudge plus separate body delivery** (agent-talk's Codex wake, `A:bin/codex_wake.py`). On a Goose pane the only idle wake is typing the content-free wake marker into an empty `> ` prompt (primer section 9). Pairing that with MOIM-file delivery means the wake marker carries nothing and the bodies arrive through `tom` on the very turn it starts. This matches our R18 content-free wake rule. INFERRED.
3. **Nudge suppression keyed to the reader cursor** (`A:bin/codex_wake.py:267-297`): do not nudge again until the consumer has moved past the last nudge mark. On Goose the consumer edge would be a `PostToolUse` or `Stop` hook firing for that `session_id` after the MOIM file was populated (Goose hooks carry `session_id`, primer section 3.3). INFERRED.

Not worth stealing: the "read the spool each turn" prose contract and the `Monitor`-tool recipe. Neither has a Goose equivalent, and both make delivery depend on the model.

## Verification

Adversarial pass on 2026-10-23 against the agent-talk clone (`8a7fc4f1`), the Goose clones (`f13f369a`, `c3bfc5f7`) and `primers/goose.md`. No code was run.

Confirmed as cited: the 0-hit grep; README supported-agent and auto-receive lists; `init` step 7 notes; Open Plugins manifest and marker lists and the "No supported plugin format found" bail; skill directories in both trees; `GOOSE_MODE` default `auto`; follower `setsid nohup` and 60 s interval; spool-writer legacy file, 8 MiB rotation, `stamp`; receive skill NDJSON shape, transcript rules, "reads it each turn" text, fallback polling; pi `triggerTurn`; Codex nudge cursor (`codex_wake.py:267-297`); `tom.rs` 64 KiB first-bytes truncation; MCP notification subscribe at `extension_manager.rs:1663`; skill byte counts.

Changed:
- Skill loading: was the `summon` extension's `load` tool; it is the `skills` platform extension's `load_skill` tool, with a name-plus-description listing in the system instructions. The listing size, previously "unknown", is now measured (5,045 bytes of descriptions).
- Registration: "a Claude-only step" was wrong. Codex step 4d also registers and allows any unique id, so the 1.36 session-id point does not block a Goose registration. Rewrote the per-session spool row and limitation 3, and added the `--no-legacy` drop case.
- Scheduler row: added the `platform__manage_schedule` tool; it also schedules recipe runs in new sessions, so the conclusion stands.
- Tom re-injection: qualified as 1.36 behaviour (main adds a block only on change).
- Citations fixed: init 4b-4d range, `spool-writer.py` line ranges, Monitor recipe `222-246`, keystroke stance `antigravity-auto-receive.md:76-82`, opencode `promptAsync` at line 82.
- "No `Monitor` tool in Goose" relabelled INFERRED (absence, not exhaustively searched).
