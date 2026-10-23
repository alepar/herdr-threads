# agent-talk on Antigravity CLI (agy)

- Upstream: https://github.com/xhluca/agent-talk. Paths are relative to the clone root. The clone is shallow at HEAD `8a7fc4f1` ("release: 0.2.4"). No code from it was executed.
- Harness facts come from the [agy primer](../../primers/agy.md). Primer sections are cited as "primer §N".
- Tags: **VERIFIED** means read in source, docs or the GitHub API. **INFERRED** means my reading. **unknown** means no source answers it.

## Support status

**Generic only. agent-talk has no agy-specific injection and no agy-specific wake.** On agy it is the shared skills plus the `retalk` CLI, and receiving means a pull that the model or the user starts.

| Claim | Tag | Evidence |
|---|---|---|
| Auto-receive is "not currently supported on the Antigravity CLI" | VERIFIED | `docs/antigravity-auto-receive.md:5` |
| The README matrix lists Antigravity as "No" for auto-receive, with "Run the `receive` skill on demand" | VERIFIED | `README.md:228` |
| The README lists auto-receive on Claude Code, Codex, pi and opencode, but not on Antigravity or Copilot | VERIFIED | `README.md:363-370` |
| The init skill says to skip the monitor and the session map on agy ("proactive auto-receive is not wired up on Antigravity … run the **receive** skill on demand") | VERIFIED | `skills/init/SKILL.md:65-68`, `:541-547` |
| The repo ships no agy hook, no `hooks.json`, and nothing that uses `injectSteps`, `PreInvocation`, `agentapi` or `/schedule` | VERIFIED by absence | `grep -rln 'hooks.json\|injectSteps\|PreInvocation\|agentapi\|/schedule'` over the clone finds nothing. Besides docs, tests and demos, the root holds `.claude-plugin/`, `.codex-plugin/`, `skills/`, `bin/`, `monitors/`, `extensions/`, `package.json`. |
| CI checks only that the plugin installs: `agy plugin install "$PWD"`, then a count of exactly 15 `SKILL.md` under `~/.gemini/config/plugins/agent-talk/skills`. No session is started and no message is exchanged. | VERIFIED | `.github/workflows/ci.yml:183-220` |

The repo gives its reasons in `docs/antigravity-auto-receive.md:36-90`. Its central claim is that "The Antigravity CLI provides no supported mechanism for an external process to deliver input into a running interactive session" (`:38-39`), based on the open ACP request [antigravity-cli#31](https://github.com/google-antigravity/antigravity-cli/issues/31). VERIFIED that the issue is still open: GitHub API on 2026-10-23 returns state `open`, 196 comments, last updated 2026-10-06. The repo rejected four approaches:

- ACP: not implemented in agy (`:57-64`).
- The CLI as an MCP server: the direction is wrong (`:66-74`).
- OS-level keystroke automation: "not a supported interface … can collide with whatever the user is doing. We do not build on it." (`:76-82`).
- Polling: it solves detection, not delivery (`:84-90`).

**The analysis misses the agy surfaces that matter.** INFERRED, but strongly supported by the primer.

- The doc frames the problem as an *external process* pushing input into a *live* session. It never considers agy's in-process hooks.
- `PreInvocation` `injectSteps` puts text into the model's context before every model call, including the calls that follow a tool result. That is the same role Codex's `UserPromptSubmit` and `Stop` hooks play in agent-talk's own Codex integration. VERIFIED: primer §2.3, §2.5. The primer also cites a genuine capture of a post-tool-call injection (`tests/fixtures/agy/delivery/README.md:11-14` in herdr-threads).
- `Stop` `decision:"continue"` with `reason` is the agy counterpart of the Codex `Stop` block-and-continue that agent-talk already ships. VERIFIED (docs): primer §2.3.
- A plugin can ship its own `hooks.json` (primer §3), and agent-talk is already installed as an agy plugin.
- The doc says the CLI exposes "exactly three ways to run", none of which accept input from another process (`:44-46`). That ignores several surfaces the primer lists as idle-wake candidates:
  - `agentapi send-message <conversation_id>` (primer §3, §11 lead 1);
  - Remote Control (primer §5);
  - `--input-format stream-json` for a headless process (primer §5);
  - the native `/schedule` tool (primer §6).

  Each one is unproven or partial for a TUI session, but none is ruled out by #31.
- Stale reference. The doc's "Path to support" points at a `retalk-mcp-server` branch and closed PR #16 (`docs/antigravity-auto-receive.md:110-113`). VERIFIED (GitHub API, 2026-10-23): PR #16 "feat(mcp): stdio MCP server wrapping the retalk CLI (codex integration)" was closed unmerged on 2026-07-13, and `branches/retalk-mcp-server` returns 404. The prepared MCP server the doc relies on is not reachable from upstream.

## Injection mechanism

### What exists: a pull the model performs through a skill

**Path.** The user asks something like "check my messages", or the model decides to check. The model opens `skills/receive/SKILL.md`, which agy lists by name and path in the system prompt and the model reads with `view_file` (primer §3). The model then runs, through its `run_command` tool:

```
retalk receive --peer <peer> --save --dir "<user>/identity" --passphrase-path "<user>/passphrase"
# NDJSON: {"id","from","name","text"}; auto-acked
```

VERIFIED: `skills/receive/SKILL.md:99-116`. For `*contacts*` mode the skill loops over the saved peers with `jq` (`:111-116`), which is a compound shell command (see Limitations).

**Payload.** The payload is the tool output: one NDJSON object per message (`{id, from, name, text}`, optionally `group`/`group_id`, or a control record with `kind`). VERIFIED: `skills/receive/SKILL.md:103,118-131`. It reaches the model as an ordinary `run_command` result, so on agy it is recorded as a `MODEL/GENERIC` tool-output step (primer §7, INFERRED mapping).

**When.** Only when the model runs the command. No hook fires on agy for agent-talk. The doc says messages appear "at the start of a turn if configured to do so" (`docs/antigravity-auto-receive.md:17,95-96`), but no such configuration exists in the repo. VERIFIED by absence: every `grep -rn 'check-mode'` hit is the auto/manual switch (`skills/init/SKILL.md:24-28,225,516`, `skills/receive/SKILL.md:25-31`, `skills/send/SKILL.md:111-115`, `skills/add/SKILL.md:37-41`, `skills/id/SKILL.md:178`, `bin/invite-watch.sh:105`, tests), and nothing reads it at turn start on agy. INFERRED: the only "turn-start" path is the model choosing to run `receive` because a skill told it to.

**Framing.** None from code. The skill tells the model to re-render every exchange as a markdown chat transcript, using 📥/📤, bold names, `HH:MM`, quoted bodies and 1-3 earlier turns of context from both sides. It says "Real text, never summaries or counts." VERIFIED: `skills/init/SKILL.md:20-23`, `skills/receive/SKILL.md:33-57`. Group mail is rendered as a room transcript threaded by `group_id` (`:59-97`).

**Size bounds.** None. No truncation, no page limit and no per-message cap appear anywhere on this path. VERIFIED by absence in `skills/receive/SKILL.md`. Whatever retalk prints goes whole into the tool result. INFERRED: agy's own tool-output limits, if it has any, are the only bound. Their values are unknown.

**Dedupe and ACK.**
- Transport level only. A one-shot `retalk receive` is "auto-acked", and the relay deletes each message on delivery (`skills/receive/SKILL.md:103`, `docs/README.md:22`; VERIFIED as agent-talk's claim, not checked in retalk source).
- There is no application-level read receipt.
- Dedupe follows from the relay delete: the same message cannot come back from the relay.
- The skill also pulls "earlier lines from the spool (`<user>/inbox.ndjson`)" for context (`:51-53`), so earlier messages are re-read on purpose. The model's own judgement keeps that context from being shown as new. INFERRED.

### The "auto" trap on agy

INFERRED, from VERIFIED parts.

- The init skill recommends **Auto-receive** as the default for every host (`skills/init/SKILL.md:501-518`).
- Session rule 2 says that `auto` means "keep a follower + persistent Monitor running … silently (start them after an add/send if missing)" (`:24-28`). `send` repeats this after every send (`skills/send/SKILL.md:111-115`).
- The only agy carve-out is in rule 7 and step 4 ("skip them", `:65-68`, `:541-547`). Step 7 has no agy exception.

On agy, the model following these rules will:

1. **Start `bin/follow.sh` if it can find it.** The skills say `<plugin>` is `${CLAUDE_PLUGIN_ROOT}` under Claude Code (`skills/receive/SKILL.md:190`). That variable does not exist on agy, so the model has to work out `~/.gemini/config/plugins/agent-talk/` by itself.
   - The README says `agy plugin install` "copies the plugin" there (`README.md:133-135`).
   - CI checks only the `skills/` directory (`ci.yml:209-220`).
   - Whether `bin/` and its executable bits are copied is unknown.
2. **The follower drains the relay.** `bin/follow.sh:135-138` pipes `retalk receive --follow` into the spool writer; the relay deletes each message on delivery (`docs/README.md:22`). Whether `--follow` acks exactly like the one-shot read is INFERRED (retalk not cloned).
3. **The messages land only in the legacy file.** The spool writer copies each record to the sessions registered in `~/.agent-talk/by-session/` (`bin/spool-writer.py:108-125,190-195`). agy never registers one (step 4 is skipped). Records therefore go only to `<user>/inbox.ndjson`, because `--legacy` is on by default (`:195-196,264-267`).
4. **A later `receive` pull returns nothing.** The follower has already consumed the messages. The model would have to know to read `inbox.ndjson`, and the receive skill only points at that file "for context" (`:52-53`) and at the per-session spool as "the reliable way" (`:216-220`). That spool does not exist on agy.
5. **No Monitor exists.** The Claude `Monitor` recipe (`:222-246`) has no agy equivalent. The no-Monitor fallback is "a scheduled wake-up/loop that polls the spool on an interval" (`:248-252`). On agy that could only be the model creating a `/schedule` cron, which is a poll loop (primer §6). Nothing in the repo spells this out for agy.
6. **When legacy writing is removed, the messages are gone.** `--legacy` is "on by default for one release" (`bin/spool-writer.py:28-30`). Once it is off, an unregistered agy user's messages are acked from the relay and written nowhere (`:195-206`; only the *requests* stream has a fallback file). They survive only in retalk's sealed `--save` history, which the follower writes because it sets `RETALK_SAVE_MESSAGE=1` (`bin/follow.sh:130`), and the `history` skill can replay them.

Result: on agy, the recommended mode can be worse than `manual`. The user ends up with a background process that turns relay delivery into a file that nothing on agy watches.

## Wake mechanism

**None.** VERIFIED: `docs/antigravity-auto-receive.md:5-17,92-103`, `README.md:228`. An idle agy session stays idle until the human types. The doc offers two "optional additions" that do not wake the session: an OS notification so the human knows to check, and a separate `agy -p` print-mode run that writes a reply in its own process (`:100-103`). VERIFIED that both are described in the doc; neither is implemented in the repo (VERIFIED by absence).

- **Idle/busy detection:** none. Not applicable.
- **Draft safety:** not applicable. Nothing types into the composer. Keystroke automation was rejected explicitly (`docs/antigravity-auto-receive.md:76-82`).
- **Latency:** unbounded. A message waits until the human prompts and the model decides to pull. In `auto` mode the relay poll adds up to 60 s on top of that (`bin/follow.sh:15-16`).
- **Failure modes:**
  - messages are silently stranded in `inbox.ndjson` (the auto trap above);
  - a model that forgets to pull misses messages;
  - nothing tells the session that mail is waiting, not even a counter, because no hook runs.

**Would the generic mechanisms wake agy if wired?** INFERRED, using the primer.

| Candidate | Would it work on agy? | How well |
|---|---|---|
| OS keystroke automation (rejected by agent-talk) | Yes, as a user turn | Fragile, as agent-talk says. Herdr's pane-level `send-text` into a composer read as empty (primer §8, §10) avoids the window-focus problem agent-talk cites. herdr-threads ships exactly this, with screen-parse guards for drafts, busy state, permission menus and subagents. |
| `agentapi send-message <conversation_id> <prompt>` | Possibly | Unestablished for a TUI-owned conversation; behaviour with a typed draft unknown (primer §11 lead 1). |
| Remote Control | Probably, as remote prompts "stream live into your local terminal TUI" | No documented API; Google sign-in; idle versus busy undocumented (primer §5). |
| `/schedule` cron that runs `receive` | Yes; agy wakes itself "with its full conversation history" | It is a poll: every tick costs a full model turn even with no mail. Only the model or the user can create it (primer §6). |
| Stop `continue` long-poll | Only extends a turn that is already running | Bounded by the hook timeout and an unknown continue cap; it keeps the session spinning rather than idle (primer §6). |
| An MCP server with notifications (the PR #16 idea) | No | agy's handling of server-pushed MCP notifications is undocumented, and nothing suggests they reach the model (primer §4). A blocking MCP tool could long-poll for up to 180 s, but only while the model is inside a tool call. |

## Setup and installation footprint on agy

VERIFIED unless marked.

- **Plugin:** `agy plugin install ./agent-talk` from a git checkout (`README.md:126-135`). agy reads `.claude-plugin/plugin.json` and the root `skills/`, then copies the plugin into `~/.gemini/config/plugins/agent-talk/`.
  - Updates: `git -C <checkout> pull && agy plugin install <checkout>` (`skills/init/SKILL.md:201`).
  - The manifest carries `version`, `author`, `homepage`, `repository`, `license` and `keywords` (`.claude-plugin/plugin.json:1-19`). agy's documented `plugin.json` schema has `additionalProperties: false`, with only `name`, `description` and `$schema` (primer §3). Installs pass in CI today (`ci.yml:200-207`). INFERRED: agy either ignores the Claude-layout manifest's extra keys or does not validate strictly. A stricter agy could reject the plugin. herdr-threads carries the same risk with its `version` key.
- **Hooks, MCP, rules, agents, sidecars on the harness:** none written. VERIFIED by absence.
- **Permissions:** none for agy.
  - The allowlist advice is Claude-only: `"permissions": {"allow": ["Bash(retalk:*)"]}` in `.claude/settings.json` (`skills/init/SKILL.md:179-185`).
  - agy honours only `~/.gemini/antigravity-cli/settings.json` `command(<word>)` rules (primer §7), and the repo never mentions `command(` (VERIFIED by absence).
  - INFERRED: under agy's `request-review` mode every `retalk` call asks for approval unless the user writes `command(retalk)` themselves. The `*contacts*` loop (`skills/receive/SKILL.md:113-116`) is a pipeline with `while`/`jq`, and agy's `command()` rules refuse chains (primer §7), so it asks every time.
- **Outside the harness:**
  - `retalk` installed with `uv tool install --upgrade retalk` or `pip` (`skills/init/SKILL.md:101-107`);
  - per-user state under `~/.agent-talk/users/<name>/` or `<project>/.agent-talk/users/<name>/` (`:8-12`);
  - in `auto` mode, a detached `setsid nohup` follower that outlives the session (`bin/follow.sh:94-114`).

## Token cost

INFERRED estimates at about 4 bytes per token, built on VERIFIED byte counts (`wc -c` over the clone).

- **Per quiet turn: zero.** No hook runs, so nothing is injected. The one standing cost is the skill listing. agy lists the 15 skills by name and path in the system prompt (primer §3). If it also includes the frontmatter descriptions, those add up to 5,086 bytes (whole `description:` lines, key and newline included) (about 1.3k tokens) for each session. Several are long: `init` 528 B, `receive` 523 B, `send` 456 B. Whether agy includes descriptions is unknown; the cost lies somewhere between a few hundred bytes and about 5 KB.
- **Per message (pull):**
  - one model turn, or part of one, to decide to pull;
  - one `view_file` of `skills/receive/SKILL.md` the first time in a session (14,617 B, about 3.7k tokens), plus the `init` skill at setup (47,368 B, about 12k tokens);
  - the tool call, with the full NDJSON body;
  - the model's re-rendered transcript, which repeats the body plus 1-3 earlier turns from both sides as output tokens;
  - the extra reads of `inbox.ndjson` for context.

  A short message plausibly costs a few hundred tokens of output for the transcript, on top of the input. Bodies are never bounded.
- **No-mail pull:** a full tool round trip that returns nothing, started by the model or by a human prompt. With a `/schedule` poll (not shipped), that cost would repeat on every tick.

## Limitations and bugs

1. **There is no in-context notification at all.** No counter and no "you have mail" line. A session learns about mail only by pulling. VERIFIED by absence.
2. **The auto-mode trap** described above. INFERRED from VERIFIED code: the recommended default starts a follower that drains the relay into a file nothing on agy reads, and once `--legacy` goes away it writes nowhere.
3. **No session identity on agy.** No `by-session` registration exists for agy. `ANTIGRAVITY_CONVERSATION_ID` is in the hook environment (primer §2.4), but agent-talk has no hook, and the model's `run_command` shell has only `ANTIGRAVITY_AGENT=1` (primer §2.4). INFERRED: even a model trying to follow the Codex step 4d pattern (`skills/init/SKILL.md:621-647`) has no documented agy id to use, although "any unique id works" (`:646-647`).
4. **`<plugin>` and `CLAUDE_PLUGIN_ROOT` are not resolvable on agy.** Whether `bin/` survives the install copy is unknown (CI checks only skills).
5. **The analysis doc is wrong in substance.** It treats #31 as the only path and overlooks agy's documented `PreInvocation`/`Stop` hooks, which match the mechanism agent-talk itself ships for Codex. It also cites an MCP branch that no longer exists. VERIFIED (doc text and GitHub API), with the substance INFERRED from the primer.
6. **"Comparable to Claude Code" is overstated.** The doc says agy's pull is comparable to Claude, "where the monitor also presents messages on the next turn" (`docs/antigravity-auto-receive.md:96-99`). On Claude the monitor pushes on its own. On agy nothing happens unless the model pulls. The repo is also inconsistent about whether Claude's monitor wakes the session (overview "Flaws").
7. **No bounds and no priority.** Every message is handled the same way, and a large body goes whole into a tool result. VERIFIED by absence.
8. **Permission friction.** Allowlist advice exists only for Claude, and the contact loop is a compound command. INFERRED.
9. **Copying the message for display.** The transcript rule makes the model repeat every body in its output. That is deliberate for the human reader, and it doubles the token cost for each message. VERIFIED rule (`skills/receive/SKILL.md:33-57`), INFERRED cost.

## Clever ideas worth stealing

Few on agy itself, since the integration there is minimal. What transfers:

1. **Stop instead of building on an unsupported path, and say so in a user-facing doc.** `docs/antigravity-auto-receive.md` names the missing capability, the upstream issue and what would change on their side. herdr-threads could publish a similar per-harness "what we cannot do and why" note, for example for Linux agy, which is unvalidated (primer §0). VERIFIED doc.
2. **CI install smoke with no auth.** `ci.yml:183-220` installs agy in a fresh Ubuntu runner without a Google login and asserts that the plugin's files landed. herdr-threads' agy coverage is macOS-only and opt-in (primer §0). A Linux, auth-free `agy plugin install` / `setup agy` smoke would catch packaging and plugin-root drift, such as `config/plugins` versus `antigravity-cli/plugins`, cheaply in CI. The comment at `ci.yml:190-193` says the installer and `agy plugin install <dir>` both run without login. VERIFIED claim in the repo, not checked here.
3. **One portable producer, a pluggable per-harness consumer.** The follower and spool writer do not depend on the harness (`docs/antigravity-auto-receive.md:21-34`), so supporting a harness means writing only a consumer. herdr-threads already splits things this way (daemon plus `HarnessAdapter`). The same lesson applies here as on pi and opencode: a consumer that is not registered loses messages silently. VERIFIED design.
4. **The path agent-talk did not take, which herdr-threads already uses.** Port the Codex hook pattern (`extensions/codex/inbox-hook.py`: a cursor advanced before output, a fingerprint check for truncate-and-refill, a persisted id window) to agy `PreInvocation` `injectSteps` and `Stop` `continue`. That is essentially herdr-threads' `HookPull` (primer §10). It confirms independently that hook-at-model-call is the right shape for agy. Using `Stop` `continue` for mail that arrived during a turn's last model call is something herdr-threads does not do yet (primer §11 lead 4). agent-talk's Codex `Stop` block-and-continue, which advances the cursor before output so it cannot loop (overview, technique 2), is the template to borrow if herdr-threads takes up that lead. INFERRED.

## Verification

Adversarial pass, 2026-10-23, against the clone at `8a7fc4f1`, the GitHub API and the agy primer.

- Checked and confirmed: every `docs/antigravity-auto-receive.md` line range; `README.md:126-135,228,363-370`; `.github/workflows/ci.yml:183-220` (sub-ranges included); the `skills/init`, `skills/receive` and `skills/send` ranges; `bin/spool-writer.py:28-30,108-125,190-206,264-267`; `bin/follow.sh:15-16,94-114,130`; `extensions/codex/inbox-hook.py:26-31,45` (cursor before output, 256-byte fingerprint, 500-id window); `.claude-plugin/plugin.json:1-19`; byte counts (`wc -c`, 15 `SKILL.md`); the absence grep; PR #16 closed unmerged 2026-07-13; branch `retalk-mcp-server` 404; antigravity-cli#31 open, 196 comments, updated 2026-10-06; the primer sections cited, and herdr-threads `tests/fixtures/agy/delivery/README.md:11-14` and the `version` key in `tests/fixtures/agy/plugin/plugin.json`.
- Changed:
  - the root listing no longer says "only" (the root also holds docs, tests, demos and `package.json`);
  - the `check-mode` grep claim now lists every hit (all are the auto/manual switch), replacing "finds only `init`";
  - the relay-delete citation now points at `docs/README.md:22` instead of the overview, and is marked as agent-talk's claim, not checked in retalk;
  - the follower "acks" step is now cited to `bin/follow.sh:135-138`, and ack-on-follow is marked INFERRED;
  - the `agentapi` citation now reads primer §3 and §11 lead 1;
  - the description byte total now says it counts whole `description:` lines.
- No claim was refuted in substance.
