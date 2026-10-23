# agent-talk: overview

- Upstream: https://github.com/xhluca/agent-talk (paths below are relative to the clone root)
- Clone read: shallow, HEAD `8a7fc4f1` (2026-09-10, "release: 0.2.4"). No code from it was executed.
- Transport dependency: [retalk](https://github.com/xhluca/retalk) CLI, **not cloned**. Claims about retalk and the relay come from agent-talk's own docs and skills only.
- Tags: VERIFIED = read in source or docs (cited); INFERRED = my reading, not confirmed by source or a run.

## What it is

A plugin of skills plus small helper scripts that lets coding agents message each other end-to-end encrypted, through an untrusted relay, with the `retalk` CLI. It covers agents on different machines and agents run by different people. VERIFIED: `README.md:7-14`, `README.md:415-420`, `docs/README.md:5`.

It is a messaging primitive only. It has no task list, no lead agent and no orchestration. VERIFIED: `README.md:382-385`, `README.md:406-413`. Most of the logic lives in prose skills (`skills/*/SKILL.md`, about 2,000 lines), which tell the model which `retalk` commands to run. The executable parts are a follower supervisor, a spool fan-out writer, one Claude monitor script, a Codex hook and waker, and pi and opencode extensions (about 1,400 lines). VERIFIED: `wc -l` over the clone.

## Architecture

| Aspect | What it does | Evidence |
| --- | --- | --- |
| Process model | There is no agent-talk daemon. Each user runs a background **follower**: `follow.sh __run` loops `retalk receive --peer … --follow --interval N --quiet \| spool-writer.py`, restarts after 2 s on exit, and detaches with `setsid nohup`. Other pieces: a Claude plugin monitor (`inbox-monitor.sh`, which tails spools), an optional invite watcher, and per-harness in-process extensions or hooks. | VERIFIED `bin/follow.sh:72-97,103-115`; `monitors/monitors.json` |
| Transport | `retalk` polls the relay over HTTP(S). The default interval is **60 s**, and `--interval 5` is the setting for a "rapid live exchange". | VERIFIED `bin/follow.sh:15-16`, `skills/receive/SKILL.md:211-212` |
| Relay | Self-hostable or public (`https://relay.retalk.dev`, best effort). It stores public keys and ciphertext and deletes each message on delivery. It sees metadata (who talks to whom, and when) but no content. | VERIFIED `README.md:25-27`, `docs/README.md:22` (claim about retalk, not checked in retalk source) |
| Crypto | Olm-style E2EE (the skills mention ratchet, Olm, pre-keys and one-time keys). Keys are pinned with `verify`, and a mismatch reports `PIN MISMATCH`. | VERIFIED `docs/README.md:28`, `skills/init/SKILL.md:41-45`, `skills/sync/SKILL.md:3` |
| Storage | Per-user dirs `~/.agent-talk/users/<name>/` or `<project>/.agent-talk/users/<name>/` (identity, contacts, passphrase-path record, `check-mode`, `receive-from`, pid files). Per-session plaintext spools `<user>/sessions/<sid>.ndjson` and `.requests.ndjson`, files with mode 0600 and an `flock` on append, rotated at 8 MiB (one `.1` generation), with a 14-day GC. A legacy `<user>/inbox.ndjson`. retalk's sealed history (`--save` / `RETALK_SAVE_MESSAGE=1`) is the durable, encrypted record. | VERIFIED `docs/README.md:9-14,44`; `bin/spool-writer.py:68-71,141-160,213-248` |
| Session registry | `~/.agent-talk/by-session/<sid>` holds the user dir. The spool writer re-reads it at most every 5 s and copies each record to every session mapped to that user (fan-out). | VERIFIED `bin/spool-writer.py:107-124,187-193` |
| Cross-machine | Yes. Being cross-machine and cross-organization is the main design goal. | VERIFIED `README.md:415-420` |

## Identity and session model

- Identity is a keypair whose 32-hex **fingerprint** is both address and verification value. There is no account directory. Peers exchange fingerprints out of band, or onboard with single-use invite codes (7-day expiry, or permanent until revoked; retalk 0.3.0+). VERIFIED `docs/README.md:9,28-32`.
- One session acts as exactly one "user". The skills tell the model to pick **distinct users for parallel sessions** so followers do not collide. VERIFIED `docs/README.md:16`, `skills/init/SKILL.md:14-16`.
- Session id to user: Claude uses `CLAUDE_CODE_SESSION_ID`. `CLAUDE_SESSION_ID` is substituted only into monitor command lines, and an earlier bug left push silently off for every session. VERIFIED `bin/inbox-monitor.sh:20-28`, `skills/init/SKILL.md:548-554`. Codex uses `$CODEX_THREAD_ID`, which the model reads and writes out literally. VERIFIED `skills/init/SKILL.md:637-647`. pi and opencode have no registry step (see Flaws).
- Each command targets identity explicitly with `--dir "<user>/identity"`, because Claude starts a fresh shell per command, so an env var cannot carry "who am I". VERIFIED `docs/README.md:16`. Identity is therefore carried by the **model's own memory of a path** (INFERRED: a model that forgets or confuses the path acts as the wrong user).
- Receive is scoped to designated peers. The skills forbid `retalk receive --all`. VERIFIED `skills/receive/SKILL.md:16-18`.

## Injection mechanisms (general)

The pipeline is the same on every harness: relay → follower poll → spool-writer stamps `ts` → per-session NDJSON spool → a harness-specific consumer. VERIFIED `bin/spool-writer.py:164-210`. The consumers:

| Consumer | Mechanism | When | Framing | Dedupe | Evidence |
| --- | --- | --- | --- | --- | --- |
| Claude plugin monitor `retalk-inbox` | `monitors/monitors.json`, `when: always`. `tail -n0 -F` on the session and legacy spools, piped through an awk dedupe on the `"id"` field. Each raw NDJSON line on stdout becomes a Claude monitor notification. | Whenever a line lands. The repo says one time that it surfaces on the **next turn** and another time that it **wakes** the session (see Flaws). | The raw JSON record, no header | awk `seen[]` map, in memory, for the life of the monitor | VERIFIED `bin/inbox-monitor.sh:1-56`, `monitors/monitors.json` |
| Claude `Monitor` tool recipe (skill-driven) | The model starts `Monitor(persistent:true, timeout_ms:3600000, command:"tail -n 0 -F <spool> \| grep --line-buffered '\"from\":'")`. The skill also keeps a ~25 min scheduled wake as a backstop. | Each new line | Raw JSON line | None | VERIFIED `skills/receive/SKILL.md:222-246` |
| Codex hooks | `SessionStart` / `UserPromptSubmit` return `additionalContext`. `Stop` returns `{"decision":"block","reason":…}`, which Codex turns into a continuation prompt carrying the bodies. Installed into `$CODEX_HOME/config.toml` and gated by `AGENT_TALK_CODEX_SPOOLS`. | Session start, prompt submit, end of each turn | `N new agent-talk message(s) …` + `From <name>:\n<text>` + an instruction to show them verbatim and reply | Byte-offset cursor, a sha256 fingerprint of the first 256 bytes (detects truncate-and-refill), and the last 500 delivered ids, persisted in `<sessions>/.codex-hook-state.json`. The cursor advances **before** output, so a repeated Stop cannot loop. | VERIFIED `extensions/codex/inbox-hook.py:1-218`, `extensions/codex/install-hooks.py:31-55` |
| pi extension | `fs.watch` plus a 1 s poll on the spools in `AGENT_TALK_PI_SPOOLS`, then `pi.sendMessage({customType:"agent-talk-inbox", content, display:true}, {triggerTurn:true})`. When idle it triggers a turn. When streaming it is queued until the current turn's tool calls finish ("steer default", as the source comment says). | On arrival | `New agent-talk message from <who>:\n\n<text>` | In-memory offset (starts at EOF) plus an in-memory id set | VERIFIED `extensions/inbox-monitor.ts:59-80,126-171` |
| opencode plugin | Same watcher, then `client.session.promptAsync({path:{id:activeSession}, body:{parts:[text]}})`. The active session id is latched from any `event` carrying `properties.sessionID` or `properties.info.id`. Records that arrive before a session is known are buffered. | On arrival | Same as pi | Same as pi | VERIFIED `extensions/opencode/inbox-monitor.ts:62-110,185-199` |
| Pull (all harnesses) | The `receive` skill runs `retalk receive --peer … --save`, whose NDJSON output is "auto-acked". The model renders a markdown chat transcript (`📥`/`📤`, quoted bodies, 1-3 turns of context). | When the model or user asks | Model-rendered transcript | The relay deletes on delivery | VERIFIED `skills/receive/SKILL.md:33-57,99-116` |

There is no truncation or bounding anywhere in the injection path. Bodies are pushed whole, and the Codex Stop hook joins every pending message into one `reason`. VERIFIED by absence in `extensions/codex/inbox-hook.py:496-547`, `extensions/inbox-monitor.ts:59-80`. The only size limit is the 8 MiB spool rotation. VERIFIED `bin/spool-writer.py:68`.

Peer text goes straight into user-authority channels on pi (`triggerTurn`), opencode (`promptAsync`, a real user prompt) and the Codex Stop `reason` (a continuation prompt that "acts as a new user message"). VERIFIED `docs/codex-auto-receive.md:12-15`. The Codex **wake** path refuses to push bodies for exactly this reason (see below), but the other three paths do push them. INFERRED inconsistency.

## Wake mechanisms (general)

| Harness | Idle wake | Busy handling | Draft protection | Evidence |
| --- | --- | --- | --- | --- |
| Claude | Plugin monitor or Monitor-tool notifications. Whether a plugin-monitor line starts a turn while idle is stated both ways in the repo: README "a monitor wakes your session the moment a message lands", but receive skill "can't make the agent speak on its own — they surface on your next turn". The Monitor-tool recipe is the documented "true spontaneous wake". | The harness interjects or queues (not agent-talk's code) | None in agent-talk | VERIFIED `docs/README.md:44`, `skills/receive/SKILL.md:167-174,217-231` |
| Codex | Opt-in. `codex-with-daemon` runs `codex app-server daemon start`, then `exec codex --remote unix://`. The spool writer, with `--wake-codex`, opens `$CODEX_HOME/app-server-control/app-server-control.sock` (a hand-rolled WebSocket client with no permessage-deflate), sends `initialize`, `thread/loaded/list`, `thread/read` (status), then `turn/start` with a **fixed, body-free NUDGE** (about 300 bytes). The Stop or UserPromptSubmit hook then supplies the bodies. | Skipped when `thread/read` status is not `idle`, because the Stop hook will deliver. An unreadable status still nudges (fail-open). | None (the app-server turn bypasses the TUI composer, INFERRED) | VERIFIED `bin/codex_wake.py:30-44,206-220,300-334`, `extensions/codex/codex-with-daemon:24-39` |
| Codex, target choice | Wakes only when **exactly one** thread is loaded. With several, it stands down ("nothing maps one to a session"). | | | VERIFIED `bin/codex_wake.py:317-322`, `docs/codex-auto-receive.md:172-176` |
| Codex, nudge suppression | Records the spool size at each nudge, and does not nudge again until the hook cursor passes that mark. A shrunken spool re-arms it. | | | VERIFIED `bin/codex_wake.py:267-297` |
| pi | `triggerTurn:true` | Queued after the current tool calls | None | VERIFIED `extensions/inbox-monitor.ts:76-78` |
| opencode | `promptAsync` schedules the agent loop | Not handled by agent-talk; opencode's own queueing applies (INFERRED) | None | VERIFIED `extensions/opencode/inbox-monitor.ts:185-199` |
| agy, Copilot | None. Pull only. Keystroke automation was considered and rejected as "not a supported interface … can collide with whatever the user is doing". Waiting on ACP (antigravity-cli#31). | | | VERIFIED `docs/antigravity-auto-receive.md:40-125` |

Idle and busy are detected only on Codex (`thread/read`). No path anywhere considers a human draft in the composer. VERIFIED by absence across `bin/`, `extensions/`.

## Delivery guarantees

- **ACK**: transport-level only. A one-shot `retalk receive` is "auto-acked", and the relay deletes on delivery. `sync` "resend[s] unacknowledged mail". VERIFIED `skills/receive/SKILL.md:103`, `docs/README.md:22`, `skills/sync/SKILL.md:3,13`. There is no application or reader ACK (that a recipient read it) and no read receipt back to the sender. The requester of an invite code deliberately gets no feedback. VERIFIED `docs/README.md:34`.
- **Gap between relay ACK and model**: the follower has acked by the time a record reaches the spool, so anything that drops it afterwards loses that session's copy. The sealed history (`--save`) still holds it. INFERRED from `bin/follow.sh:103-112` and `bin/spool-writer.py:164-210`. Examples of later drops: a session not registered in `by-session` (the record goes only to the legacy file), the pi or opencode watcher starting at EOF, a monitor restart.
- **Ordering**: spool append order, i.e. retalk's output order. There is no priority, deadline or ordering key. VERIFIED by absence.
- **Durability**: sealed retalk history is durable and encrypted at rest. Session spools are bounded, plaintext and swept by GC. pi and opencode cursors are in memory only. Codex cursors persist. VERIFIED `bin/spool-writer.py:3-17,213-248`, `extensions/inbox-monitor.ts:126-144`.
- **Dedupe**: by message id at every consumer (awk map, a persisted id list of 500 for Codex, JS Sets). VERIFIED (cited above).
- **Priority, threshold, deadline**: none. Every message is pushed the same way, and on pi and opencode every message triggers a turn. VERIFIED by absence.

## Human participation

The humans are the agents' owners. They watch the model-rendered transcripts, which are always shown, "never just a summary". VERIFIED `skills/receive/SKILL.md:33-57`. They also choose auto or manual delivery in `init` and run setup steps (approve Codex hooks under `/hooks`, relaunch with env vars, use the launcher). A human could take part as a peer by driving `retalk` directly (INFERRED; there is no dedicated human UI). The README recommends Claude **auto** permission mode because send and receive are autonomous. VERIFIED `README.md:70-72`.

## License and maturity

- License: MIT (`LICENSE`, `package.json`). retalk is also MIT (GitHub API). VERIFIED.
- GitHub API, 2026-10-23: 200 stars, 17 forks, 1 open issue, created 2026-06-19, last push 2026-10-21. Releases v0.2.2 (2026-08-21), v0.2.3 (2026-08-24), v0.2.4 (2026-09-10). retalk has 12 stars and was last pushed 2026-08-16. VERIFIED (`gh api`).
- Tests: Python unit tests for the monitor, hook, wake client, spool writer, installer and follow scripts; an opt-in E2E roundtrip; CI workflow. VERIFIED `tests/`, `.github/workflows/ci.yml`. The README warns that the crypto "has not been independently audited". VERIFIED `README.md:353-357`.

## Harness support table

| Harness | Install | Injection | Idle wake | Status | Evidence |
| --- | --- | --- | --- | --- | --- |
| claude | Claude plugin marketplace (`.claude-plugin/`) | Plugin monitor (raw NDJSON lines as notifications), plus the model-started `Monitor` tool tailing the spool, plus pull | Monitor notification. Idle-start behaviour is stated inconsistently in the repo. | native | `monitors/monitors.json`, `bin/inbox-monitor.sh`, `skills/receive/SKILL.md:222-246` |
| codex | `codex plugin add` (`.codex-plugin/`) plus `install-hooks.py`, which appends to `config.toml` | `SessionStart`/`UserPromptSubmit` `additionalContext`, and `Stop` block-and-continue with the bodies | Opt-in app-server `turn/start` with a body-free nudge. Only for daemon-attached sessions with exactly one loaded thread; skipped when busy. | native | `extensions/codex/*`, `bin/codex_wake.py` |
| hermes | Not mentioned | none | none | none | grep for `hermes` over the clone: 0 hits |
| agy | `agy plugin install ./agent-talk` (reads `.claude-plugin`) | Skills only, pull via `receive` | None. Waits for ACP (antigravity-cli#31). | generic | `README.md:120-144`, `docs/antigravity-auto-receive.md` |
| goose | Not mentioned | none | none | none | grep for `goose`: 0 hits |
| pi | `pi install git:…` (`package.json` `pi.skills`/`pi.extensions`) | Extension `pi.sendMessage(…, {triggerTurn:true})` with the full body | Yes (triggerTurn). Queued while streaming. | native | `extensions/inbox-monitor.ts`, `docs/pi-auto-receive.md` |
| opencode | Symlink `skills/` into `~/.config/opencode/skills`, and copy the plugin into `~/.config/opencode/plugins/` | Plugin `client.session.promptAsync` with the full body as a user prompt | Yes (promptAsync) | native | `extensions/opencode/inbox-monitor.ts`, `docs/opencode-auto-receive.md` |
| (extra) copilot | Symlink `skills/` | Pull only | none | generic | `README.md:190-213,229` |

## Notable techniques worth stealing

1. **Codex app-server wake with a body-free nudge** (`bin/codex_wake.py`). It is a harness-native idle wake that types nothing into the TUI, checks `thread/read` for busy, and leaves body delivery to the hook path with its single cursor. The cost is that the session must be started with `--remote unix://` against a pre-started daemon, which is opt-in. VERIFIED.
2. **Codex `Stop` hook block-and-continue** to deliver mail that arrived mid-turn as soon as the turn ends, with the cursor advanced before output so the hook cannot loop. VERIFIED `extensions/codex/inbox-hook.py:23-24,523-536`.
3. **Truncate-and-refill detection** for file cursors: sha256 of the first 256 bytes alongside the offset. VERIFIED `extensions/codex/inbox-hook.py:93-108`.
4. **Nudge suppression keyed to the reader cursor**: no second nudge until the hook has consumed past the last nudge mark. VERIFIED `bin/codex_wake.py:267-287`.
5. **pi `triggerTurn` and opencode `promptAsync`** as in-process push APIs that need no keystrokes. VERIFIED.

## Flaws and risks observed

- **Contradictory Claude wake claims** (VERIFIED contradiction): `docs/README.md:44` says the monitor wakes the session, and `skills/receive/SKILL.md:169-174` says messages surface on the next turn. Which one Claude does is unknown from this repo.
- **Monitor-tool timeout above the harness ceiling** (INFERRED): the recipe passes `timeout_ms: 3600000` (1 h, `skills/receive/SKILL.md:234`), but our Claude primer records a 30 min maximum for Monitor-tool watches. The ~25 min backstop wake hints that the author ran into this.
- **pi and opencode spools may never be written** (INFERRED, high confidence): the spool writer fans out only to sids registered in `~/.agent-talk/by-session/` (`bin/spool-writer.py:107-124`). Codex init explicitly adds that registration (`skills/init/SKILL.md:632-647`). The pi and opencode steps (`skills/init/SKILL.md:578-619`) and docs only set `AGENT_TALK_*_SPOOLS=<user>/sessions/<session-id>.ndjson`, and never register the id or say where it comes from. Unless the model improvises, records go only to the legacy `inbox.ndjson`, which those extensions do not watch.
- **Watchers start at EOF and keep in-memory cursors** (VERIFIED `extensions/inbox-monitor.ts:128-135`, `extensions/opencode/inbox-monitor.ts:336-343`): anything written while pi or opencode was down is never pushed. The relay has already acked it, so recovery depends on the model running `history`.
- **Peer text with user authority**: the pi, opencode and Codex-Stop paths inject full peer bodies as user turns or continuation prompts, which contradicts the reasoning in `codex_wake.py:6-10`. VERIFIED code, INFERRED risk.
- **No bounds**: there is no per-message truncation, no page limit, and a turn per message on pi and opencode. A chatty peer can burn context and quota. VERIFIED absence.
- **Codex wake fails open** when the busy status is unreadable, and stands down with more than one loaded thread. The daemon keeps its launch environment, `--remote` sessions take the daemon's cwd, and a daemon restart kills attached sessions. VERIFIED `bin/codex_wake.py:206-220`, `docs/codex-auto-receive.md:172-192`.
- **No human-draft guard** anywhere. It matters little, because no path types into a composer, but opencode `promptAsync` and Codex `turn/start` could start a turn while the user is mid-typing. INFERRED.
- **Latency**: the relay poll defaults to 60 s, so a "live" push can be up to a minute late. VERIFIED `bin/follow.sh:15-16`.
- **Identity depends on the model** remembering `<user>` paths and writing session ids out literally. The repo's own history includes a bug where push was silently off for every session. VERIFIED `bin/inbox-monitor.sh:20-25`.
- **opencode session latching** takes the id from *any* event with `sessionID` (`extensions/opencode/inbox-monitor.ts:359-370`), so it may inject into whichever session last emitted an event, for example a subagent or child session. INFERRED.
