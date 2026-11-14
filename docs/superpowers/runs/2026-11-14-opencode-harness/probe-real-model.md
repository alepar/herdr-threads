# opencode probe with a real model

Date 2026-11-14. Evidence only, no production code. Tests the six items in the design's "Probe" section ([design](2026-11-14-opencode-harness-design.md)).

## Setup

- opencode `1.18.1` (`~/.opencode/bin/opencode`, executed only). Model `openrouter/deepseek/deepseek-v4-pro`, the exact id listed by `opencode models openrouter`. Small model `openrouter/deepseek/deepseek-v4-flash`.
- Isolation: `env -i` with `HOME`, `XDG_{CONFIG,DATA,STATE,CACHE}_HOME` and `TMPDIR` under a scratch root, `OPENCODE_DISABLE_AUTOUPDATE=1`, and `PATH=<scratch>/bin:/usr/bin:/bin`. No `HERDR_*`, `HC_*` or `TMUX` variables were passed through. The key came from a mode-0600 env file loaded with `set -a`. Every capture was scanned for the key value and for `sk-or-` before it was copied (no match). The env file and the scratch root are deleted.
- The TUI ran at 120x40 in a private `tmux -L oc-probe-<hex>`. Item 5 used an isolated Herdr server instead (see below).
- Global config `opencode.json`: `permission.bash = {"*": "ask"}` ([captures/probe/opencode.json.txt](captures/probe/opencode.json.txt)).
- Probe plugin: [captures/probe/probe-plugin.js.txt](captures/probe/probe-plugin.js.txt), installed as `<XDG_CONFIG_HOME>/opencode/plugins/probe.js`. It does the following:
  - logs hooks and events to a JSONL file, never headers or env;
  - takes control files for `append` (`session.prompt` with `noReply: true` and a `synthetic: true` text part), `wake` (`promptAsync`), `appendwake`, `armtext` (append on the first assistant text part, after a delay), `messages` and `status`;
  - `PROBE_PERM=1` makes its `config` hook add the bash grants;
  - `PROBE_THROW=1` makes init throw.
- A stub `herdr-threads` (also linked as `ht`) is first on PATH. It prints its args and `HERDR_THREADS_OPENCODE_SESSION`, and runs `sleep N` when called as `herdr-threads sleep N`.
- Logs: plugin log [captures/probe/probe-log.jsonl](captures/probe/probe-log.jsonl). Screens are under `captures/probe/NN-*.txt` (`capture-pane -p`) and `.ansi` (`capture-pane -e -p`).

## 1. Appended `noReply` synthetic message: CONFIRMED

The framed text used was `[herdr-threads] new messages:\n\n[herdr-threads] message mTESTn in tTEST from sTEST:\n  <instruction>`.

| Case | What was done | Observed |
|---|---|---|
| (a) idle | Appended to the idle session, then the user typed a neutral `hi` | No `session.status` change and no turn. The next turn's reply was exactly `PINEAPPLE` (`05-idle-next-turn.txt`). |
| (b) mid-turn, during a tool | Appended while `herdr-threads sleep 12` ran | The same turn's final text ended with `MANGO` (`07-busy-tool-result.txt`). One busy run, no second turn. |
| (b) mid-turn, during the final text step | Appended 150 ms after the first assistant text part began (prompt "Write twelve sentences…", no tools) | The loop ran **one extra step in the same busy run**: a second assistant message with `finish: stop` that said `GUAVA` (`10-final-text-append-mid.txt`; log: two assistant messages, one `idle`). |
| race: just after idle | With a 1500 ms delay the append landed about 0.5 s after `idle` | Stored as a passive message; **no turn started**. |

Rendering:
- **A synthetic-only message shows no bubble at all.** No `QUEUED` badge appears while busy, and no transcript entry appears when idle. None of the screens and ANSI captures contains the framed text. The model's reply is the only visible trace.
- By contrast, the wake prompt (`herdr-threads: wake up`, not synthetic) renders as a normal user bubble.

Return value of `client.session.prompt({noReply: true, ...})`:
- HTTP 200;
- `data = {info, parts}`;
- **the stored message id is `data.info.id`** (`msg_…`), with `data.info.role = "user"` and `data.parts[0] = {id: "prt_…", type: "text", synthetic: true}`.

Side findings that matter for the plugin:
- **`chat.message` fires for every noReply append and every `promptAsync` wake the plugin makes itself.** Those calls carry no `agent` field, while TUI-submitted user messages carry `agent: "build"`. D3 runs a lifecycle check on `chat.message`, so the plugin must skip its own messages. It can do that by its own stored message ids, by a synthetic-only parts list, or by the missing `agent`. Otherwise every append looks like a turn start.
- A KIWI append that landed after idle was still pending when the next turn began. In that turn the model answered only the newer GUAVA item and ignored KIWI. Model behaviour on several stale items is not characterised.

Consequence: D4's append primitive is confirmed for idle and busy, including during the final text step. The fallback (`chat.message` / `tool.execute.after` injection) is not needed. D4 still needs one addition: **after an append returns, re-check the session status. If it went idle in the meantime and the item is waking, wake it.** An append that lands between the final step and `idle` turns into a passive message and is not consumed in that turn.

## 2. `promptAsync` wake: CONFIRMED (the hidden-session case raises a new hazard)

- **Idle, visible session with a typed draft:** the composer held `my unsent draft`. `appendwake` appended the p0 item (HTTP 200), and `promptAsync` with `herdr-threads: wake up` returned HTTP 204 with `data: {}`. The turn started at once (`busy`) and the model replied `PAPAYA`. The draft was still in the composer afterwards (`11-wake-with-draft.txt`).
- **Session the TUI is not showing:** after `/new` the TUI sat at the home screen while the plugin's bound session stayed the old one. The append and wake went to the old session.
  - The turn ran in the background and replied `LYCHEE`; `session.messages` shows the stored sequence: synthetic append, wake, reply.
  - **The home screen did not change at all**: the text capture is identical before and after (`12-…`, `14-…`). There was no toast and no navigation.
  - The next user prompt from the home screen created a new session, and the plugin rebound to it (as D3 says).
- **New hazard, hidden session plus permission:** a hidden woken turn whose tool needed an `ask` permission went `permission.asked` and stayed `busy` with no visible prompt anywhere:
  - The home screen was unchanged (`28-hidden-session-permission-home.txt`).
  - The only cue was a spinner next to that session in the session list (`29-…`).
  - Switching to that session showed the permission dialog (`30-…`).
  - Herdr reported `idle` the whole time (item 5).
  - The session stays busy until someone opens it, so every later item for it is a busy append that never gets read.

Consequence: the D1 wake mechanism is confirmed. The D8 row "the previous session stays bound and is still woken" is accurate, but it understates the cost. Possible changes:
- Add a D8 / TRUST-POLICY accepted-limit line: a hidden woken session can block invisibly on a permission or question.
- Have the plugin watch `permission.asked` / `question.asked` for the bound session and surface them, for example with a toast (`client.tui.showToast`).
- Treat a pending permission as "not idle" in the wake gate (the design already does this in D4).
- Stop waking a bound session after `/new`. That cannot be detected: no event fires.

## 3. Permission grant from the `config` hook: CONFIRMED, with an ordering caveat

- **Static:** with `PROBE_PERM=1`, `opencode debug config` shows the merged result `{"*": "ask", "herdr-threads *": "allow", "ht *": "allow"}`. The hook runs in `debug config` too.
- **Behaviour:** one turn ran `herdr-threads probe one` and then `ls /etc/hostname`:
  - the stub ran with **no prompt** and saw `HERDR_THREADS_OPENCODE_SESSION=ses_…` from `shell.env`;
  - `ls` raised exactly one `permission.asked` (`02-perm-turn.txt`).
  - `ht sleep 12` also ran unprompted in item 5.
- **Interplay.** Rules are evaluated last match wins, in JS key order. Merged shapes are in `24-permission-merge-cases.txt`, behaviour in `25-perm-behaviour-C{1,2}.txt`.

  | User rule | Merged after the hook | `herdr-threads probe one` |
  |---|---|---|
  | `{"*":"ask"}` | grant appended last | runs, no prompt |
  | C1 `{"herdr-threads *":"deny","*":"ask"}` | the existing key is **overwritten in place** (deny becomes allow) but keeps position 0, so `*` stays last | **asks**: the grant is lost, and the user's deny is silently rewritten |
  | C2 `{"*":"ask","herdr-threads probe *":"deny"}` | grant appended after the user's narrower deny | **runs**: the plugin overrides the user's explicit deny |
  | C3 `"ask"` (string) | the probe converts it to `{"*":"ask"}` and appends | runs (shape only) |

Consequence: D6's mechanism works. The rendered plugin must still settle two points:
- Use `delete cfg.permission.bash[k]` and then set the key, so the grant always lands last. A plain assignment can leave it ineffective (C1).
- Decide what an existing user `deny` that matches a herdr-threads command should do. Leaving the user's `deny` and `ask` rules alone fits "never override the user's permissions" better than overriding them (C2).

Also:
- The string form `bash: "ask"` must be converted to an object, not replaced.
- Per-agent `permission` overrides were not tested.

## 4. Duplicate skill names: CONFIRMED deduped, REFUTED as deterministic

Three `herdr-threads/SKILL.md` copies with distinct descriptions sat in `<XDG_CONFIG_HOME>/opencode/skills/`, `$HOME/.agents/skills/` and `$HOME/.claude/skills/`.

- `opencode debug skill` and the system prompt always list **exactly one** `herdr-threads` entry. No duplicate warning appears in `opencode.log`.
- **Which copy wins changes from run to run:**
  - with all three present, 12 `debug skill` runs gave the config dir 9 times and `~/.agents` 3 times (`18-…`);
  - in the TUI turn, the system prompt listed the `~/.agents` copy (`17-system-prompt-skill-lines.jsonl`), even though the preceding `debug skill` had listed the config-dir copy;
  - with only the two external copies, 8 runs gave `~/.agents` 7 times and `~/.claude` once (`16-…`).
- Side finding, model behaviour: when a prompt merely mentioned `herdr-threads`, DeepSeek loaded the skill unprompted and read its file. The stub skill's body was useless, so the model wandered (`which herdr-threads`, `file …`) instead of running the command.

Consequence:
- The installed skill can be shadowed at random by any stale same-named copy in `~/.agents/skills` or `~/.claude/skills`. These are common, because other harness installs put the shared herdr-threads skill there.
- Options:
  - give the opencode skill its own name (for example `herdr-threads-opencode`);
  - make doctor detect same-named external copies and report "shadowing possible" (and ensure they carry the same content);
  - or ship nothing for opencode when an identical shared copy is already visible.
- D6 needs one of these. The skill content itself has a real effect on this model.

## 5. Herdr agent kind and status: CONFIRMED (with a hidden-session gap)

- An isolated Herdr 0.9.5 server ran through `scripts/lib/isolated-herdr.sh` (`ih_root_new`, `ih_start`, `ih_teardown`; private `/tmp/ih.*` root, socket, config and state; the owner was a dedicated `sleep`).
- opencode ran in pane `w1:p1` of a workspace created there. It was driven only with `pane send-text` / `send-keys` on that server.
- Herdr fetched its remote detection manifest `opencode.toml 2026.06.10.1`.

| opencode state | `agent` | `agent_status` | `agent explain` |
|---|---|---|---|
| home screen, no session | `opencode` | `idle` | `rule: none`, `fallback_reason: default_known_agent_idle_fallback` |
| generating or running a tool | `opencode` | `working` | |
| permission prompt shown | `opencode` | `blocked` | `rule: permission_required (region=whole_recent priority=300)` |
| idle in a session after a turn | `opencode` | `idle` | fallback, as at home |
| hidden bound session busy for 13 s (plugin wake, TUI at home) | `opencode` | **`idle` throughout** | |

The timeline is in `26-herdr-status-timeline.txt`, explain output in `27-herdr-explain-idle-in-session.txt`. Detection is screen-based, so it sees only the displayed session.

Consequence:
- The adapter's host kinds can rely on Herdr knowing the kind `opencode`, with idle / working / blocked.
- For wake gating, the plugin's own `session.status` must stay the authority, as D3 and D4 intend. Herdr's status is wrong for a hidden session, and Herdr reported the hidden permission wait as `idle`.

## 6. Plugin init failure and `--pure`: CONFIRMED

**Init throw (`PROBE_THROW=1`):**
- The TUI shows nothing: home screen and turn are normal (`19-…`, `20-…`), and the turn ran normally without the plugin.
- The only trace is one line in `<XDG_DATA_HOME>/opencode/log/opencode.log`: `level=ERROR … message="failed to load plugin" path=file:///…/plugins/probe.js error="PROBE_THROW: deliberate init failure"` (`opencode-log-plugin-failure.txt`).
- Note: code that ran before the throw did run (the probe's `init` log line was written). A heartbeat written at the start of init therefore does not prove the plugin loaded. D6's heartbeat must be written **after** hook setup, at the end of init.

**`--pure`:**
- No plugin hook ran: no probe log lines, and no grant, so both commands asked.
- A shell tool sees exactly `AGENT=1`, `OPENCODE=1`, `OPENCODE_PID=<pid>`, **`OPENCODE_PURE=1`** (plus the inherited `OPENCODE_DISABLE_AUTOUPDATE=1`).
- The stub saw no `HERDR_THREADS_OPENCODE_SESSION` (`23-pure-result.txt`).

Consequence:
- D2's `session_claim_required` path is confirmed.
- The CLI can name the exact cause when `OPENCODE_PURE=1` is set ("plugins disabled by --pure") instead of the generic "plugin missing". This uses only an existing message slot.

## Contradictions with the design

1. **Research row "noReply message shows in the transcript / `QUEUED`"**: with `synthetic: true` it shows nothing. The design's Decision 3 calls append delivery "visible in the session transcript", which is not the case for synthetic parts. It is hidden, like hook injection. Only the model's reaction is visible.
2. **D3 `chat.message` trigger**: the plugin's own appends and wakes fire `chat.message`. The design does not say to exclude them.
3. **D4**: an append can race the end of a turn and become passive. A post-append status check is needed for waking items.
4. **D6 permission**: a naive assignment fails when the user already has the key, and it silently overrides a narrower user `deny`.
5. **D6 skill**: duplicate names are deduped at random, not by location precedence.
6. **D6 heartbeat**: written "at init" is not enough. It must come after successful setup.
7. **D8 hidden-session row**: it omits the invisible-permission block, and Herdr reports `idle` for that pane.

## Cleanup and secrets

- All processes stopped and checked with `pgrep -af 'oc-probe|opencode/bin/opencode'` (empty): opencode instances, the private tmux server (socket file removed), the isolated Herdr server and its root `/tmp/ih.*` (`ih_teardown`), the reaper and the owner `sleep`.
- The env file and the scratch root are deleted.
- The key was never printed. Captures, plugin log and opencode log were checked for the key value (compared in-shell) and for `sk-or-`: no match.
- No real config was read or written, no real Herdr server or pane was touched, and there were no herdr-threads writes and no commits.
- The early `08-final-text-append.txt` run is invalid. Its trigger fired on the user prompt's own text part, before the step started. It is superseded by `09` (delay too long) and `10` (valid).
