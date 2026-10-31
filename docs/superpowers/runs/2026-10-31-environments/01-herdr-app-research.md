# herdr-core / herdr-app vs Herdr TUI: plugin system and pane identification, and what it means for herdr-threads

Date: 2026-10-31. Read-only research; no binaries were run.

Trees:
- herdr-core `dc276b7` (crate 0.1.15). `C:` = `/var/home/alepar/AleCode/herdr-core/crates/herdr-core/src/`, `C/` = repo root.
- herdr-app `0c1a102` (0.1.10). `A:` = `/var/home/alepar/AleCode/herdr-app/`.
- Herdr TUI (herdr-upstream) `ee5bfb50` (0.9.5, protocol 22). `H:` = `/var/home/alepar/AleCode/herdr-upstream/src/`.
- herdr-threads `badccf481` (this worktree). `T:` = `/var/home/alepar/AleCode/herdr-threads/.worktrees/external-seat/`.

Companion: the herdr-threads host-coupling map, `03-ht-host-coupling.md` in this directory.

## TL;DR

1. **Herdr is now two products.** Since about 2026-10-23, "Herdr" is a native desktop app (herdr-app) that is a client of a separate headless runtime, herdr-core (`hc`). The TUI (`herdr` 0.9.5, 2026-10-14) is the old product. It is not deprecated ("Herdr TUI keeps working", https://herdr.dev/faq/), but it has had no release since 0.9.5. Whether it moves onto herdr-core is officially undecided (https://github.com/herdrdev/herdr-app/issues/63). The two do not share agents, and the app will not attach to a TUI server (same issue).
2. **Nothing herdr-threads uses from the TUI exists on the new stack.** herdr-threads depends on all of these, and none has a new-stack equivalent:
   - the `herdr-plugin.toml` plugin system (startup hook, actions, overlay pane, `HERDR_PLUGIN_STATE_DIR`);
   - `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` and `HERDR_ENV` in every pane;
   - workspace/tab/pane topology;
   - screen-detected agent status;
   - composer reads (`agent.read --source detection`);
   - raw typing (`pane.send_text`).

   herdr-app has no plugins at all. Its extension model is "mods", trusted QuickJS/GPUI JavaScript, with no startup hook, no event subscription and a 30 s `exec` cap (`A:defaults/AGENTS.md:716-828`, `A:herdr.d.ts:10-16`).
3. **Identity on the new stack is `HC_TERMINAL_ID` (`t7`), plus `HC_TERMINAL_SOCKET` and a per-spawn `HC_SCOPE`** (`C:api/run.rs:196-208`).
   - Terminal ids are core-global and never reused within a core's state (`C:terminals.rs:15,204,246-249`). They survive core restart and update, because holders own the PTYs (`C:holder.rs:1-4`, `C:runtime.rs:268-311`).
   - The id is more stable than a TUI pane id, which changes when a pane moves to another workspace (`H:workspace.rs:931-966`).
   - The core's id (`core.json`) is not in the environment. A cloned disk wipes terminals.json, so `t1` can recur under a new core id (`C:server.rs:90-115`).
   - There is no pane, tab or workspace in hc: "Core is a runtime, not a multiplexer" (`C/AGENTS.md:22-24`). The app's spaces and tabs are just labels `herdr.space`/`herdr.tab` plus a `doc.*` document (`A:defaults/AGENTS.md:95-99,136-149`).
4. **Agent status on hc comes only from OSC 7501, reported by the program itself** (`C/AGENTS.md:34-36`). There is no status-write RPC.
   - Hooks have no tty, so a hook cannot emit OSC 7501 (https://github.com/herdrdev/herdr-core/issues/42).
   - Harnesses that do not emit it are `status:null` forever, and `wait` treats them as working.
   - Whether current Claude Code and Codex emit OSC 7501 natively is not established in-tree. This is the single biggest unknown.
5. **hc ships its own agent messaging, and it conflicts with herdr-threads' delivery contract.**
   - `hc send` pastes and submits even into a *working* agent (`C:notes.rs:54-58`; `C:input.rs:156-212` refuses only `blocked`).
   - The core itself types `<hc>…</hc>` notes into parents (`C:notes.rs:71-95`).
   - The hc skill tells agents to prefer hc for "talk to or coordinate other agents" (`C/skills/herdr-core/SKILL.md:3`).
   - On the useful side, `send --if idle` is an atomic compare-and-type on the reported status word (`C:input.rs:179-193`; flag `--if`, `C/docs/commands.md:532`). It is the natural wake primitive for a herdr-threads hc host.
6. **There is a probable env leak.** herdr-core never strips `HERDR_*` vars (only grep hit: `C:releases.rs:26`). Programs get the core's own environment (`C:platform/mod.rs:65-88`, `C:api/run.rs:213-237`).
   - If a core is first auto-started from inside a TUI pane, every hc terminal may inherit that pane's `HERDR_PANE_ID` and `HERDR_SOCKET_PATH` (inferred; `C:platform/systemd.rs:215-230` keeps the caller env).
   - herdr-threads would then bind an hc-hosted agent to a foreign TUI pane.
   - Cores started by the app have no `HERDR_*` vars (herdr#5702 comment, 2026-10-30).
7. **What this means for herdr-threads.** Supporting the app means a second host backend keyed on `HC_TERMINAL_ID` + `HC_TERMINAL_SOCKET` + core id, behind `HostPort` (`T:src/ports.rs:3907-4130`). It also means fixing the CLI and hook paths that construct `NativeCli` directly and read `HERDR_*` themselves (15 sites; see `03-ht-host-coupling.md`).
   - Install and discovery cannot use a plugin manifest. A mod or a documented `hc run --keep` daemon terminal would replace `startup`.
   - Wake has no composer-empty check and no raw-type path, so draft safety (stash/restore) cannot be carried over as is.
   - Recommendation: gate the work on spike Q1 (OSC 7501 from real harnesses). Meanwhile, cheaply defend the TUI path against the env leak (Q3) and against `HC_TERMINAL_ID` callers.

## 1. Release facts and positioning

| Item | Fact | Source |
|---|---|---|
| Repos created | herdrdev/herdr-core and herdrdev/herdr-app, 2026-10-16 | GitHub API `created_at` |
| herdr-core first public release | 0.1.10, 2026-10-23; latest 0.1.15, 2026-10-30 (kept terminals survive reboot) | `C/CHANGELOG.md:5-17,56-59` |
| herdr-app | 0.1.4 on 2026-10-23 (earliest entry); 0.1.6 on 2026-10-29 added "Coming from herdr?"; latest 0.1.10 on 2026-10-30 | `A:CHANGELOG.md:3-102` |
| TUI | v0.9.5, 2026-10-14; commits until 2026-10-29; not archived; empty Unreleased | GitHub releases; herdr-upstream log |
| Positioning | "Herdr TUI is the original Herdr ... with its own server"; the two "don't share agents"; "Herdr TUI keeps working" | https://herdr.dev/faq/, https://herdr.dev/tui/ |
| Convergence | "whether the tui moves onto herdr-core later is still open" (maintainer) | https://github.com/herdrdev/herdr-app/issues/63 |
| Migration | Only a copyable agent prompt to port keybinds/settings from `~/.config/herdr/config.toml`; "nothing moves over" | `A:defaults/frame/welcome/main.js:11,274-279` |
| Plugin porting gaps reported | No `link_handlers` equivalent; hook-based status impossible | herdr-app#228, herdr-core#42 |
| Distribution | releases.herdr.dev install scripts; no GitHub Releases | `C/README.md:10`, `A:README.md:9` |
| App ships no core | Finds an installed `hc` (settings, PATH, login PATH, usual places) and talks to it through `hc connect` over stdio | `A:src/core_bin.rs:1-7`, `A:src/hc.rs:1-2,254-270` |

## 2. Environment model comparison

| Dimension | Herdr TUI (0.9.5) | herdr-core + herdr-app |
|---|---|---|
| **Identity env vars** | `HERDR_ENV=1`, `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`, `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`, `HERDR_PANE_ID`, set last so callers cannot override them. Strips `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CODEX_THREAD_ID`, etc. (`H:pane.rs:184-220`, `H:integration/env.rs:8-10,29-34`) | `HC_TERMINAL_ID=t<base30>`, `HC_TERMINAL_SOCKET`, `HC_SCOPE` (fresh per spawn), `PWD`, plus `HC_SESSION` if the core has it (`C:api/run.rs:196-224`). Base env is the **core's**, not the caller's; only TERM/TERM_PROGRAM/COLORTERM come from the caller (`C:cli.rs:320-323`, `C:api.rs:19`). No `HERDR_*` stripping (`C:platform/mod.rs:65-88`) |
| **Who sets the socket var, and what it does** | `HERDR_SOCKET_PATH` routes the CLI and wins over `HERDR_SESSION` (`H:session.rs:84-103`) | `HC_TERMINAL_SOCKET` only decides whether `HC_TERMINAL_ID` is trusted (`C:client.rs:249-253`). Routing uses `HC_SESSION`/`HC_SOCKET`/config (`C:paths.rs:17-35`) |
| **Id format** | Pane `<ws>:p<n>`, tab `<ws>:t<n>`, ws `w<n>`, bijective base-32 (`H:workspace.rs:104-149`); terminal `term_<hex>` (`H:terminal/id.rs:15-23`) | `t` + base-30 without vowels/`l` (`C:terminals.rs:161-200`); names 1-32 chars, unique among live terminals (`C:addr.rs:6-15`); remote `box/t7` (`C:addr.rs:28-45`) |
| **Id stability** | Pane id is persisted across restore (`H:persist/restore.rs:159-171,327-403`). It **changes on a cross-workspace move**; the alias is not persisted (`H:workspace.rs:931-966`, `H:app/ids.rs:107-152`). `terminal_id` is reallocated on every restore and live-handoff import (`H:persist/restore.rs:593,689`) | `tN` is never reused in a core's state (`C:terminals.rs:204,246-249`). It survives core restart, update and crash through holders (`C:holder.rs:1-15`, `C:runtime.rs:268-311`). `--keep` survives reboot with the same id, but a **new shell replaces the program** and pid and `HC_SCOPE` are new (`C:kept.rs:43-114`). A cloned disk gives a new core id and wipes terminals (`C:server.rs:90-115`) |
| **Remote identity** | `herdr machine` / SSH targets; CLI ignores `HERDR_PANE_ID` for remote targets (`H:cli/target.rs:207-214`) | Inside a remote terminal `HC_TERMINAL_ID` is the bare local `t3`; the parent sees `vps/t3` (`C:remote.rs:1070-1098`) |
| **Topology API** | `session.snapshot`, `pane.list/get/current`, workspaces/tabs/layouts (`H:api/schema.rs:45-147`, `H:api/schema/panes.rs:499-541`) | `terminal.list/get` with labels, parent, `children_of`; no panes, tabs or workspaces (`C:api.rs:56-150`, `C/AGENTS.md:22-24`). App topology lives in a `doc.*` CAS document (1 MiB, `C:api/doc.rs:4-38`) and the labels `herdr.space`/`herdr.tab` (`A:defaults/AGENTS.md:136-149`) |
| **Events** | Typed `events.subscribe`; a retained window of 512 events, then a hard `events_lost` error (`H:api/event_hub.rs:19`, `H:api/subscriptions.rs:329-339`) | `events.subscribe` returns a snapshot, then `created/lost/restarted/keep/finished/removed/status/turn/title/labels/size/foreground/doc`, a 15 s heartbeat, and `missed` with an inline resnapshot. `seq` resets per core process (`C/schema/README.md:83-148`, `C:events.rs:13-14,149-160`) |
| **Input / wake** | `pane.send_text` (raw, no submit), `pane.send_keys`, `pane.send_input`, `agent.prompt` (bracketed paste + Enter, refuses blocked/not ready) (`H:app/api/panes.rs:1912-2057`, `H:app/api/agents.rs:82-270`) | `terminal.send` = paste + submit, always. It waits 3 s after human typing (`HC_QUIET_MS`), refuses `blocked` unless `force`, and supports `if: idle\|working` CAS (`C:input.rs:65-212`). `terminal.keys` sends named keys (`C:input.rs:248-260`). **No raw type without submit**. Raw DATA on an attach channel counts as human typing (`C:viewers.rs:197-204`) |
| **Composer read** | `agent.read --source detection` / `recent` with ANSI (`H:api/schema/common.rs:86-100`) | `terminal.read` gives plain text only, stitched for full-screen apps but skipped while working (`C/schema/README.md:61-81`); styled bytes only via framed `terminal.attach observe:true` |
| **Agent status** | Screen-regex manifests per harness, arbitrated with `pane.report_agent` hook reports; `Done` = idle and unseen (`H:detect/mod.rs:1-4`, `H:app/api_helpers.rs:96-105`) | OSC 7501 only; free-form words; no RPC to report; `status:null` if never reported (`C:status.rs:1-80`, `C/AGENTS.md:34-36`) |
| **Foreground process** | `pane.process_info` (shell pid, pgid, tty, all fg processes) (`H:api/schema/panes.rs:544-570`) | `foreground` field/event: pgrp leader only, polled every 500 ms (`C:foreground.rs:154-174`) |
| **Extension model** | `herdr-plugin.toml` with build/startup/actions/events/panes/link_handlers; subprocess entry points; ~2000 community plugins (`H:app/api/plugins/manifest.rs:11-140`) | Mods: `~/.config/herdr-app/mods/<name>/main.js` (+`mod.json`), QuickJS + GPUI. No manifest, installer or startup hook; reactive re-render instead of events; `exec` capped at 30 s/8 MiB (`A:defaults/AGENTS.md:716-828`, `A:src/mods.rs:698`). herdr-core has no extension concept |
| **Install / state** | Registry `~/.config/herdr/plugins.json` and state `~/.local/state/herdr/plugins/<id>`, both per user and shared across named sessions (`H:persist/plugin_registry.rs:11-13`, `H:plugin_paths.rs:44-54`) | Core state `$XDG_STATE_HOME/herdr-core/<session>`; socket `$XDG_RUNTIME_DIR/herdr-core/<session>.sock` in a 0700 dir (`C:paths.rs:17-55`). App home `~/.config/herdr-app`; per-mod `localStorage` under `data/<mod>/` (`A:src/mods.rs:47-55,1084-1100`) |
| **Wire** | Newline JSON `{id,method,params}`, one request per connection; `PROTOCOL_VERSION=22`, exact match required by its CLI (`H:protocol/wire.rs:20`, `H:cli/protocol_guard.rs:16-40`) | JSON-RPC 2.0 lines, or framed mode (`C:wire.rs:7-47`); OpenRPC schema with a CI test that the API only grows (`C/tests/compat.rs:126`); `GENERATION=1` (`C:api.rs:15`) |
| **Auth** | Socket 0600 (`H:server/socket_paths.rs:11`); no per-caller authorization | 0700 parent dir, no peer-credential check (`C:paths.rs:50-55`); "Core is not a sandbox" (`C/docs/security.md:1-25`) |
| **Multi-machine** | `herdr machine` | `hc machine add`: one framed JSON-RPC line over `ssh -T` BatchMode or `--via`, ControlMaster login, peers outbox, 30 s backoff (`C:machines.rs:52-233`, `C:remote.rs:197-199,548-718`). A dialed core only lets peers call `send/get/read/wait` on terminals that started work there (`C:remote.rs:548-587`) |

## 3. Per-capability mapping

For each TUI capability herdr-threads uses (sources in the herdr-threads column), the herdr-core/app equivalent or the gap. Status: **=** close equivalent, **~** partial, **x** gap.

| # | TUI capability | herdr-threads use | herdr-core / app equivalent | Status |
|---|---|---|---|---|
| 1 | `herdr-plugin.toml` manifest, `herdr plugin link` | `T:herdr-plugin.toml`, `T:scripts/install.sh` | None. A mod folder copied by hand (`A:defaults/AGENTS.md:410-442`) | x |
| 2 | `[[startup]]` hook runs `view.sh ensure` at server start and after handoff (`H:server/headless/bootstrap.rs:97,210`) | daemon autostart | None. Options: a mod that calls `hc(["run","--bg","--keep",…])`, which `close()` would kill (`A:defaults/AGENTS.md:818-828`); a user-run `hc run --keep --outlive-parent herdr-threads daemon` (inferred); or herdr-threads' own lazy start from the CLI | x |
| 3 | `[[actions]]` health/doctor/ensure/stop/view | operator actions | `<mod>.<command>` via keys.json → `nextCommand()`; not listed in `commands` or the palette (`A:herdr.d.ts:75,233-235,390-394`) | ~ |
| 4 | `[[panes]]` overlay `operator` + `herdr plugin pane open` (`T:scripts/view.sh:10-11`) | operator view | Mod pane/overlay (`openOverlay`, `pane.split {mod}`), or `terminal.open {argv}` / an `hc run` terminal labeled `herdr.space`/`herdr.tab` (`A:herdr.d.ts:52-58,327-363`) | ~ |
| 5 | `HERDR_PLUGIN_STATE_DIR` | state dir (`T:src/cli/instance.rs:75,99-107`) | None. herdr-threads must choose its own XDG state dir | x |
| 6 | `HERDR_PANE_ID` / `HERDR_ENV` in every pane | caller identity (`T:src/cli/mod.rs:311`, `T:src/cli/hook.rs:255-262`) | `HC_TERMINAL_ID` validated by `HC_TERMINAL_SOCKET` (`C:client.rs:249-253`); no core id in env | ~ |
| 7 | `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH` | host locator (`T:src/daemon/paths.rs:136-140`, `T:src/protocol/ids.rs:300-345`) | `HC_TERMINAL_SOCKET` (a real socket path, so `HostLocator` can hold it); `hc` on PATH | = |
| 8 | `ping` → version + protocol + capabilities | admission (`T:src/host/compatibility.rs:19-25,173-200`) | `hello` → generation, version, core {id,name}, methods[], features[] (`C:api.rs:168-184`); `server.status` | = |
| 9 | `pane.current {caller_pane_id}` | resolve caller | `terminal.get {terminal: $HC_TERMINAL_ID}` | = |
| 10 | `pane.get` → workspace/tab/terminal/agent_status/agent/agent_session/cwd | target observation (`T:src/host/observation.rs:118-170`) | `terminal.get` → address, name, argv, cwd, pid, state, status (7501), labels, parent, foreground, template. **No agent kind or session id** unless reported in 7501 `app` or chosen by template | ~ |
| 11 | `session.snapshot` | `enumerate_targets` / reconcile (`T:src/identity/reconcile.rs:538`) | `terminal.list {all}` or the `events.subscribe` snapshot; boot/epoch analog = core id + core pid/uptime from `server.status` (inferred) | ~ |
| 12 | `pane.process_info` → foreground pids | foreground pids (`T:src/host/native.rs:740,1309`) | `foreground {pid,name,argv,cwd}`, group leader only (`C:foreground.rs:23-31`) | ~ |
| 13 | `agent.get` → kind, status, launch_pending | wake gating (`T:src/host/native.rs:643,1237,1966`) | `status` word from 7501 or null; no kind detection | ~ (x if harness emits no 7501) |
| 14 | `agent.read --source detection` / `recent` ANSI → composer | `ComposerPolicy`, empty-composer check, draft stash | `terminal.read` plain text; styled bytes only via observe attach. No detection source | x |
| 15 | `pane.send_keys ctrl+u` + `pane.send_text` | stash/restore composer (`T:src/host/native.rs:1562,1603,2361`) | `terminal.keys ["ctrl+u"]` exists; **raw text without submit does not**. Attach DATA counts as human (`C:viewers.rs:197-204`) | x (restore) |
| 16 | `agent.prompt` | content-free wake submit (`T:src/host/native.rs:2041`) | `terminal.send {text, if:"idle"}`: paste + submit with a human-quiet gate (`C:input.rs:113-212`) | = (better CAS) |
| 17 | `agent.send_keys enter` | submit key (`T:src/host/native.rs:1693`) | `terminal.keys ["enter"]` | = |
| 18 | `agent.start {name,kind,pane_id,args}` | managed launch (`T:src/host/native.rs:1108-1128`) | `terminal.run {argv, cwd, env, labels, keep, template}`, optionally `--outlive-parent` | = (different shape) |
| 19 | `tab.create {workspace_id,cwd,label,env}` | handoff/new tab (`T:src/host/transport.rs:776-786`) | `terminal.run` + labels `herdr.space`/`herdr.tab` makes it show in the app (`A:defaults/AGENTS.md:136-149`) | ~ |
| 20 | Workspace/tab/pane labels | `SeatHostLabels`, `seat list`, peer `seat_status` (`T:src/host/observation.rs:283,448`, `T:src/sync/frame.rs:412-430`) | terminal `name`, `title`, labels; app space/tab names only in the `doc.*` doc (schema owned by app, `A:src/mirror.rs:1`) | ~ |
| 21 | Incarnation witness: peer pid/uid/start (`T:src/host/native.rs:104-130`) | epoch | Same kernel peer-cred technique works on the hc socket; plus core id | = (inferred) |
| 22 | `herdr plugin list --json`, `herdr status server --json` fallbacks (`T:src/cli/setup.rs:279-339`) | discovery | `hc server status` / `hello`; socket path from `C:paths.rs` rules | = |
| 23 | `herdr machine list` peer discovery (`T:src/sync/discovery.rs:119-175`) | `MachineLister` | `machine.*` list in `terminal.list` reply (`machines[]`) / `hc machine list` | = (inferred) |
| 24 | Herdr events | not used by herdr-threads | `events.subscribe` would let the observation lane drop polling | n/a (upside) |

## 4. Risks

**R1. `HERDR_*` leak into hc terminals (correctness; affects herdr-threads today).**
- Mechanism: the core never strips `HERDR_*` (`C:platform/mod.rs:65-88`; `C:api/run.rs:213-237` strips only TERM identity and `HC_*`). An auto-start from a CLI call carries the caller's env into the systemd unit (`C:platform/systemd.rs:83-90,215-230`).
- Effect: if `hc` is first used from a TUI pane, every hc terminal (agents included) may carry that pane's `HERDR_PANE_ID`/`HERDR_SOCKET_PATH`/`HERDR_ENV`. herdr-threads' hooks and CLI would then treat an app-hosted agent as that TUI pane. They would attribute and bind to it, or refuse with a target mismatch (`T:src/cli/hook.rs:255-262`) (inferred).
- Defensive rule (inferred): when `HC_TERMINAL_ID` is set and `HERDR_PANE_ID` does not resolve to a pane whose pid ancestry contains the caller, do not bind. This needs a TRUST-POLICY check; it is evidence-based refusal, not a heuristic move.

**R2. Delivery-contract conflict.**
- `hc send` and core notes type into working agents (`C:notes.rs:54-58`), which violates herdr-threads rule 4.
- Two framing families (`<hc from=…>` and `[herdr-threads]`) would reach the same context.
- `notes = false` turns hc's notes and tags off globally (`C/docs/commands.md:59-60`), at the cost of losing hc's blocked/exited notes.

**R3. Skill trigger contention.** `C/skills/herdr-core/SKILL.md:3` says to prefer hc for coordinating agents. The herdr-threads skill (`T:integrations/skill/SKILL.md:3,27`) says "Requires running inside your own Herdr pane", which is false for app users. Models will route messaging to `hc send` (inferred).

**R4. Status dependency on OSC 7501.**
- If Claude Code/Codex do not emit 7501, `status` stays null, `send --if idle` always fails `status_mismatch`, and there is no idle wake at all.
- Hooks cannot help because they have no tty (herdr-core#42).
- The TUI path worked through screen detection, which hc forbids by project rule (`C/AGENTS.md:34-36`).

**R5. Draft safety regresses.** There is no composer read with detection semantics and no raw re-type, so herdr-threads' stash/restore (`T:src/ports.rs` `stash_composer`/`restore_composer`) cannot be implemented.
- The mitigating factor is `send`'s 3 s human-quiet gate, and keys typed during the paste are held and flushed after (`C:input.rs:22-52,143-149`).
- A typed but not-yet-submitted draft would be submitted together with the wake text (inferred).

**R6. Identity edge cases.**
- After a `--keep` reboot recovery, the id is the same but the program is a different one, a shell (`C:kept.rs:82-114`). A seat bound to `t7` must not survive that as the same agent; key on `HC_SCOPE` or pid as the incarnation.
- A cloned disk reuses `t1` under a new core id (`C:server.rs:90-115`). Seats must key on (core id, tN).
- A remote child's `HC_TERMINAL_ID` is bare `t3` (`C:remote.rs:1070-1098`).

**R7. No daemon home in the app.** There is no startup hook, and mod processes die with the mod copy or are capped at 30 s. A herdr-threads daemon needs a lifecycle independent of the app, such as a lazy CLI start or a kept hc terminal (inferred).

**R8. TUI-side mismatches, independent of the app.** Already present today (`H:`/`T:` cites from the reader report):
- (a) A cross-workspace `pane.move` breaks a bound seat: alias answers are refused as a target mismatch (`T:src/host/observation.rs:212-219`, `H:app/ids.rs:146-152`).
- (b) Plugin action and startup commands get the *focused* pane as `HERDR_PANE_ID` (`H:app/api/plugins/runtime.rs:100-102`). The daemon spawn does not scrub it (`T:src/daemon/lifecycle.rs:356-390`).
- (c) Live handoff reallocates `terminal_id` (`H:persist/restore.rs:689`).
- (d) The `[[build]]`/`[[startup]]` `id` keys in `T:herdr-plugin.toml` are ignored (`H:app/api/plugins/manifest.rs:36-48`).
- (e) The comment at `T:src/host/native.rs:283-286` claims `agent.start` waits for readiness. It returns `launch_pending` (`H:app/agents.rs:212-235`).
- (f) `T:src/cli/instance.rs:57,248` assume per-session registries. Upstream has one registry per XDG home.
- (g) Hermes managed launch always refuses on upstream: `agent_start_process_hint_v1` does not exist (`T:src/host/transport.rs:576-585`).

**R9. Product direction is unsettled** (herdr-app#63). A dual backend may become a single hc backend, or the TUI may move onto core and carry 7501. Avoid over-investing before that is decided.

## 5. Open questions to settle by spikes on a real install

All spikes use an isolated `HC_SESSION=ht-spike-<uuid>`, scratch `XDG_STATE_HOME`/`XDG_RUNTIME_DIR`/`XDG_CONFIG_HOME`, scratch `HOME`/`CLAUDE_CONFIG_DIR`/`CODEX_HOME` where harnesses run, and a private Herdr test session (`T:scripts/lib/isolated-herdr.sh`) where the TUI is needed. Stop the spike core with `hc server stop` under the same `HC_SESSION`. Never touch the shared Herdr server or `~/.local/state/herdr`.

1. **Do Claude Code and Codex emit OSC 7501 natively?** Run each in `hc run --session … -- claude` / `-- codex`. Drive one turn with `hc send`, then poll `hc get tN --json` for `status`, `reported` and `turn` across idle → working → idle, and capture `hc events` concurrently. Also grep the harness binaries/bundles for `7501`. Decides whether an hc host can wake at all (R4).
2. **Can a hook reach OSC 7501 at all?** In the scratch Claude config, add a `UserPromptSubmit`/`Stop` hook that writes `\e]7501;…\a` to `/proc/$PPID/fd/1`, to the controlling tty found via `ps -o tty= $PPID`, and to `/dev/tty`. Record which write `hc get` sees. Confirms or refutes herdr-core#42 for a herdr-threads hook shim.
3. **HERDR_* leak.** In a private Herdr test session pane, with no spike core running, run `HC_SESSION=ht-spike hc run --bg -- env`, then `hc read` and look for `HERDR_PANE_ID`/`HERDR_SOCKET_PATH`/`HERDR_ENV`. Repeat with the core first started from a plain shell. Then run `herdr-threads hook` resolution (dry, against a scratch state dir) inside the leaked terminal and record what it binds to (R1).
4. **`send --if idle` as wake.** With an emitting agent idle and a typed draft in its composer, run `hc send tN --if idle "herdr-threads: wake up"`. Record whether the draft is submitted with it, whether the 3 s quiet gate held, and the `started` field. Repeat while working (expect `status_mismatch`, nothing typed) and while blocked (expect `blocked`). Decides R5 and invariant 4 compliance.
5. **Composer emptiness without detection.** On an idle Claude and an idle Codex under hc, compare `hc read tN --lines 12` with and without a typed draft. Check whether a per-harness `ComposerPolicy` can parse plain text reliably, and whether an observe attach's first paint gives styled bytes. Decides whether draft safety can be rebuilt.
6. **Identity across lifecycle.** For a `--keep` terminal running `sleep infinity`, record `HC_TERMINAL_ID`, pid, `HC_SCOPE` and core id (`hello`) across `hc server restart`, `hc update` (skip if no newer build), core SIGKILL, and a simulated holder loss. Check which fields change and which events (`lost`/`restarted`/`finished`) fire. Defines the seat incarnation key (R6).
7. **Remote child addressing.** With `hc machine add` to a local container or `--via` loopback, run a child `--on m`. Record its `HC_TERMINAL_ID` and whether the `reach=` attribute and parent notes are rewritten with `calls_me` (`C:remote.rs:734-737`, `C:notes.rs:157-158`). Decides cross-machine seat ids.
8. **Event stream robustness.** Hold a plain-mode `events.subscribe` connection across `hc server restart` and a burst of more than 1024 events (spawn/kill loop). Confirm the connection drops, `seq` resets and the `missed` + snapshot behaviour. Decides whether the observation lane can be event-driven.
9. **Unknown-param tolerance.** Call `terminal.get`, `terminal.send` and `hello` with an extra field `{"ht_probe":1}` over the socket. Confirm they are accepted. Needed for version-tolerant adapter calls.
10. **Daemon lifecycle under the app.** In a scratch `HERDR_APP_HOME`, write a minimal mod whose `init` runs `hc(["run","--bg","--keep","--outlive-parent","--label","ht.daemon=1","--","sleep","600"])` guarded by an `hc ls --label ht.daemon=1` lookup. Hot-reload the mod, switch layouts and quit/restart the app. Record whether the worker survives and is found again rather than duplicated (R7).
11. **App topology readability.** In the scratch app, create two spaces and tabs with terminals. Read `hc doc get` and `hc ls --json` labels, and record whether space/tab names and the terminal→tab mapping can be recovered without the app. Decides `SeatHostLabels` on hc.
12. **Double delivery.** Start a parent agent and a `notify` child under hc, and also deliver a herdr-threads item to the parent. Count injected texts in the parent's context with `notes` on and off. Decides the `notes = false` recommendation (R2).
13. **Skill contention.** With both skills installed in a scratch Claude config, ask the agent to "message the reviewer agent". Record whether it uses `hc send` or `herdr-threads send` (R3).
14. **Claude mod/channel inside an app pane.** Run the herdr-threads Claude mod setup into a scratch `CLAUDE_CONFIG_DIR` and start Claude inside an hc terminal with no `HERDR_*` vars. Record what `setup`/hooks detect and whether the channel can wake the session itself (`Channel` wake needs no host typing). This may give a host-independent wake path for Claude.
15. **TUI pane move and handoff (R8a/c).** In a private Herdr test session, bind a seat, then `pane.move` it to another workspace and run `server live-handoff`. Record whether re-observation via `pane.current` with the old id recovers the seat or whether it is lost.
16. **Convergence watch.** Not an experiment: track herdr-app#63, herdr discussion #5684 Q7 and herdr-core#42 before committing to a dual backend (R9).
