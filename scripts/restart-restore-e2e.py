#!/usr/bin/env python3
"""Reproduce ht-iygh: seats after a cold Herdr restart with session restore, in a PRIVATE Herdr server.

  python3 scripts/restart-restore-e2e.py --expect current|fixed --codex-auth-home DIR [--bin PATH]
      [--herdr PATH] [--keep]

Run from an ordinary shell, never inside Herdr, and outside any agent's process tree (for example
`setsid -f env -i HOME="$HOME" PATH="$PATH" python3 ...`), or the CLI attributes its calls to that agent.
`--codex-auth-home` names a Codex home whose auth.json is copied, mode 0600, into the scratch CODEX_HOME. Everything lives under one fresh root /tmp/hrr.XXXXXX: the
Herdr API socket, config (`default_shell = '/bin/sh'`, no user plugins), XDG config/state/runtime dirs, a
scratch HOME with CODEX_HOME=$HOME/.codex and a scratch CLAUDE_CONFIG_DIR, and the herdr-threads state dir.
Inherited HERDR_*, HC_*, CODEX_*, CLAUDE* caller variables are scrubbed. The shared Herdr server, the real
herdr-threads state and the real harness configs are never touched; no Herdr plugin is linked. The candidate
(`--bin`, default bin/herdr-threads) reaches the panes through a PATH shim that pins the private state dir and
socket, as scripts/wake-e2e.py does; its daemon is started with `daemon ensure` against the private socket.
Hooks are installed by the public `setup` CLI (codex, claude) into the scratch dirs only, and Herdr's own
Codex integration (`herdr integration install codex`, private HOME) reports the session Herdr resumes.

Scenario: workspace W with pane A (a real Codex, one tiny prompt) and pane B (a stand-in Claude: the installed
SessionStart hook under a `CLAUDE_PID` shell). Cold restart: `herdr server stop`, then a new `herdr server` on
the same root. Observe the seats, start a fresh stand-in Claude in B, send p1 then p0 to A's seat from a person
seat made after the restart, and record whether Codex wakes. Evidence: docs/validation/ht-iygh-restart-restore/
<UTC>/ (report.md, raw JSON, pane captures; credentials redacted; the repo root reads <repo>, $HOME ~, the
hostname <host>). Exit 0 iff every assertion of --expect holds.
Cleanup always runs (with --keep too): daemon, Herdr server, every process naming the root, the root itself.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRUB_PREFIXES = ("HERDR_", "HC_", "CODEX_", "CLAUDE", "TMUX")
SCRUB_NAMES = ("AI_AGENT", "HERMES_SESSION_ID", "ANTIGRAVITY_CONVERSATION_ID", "ANTIGRAVITY_AGENT",
               "PI_CODING_AGENT", "OPENCODE", "ANTHROPIC_API_KEY", "OPENROUTER_API_KEY", "GEMINI_API_KEY")
IDLE = ("idle", "done")
WAKE_LINE = "herdr-threads: wake up"


def utc():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def walk(x):
    if isinstance(x, dict):
        yield x
        for v in x.values():
            yield from walk(v)
    elif isinstance(x, list):
        for v in x:
            yield from walk(v)


def find_key(x, key):
    """Every value of `key` anywhere in `x`, in document order."""
    return [d[key] for d in walk(x) if key in d]


class Driver:
    def __init__(self, args):
        self.args = args
        self.stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
        self.ev = (Path(args.evidence_dir) if args.evidence_dir else
                   ROOT / "docs/validation/ht-iygh-restart-restore" / self.stamp).resolve()
        self.ev.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix="hrr.", dir="/tmp"))
        os.chmod(self.root, 0o700)
        r = self.root
        self.sock, self.state, self.home, self.work = r / "h.sock", r / "ht-state", r / "home", r / "work"
        self.codex_home, self.claude_cfg, self.bindir = r / "home/.codex", r / "claude-config", r / "bin"
        for d in (r / "cfg", r / "st", r / "rt", self.home, self.codex_home, self.claude_cfg, self.bindir,
                  self.state, self.work):
            d.mkdir(mode=0o700, parents=True, exist_ok=True)
        (r / "cfg/herdr.toml").write_text("default_shell = '/bin/sh'\nonboarding = false\n")
        self.bin = str(Path(args.bin).resolve())
        self.herdr_bin = str(Path(args.herdr or shutil.which("herdr") or "herdr").resolve())
        (self.bindir / "herdr").symlink_to(self.herdr_bin)
        q = shlex.quote
        shim = self.bindir / "herdr-threads"
        shim.write_text("#!/bin/sh\npin_state=1 pin_host=1\nfor word in \"$@\"; do\n  case \"$word\" in\n"
                        "    --state-dir|--state-dir=*) pin_state=0 ;;\n"
                        "    --host-endpoint|--host-endpoint=*) pin_host=0 ;;\n  esac\ndone\n"
                        f"if [ \"$pin_host\" = 1 ]; then set -- --host-endpoint {q(str(self.sock))} \"$@\"; fi\n"
                        f"if [ \"$pin_state\" = 1 ]; then set -- --state-dir {q(str(self.state))} \"$@\"; fi\n"
                        f"exec {q(self.bin)} \"$@\"\n")
        shim.chmod(0o700)
        # A user's `codex` wrapper, as on the Mac (Herdr's restore ran `codex --no-daemon
        # --approve-for-me resume <id>`): Herdr 0.9.5 re-runs only `codex resume <id>` through the
        # pane shell, so the flags that keep hooks and commands in the pane come from the user's
        # own command. Adds each flag the argv lacks.
        real_codex = shutil.which("codex") or "codex"
        wrapper = self.bindir / "codex"
        wrapper.write_text("#!/bin/sh\nset -- \"$@\"\nextra=''\n"
                           "for f in --no-daemon --approve-for-me --dangerously-bypass-hook-trust; do\n"
                           "  case \" $* \" in *\" $f \"*) ;; *) extra=\"$extra $f\" ;; esac\ndone\n"
                           f"exec {q(real_codex)} $extra \"$@\"\n")
        wrapper.chmod(0o700)
        env = {k: v for k, v in os.environ.items()
               if not (k.startswith(SCRUB_PREFIXES) or k in SCRUB_NAMES)}
        env.update(HOME=str(self.home), XDG_CONFIG_HOME=str(r / "cfg"), XDG_STATE_HOME=str(r / "st"),
                   XDG_RUNTIME_DIR=str(r / "rt"), HERDR_CONFIG_PATH=str(r / "cfg/herdr.toml"),
                   HERDR_SOCKET_PATH=str(self.sock), CODEX_HOME=str(self.codex_home),
                   CODEX_SQLITE_HOME=str(self.codex_home), CLAUDE_CONFIG_DIR=str(self.claude_cfg),
                   PATH=f"{self.bindir}:{env.get('PATH', '')}")
        self.env = env
        self.server = None
        self.server_starts = 0
        self.daemon_started = False
        self.checks = []  # (mode, name, ok, observed)
        self.facts = {}
        self.secrets = []
        self.cmdlog = (self.ev / "commands.jsonl").open("a")

    # ---------- helpers ----------
    def redact(self, text):
        for s in self.secrets:
            text = text.replace(s, "[REDACTED]")
        text = re.sub(r"eyJ[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-.]+", "[REDACTED-JWT]", text)
        return scrub_paths(text)

    def save(self, name, data):
        text = data if isinstance(data, str) else json.dumps(data, indent=2, sort_keys=True)
        (self.ev / name).write_text(self.redact(text))

    def run(self, argv, env=None, timeout=60, stdin=None):
        t0 = time.monotonic()
        try:
            p = subprocess.run(argv, env=env or self.env, capture_output=True, text=True, timeout=timeout,
                               input=stdin, stdin=None if stdin is not None else subprocess.DEVNULL)
            rc, out, err = p.returncode, p.stdout, p.stderr
        except subprocess.TimeoutExpired as e:
            rc, out, err = "timeout", e.stdout or "", e.stderr or ""
            out, err = (out.decode() if isinstance(out, bytes) else out), (err.decode() if isinstance(err, bytes) else err)
        self.cmdlog.write(self.redact(json.dumps({"utc": utc(), "argv": argv, "rc": rc,
                                                  "elapsed_s": round(time.monotonic() - t0, 2),
                                                  "stdout": out[-4000:], "stderr": err[-4000:]})) + "\n")
        self.cmdlog.flush()
        return rc, out, err

    def herdr(self, *argv, check=True, timeout=30):
        rc, out, err = self.run([self.herdr_bin, *argv], timeout=timeout)
        if check and rc != 0:
            raise RuntimeError(f"herdr {' '.join(argv[:3])}: rc={rc} {err.strip()[:300]}")
        try:
            return json.loads(out) if out.strip() else {}
        except ValueError:
            return {"raw": out}

    def ht(self, *argv, pane=None, human=False, check=True, timeout=60):
        env = dict(self.env)
        if pane:
            env["HERDR_PANE_ID"] = pane
        prefix = ["--state-dir", str(self.state), "--host-endpoint", str(self.sock), "--json"]
        argv = list(argv)
        full = [self.bin] + (argv + prefix if argv[:1] == ["setup"] else (["human"] if human else []) + prefix + argv)
        rc, out, err = self.run(full, env=env, timeout=timeout)
        if check and rc != 0:
            raise RuntimeError(f"herdr-threads {' '.join(argv[:3])}: rc={rc} {(err or out).strip()[:300]}")
        return rc, out, err

    def ht_json(self, *argv, **kw):
        rc, out, err = self.ht(*argv, check=False, **kw)
        try:
            return json.loads(out) if out.strip() else {"rc": rc, "stderr": err.strip()}
        except ValueError:
            return {"rc": rc, "raw": out, "stderr": err.strip()}

    def check(self, modes, name, ok, observed):
        """Record one assertion; `modes` names the --expect modes it gates ("both", "current", "fixed", "info")."""
        self.checks.append((modes, name, bool(ok), observed))
        tag = "PASS" if ok else "FAIL"
        print(f"  [{modes}] {tag} {name}: {observed}", flush=True)

    # ---------- Herdr server ----------
    def start_server(self):
        self.server_starts += 1
        log = self.root / f"server-{self.server_starts}.out"
        if self.sock.exists():
            self.sock.unlink()
        with log.open("wb") as f:
            self.server = subprocess.Popen([self.herdr_bin, "server"], env=self.env, stdin=subprocess.DEVNULL,
                                           stdout=f, stderr=subprocess.STDOUT, start_new_session=True)
        end = time.time() + 20
        while not (self.sock.exists() and "api socket" in log.read_text(errors="replace")):
            if self.server.poll() is not None or time.time() > end:
                raise RuntimeError(f"herdr server did not start: {log.read_text(errors='replace')[-800:]}")
            time.sleep(0.1)
        return self.server.pid

    def stop_server(self):
        if self.server is None or self.server.poll() is not None:
            return
        self.run([self.herdr_bin, "server", "stop"], timeout=30)
        try:
            self.server.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(self.server.pid, signal.SIGKILL)
            self.server.wait()

    # ---------- observation ----------
    def panes(self):
        return self.herdr("pane", "list", check=False)

    def agent(self, pane):
        return (self.herdr("agent", "get", pane, check=False).get("result") or {}).get("agent") or {}

    def pane_info(self, pane):
        return (self.herdr("pane", "get", pane, check=False).get("result") or {}).get("pane") or {}

    def terminal_id(self, pane):
        for src in (self.pane_info(pane), self.agent(pane)):
            for k in ("terminal_id", "terminal"):
                v = find_key(src, k)
                if v and isinstance(v[0], str):
                    return v[0]
        return None

    def read(self, pane, source="recent-unwrapped", lines=200):
        for kind in ("agent", "pane"):
            args = ["read", pane, "--source", source] + (["--lines", str(lines)] if source == "recent-unwrapped" else [])
            data = self.herdr(kind, *args, check=False)
            if "raw" in data:
                return data["raw"]
        return ""

    def capture(self, pane, name):
        self.save(f"pane-{name}.txt", self.read(pane))

    def pane_processes(self, pane):
        """(pid, argv) of every process whose environment names this private socket and HERDR_PANE_ID=pane."""
        out = []
        for d in Path("/proc").iterdir():
            if not d.name.isdigit():
                continue
            try:
                envb = (d / "environ").read_bytes().split(b"\0")
                argv = [a.decode(errors="replace") for a in (d / "cmdline").read_bytes().split(b"\0") if a]
            except OSError:
                continue
            if f"HERDR_SOCKET_PATH={self.sock}".encode() in envb and f"HERDR_PANE_ID={pane}".encode() in envb:
                out.append((int(d.name), argv))
        return out

    def seats(self):
        return self.ht_json("seat", "list")

    def seat_at(self, pane):
        """The seat id whose seat-list item names `pane` (first match), else None."""
        for item in find_key(self.seats(), "items"):
            if isinstance(item, list):
                for it in item:
                    if pane in json.dumps(it):
                        sid = it.get("seat") or it.get("seat_id") or it.get("id")
                        if isinstance(sid, str):
                            return sid
        return None

    def inspect(self, seat):
        return self.ht_json("seat", "inspect", seat, "--binding-details")

    @staticmethod
    def mapping_state(insp):
        for k in ("mapping_state", "mapping", "state"):
            for v in find_key(insp, k):
                if isinstance(v, str) and v in ("resolved", "unresolved", "retired", "held"):
                    return v
                if isinstance(v, dict):
                    s = v.get("state") or v.get("kind")
                    if isinstance(s, str):
                        return s
        return None

    @staticmethod
    def decision_kinds(insp):
        return [v for v in find_key(insp, "decision_kind") + find_key(insp, "kind")
                if isinstance(v, str) and ("continuity" in v or v in ("operator_rebind", "rebind", "new_seat"))]

    @staticmethod
    def connected(insp):
        return any(v is True for v in find_key(insp, "connected"))

    def health(self):
        return self.ht_json("daemon", "health")

    # ---------- setup ----------
    def setup(self):
        self.secrets = [s for d in walk(json.loads(self.args.codex_auth.read_text()))
                        for s in d.values() if isinstance(s, str) and len(s) >= 16]
        (self.state / "node").mkdir(mode=0o700, exist_ok=True)
        (self.state / "node/settings.json").write_text(json.dumps({"sync": {"discover": False}}) + "\n")
        (self.state / "node/settings.json").chmod(0o600)
        pid = self.start_server()
        self.facts["server_pid_1"] = pid
        self.daemon_started = True
        self.ht("daemon", "ensure", timeout=60)
        self.save("health-0.json", self.health())
        self.ht("setup", "codex", "--with-permissions", timeout=120)
        self.ht("setup", "claude", "--with-permissions", timeout=120)
        # Herdr's own Codex integration reports the session (and resume command) Herdr restores.
        rc, out, err = self.run([self.herdr_bin, "integration", "install", "codex"], timeout=60)
        self.facts["herdr_integration_install_codex"] = {"rc": rc, "out": out.strip()[-500:], "err": err.strip()[-500:]}
        dest = self.codex_home / "auth.json"
        fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as f, open(self.args.codex_auth, "rb") as src:
            shutil.copyfileobj(src, f)
        cfg = self.codex_home / "config.toml"
        text = cfg.read_text() if cfg.exists() else ""
        text = text.rstrip("\n") + (f'\n\n[projects."{self.work}"]\ntrust_level = "trusted"\n'
                                    "\n[notice]\nhide_rate_limit_model_nudge = true\n")
        cfg.write_text(text)
        for f in ("hooks.json", "config.toml"):
            if (self.codex_home / f).exists():
                self.save(f"codex-{f}", (self.codex_home / f).read_text())
        settings = json.loads((self.claude_cfg / "settings.json").read_text())
        self.save("claude-settings.json", settings)
        cmds = [h.get("command") for g in settings.get("hooks", {}).get("SessionStart", [])
                for h in g.get("hooks", []) if h.get("command")]
        if not cmds:
            raise RuntimeError("setup claude installed no SessionStart hook")
        self.claude_session_start = cmds[0]
        self.facts["claude_session_start_command"] = cmds[0]

    def codex_line(self):
        argv = ["codex", "--no-daemon", "-C", str(self.work), "--dangerously-bypass-hook-trust",
                "-c", "check_for_update_on_startup=false", "-c", "features.shell_snapshot=false",
                "-c", "model_reasoning_effort=low", "--approve-for-me"]
        return shlex.join(argv)

    def standin_claude(self, pane, tag):
        """Run the installed SessionStart hook in `pane` under a stand-in Claude process (a shell whose pid is
        CLAUDE_PID, which then stays alive as `sleep`), with a new session id. Returns (session, rc, output)."""
        session = str(uuid.uuid4())
        payload = self.root / f"ss-{tag}.json"
        payload.write_text(json.dumps({"session_id": session, "transcript_path": str(self.work / f"{tag}.jsonl"),
                                       "cwd": str(self.work), "hook_event_name": "SessionStart",
                                       "source": "startup"}))
        out, rcf = self.root / f"ss-{tag}.out", self.root / f"ss-{tag}.rc"
        inner = (f"export CLAUDE_PID=$$ CLAUDE_CODE_SESSION_ID={session}; cd {shlex.quote(str(self.work))}; "
                 f"( {self.claude_session_start} ) < {shlex.quote(str(payload))} > {shlex.quote(str(out))} 2>&1; "
                 f"echo $? > {shlex.quote(str(rcf))}; exec /bin/sleep 86400")
        self.herdr("pane", "run", pane, f"sh -c {shlex.quote(inner)}")
        end = time.time() + 60
        while not rcf.exists() and time.time() < end:
            time.sleep(0.3)
        time.sleep(0.3)
        rc = rcf.read_text().strip() if rcf.exists() else "timeout"
        text = out.read_text(errors="replace") if out.exists() else ""
        self.save(f"hook-{tag}.txt", f"session={session} rc={rc}\n{text}")
        return session, rc, text

    def wait_codex_idle(self, pane, timeout=180):
        end = time.time() + timeout
        last = None
        while time.time() < end:
            a = self.agent(pane)
            last = a
            if a.get("agent") == "codex" and a.get("agent_status") in IDLE:
                time.sleep(1.0)
                if self.agent(pane).get("agent_status") in IDLE:
                    return a
            if a.get("agent_status") == "blocked":
                raise RuntimeError(f"codex blocked at a dialog: {self.read(pane, 'visible')[-600:]}")
            time.sleep(1)
        raise RuntimeError(f"codex not idle within {timeout}s: {last} {self.read(pane, 'visible')[-600:]}")

    @staticmethod
    def agent_session(a):
        s = a.get("agent_session")
        return s.get("value") if isinstance(s, dict) else s

    # ---------- scenario ----------
    def scenario(self):
        exp = self.args.expect
        self.setup()
        print(f"restart-restore-e2e: root {self.root}, evidence {self.ev}", flush=True)
        # (a) workspace W: pane A (Codex), pane B (stand-in Claude)
        data = self.herdr("workspace", "create", "--label", "rr-W", "--cwd", str(self.work), "--no-focus")
        self.ws = data["result"]["workspace"]["workspace_id"]
        pane_a = data["result"]["root_pane"]["pane_id"]
        split = self.herdr("pane", "split", pane_a, "--direction", "right", "--cwd", str(self.work))
        pane_b = (find_key(split, "pane_id") or [None])[0]
        self.facts.update(workspace=self.ws, pane_a=pane_a, pane_b=pane_b)
        time.sleep(1.5)
        # Herdr's guarded start records the agent's arguments, so its session restore re-runs the
        # resume with them (`--no-daemon` keeps hooks and commands in the pane, as on the Mac).
        self.herdr("agent", "start", "rr-codex", "--kind", "codex", "--pane", pane_a, "--timeout", "120000",
                   "--", *shlex.split(self.codex_line())[1:], timeout=150)
        self.wait_codex_idle(pane_a)
        self.herdr("agent", "prompt", pane_a, "Reply with just the word: ok", "--wait", "--timeout", "180000",
                   timeout=220, check=False)
        a_before = self.wait_codex_idle(pane_a)
        self.capture(pane_a, "A-before")
        self.save("agent-A-before.json", a_before)
        sess_a = self.agent_session(a_before)
        rollouts = sorted(self.codex_home.glob("sessions/**/rollout-*.jsonl"))
        self.facts["codex_rollouts"] = [p.name for p in rollouts]
        if not sess_a and rollouts:
            m = re.search(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})", rollouts[-1].name)
            sess_a = m.group(1) if m else None
        self.facts["codex_session"] = sess_a
        seat_a = self.seat_at(pane_a)
        sess_b1, rc_b1, out_b1 = self.standin_claude(pane_b, "B1")
        time.sleep(1)
        seat_b = self.seat_at(pane_b)
        self.facts.update(seat_a=seat_a, seat_b=seat_b, claude_session_1=sess_b1)
        self.save("seat-list-before.json", self.seats())
        insp_a0, insp_b0 = (self.inspect(seat_a) if seat_a else {}), (self.inspect(seat_b) if seat_b else {})
        self.save("inspect-A-before.json", insp_a0)
        self.save("inspect-B-before.json", insp_b0)
        self.save("panes-before.json", self.panes())
        term_before = {p: self.terminal_id(p) for p in (pane_a, pane_b)}
        self.facts["terminal_ids_before"] = term_before
        self.check("both", "pre: A has a seat (Codex hook enrolled)", bool(seat_a), f"seat={seat_a} session={sess_a}")
        self.check("both", "pre: B has a seat (stand-in SessionStart)", bool(seat_b),
                   f"seat={seat_b} hook_rc={rc_b1}")
        self.check("both", "pre: A resolved", self.mapping_state(insp_a0) == "resolved", self.mapping_state(insp_a0))
        if not (seat_a and seat_b and sess_a):
            raise RuntimeError("pre-restart enrollment incomplete; see evidence")

        # (b) cold restart
        self.stop_server()
        self.facts["server_1_exit"] = self.server.returncode
        time.sleep(1)
        self.facts["server_pid_2"] = self.start_server()
        end = time.time() + 60
        a_after = {}
        while time.time() < end:
            a_after = self.agent(pane_a)
            if self.agent_session(a_after) and a_after.get("agent") == "codex":
                break
            time.sleep(1)
        time.sleep(3)
        a_after = self.agent(pane_a)
        self.save("agent-A-after.json", a_after)
        panes_after = self.panes()
        self.save("panes-after.json", panes_after)
        term_after = {p: self.terminal_id(p) for p in (pane_a, pane_b)}
        self.facts["terminal_ids_after"] = term_after
        procs = self.pane_processes(pane_a)
        self.save("pane-A-processes-after.json", procs)
        resume = [argv for _, argv in procs if any("codex" in Path(a).name for a in argv[:2])
                  and "resume" in argv and sess_a in argv]
        self.check("both", "restart: new Herdr server process",
                   self.facts["server_pid_1"] != self.facts["server_pid_2"],
                   f"pid {self.facts['server_pid_1']} -> {self.facts['server_pid_2']}")
        listed = json.dumps(panes_after)
        self.check("both", "restart: pane ids kept", pane_a in listed and pane_b in listed, f"A={pane_a} B={pane_b}")
        self.check("both", "restart: terminal ids new",
                   all(term_before[p] and term_after[p] and term_before[p] != term_after[p] for p in term_before),
                   f"before={term_before} after={term_after}")
        self.check("both", "restart: pane A runs `codex ... resume <session>`", bool(resume),
                   shlex.join(resume[0]) if resume else f"argvs={[a for _, a in procs]}")
        self.check("both", "restart: herdr agent get A agent_session == session",
                   self.agent_session(a_after) == sess_a and a_after.get("agent") == "codex",
                   f"agent={a_after.get('agent')} agent_session={self.agent_session(a_after)}")

        # (c) observe, Codex untouched
        timeline = []
        end = time.time() + self.args.observe_s
        while time.time() < end:
            ia, ib, h = self.inspect(seat_a), self.inspect(seat_b), self.health()
            timeline.append({"utc": utc(), "A": self.mapping_state(ia), "A_connected": self.connected(ia),
                             "A_decisions": self.decision_kinds(ia), "B": self.mapping_state(ib),
                             "unresolved_seats": (find_key(h, "unresolved_seats") or [None])[0],
                             "held_targets": (find_key(h, "held_targets") or [None])[0],
                             "codex_status": self.agent(pane_a).get("agent_status")})
            if exp == "fixed" and timeline[-1]["A"] == "resolved" and timeline[-1]["B"] != "unresolved":
                break
            time.sleep(10)
        self.save("observe-timeline.json", timeline)
        ia, ib, h = self.inspect(seat_a), self.inspect(seat_b), self.health()
        self.save("inspect-A-after-restart.json", ia)
        self.save("inspect-B-after-restart.json", ib)
        self.save("health-after-restart.json", h)
        self.capture(pane_a, "A-after-restart")
        st_a, st_b = self.mapping_state(ia), self.mapping_state(ib)
        kinds_a = self.decision_kinds(ia)
        held = (find_key(h, "held_targets") or [None])[0]
        unresolved = (find_key(h, "unresolved_seats") or [None])[0]
        idle_a = self.agent(pane_a).get("agent_status")
        self.check("info", "observe: codex stayed idle (no prompt sent)", idle_a in IDLE, idle_a)
        self.check("current", "current: A's seat unresolved, not restored", st_a == "unresolved",
                   f"state={st_a} decisions={kinds_a}")
        self.check("current", "current: B's seat unresolved", st_b == "unresolved", f"state={st_b}")
        self.check("fixed", "fixed: A restored by host_session_continuity, no operator step",
                   st_a == "resolved" and "host_session_continuity" in kinds_a, f"state={st_a} decisions={kinds_a}")
        self.check("info", "observe: daemon health", True, f"unresolved_seats={unresolved} held_targets={held}")

        # Herdr's restore re-runs plain `codex resume <id>`, without the first start's
        # --dangerously-bypass-hook-trust, so this scratch CODEX_HOME's Codex asks to trust its hooks.
        # A user's real Codex has them trusted already. Answer that startup gate only (a trust prompt
        # in a pane this driver owns, not a model turn), so the wake below meets an idle composer.
        gate = False
        for _ in range(10):
            if "Hooks need review" not in self.read(pane_a):
                break
            gate = True
            self.herdr("pane", "send-keys", pane_a, "2", check=False)
            time.sleep(1)
            self.herdr("pane", "send-keys", pane_a, "enter", check=False)
            time.sleep(2)
        self.capture(pane_a, "A-after-trust-gate")
        self.check("info", "observe: resumed Codex hook-trust gate answered", True,
                   f"gate_seen={gate} cleared={'Hooks need review' not in self.read(pane_a)}")

        # (d) fresh stand-in Claude in pane B
        sess_b2, rc_b2, out_b2 = self.standin_claude(pane_b, "B2")
        time.sleep(1)
        ib2 = self.inspect(seat_b)
        self.save("inspect-B-after-fresh.json", ib2)
        refused = "unresolved seat mapping" in out_b2
        seat_b2 = self.seat_at(pane_b)
        kinds_b = self.decision_kinds(ib2)
        self.facts.update(claude_session_2=sess_b2, seat_at_B_after_fresh=seat_b2)
        self.check("current", "current: fresh Claude in B refused (unresolved seat mapping)", refused,
                   f"rc={rc_b2} out={out_b2.strip()[:240]!r}")
        self.check("fixed", "fixed: fresh Claude takes B's seat by pane_continuity",
                   not refused and self.mapping_state(ib2) == "resolved" and "pane_continuity" in kinds_b,
                   f"state={self.mapping_state(ib2)} decisions={kinds_b} seat_at_B={seat_b2}")

        # (e) person sender made after the restart; p1 then p0 to A's seat
        # The sender workspace takes focus, as a person typing a message there would: a focused
        # pane's composer must be observed empty for a minute before a wake (focused_empty_ready).
        sw = self.herdr("workspace", "create", "--label", "rr-sender", "--cwd", str(self.work))
        self.sender_ws = sw["result"]["workspace"]["workspace_id"]
        sender_pane = sw["result"]["root_pane"]["pane_id"]
        time.sleep(1)
        rc, out, err = self.ht("me", "init", pane=sender_pane, human=True, check=False)
        m = re.search(r'"(s[A-Za-z0-9]{11})"', out)
        sender = m.group(1) if m else None
        self.facts.update(sender_pane=sender_pane, sender_seat=sender,
                          sender_me_init={"rc": rc, "out": out.strip()[-400:], "err": err.strip()[-400:]})
        self.check("both", "send: person sender seat made after restart", bool(sender), f"rc={rc} seat={sender} {err.strip()[:200]}")
        wake_p1 = wake_p0 = None
        if sender:
            before = self.read(pane_a).count(WAKE_LINE)
            rc1, o1, e1 = self.ht("send", "--seat", seat_a, "--body", "rr p1 probe: no reply needed",
                                  pane=sender_pane, human=True, check=False)
            self.facts["send_p1"] = {"rc": rc1, "out": o1.strip()[-400:], "err": e1.strip()[-400:]}
            wake_p1 = self.watch_wake(pane_a, 25, before)
            self.check("info", "send: p1 accepted by the CLI", rc1 == 0, f"rc={rc1} {(e1 or o1).strip()[:200]}")
            self.check("fixed", "fixed: p1 does not wake A", rc1 == 0 and wake_p1 is None, f"wake={wake_p1}")
            self.check("current", "current: p1 does not wake A", wake_p1 is None, f"wake={wake_p1}")
            before = self.read(pane_a).count(WAKE_LINE)
            rc0, o0, e0 = self.ht("send", "--seat", seat_a, "--body", "rr p0 probe: reply with just: ok",
                                  "--nudge", pane=sender_pane, human=True, check=False)
            self.facts["send_p0"] = {"rc": rc0, "out": o0.strip()[-400:], "err": e0.strip()[-400:]}
            self.check("info", "send: p0 accepted by the CLI", rc0 == 0, f"rc={rc0} {(e0 or o0).strip()[:200]}")
            # A focused pane needs its composer observed empty for a minute first (focused_empty_ready).
            wake_p0 = self.watch_wake(pane_a, 150, before)
            if wake_p0 is not None:
                try:
                    self.wait_codex_idle(pane_a, timeout=180)
                except RuntimeError as e:
                    self.facts["after_wake_idle_error"] = str(e)[:300]
            time.sleep(3)
            self.capture(pane_a, "A-after-sends")
            ia3 = self.inspect(seat_a)
            self.save("inspect-A-after-sends.json", ia3)
            self.check("fixed", "fixed: p0 wakes Codex in A", rc0 == 0 and wake_p0 is not None, f"wake={wake_p0}")
            self.check("fixed", "fixed: A's seat resolved and connected after the wake",
                       self.mapping_state(ia3) == "resolved" and self.connected(ia3),
                       f"state={self.mapping_state(ia3)} connected={self.connected(ia3)}")
            self.check("current", "current: p0 does not wake A (seat unresolved, never woken)", wake_p0 is None,
                       f"wake={wake_p0} send_rc={rc0}")
            self.check("info", "send: A's seat after sends", True,
                       f"state={self.mapping_state(ia3)} connected={self.connected(ia3)} "
                       f"decisions={self.decision_kinds(ia3)}")
        h = self.health()
        self.save("health-final.json", h)
        self.save("seat-list-final.json", self.seats())
        self.save("panes-final.json", self.panes())
        self.capture(pane_b, "B-final")
        held = (find_key(h, "held_targets") or [None])[0]
        self.check("fixed", "fixed: daemon health shows 0 held targets", held == 0, f"held_targets={held}")
        self.check("info", "final: daemon health", True,
                   f"unresolved_seats={(find_key(h, 'unresolved_seats') or [None])[0]} held_targets={held}")

    def watch_wake(self, pane, window, before):
        """Seconds until Codex in `pane` leaves idle or shows a new wake line, else None."""
        t0 = time.time()
        while time.time() - t0 < window:
            if self.agent(pane).get("agent_status") not in IDLE or self.read(pane).count(WAKE_LINE) > before:
                return round(time.time() - t0, 1)
            time.sleep(0.5)
        return None

    # ---------- cleanup ----------
    def root_pids(self):
        me = os.getpid()
        pids = []
        needle = str(self.root).encode()
        for d in Path("/proc").iterdir():
            if not d.name.isdigit() or int(d.name) == me:
                continue
            try:
                if needle in (d / "cmdline").read_bytes() or needle in (d / "environ").read_bytes():
                    pids.append(int(d.name))
            except OSError:
                pass
        return pids

    def cleanup(self):
        problems = []
        if self.daemon_started:
            rc, _, err = self.ht("daemon", "stop", check=False, timeout=30)
            if rc != 0 and "not running" not in err:
                problems.append(f"daemon stop rc={rc} {err.strip()[:200]}")
        try:
            self.stop_server()
        except Exception as e:  # keep going: the sweep below catches the rest
            problems.append(f"server stop: {e}")
        for sig in (signal.SIGTERM, signal.SIGKILL):
            pids = self.root_pids()
            for p in pids:
                try:
                    os.kill(p, sig)
                except ProcessLookupError:
                    pass
            if pids:
                time.sleep(2)
        left = self.root_pids()
        if left:
            problems.append(f"processes still naming the root: {left}")
        if self.args.keep:
            keep = self.ev / "root"
            shutil.copytree(self.root, keep, symlinks=True, ignore=shutil.ignore_patterns("auth.json", "*.sock"),
                            dirs_exist_ok=True)
        shutil.rmtree(self.root, ignore_errors=True)
        if self.root.exists():
            problems.append(f"root not removed: {self.root}")
        return problems

    def report(self, error, problems):
        exp = self.args.expect
        gating = [c for c in self.checks if c[0] in ("both", exp)]
        ok = error is None and not problems and gating and all(c[2] for c in gating)
        lines = [f"# ht-iygh restart-restore e2e ({self.stamp})", "",
                 f"- expect: `{exp}`  result: **{'PASS' if ok else 'FAIL'}**",
                 f"- candidate: `{self.bin}` ({self.facts.get('bin_version', '?')})",
                 f"- herdr: `{self.herdr_bin}` ({self.facts.get('herdr_version', '?')})", ""]
        for modes, name, passed, observed in self.checks:
            gate = "gates" if modes in ("both", exp) else "not gating"
            lines.append(f"- {'PASS' if passed else 'FAIL'} [{modes}, {gate}] {name}: `{observed}`")
        if error:
            lines += ["", f"ERROR: `{error}`"]
        if problems:
            lines += ["", "Cleanup problems:"] + [f"- {p}" for p in problems]
        lines += ["", "Facts: see facts.json; raw JSON and pane captures alongside."]
        self.save("report.md", "\n".join(lines) + "\n")
        self.save("facts.json", self.facts)
        return ok


def scrub_paths(text):
    """Evidence is committed to a public repository: the repo root reads <repo>, $HOME reads ~ and
    the hostname reads <host>."""
    home = str(Path.home())
    text = text.replace(str(ROOT), "<repo>")
    if home and home != "/":
        text = text.replace(home, "~")
    host = socket.gethostname()
    if host:
        text = re.sub(rf"\b{re.escape(host)}\b", "<host>", text)
    return text


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--expect", choices=("current", "fixed"), required=True)
    ap.add_argument("--bin", default=str(ROOT / "bin/herdr-threads"))
    ap.add_argument("--herdr", help="herdr binary (default: the one on PATH)")
    ap.add_argument("--observe-s", type=int, default=90)
    ap.add_argument("--evidence-dir")
    ap.add_argument("--codex-auth-home", required=True,
                    help="Codex home whose auth.json is copied (mode 0600) into the scratch CODEX_HOME; read only")
    ap.add_argument("--keep", action="store_true",
                    help="copy the scratch root (minus credentials, paths not scrubbed) into the evidence; local diagnosis only")
    args = ap.parse_args()
    if os.environ.get("HERDR_ENV") == "1" or os.environ.get("HERDR_PANE_ID"):
        print("restart-restore-e2e: run from an ordinary shell, not inside Herdr", file=sys.stderr)
        return 2
    args.codex_auth = Path(args.codex_auth_home).expanduser() / "auth.json"
    if not os.access(args.bin, os.X_OK) or not args.codex_auth.is_file():
        print(f"restart-restore-e2e: need an executable --bin ({args.bin}) and --codex-auth-home/auth.json",
              file=sys.stderr)
        return 2
    d = Driver(args)
    for k, argv in (("bin_version", [d.bin, "--version"]), ("herdr_version", [d.herdr_bin, "--version"])):
        d.facts[k] = d.run(argv, timeout=10)[1].strip()
    error = None
    old = {s: signal.signal(s, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt())) for s in (signal.SIGTERM, signal.SIGHUP)}
    try:
        d.scenario()
    except BaseException as e:  # noqa: BLE001 - cleanup must always run
        error = f"{type(e).__name__}: {e}"
        print(f"restart-restore-e2e: {error}", file=sys.stderr)
    finally:
        problems = d.cleanup()
        for s, h in old.items():
            signal.signal(s, h)
    ok = d.report(error, problems)
    print(f"restart-restore-e2e: {'PASS' if ok else 'FAIL'} (--expect {args.expect}); evidence {d.ev}", flush=True)
    for p in problems:
        print(f"  cleanup: {p}", flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
