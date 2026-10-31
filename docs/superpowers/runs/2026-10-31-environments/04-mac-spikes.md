# Mac spike results: herdr-core 0.1.15 / herdr-app 0.1.10 (2026-10-31, mbp-alepar)

> **Spike 2 below resolves the composer question:** `hc send --if idle` appends to and SUBMITS a typed draft (Claude and Codex report `idle` with a draft). No composer field in RPC; only `terminal.read` screen text shows it. Wake design must check the composer from screen text (as the TUI composer policies do) before sending.

Run by the mac tab agent (seat sQ0VRKjSectV); evidence on the laptop in ~/ht-spikes/herdr-app/. Verbatim report:

herdr-app spike results. Evidence: ~/ht-spikes/herdr-app/ on the laptop (mbp-alepar), 00-summary.md plus one file per step. All tests ran in an isolated core (HC_SESSION=htspike, now stopped). Step 8 used the default core for a test space, which is now closed. The default core (pid 45676) predates me and was not touched; it still holds only the user's finished t1. Herdr TUI not touched.

1 Install (01-install.txt). Both were already installed on 10-30/31 at the latest versions (hc 0.1.15, herdr-app 0.1.10), so I didn't re-run the installers; I read them. Paths: ~/.local/bin/herdr-core (hc is a symlink to it). The app is in /Applications/Herdr.app, not ~/Applications; it is Developer-ID signed (Herdr, Inc.) and ~/.local/bin/herdr-app links to it. Nothing is in launchd: the core is a `herdr-core serve` daemon (ppid 1) that any hc call autostarts, plus a `hold --socket ...holder.<x>.sock` process that owns the PTYs. State is in ~/.local/state/herdr-core/<session>/: <session>.sock, core.json (id, name, machine_id), terminals.json, docs.json, screens/, core.log. App home is ~/.config/herdr-app/ (herdr-app.sock, AGENTS.md, herdr.d.ts, mods/). Surprise: `hc server status` autostarts a core on macOS.

2 Help (skill-hc.txt, skill-app.txt, help-hc.txt, help-subcommands.txt, help-app.txt, schema-hc.json = full OpenRPC).

3 Identity (03-identity.txt). Terminal env: HC_TERMINAL_ID=t<N>, HC_SESSION, HC_TERMINAL_SOCKET=<state>/<session>.sock, HC_SCOPE=<digits>-<hex32>. Nothing else is hc's own; TERM/TERM_PROGRAM come from the CLI's terminal.
- The address tN ("this terminal forever on this core") stayed the same across attach/detach, --keep recovery after kill -9 (the replacement shell is the same t3), `hc server restart` (same pids, holder adopted) and `server stop`+`start` (kept t3 gets a fresh shell; non-kept ones finish with reason=server_stop).
- HC_SCOPE survived keep recovery and restart, but changed after stop+start (a new process tree). It looks like a process-tree/run scope, not identity.
- Big surprise: a terminal's env is the CORE process env, not the caller's. The core is autostarted by the first hc call, so it froze that caller's env. Started from an agent's Herdr pane, every hc terminal inherited HERDR_PANE_ID=w3V:p1, HERDR_AGENT, HERDR_SOCKET_PATH, CLAUDE_CODE_SESSION_ID and CLAUDE_CODE_MESSAGING_SOCKET/TOKEN, among others. A FOO var set in the caller did not arrive. The RPC terminal.run `env` param is added on top of that; the CLI has no flag to unset.
- No pane id in env for app panes either: the app pane id (p<hex>) exists only in the app layout.

4 Input/status (04-input-status.txt).
- `hc send <t> <text>` pastes and presses Enter. It waits for ~3 s of no human typing (one send took 5.2 s). It refuses a blocked program (exit 9, kind=blocked) unless --force. `--if idle|working` types only when the status matches (exit 10 status_mismatch otherwise). It returns {started: bool|null, status, turn}; started=null means the program reports no status, so nobody can tell.
- `hc keys <t> enter esc ctrl+c ...`, `hc interrupt`, `hc wait <t>` (until not working) and `send --wait`.
- Status is NOT inferred. Only the program reports it, with OSC 7501: `ESC]7501;state=<idle|working|done|blocked|error>[:kind=permission|question|auth][:msg=..]BEL`, plus `state=clear`. I set it from a plain shell with printf, and `send` honoured it. Terminal fields: status{state,app,kind,msg,progress}, status_ms, turn (++ each time it leaves working), reported. A query, `OSC 7501;?`, gets the reply `7501;?:states=...:kinds=...`, which is how agents detect support.
- There is no field or API for "input line empty"; only the status word and `hc read` screen text.

5 Events (05-events.txt, 05-rpc-methods.txt). The socket API is JSON-RPC 2.0, one message per line, on <state>/<session>.sock (dir 0700, no auth beyond that), or framed after hello{framed:true}. The method list is in the file; it includes hello, terminal.run/get/list/send/keys/wait/read/attach/label/keep/kill/rm, events.subscribe, doc.get/doc.set (opaque versioned JSON docs, ≤1 MiB), machine.*, file.* and rpc.discover.
- `events.subscribe {terminal?|labels?|children_of?}` returns a snapshot+seq, then `event` notifications typed created, foreground (foreground process changes), status, turn, title, labels, size, finished, lost, restarted, keep, removed and doc. It also sends `heartbeat` every 15 s, and `missed`+snapshot when a subscriber falls behind. The CLI form is `hc events --json`.
- A daemon can connect like ours does to herdr.sock. I tested hello + subscribe from python.
- `hello` has core/calls_you for core-to-core links (remote terminals are named `machine/tN`).

6 Agents (06-agents.txt). The isolated core was restarted from `env -i` so nothing leaked.
- Claude Code 2.1.316 (`env CLAUDE_CONFIG_DIR=claude-1 claude`): status stayed null at the folder-trust dialog (not "blocked"). After I accepted it, status became {app: claude-code, state: idle}, title "Claude Code". That is native: the Claude binary has the "probe: OSC 7501" strings, and no hooks are involved.
- Codex 0.165 (`cx --no-daemon`): status {app: codex, state: idle}; the binary has "]7501;state=" and "state=clear".
- Inside each there was only HC_* and TERM*. HERDR_AGENT=codex came from the aisw launcher, not hc.
- Both quit via `hc keys ctrl+c ctrl+c` with exit 0.
- The hc skill says Codex must run with `--no-daemon`, or its commands run outside hc.

7 herdr-threads 2.8.0 in an hc terminal (07-herdr-threads-in-hc.txt).
- `me` prints usage; it needs a subcommand (`me init` writes, so I skipped it).
- `inbox`: "this command acts as a seat: run it inside the agent's own Herdr pane (HERDR_PANE_ID)...", exit 2.
- `diagnostics`: "defaults to the caller's seat: pass --seat SEAT, or run it inside ... (HERDR_PANE_ID)", exit 2.
- `view`: continuous view unsupported, use --once (exit 4).
- `doctor` works (degraded only for the pre-existing legacy-data/Codex hook-review issues) because it doesn't need a pane.
- So we have no seat resolution without HERDR_PANE_ID. And the leaked-env case in step 3 is worse than a refusal: an hc terminal whose core was started from a Herdr pane would resolve as THAT pane's seat.

8 App (08-app.txt, 08-app-shot.png).
- I opened it with `open -g`. `herdr-app ls/status/do` talk to ~/.config/herdr-app/herdr-app.sock.
- Mapping: one pane = one core terminal. `space.new` returned {space, tab, terminal: t2}, and `pane.split` returned {pane, terminal: t3}. Each terminal has labels herdr.space=<sp..>, herdr.tab=<tb..> and where="<space> / <tab>", with keep=true.
- The layout (pane id ↔ terminal) is stored in the CORE as doc "herdr-app" (doc.get), in a spaces→tabs→root tree of blocks {id: p.., content: {terminal: tN}}. An hc terminal started with label herdr.space=<id> (and optionally herdr.tab) appears in the app; unlabeled ones stay hidden.
- Mods are trusted JS (gpui-kit) in ~/.config/herdr-app/mods/<name>/main.js + mod.json ({place, title}). There is no registration or link command: drop the folder in, run `herdr-app check <dir>`, and save to reload. mod.clone exists for the shipped ones.
- Mod API (herdr.d.ts): exec(cmd, args, {cwd, on}) has a 30 s limit and 8 MiB of output, so a mod can't host a long-running process directly. A mod could start one as a hidden core terminal with `hc run --bg --outlive-parent`.
- There is also hc(args), live terminals()/agents()/panes()/spaces() (render reruns on change; "don't poll"), run(appCommand), notify(), shared state, watchFolder, fetch, and fs writes under ~/.config/herdr-app.
- Mods get no raw core event subscription; they get only the live data views. A daemon would use events.subscribe on the core socket.

Takeaways for us:
(a) Identity is HC_TERMINAL_ID + core id (core.json / hello) + HC_SESSION; we could key seats on them, and they are stable across keep/restart/stop-start.
(b) Wake: `hc send --if idle` gives a race-free idle-only wake, plus `blocked` refusal, for any OSC 7501 reporter. Claude and Codex both report natively.
(c) Events: events.subscribe (status/turn/foreground/finished/removed) replaces Herdr's event stream.
(d) Danger: HERDR_* and CLAUDE_CODE_* vars leak through the core env, so we must prefer HC_TERMINAL_ID over an inherited HERDR_PANE_ID when both are present.

Leftovers: ~/.local/state/herdr-core/htspike/ (state of the stopped test core; delete if you like). The folder ~/ht-spikes/herdr-app is now trusted in the claude-1 profile.

## Spike 2 (RESOLVES the open composer question: hc send --if idle CLOBBERS drafts; see C)

Spike 2 results. Evidence: ~/ht-spikes/herdr-app/spike2-{A..E}.txt on the laptop. Test cores htspike2/2b/3 and every terminal I started are stopped. The default core (pid 45676) was only used for E's app test space, now closed. One slip: my first D start command put HC_SESSION outside `env -i`, so it reached the default core. It only returned that core's status: same pid, uptime 10h46, not restarted.

A Process↔terminal mapping.
- terminal.get, terminal.list (verbose) and the events snapshot share one Terminal shape: address, argv, pid, foreground{pid, name, argv, cwd}, parent, labels, state, status, turn, and more.
- `pid` is the program's root pid. `foreground.pid` is the foreground process group's leader (= pgid; schema: "The foreground process group's leader"), updated by `foreground` events.
- Not exposed: tty/pty path, session id, holder pid per terminal, and any descendant list. Terse `hc ls` gives only address/agent/cwd/name/state/turn/where.
- Proof for an arbitrary pid:
  (1) Ancestor walk. A grandchild `sleep 300` has ppid = the terminal's pid; the root's parent is `hc hold --socket <state>/<session>.holder.<x>.sock`, whose parent is `hc serve`.
  (2) Controlling tty. ps shows ttys028 for the root and the grandchild, even though the grandchild's fd0 was /dev/null. The core doesn't report a tty, so a client compares it with `ps -o tty= -p <terminal.pid>`.
  (3) A double-fork+setsid daemon broke both: ppid=1, tty ??. Only its inherited HC_TERMINAL_ID=t2 still named the terminal. So there is no proof for detached descendants; the env is the only trail.

B Leaks.
- An hc terminal's env holds HC_SCOPE, HC_SESSION, HC_TERMINAL_ID and HC_TERMINAL_SOCKET. A grandchild keeps them, and so does the setsid daemon above. A server started from inside an hc terminal therefore carries HC_TERMINAL_ID=tN forever.
- A core autostarted from this Herdr TUI pane (`HC_SESSION=htspike2b hc run --bg`, no env -i) gave its terminal all 7 HERDR_* vars, identical to the caller pane: HERDR_AGENT=claude, HERDR_BIN_PATH, HERDR_ENV=1, HERDR_PANE_ID=w3V:p1, HERDR_SOCKET_PATH, HERDR_TAB_ID=w3V:t1, HERDR_WORKSPACE_ID=w3V.
- It also gave 14 CLAUDE*/AISW* vars, among them CLAUDE_CODE_SESSION_ID, CLAUDE_PID, CLAUDE_CODE_MESSAGING_SOCKET/TOKEN, CLAUDE_CODE_PLUGIN_DIRS (our claude-mod!), CLAUDECODE=1 and CLAUDE_CONFIG_DIR.

C Composer safety.
- Neither `hc send --no-enter` nor a non-submitting paste exists. Drafts were typed via `hc attach` (script + printf "draft text", ctrl+\).
- With a draft in the composer, both Claude 2.1.316 and Codex 0.165 still report status idle.
- `hc send --if idle t "wake up"` returned {started: true, status: idle} in 0.24 s. The quiet rule waits only for ~3 s without keystrokes, which had already passed. Both agents APPENDED the wake to the draft and SUBMITTED "draft textwake up". No refusal, no wait.
- No RPC field shows composer contents. Only terminal.read screen text does: Claude "❯ <text>", Codex "› <text>" vs the placeholder "› Ask Codex to do anything".
- Side finding: Claude in a core started with only HOME+PATH came up as blocked kind=auth "Not logged in". Keychain needs USER etc.; the spike-1 core with USER/LOGNAME/SHELL/TMPDIR logged in. So `env -i HOME PATH` is too thin for a core that hosts Claude.

D Multiple cores.
- Several run at once per user: default + htspike2 + htspike3 were simultaneous `serve` processes.
- No RPC enumerates sessions. Enumerate ~/.local/state/herdr-core/*/ (core.json = {id, name, machine_id}; socket <dir>/<session>.sock). Stale dirs stay after stop, so the socket is gone but core.json remains.
- Liveness: connect to the .sock and send `hello` (returns core{id, name}, version, methods, features). Don't use `hc server status`, which autostarts a core on macOS.
- Each session has its own core id; machine_id is shared and name = hostname.
- The core id stayed the same across `server restart` and `stop`+`start` (htspike2: 32548e1c… throughout; spike 1 htspike: 2b2e547f… across restart + 2× stop/start).
- `hc update` was not run (it replaces the user's binary). Per the skill it is a binary swap + planned restart, and the id lives in core.json, which survived restart.

E App labels.
- App terminals carry labels herdr.space=<sp…id>, herdr.tab=<tb…id>, and `where`="<space name> / <tab name>". `where` also appears as a top-level field in terse `hc ls --json`.
- The app rewrites `where` live on rename (labels events seen: "ht-spike2 / fix login" → "ht-spike2 / fix login 2" → "ht-spike2-renamed / fix login 2").
- Split on " / " is ambiguous if a name contains it. Better: resolve ids against doc "herdr-app" (doc.get).
  - Doc shape: {spaces: [{id, name, folder, machine?, tabs: [{id, name, root: {type: row|column|block, id, children: [{type: block, id: p…pane id, content: {terminal: tN}}]}}]}]}.
  - Every app change bumps the version and emits a `doc` event (name herdr-app, version).
- An unnamed tab has name "" in the doc, but the app shows its position ("1"), and so do herdr-app ls and `where`.
- Pane ids (p…) exist only in the doc and in herdr-app ls; there is no label or env var.
- terminal `name` is null and `title` is "" for app shells (title is the program's OSC title).
