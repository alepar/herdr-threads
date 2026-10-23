# agentchattr x Antigravity CLI (agy)

- Upstream: https://github.com/bcurts/agentchattr (MIT). Clone examined at HEAD
  `d775776a30e8af4c85ea16b3caaf9c79d65cb239` (2026-09-10, VERSION 0.5.0), `--depth 1`.
- Paths are relative to the clone root. Line numbers are at that HEAD.
- **VERIFIED** means read in source, the GitHub API or docs. **INFERRED** means reasoned from them and not observed.
  **unknown** means no source answers it.
- Read this alongside `overview.md` (the generic architecture) and `../../primers/agy.md` (the agy surface).
  This file covers only what is specific to agy, or what behaves differently under agy.

## Support status

**Native but thin. It was added in one commit, live-verified once against agy 1.1.7, and has no tests.**

- **Commit history.** VERIFIED (GitHub API, commits touching `macos-linux/start_antigravity.sh`):
  - `0e8928e341` (2026-07-26) "feat: add Antigravity (agy) CLI as a built-in agent". It changed
    `config.toml` (+16), `macos-linux/start_antigravity.sh` (+77) and `windows/start_antigravity.bat` (+44).
  - Two follow-ups the same day fixed the launcher's PATH handling and its exec bit.
- **The live check.** The commit message says: "Verified live against agy 1.1.7: agy connects and sees
  all 12 chat_* tools with working bearer auth." That is the only evidence of a live run.
- **What agy support consists of.** It is configuration plus a launcher. There is no agy-specific
  Python code path: `wrapper.py` has no `agy`/`antigravity` branch. VERIFIED: grep finds `agy` only in
  `config.toml:28-42` and the two launchers.
  - The generic `settings_file` MCP writer does the injection.
  - The generic tmux paste does the wake.
- **Origin.** [Issue #76](https://github.com/bcurts/agentchattr/issues/76) (2026-06-20, closed 2026-07-26). VERIFIED.
  - A contributor proposed an `antigravity_config` inject mode writing
    `~/.gemini/antigravity-cli/mcp_config.json` with `serverUrl`.
  - A second user reported on agy 1.0.14 that agy "never attempts to read/parse that file at all".
  - The maintainer's merge used `~/.gemini/config/mcp_config.json` with the key `serverURL` instead.
  - The issue's proposed "Antigravity identity text" in the MCP instructions was **not** merged (see Limitations).
- **Discoverability.** The README does not mention Antigravity at all. Its supported list at `README.md:5` and its
  launcher lists at `README.md:22-32` and `:66-80` omit it. VERIFIED: grep of `README.md` for
  `antigravity|agy` finds nothing. Users find it only in `config.toml` or the launcher folder.
- **Tests.** None. `tests/test_wrapper_mcp_config.py` covers the `httpUrl`/`url` writers only; no test
  covers `serverURL` or the agy path. VERIFIED: grep of `tests/`.
- **Versions.** Nothing checks the agy version. The launcher freezes whatever agy is installed by setting
  `AGY_CLI_DISABLE_AUTO_UPDATE=1`. VERIFIED: `macos-linux/start_antigravity.sh:5-6`,
  `windows/start_antigravity.bat:5-6`.
  - The primer's verified surface is 1.3.2/1.3.3. Nothing shows agentchattr was re-tested beyond 1.1.7.
  - INFERRED: users who installed agy later than 1.1.7 run an untested combination.
  - The config schema is the main risk; see the `serverURL` note below.

## Injection mechanism

agentchattr uses **no agy hook at all**: no `SessionStart`, `PreInvocation`, `PostInvocation` or `Stop`
`injectSteps`, and no plugin. VERIFIED: no code writes `hooks.json`, `plugin.json` or anything under
`~/.gemini/config/plugins`.

Message content reaches the agy model in only two ways.

### 1. MCP tool results (the main content path)

**Wiring.** VERIFIED: `config.toml:33-42`.

```toml
[agents.antigravity]
command = "agy"
mcp_inject = "settings_file"
mcp_settings_path = "~/.gemini/config/mcp_config.json"
mcp_transport = "http"
mcp_http_key = "serverURL"
mcp_merge_project = true
```

**How the server entry is written.**
- `_resolve_mcp_inject` takes the explicit `mcp_inject` (`wrapper.py:185-186`), and `_apply_mcp_inject`'s
  `settings_file` branch expands `~` (`wrapper.py:234-248`).
- It then calls `_write_json_mcp_settings(target, "http://127.0.0.1:8200/mcp", http_key="serverURL", token=<instance token>)`.
- That writer (`wrapper.py:38-82`) loads the existing file, replaces only `mcpServers.agentchattr`, and writes:

  ```python
  entry: dict = {"type": "http", http_key: url, "trust": True}
  if token:
      entry["headers"] = {"Authorization": f"Bearer {token}"}
  ...
  folder_trust["enabled"] = True          # security.folderTrust.enabled (a Gemini CLI key)
  ```

- The result in agy's global file is
  `{"mcpServers":{..., "agentchattr":{"type":"http","serverURL":"http://127.0.0.1:8200/mcp","trust":true,"headers":{"Authorization":"Bearer <32 hex>"}}}, "security":{"folderTrust":{"enabled":true}}}`.

**Connection.**
- This mode connects directly; no per-instance proxy is involved. `needs_proxy` is false for `settings_file`
  (`wrapper.py:639`).
- The server maps the bearer token to the instance and overrides any self-declared sender
  (`mcp_bridge.py:139-190`). VERIFIED.

**Key spelling.** The primer (agy docs) spells the key `serverUrl`; agentchattr uses `serverURL`, and its comment says plain
`url`/`httpUrl` are ignored (`config.toml:31-32`). The only evidence that `serverURL` works is the commit
message's live check on 1.1.7; `serverUrl` working is the docs' claim. Whether agy matches the key
case-insensitively is unknown.
- INFERRED: if agy is a Go binary using `encoding/json` (unknown; the primer does not say), field names match
  case-insensitively, which would explain both spellings working.
- Nothing in the repo tests it. If agy's decoder is or becomes case-sensitive for `serverUrl`, this entry
  silently stops loading.

**The delivery loop.** VERIFIED: `wrapper.py:526-531`, `mcp_bridge.py:575-689`.
1. The wake prompt (below) tells the model to "use mcp to read #channel".
2. The model calls `chat_read`.
3. `chat_read` returns a JSON array of
   `{id, sender, text, type, time, channel[, attachments, reply_to]}` (`mcp_bridge.py:409-426`).
4. A multi-instance agent also gets an `[identity: <name> | label: <label>]` breadcrumb first (`mcp_bridge.py:681-688`).
5. The model replies with `chat_send`.

**When it fires.** Only when the agy model chooses to call the tool, which in practice means only right after a wake prompt.
- A busy agy session gets nothing mid-turn unless it happens to call `chat_read`. VERIFIED: there is no other path.
- This throws away the primer's strongest agy primitive: `PreInvocation` `injectSteps`, which fires
  before every model call, including the calls after a tool result (primer §2.5).

**Bounds.** `limit` defaults to 20 messages (`mcp_bridge.py:578`), applied as `msgs[-limit:]` (`:662`).
- There is no per-message or per-page byte cap. VERIFIED: `_serialize_messages` copies `text` whole.
- agy's own MCP tool-call timeout (180 s default, primer §4) is irrelevant here because `chat_read` returns immediately.

**Framing.** The data is raw JSON. The behavioural contract lives in the FastMCP `instructions` string:
5,736 characters, `mcp_bridge.py:54-124`, measured by parsing the literal.
- Whether agy puts MCP server `instructions` into the model's context is **unknown**: the primer's MCP
  section documents tools only.
- If agy drops them, the agy model sees only the 12 tool docstrings (about 3.9k characters in total), and
  loses the identity, "always respond in chat" and anti-polling rules. INFERRED.

**Dedupe and ack.**
- A per-sender, per-channel cursor is persisted to `data/mcp_cursors.json` (`mcp_bridge.py:566-572`).
- There is no ack.
- The cursor moves to the last *returned* id. With more than 20 unread messages, the older ones are skipped
  silently (`:655-663`). VERIFIED. The issue is generic; agy does not change it.
- Repeated empty reads return the escalating "STOP." text (`:666-679`).

### 2. The wake prompt text itself (a secondary content path)

**How it is built.** The queue watcher (`wrapper.py:473-571`) composes:
- `use mcp to read #<channel> - you're mentioned, take appropriate action and respond` (80 bytes for `#general`), or
- a job variant (`:529`), or
- a session-engine custom prompt (`:519-527`).

It then appends:
- `ROLE: <role>` if one is set (`:536-540`);
- `RULES: <all active rules joined by "; ">` on the first trigger, on an epoch change, or every N triggers
  (default 10) (`:542-559`);
- a one-time identity hint for multi-instance agents (`:561-563`).

Newlines are flattened to spaces (`:567`). The text arrives in agy as a genuine user turn
(`USER_EXPLICIT/USER_INPUT` in the transcript, per primer §7). INFERRED: it is typed, so agy cannot tell it from the user.

## Wake mechanism

**How it is delivered.** The generic Unix path is unchanged for agy.
- The wrapper starts agy inside its own tmux session, `agentchattr-<name>` (`wrapper.py:890`, `wrapper_unix.py:244-249`), and attaches the
  user's terminal to it (`:255`).
- On a trigger, `inject()` runs `tmux load-buffer`, `paste-buffer -p -d` and, after
  `max(inject_delay, len*1 ms)`, `send-keys Enter`. VERIFIED: `wrapper_unix.py:64-136`.
- `inject_delay` is the default 0.3 s for agy; `config.toml` sets no override.
  - Issue #76's proposal used 1.0 s. The merged config dropped it.
- On Windows the path is `WriteConsoleInputW` (`wrapper_windows.py:158-191`).
- VERIFIED.

**Bracketed paste under agy.** `paste-buffer -p` brackets only if the application enabled bracketed paste.
agy's support is **unknown** (primer §8). Either way the text is one short line, and Herdr's typed
`hello draft` worked under agy (primer §8), so plain bytes plus Enter most likely submit correctly.
- INFERRED.
- A RULES-laden prompt of several hundred bytes is the case that could split across pty reads, if agy
  treats large reads specially. Unknown for agy.

**Trigger source.** The server appends a JSON line to `data/<name>_queue.jsonl` (`agents.py:32-54`). The wrapper polls
every 1 s, reads every line, truncates the file, and makes one injection per poll (`wrapper.py:482-571`).

**Idle and busy detection.** **None before injecting.** VERIFIED: the only condition is `has_trigger` (`wrapper.py:502`).
- The activity checker, a hash of `tmux capture-pane -p` (`wrapper_unix.py:159-183`), feeds only the UI status pill.
- For agy the activity checker runs with `provider=""`, because `_provider_from_command("agy")` returns `""`
  (`wrapper.py:166-180`; `agy` is not in `_BUILTIN_DEFAULTS`). Only Codex's particle filter exists,
  so agy's animated `Working · Ns` subagent region and spinner count as activity. That is harmless, since
  activity is cosmetic. VERIFIED code, INFERRED effect.

**Draft safety: none.** Each agy screen state from primer §8 meets the blind paste-and-Enter as follows.
INFERRED unless noted.

| agy screen state (primer §8) | Effect of agentchattr's paste + Enter |
|---|---|
| Empty composer (`? for shortcuts`) | Submits the wake prompt. It starts with `u`, so it avoids the `?` help-overlay trap. |
| Human draft present | The wake text is appended to the draft and the whole thing is submitted: the draft is sent half-written. |
| Turn running (`esc to cancel`) | Unknown. agy's mid-turn queued-input behaviour is undocumented. The text either queues as the next turn or sits in the composer, and the Enter may be consumed. |
| **Permission prompt** (a `>`-selected menu) | Pasted characters go to a menu. **Enter confirms the currently selected option, which can approve a pending command** without the human. That is a safety defect, not just a lost wake. |
| Subagent running (Herdr reports false idle) | Same as a running turn. agentchattr does not consult Herdr, so it gains nothing and loses nothing here. |

The permission-prompt case is a real risk for agy in particular.
- MCP calls are themselves permission-gated in agy (`mcp(server/tool)` rules, primer §4/§7).
- agentchattr writes `"trust": true`, a Gemini CLI field. Whether agy honours `trust` is **unknown**.
- agentchattr adds no `permissions.allow` entry to `~/.gemini/antigravity-cli/settings.json`. VERIFIED: no write to that path.
- So in `request-review` mode each `chat_read` or `chat_send` may raise a permission menu. A second @mention
  arriving while that menu is up gets pasted into it and Enter is pressed. INFERRED; untested.

**Latency.** Up to 1 s queue poll, plus a fixed 0.5 s pre-inject sleep (`wrapper.py:506`), plus the 0.3 s
paste-to-Enter delay. That is about 0.8-1.8 s from @mention to submitted prompt. Then comes one model round
trip to decide to call `chat_read`, and a second to act on the result. INFERRED from the code.

**Failure modes.**
- **Wake loss.** Triggers queued while the wrapper is down are cleared at startup (`wrapper.py:719-721`).
  The read-then-truncate in `_queue_watcher` (`:484-486`) can drop a line appended in between. INFERRED race.
  A tmux failure prints `INJECT FAILED` and is not retried (prints at `wrapper_unix.py:89,101,110,121,133`;
  the no-retry rationale is the docstring at `:77-83`; the watcher ignores the return value, `wrapper.py:567`).
- **Fresh conversation on every restart.** After agy exits, the wrapper relaunches the same command line,
  `agy` with no `-c`/`--conversation` (`wrapper_unix.py:236-273`).
  - So every restart starts a brand-new agy conversation, and the model's in-context identity is gone.
  - The `/resume` identity-reclaim flow (`README.md:177-184`) applies only if the user resumes by hand.
  - VERIFIED code, INFERRED effect.
- **Stale token after re-registration.** On a 409 heartbeat, the wrapper re-registers and gets a new token
  (`wrapper.py:780-786`). It then rewrites `~/.gemini/config/mcp_config.json` (`:678-690`, `:715`). The code
  comment admits: "Most CLIs won't re-read mid-session, but the file is correct for next restart."
  - Proxy-based harnesses (Codex) swap the token live in the proxy (`:707-709`). agy, in direct-connect mode, does not.
  - If agy keeps its loaded headers, every `chat_*` call answers "Error: stale or unknown authenticated agent
    session. Re-register and retry." (`mcp_bridge.py:173-174`), which the model cannot fix.
  - Mitigation: the registry keeps crash-timed-out tokens reclaimable and reactivates them on use
    (`registry.py:257-281`, `:637-674`). So the common case of a laptop sleep is survived, and only a true 409
    supersession strands agy. VERIFIED code.
  - Whether agy hot-reloads `mcp_config.json` is unknown.
- **Identity crossover between agy instances.** See Setup. INFERRED.

## Setup and installation footprint on the harness

**Real user config written, with no consent prompt and no cleanup.** VERIFIED:
- `~/.gemini/config/mcp_config.json` (agy's **global** MCP file, primer §4; that the IDE and App share it, as they
  share the global `hooks.json` per primer §2.1, is INFERRED).
  - **Writes.** On every wrapper start and every re-registration it merges in `mcpServers.agentchattr` with a live
    bearer token, plus `security.folderTrust.enabled=true`. The `security` key is meaningless to agy: the writer
    is shared with Gemini CLI (`wrapper.py:74-79`).
  - **Non-atomic write.** It calls `write_text` (`:81`) directly, with no temp file and rename.
  - **Unparseable file.** If the existing file is unparseable, for example hand-edited with a trailing comma, the
    `except: pass` at `:58-61` starts from `{}`, and the user's other MCP servers are **overwritten and lost**.
  - **Never removed.** The wrapper's `finally` (`wrapper.py:918-934`) deregisters but does not remove the entry.
- **Launcher-scoped environment only.** `AGY_CLI_DISABLE_AUTO_UPDATE=1` and a PATH prepend
  (`macos-linux/start_antigravity.sh:6-19`). Neither persists.
- **Not written.** Hooks, plugins, skills, permissions (`~/.gemini/antigravity-cli/settings.json`), and
  `~/.gemini/trustedFolders.json`. `_ensure_gemini_folder_trusted` runs only for `agent=="gemini"` or `env`
  mode (`wrapper.py:737-738`).
- **Dead config.** `mcp_merge_project = true` is a no-op in `settings_file` mode: project `.mcp.json` merging
  exists only in the `env` branch (`wrapper.py:264-286`). The `settings_file` branch (`:234-253`) never reads
  it. VERIFIED. The commit message claims "merge project servers", but that is not what the code does.

Consequences of using the one global file. INFERRED.
- **Every agy session on the machine gets the 12 chat_* tools.** That includes sessions in other projects,
  the IDE and the App, and it continues after agentchattr is stopped.
  - With the server down they point at a dead `127.0.0.1:8200`, so expect a connect error or a timeout.
  - With the server up they hold the last instance's token. They can then post into the chat **as that
    agentchattr instance**, because the server trusts the token (`mcp_bridge.py:167-172`).
- **Two agy wrappers race on one file.** For example `antigravity` and `antigravity-2`:
  - The second writes its token over the first's.
  - The first agy, if it has not yet loaded the file, or on any later restart (the restart loop does not
    rewrite the file), loads the second's token.
  - Both agy processes then speak as `antigravity-2`.
  - Claude (per-instance `--mcp-config` file) and Codex (per-instance proxy URL) are immune; agy is not,
    because it has no config-dir flag (`config.toml:29-30`).
- **Overlap with herdr-threads.** On a machine that also runs herdr-threads, agy sessions in Herdr panes
  see both systems' surfaces at once.

## Token cost

**Per delivered message batch.** INFERRED estimates; the code shapes are VERIFIED.
- The wake prompt: about 20 tokens. Add ROLE, and every 10th trigger, or on a change, the full RULES text.
- The model's `chat_read` call: tens of tokens.
- The result: about 40 tokens of JSON framing per message, plus the bodies (README's estimate, `README.md:278`).
  The README puts tool definitions at about 850 tokens (`README.md:280`), low against the 3.9k characters measured below. Up to 20 messages, unbounded bytes.
- The `chat_send` reply.
- That is a minimum of two model round trips per wake, versus one for herdr-threads, where `PreInvocation`
  already carries the bodies into the woken turn.

**Per quiet turn.** Zero injected bytes: there are no hooks, so idle and unrelated turns pay nothing per turn.

**Standing cost.**
- 12 tool schemas, about 3.9k characters of docstrings plus parameters.
- The 5.7k-character instructions, if agy surfaces them (unknown).
- This sits in **every** agy session's tool list, globally (see Setup).

**Waste.** If the model polls, the anti-polling replies cost a round trip each.

## Limitations and bugs visible for agy

1. **No identity guidance for agy.**
   - `_MCP_INSTRUCTIONS` names base identities for Anthropic, OpenAI, Google ("Gemini CLI, gemini-cli,
     aistudio" → `"gemini"`), Qwen and Kilo (`mcp_bridge.py:61-68`). It tells the model: "Always use your base
     agent name (claude/codex/gemini/qwen/kilo) as sender" (`:73`).
   - An agy model (a Google product) is thus told to call itself `gemini`, while the registry named it `antigravity`.
   - The bearer token overrides the `sender` in `chat_send`/`chat_read` (`:167-172`), so routing is unharmed.
     But `chat_claim(sender='gemini', ...)` and the model's self-references in chat text can be wrong, and the
     /resume reclaim hint asks for a name the model was told to get wrong.
   - VERIFIED text, INFERRED effect. Issue #76's "Antigravity identity text" fix was not merged.
2. Writes to the global `~/.gemini/config/mcp_config.json`, with the consequences listed under Setup:
   cross-instance token clobbering, leakage into unrelated agy sessions, a token left at rest in a
   user-readable config file, and no cleanup.
3. The unparseable-file overwrite (`wrapper.py:56-61`, `:81`) can destroy the user's agy MCP config.
4. `mcp_merge_project = true` does nothing (`wrapper.py:234-253` versus `:264`).
5. There is no idle, draft or permission-menu gate before paste plus Enter. Under agy that includes the risk of
   confirming a permission menu.
6. A fresh agy conversation on every auto-restart: no `-c`/`--conversation`.
7. A stale token after a 409 re-registration is unrecoverable without restarting agy.
8. **Auto-update is pinned off by the launcher.** That is good for reproducibility, but it freezes users on a
   version agentchattr never re-verified.
   - The "verified against 1.1.7" claim is two minor versions behind the 1.3.x surface the primer documents.
   - The issue #76 history shows the MCP config path already moved once between 1.0.14 and 1.1.7.
9. Not documented in the README, and no tests.
10. Unknown keys go into agy's file: `type`, `trust`, `security.folderTrust`. agy tolerated them on 1.1.7.
    The primer notes that agy's `hooks.json` drops the *whole file* on one malformed key. If
    `mcp_config.json` ever gets the same strictness, the user's other servers disappear too. INFERRED risk.

## How this compares to herdr-threads on agy (brief)

| Concern | agentchattr | herdr-threads (primer §10) |
|---|---|---|
| Content path | MCP pull (`chat_read`), model-initiated | `PreInvocation` `injectSteps` `systemMessage` push, falling back to `userMessage` |
| Mid-turn delivery | None | Before every model call, including after tool results |
| Wake | tmux paste + Enter, blind | Herdr types a content-free `herdr-threads: wake up` only when `AgyComposer` reads an empty idle composer |
| Draft / permission menu | Clobbered / can confirm | Read as `Unreadable`, so no wake |
| Delivery confirmation | Read cursor only | Transcript scan for the `SYSTEM_SDK` header |
| Config footprint | Global `mcp_config.json` with a live token, never removed | Owned plugin dir, removed by `unsetup` |
| Round trips per wake | 2+ (wake, then `chat_read`) | 1 (the woken turn's PreInvocation carries the bodies) |

## Clever ideas worth stealing

1. **A bearer token per instance, resolved server-side, overriding the self-declared sender**
   (`mcp_bridge.py:139-190`), with a reclaimable-token reactivation after sleep (`registry.py:637-674`).
   - It is a clean way to attribute MCP calls without trusting model-supplied names.
   - It is relevant if herdr-threads ever offers an MCP surface on agy. That surface would fit the primer's
     "blocking MCP tool as long-poll" lead.
   - To be per-instance, it would need a per-process config. agy has no config-dir flag, so it would have to
     be a plugin-scoped `mcp_config.json` or a header that carries `ANTIGRAVITY_CONVERSATION_ID`. Note that
     agentchattr shows what goes wrong if it is not per-instance.
2. **A bracketed paste via `tmux load-buffer` + `paste-buffer -p`, then a separate Enter after a
   length-scaled delay** (`wrapper_unix.py:64-136`).
   - Our wake text is short and content-free, so this is not needed today.
   - It is cheap insurance if any agy wake text ever grows, and a good test case for agy's unknown
     bracketed-paste support.
3. **Escalating "No new messages … STOP." replies to empty reads** (`mcp_bridge.py:666-679`).
   - A token-saving nudge against polling.
   - Applicable to our `inbox` output if agents are seen polling it.
4. **`AGY_CLI_DISABLE_AUTO_UPDATE=1` in a managed launch** (`macos-linux/start_antigravity.sh:6`).
   - A documented knob, at least as far as agentchattr relies on it (unknown in official docs).
   - We should *not* pin it: AGENTS.md "Harness and Herdr versions" forbids version pinning that forces updates
     on users. It is still worth knowing the variable exists, for test harnesses that need a stable agy during a gate run.
5. **Periodic re-injection of the active rules on an epoch change or every N triggers** (`wrapper.py:542-559`).
   - Not stealable as is: it puts content in the wake prompt, against R18.
   - The underlying idea is sound: re-assert a small shared policy on change and every N deliveries, with a
     server-recorded "seen epoch" (`_report_rule_sync`).
   - It could map onto our skill or check-in text via the agy `PreInvocation` path rather than the wake.

Nothing in agentchattr's agy integration improves on our agy wake or injection mechanics. Its value here
is mainly as a catalogue of what to avoid: a global config with a live token, blind paste plus Enter into
a screen that can hold a permission menu, and restarts that lose the conversation.

## Verification

Adversarial pass on 2026-10-23 against the clone at `d775776a30` and the GitHub API.

- Checked, held: HEAD date and VERSION 0.5.0; the three agy commits, their dates and the `0e8928e341` file stats and
  message (GitHub API); issue #76 dates, the proposed `antigravity_config`/`serverUrl`/`inject_delay = 1.0`, the 1.0.14
  report and the unmerged identity text; `config.toml:28-42`; grep for `agy|antigravity` (only `config.toml` and the
  two launchers; none in `README.md` or `tests/`); launcher `:5-6`; `wrapper.py` ranges for the writer (`:38-82`),
  inject resolution, `needs_proxy` (`:639`), queue watcher (`:473-571`), 0.5 s sleep, queue clear at start,
  Gemini folder trust (`:737-738`), 409 re-register, rewrite comment, `finally`; `mcp_bridge.py` token resolution
  (`:139-190`), `chat_read` limit/cursor/skip/escalation/breadcrumb, `_serialize_messages`; `agents.py:32-54`;
  `registry.py` reclaim paths; `wrapper_unix.py` inject, activity checker and restart loop; Windows
  `WriteConsoleInputW`. Re-measured: instructions literal 5,736 characters; 12 `chat_*` tools with 3,928 characters
  of docstrings; the `#general` wake prompt is 80 bytes.
- Changed: `url`/`httpUrl` comment citation `:30-31` to `:31-32`; "both spellings seem to work" and the Go-decoder
  explanation downgraded to unknown/INFERRED (the primer does not say agy is Go); `INJECT FAILED` citation pointed at
  the print sites instead of the docstring; tmux session name cited to `wrapper.py:890`; "global file read by the CLI,
  IDE and App" marked INFERRED (the primer says that of `hooks.json`, not `mcp_config.json`); the token-cost README
  reference resolved to `README.md:278` and its 850-token tool estimate noted.
