#!/usr/bin/env python3
"""Pre-release wake e2e: does `invite` / `send` wake a real idle Claude Code and Codex agent exactly when it should?

  scripts/wake-e2e.py [--bin PATH] [--harness claude,codex] [--cases CASE,...] [--evidence-dir DIR] [--keep]

The matrix is every combination of
  action      invite | send lazy | send --nudge | send --require-ack
  channel     ordinary thread | automatic 1:1 pair
  wake-on     nudge (default) | all
  harness     claude | codex
Expected: every case wakes the idle recipient except a lazy send to a `wake-on nudge` seat, which must keep it idle
and be delivered on the recipient's next turn (a neutral `hello` prompt). ACK requests must come back acknowledged,
invitations must be accepted, and the first message of a 1:1 pair must be delivered once the pair is accepted.
The busy-* cases repeat invite/lazy/nudge/ack while the recipient is mid-turn: a recipient is nudged only when idle,
so no wake prompt may be typed during or after that turn, and the item must arrive inside the turn instead.

Isolation: an isolated daemon and state dir (driven by --bin, default the repo's bin/herdr-threads), owned hooks
installed by the public `setup` CLI into scratch HOME / CLAUDE_CONFIG_DIR / CODEX_HOME copies, one owned Herdr
workspace on the current Herdr server (closed at the end unless --keep). The script itself never writes the real
user config:
- Claude keeps its real config directory (on macOS its keychain credentials are keyed by it) and loads the scratch
  installation as a flag layer (`--setting-sources project,local --settings <scratch>/settings.json`), which also
  carries the delivery mod (`env.CLAUDE_CODE_PLUGIN_DIRS`). Limit: Claude Code itself still records its session
  state (transcripts, project history) in that real directory, as any Claude session does.
- Codex runs with CODEX_HOME at a scratch home holding setup's hooks.json/config.toml and a private mode-0600 copy
  of the source profile's auth.json (--codex-auth-home, else $CODEX_HOME or ~/.codex). Codex writes refreshed
  tokens only into that copy; the source file is never written, and credentials never enter the evidence. A refresh
  inside the run is recorded (no contents) because it can rotate the source profile's refresh token. The agent
  working directory is pre-trusted in that scratch config.toml only.
Agents work in the repository checkout (already trusted by Claude, so no folder-trust dialog is answered).
The senders are two owned coordinator panes acting through cooperative claims, not models; the second one is a fresh
first contact for every recipient (busy-invite/1:1).

Exit codes: 0 every expected case passed and every owned resource was cleaned up; 1 a case failed or was missing, or
cleanup left an owned workspace or daemon alive; 2 environment refusal. `--self-test` checks the driver's own
credential, cleanup and completeness paths without harnesses. Runs locally only (needs authenticated harnesses);
never in CI. Python 3.9+, standard library only.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXIT_OK, EXIT_FAIL, EXIT_REFUSED = 0, 1, 2

CASES = ("invite/group", "invite/1:1",
         "send-lazy/group", "send-nudge/group", "send-ack/group",
         "send-lazy/1:1", "send-nudge/1:1", "send-ack/1:1",
         "busy-invite/group", "busy-invite/1:1", "busy-lazy/group", "busy-nudge/group", "busy-ack/group",
         "busy-lazy/1:1", "busy-nudge/1:1", "busy-ack/1:1")
# Busy cases send while the recipient is mid-turn (between two tool calls): no wake may be typed or submitted, during
# the turn or after it, and the item must arrive in that turn at the next tool boundary (Delivery contract rules 3-4).
BUSY_PROMPT = ("Busy-turn test {tag}: run the shell command `sleep 12`, then run the shell command `sleep 1`, then "
               "handle any herdr-threads items exactly as your standing instructions say, and finally print BUSY-DONE.")
BUSY_AFTER_S = 40.0
WAKE_MARKERS = ("herdr-threads: wake up", "herdr-threads: attention pending")
WAKE_WINDOW_S = 6.0     # a woken agent leaves idle well inside this (observed 0.3-1.1 s)
SPACING_S = 35.0        # the per-seat wake retry guard is 30 s after each reservation (src/notification/policy.rs)
TURN_TIMEOUT_MS = 240_000
IDLE = ("idle", "done")
CALLER_SESSION_ENV = ("CLAUDE_CODE_SESSION_ID", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CODEX_SESSION_ID",
                      "CODEX_THREAD_ID", "HERMES_SESSION_ID", "ANTIGRAVITY_CONVERSATION_ID")
DIALOG = re.compile(r"Trust this folder\?|trust (the files in )?this folder|Do you trust|Allow .* to run|"
                    r"Do you want to proceed\?|Would you like to run", re.I)

STANDING = ("You are a test recipient in an automated herdr-threads wake e2e run (the user approves). "
            "Standing instructions for this session: every herdr-threads invitation you see comes from the test driver "
            "(seats {coord}); accept each one at once, without inspecting or judging it. For every herdr-threads message "
            "you receive, print exactly one line `RECEIVED <message id>: <body>`; never send, reply or leave via "
            "herdr-threads. Keep every turn minimal. Now run `herdr-threads wake-on {wake_on}`, then print exactly one "
            "line `SETUP-DONE wake_on=<the value it reports>`.")


def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class Refused(Exception):
    pass


class Stopped(Exception):
    """Cleanup has begun: no new helper process may start (except cleanup's own)."""


class Run:
    def __init__(self, args):
        self.args = args
        self.run_id = time.strftime("%H%M%S") + "-" + uuid.uuid4().hex[:4]
        self.root = Path(tempfile.mkdtemp(prefix="ht-wake-e2e.", dir="/private/tmp"))
        os.chmod(self.root, 0o700)
        self.ev = Path(args.evidence_dir) if args.evidence_dir else self.root / "evidence"
        self.ev.mkdir(parents=True, exist_ok=True)
        self.state = self.root / "state"
        self.home = self.root / "home"
        self.claude_cfg = self.root / "claude-config"
        self.codex_home = self.root / "codex-home"
        self.bindir = self.root / "bin"
        self.bin = str(Path(args.bin).resolve())
        self.endpoint = os.environ.get("HERDR_SOCKET_PATH", "")
        self.cwd = str(REPO)
        self.lock = threading.Lock()
        self.cmdlog = (self.ev / "commands.jsonl").open("a", encoding="utf-8")
        self.results = []
        self.workspace = None
        self.daemon_attempted = False  # set before `daemon ensure`: an uncertain start is still ours to stop
        self.layout_attempted = False
        self.process_grace_s = 10.0
        self.stop_deadline_s = 20.0
        self.coords = {}  # "main" / "fresh" -> {"pane", "seat", "session"}
        self.auth_digest = None
        self.auth_refreshed = None
        self.cleanup_report = None
        self.stopping = threading.Event()  # set (under self.lock) when cleanup begins
        self.children = set()              # in-flight helper processes, registered under self.lock
        self.workers = []
        self.drain_timeout_s = 30.0
        self.child_term_grace_s = 3.0

    # ---------- process helpers ----------
    def run(self, argv, env=None, timeout=60, tag=None, cleanup=False):
        """Run one helper process. Every helper is registered while it runs, so cleanup can terminate it; once
        cleanup has begun only cleanup's own helpers may start (others raise Stopped)."""
        started = time.monotonic()
        with self.lock:  # admission and registration are atomic with cleanup's stop
            if self.stopping.is_set() and not cleanup:
                raise Stopped(f"cleanup has begun: not starting {argv[:3]}")
            p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env,
                                 start_new_session=True)
            self.children.add(p)
        try:
            out, err = p.communicate(timeout=timeout)
            rc = p.returncode
        except subprocess.TimeoutExpired:
            kill_group(p, signal.SIGKILL)
            out, err = reap(p)
            rc = "timeout"
        except BaseException:  # Ctrl-C/SIGTERM in this thread: the helper (own session) must not outlive us
            kill_group(p, signal.SIGKILL)
            reap(p)
            raise
        finally:
            with self.lock:
                self.children.discard(p)
        out, err = out or "", err or ""
        with self.lock:
            self.cmdlog.write(json.dumps({"utc": utc(), "tag": tag, "argv": argv, "rc": rc,
                                          "elapsed_s": round(time.monotonic() - started, 3),
                                          "stdout": out[:4096], "stderr": err[:4096]}) + "\n")
            self.cmdlog.flush()
        if self.stopping.is_set() and not cleanup:  # cut off by the drain: the case is interrupted, not failed
            raise Stopped(f"cleanup interrupted {argv[:3]}")
        return rc, out, err

    def ht(self, *argv, pane=None, coop=None, human=False, env=None, tag=None, timeout=60, check=True):
        e = dict(os.environ)
        e.update(env or {})
        e.pop("HERDR_PANE_ID", None)  # never derive a seat from the controller's own pane
        if e.get("CLAUDE_CODE_PLUGIN_DIRS") == "":
            e.pop("CLAUDE_CODE_PLUGIN_DIRS")
        for var in CALLER_SESSION_ENV:  # nor a session from the agent that runs this script
            e.pop(var, None)
        if pane:
            e["HERDR_PANE_ID"] = pane
        prefix = ["--state-dir", str(self.state), "--host-endpoint", self.endpoint, "--json"]
        if coop:
            c = self.coords[coop]
            e["CLAUDE_CODE_SESSION_ID"] = c["session"]  # the coordinator's own claimed session
            prefix += ["--cooperative-seat", c["seat"], "--cooperative-target", c["pane"],
                       "--cooperative-harness", "claude", "--cooperative-role", "top-level"]
        argv = list(argv)
        # A self-granting command takes its words first, every option after.
        # Person/operator actions take the immediate `human` namespace before the global options.
        full = [self.bin] + (argv + prefix if argv[:1] == ["setup"] else (["human"] if human else []) + prefix + argv)
        rc, out, err = self.run(full, env=e, timeout=timeout, tag=tag or "ht:" + " ".join(argv[:2]))
        if check and rc != 0:
            raise RuntimeError(f"herdr-threads {' '.join(argv[:3])}: rc={rc} {(err or out).strip()[:300]}")
        return out

    def herdr(self, *argv, tag=None, timeout=30, check=True):
        rc, out, err = self.run(["herdr", *argv], tag=tag or "herdr:" + " ".join(argv[:2]), timeout=timeout)
        if check and rc != 0:
            raise RuntimeError(f"herdr {' '.join(argv[:3])}: rc={rc} {(err or out).strip()[:300]}")
        try:
            return json.loads(out) if out.strip() else {}
        except ValueError:
            return {"raw": out}

    # ---------- setup ----------
    def preflight(self):
        problems = []
        if os.environ.get("HERDR_ENV") != "1":
            problems.append("must run inside a Herdr pane (HERDR_ENV!=1)")
        if not self.endpoint:
            problems.append("HERDR_SOCKET_PATH is unset")
        if not os.access(self.bin, os.X_OK):
            problems.append(f"herdr-threads binary not executable: {self.bin} (run scripts/build.sh)")
        for tool in ("herdr", *self.args.harness):
            if not shutil.which(tool):
                problems.append(f"{tool} not on PATH")
        auth = self.codex_auth_home() / "auth.json"
        if "codex" in self.args.harness and not auth.exists():
            problems.append(f"no Codex credentials at {auth} (--codex-auth-home)")
        if problems:
            raise Refused("; ".join(problems))
        versions = {}
        for name, argv in (("herdr", ["herdr", "--version"]), ("herdr-threads", [self.bin, "--version"]),
                           *((h, [h, "--version"]) for h in self.args.harness)):
            versions[name] = self.run(argv, tag="version")[1].strip()[:120]
        sha = self.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], tag="git")[1].strip()
        dirty = bool(self.run(["git", "-C", str(REPO), "status", "--porcelain"], tag="git")[1].strip())
        self.facts = {"run_id": self.run_id, "sha": sha, "dirty": dirty, "versions": versions,
                      "bin": self.bin, "bin_sha256": file_sha256(self.bin), "root": str(self.root),
                      "started_utc": utc()}
        print(f"wake-e2e {self.run_id}: sha {sha[:12]}{' (dirty)' if dirty else ''}, root {self.root}", flush=True)
        for k, v in versions.items():
            print(f"  {k}: {v}", flush=True)

    def codex_auth_home(self):
        return Path(self.args.codex_auth_home or os.environ.get("CODEX_HOME") or Path.home() / ".codex").expanduser()

    def scratch(self):
        for d in (self.state, self.home, self.claude_cfg, self.codex_home, self.bindir):
            d.mkdir(mode=0o700, parents=True, exist_ok=True)
        q = shlex.quote
        shim = self.bindir / "herdr-threads"
        # Pins the run's --state-dir/--host-endpoint unless a rendered ready command already names them.
        shim.write_text("#!/bin/sh\npin_state=1 pin_host=1\nfor word in \"$@\"; do\n  case \"$word\" in\n"
                        "    --state-dir|--state-dir=*) pin_state=0 ;;\n    --host-endpoint|--host-endpoint=*) pin_host=0 ;;\n"
                        "  esac\ndone\n"
                        f"if [ \"$pin_host\" = 1 ]; then set -- --host-endpoint {q(self.endpoint)} \"$@\"; fi\n"
                        f"if [ \"$pin_state\" = 1 ]; then set -- --state-dir {q(str(self.state))} \"$@\"; fi\n"
                        f"exec {q(self.bin)} \"$@\"\n")
        shim.chmod(0o700)
        self.daemon_attempted = True
        self.ht("daemon", "ensure", tag="daemon:ensure", timeout=60)
        self.ht("daemon", "health", tag="daemon:health", timeout=20)
        setup_env = {"HOME": str(self.home), "CLAUDE_CONFIG_DIR": str(self.claude_cfg), "CODEX_HOME": str(self.codex_home),
                     "CLAUDE_CODE_PLUGIN_DIRS": ""}  # never copy a real installed mod into the scratch layer
        for h in self.args.harness:
            self.ht("setup", h, "--with-permissions", env=setup_env, tag=f"setup:{h}", timeout=120)
        if "codex" in self.args.harness:
            self.auth_digest = provision_codex_auth(self.codex_auth_home() / "auth.json", self.codex_home / "auth.json")
            cfg = self.codex_home / "config.toml"
            text = cfg.read_text() if cfg.exists() else ""
            key = f'[projects."{self.cwd}"]'
            if key not in text:
                text = text.rstrip("\n") + f"\n\n{key}\ntrust_level = \"trusted\"\n"
            # A low-quota account otherwise raises a "switch model?" modal over the composer: every wake is then
            # (correctly) refused, and the driver's next Enter would pick a menu option.
            if "[notice]" not in text:
                text = text.rstrip("\n") + "\n\n[notice]\nhide_rate_limit_model_nudge = true\n"
            cfg.write_text(text)
        print(f"  isolated daemon up; hooks installed by setup for {', '.join(self.args.harness)}", flush=True)

    def tab(self, label):
        data = self.herdr("tab", "create", "--workspace", self.workspace, "--cwd", self.cwd, "--label", label,
                          "--no-focus")
        return data["result"]["root_pane"]["pane_id"]

    def seat(self, pane):
        out = self.ht("seat", "resolve", "--pane", pane, "--new-seat", "--operator", human=True,
                      tag="seat:resolve", timeout=30)
        seat = first_id(out, "s")
        if not seat:
            raise RuntimeError(f"seat resolve {pane}: no seat id in {out[:200]}")
        return seat

    def layout(self):
        self.layout_attempted = True  # an uncertain create is found by its label at cleanup
        data = self.herdr("workspace", "create", "--label", self.workspace_label(), "--cwd", self.cwd, "--no-focus")
        self.workspace = data["result"]["workspace"]["workspace_id"]
        self.herdr("tab", "rename", data["result"]["tab"]["tab_id"], "coordinator")
        # "fresh" never contacts a recipient before busy-invite/1:1, so that case is a real first contact.
        for role, pane in (("main", data["result"]["root_pane"]["pane_id"]), ("fresh", None)):
            pane = pane or self.tab("coordinator-fresh")
            self.coords[role] = {"pane": pane, "seat": self.seat(pane), "session": str(uuid.uuid4())}
            self.ht("check-in", "--lifecycle-event", f"wake-e2e-coordinator-{uuid.uuid4()}",
                    pane=pane, coop=role, tag=f"coordinator:{role}:check-in")
        self.recipients = []
        for h in self.args.harness:
            for wake_on in ("nudge", "all"):
                name = f"{h[:2]}-{wake_on}"
                pane = self.tab(name)
                self.recipients.append(Recipient(self, name, h, wake_on, pane, self.seat(pane)))
        print(f"  workspace {self.workspace}: coordinators {self.coord_seats()}; " +
              ", ".join(f"{r.name}={r.seat}" for r in self.recipients), flush=True)

    def launch_line(self, harness):
        q = shlex.quote
        path = f"PATH={q(str(self.bindir))}:\"$PATH\""
        if harness == "claude":
            argv = ["command", "claude", "--setting-sources", "project,local", "--settings",
                    str(self.claude_cfg / "settings.json"), "--permission-mode", self.args.claude_permission_mode]
            if self.args.claude_model:
                argv += ["--model", self.args.claude_model]
            cfg = f"CLAUDE_CONFIG_DIR={q(self.args.claude_config_dir)} " if self.args.claude_config_dir else ""
            cfg += f"CLAUDE_CODE_PLUGIN_DIRS={q(str(self.state / 'claude-mod' / 'herdr-threads'))} "
            return f"unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT; {path} {cfg}HERDR_AGENT=claude {shlex.join(argv)}"
        argv = ["command", "codex", "--no-daemon", "-C", self.cwd, "--dangerously-bypass-hook-trust",
                "-c", "check_for_update_on_startup=false", *shlex.split(self.args.codex_args)]
        if self.args.codex_model:
            argv += ["-m", self.args.codex_model]
        return f"{path} HERDR_AGENT=codex CODEX_HOME={q(str(self.codex_home))} {shlex.join(argv)}"

    def coord_seats(self):
        return " and ".join(c["seat"] for c in self.coords.values()) or "none"

    def cleanup(self):
        """Attempt every owned resource independently, then verify it is gone. Returns the problems found;
        any problem makes the run fail. `--keep` is the explicit diagnostic opt-out."""
        with self.lock:  # stop admission first, whatever the drain then does
            self.stopping.set()
        try:
            problems = self.drain()
        except BaseException as e:  # a drain error never skips the owned resources below
            problems = [f"drain raised: {e!r}"]
        steps = []
        if self.args.keep:
            self.cleanup_report = {"skipped": "--keep", "workspace": self.workspace,
                                   "daemon_attempted": self.daemon_attempted, "problems": problems}
            return problems
        if not self.workspace and self.layout_attempted:
            try:
                self.workspace = self.workspace_by_label()
                steps.append({"workspace_found_by_label": self.workspace})
            except Exception as e:
                problems.append(f"workspace {self.workspace_label()}: create outcome not verified: {str(e)[:200]}")
        if self.workspace:
            try:
                rc = self.run(["herdr", "workspace", "close", self.workspace], tag="cleanup:workspace", timeout=30,
                              cleanup=True)[0]
                steps.append({"workspace_close_rc": rc})
            except Exception as e:  # keep going: the daemon is ours too
                steps.append({"workspace_close_error": str(e)[:200]})
            try:
                if self.workspace_alive():
                    problems.append(f"workspace {self.workspace} still open after close")
            except Exception as e:
                problems.append(f"workspace {self.workspace}: closure not verified: {str(e)[:200]}")
        if self.daemon_attempted:
            try:
                rc = self.run_rc("daemon", "stop", timeout=30)
                steps.append({"daemon_stop_rc": rc})
            except Exception as e:
                steps.append({"daemon_stop_error": str(e)[:200]})
            end = time.time() + self.stop_deadline_s
            healthy = True
            while True:
                try:
                    rc = self.run_rc("daemon", "health")
                    healthy = rc == 0 or not isinstance(rc, int)  # a timed-out probe is unknown, not stopped
                except Exception:
                    healthy = True  # unknown is not stopped
                if not healthy or time.time() >= end:
                    break
                time.sleep(0.5)
            if healthy:
                problems.append(f"isolated daemon still healthy {self.stop_deadline_s:.0f}s after stop")
        # Exiting agents and their session-end hooks may still name the root for a moment: allow a grace period.
        # Codex agents carry the root only in their environment; the workspace check above covers them.
        end = time.time() + self.process_grace_s
        while True:
            try:
                leftover = self.owned_processes()
            except Exception as e:  # a failed scan is unknown, never "nothing left"
                leftover = None
                scan_error = str(e)[:200]
            if leftover == [] or time.time() >= end:
                break
            time.sleep(0.5)
        if leftover is None:
            problems.append(f"owned-process scan failed: {scan_error}")
        elif leftover:
            problems.append(f"owned processes still running {self.process_grace_s:.0f}s after cleanup: {leftover}")
        if self.auth_digest is not None:
            steps.append(self.retire_auth_copy())
        self.cleanup_report = {"steps": steps, "problems": problems}
        return problems

    def drain(self):
        """Stop admitting helpers, terminate the in-flight ones (each in its own process group), then join the
        recipient workers, all with bounded waits. Returns the problems found."""
        problems = []
        with self.lock:
            self.stopping.set()
            inflight = list(self.children)
        for p in inflight:
            kill_group(p, signal.SIGTERM)
        end = time.time() + self.child_term_grace_s
        for p in inflight:
            try:
                p.wait(timeout=max(0.0, end - time.time()))
            except subprocess.TimeoutExpired:
                kill_group(p, signal.SIGKILL)
                try:
                    p.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    problems.append(f"helper pid {p.pid} did not exit: {p.args[:3]}")
        end = time.time() + self.drain_timeout_s
        for w in self.workers:
            if w.ident is None:  # never started (interrupted or failed start): nothing to drain
                continue
            w.join(timeout=max(0.0, end - time.time()))
            if w.is_alive():
                problems.append(f"worker {w.name} did not stop within {self.drain_timeout_s:.0f}s")
        with self.lock:
            late = [p.pid for p in self.children if p.poll() is None]
        if late:
            problems.append(f"helpers still running after drain: {late}")
        return problems

    def workspace_label(self):
        return f"wake-e2e-{self.run_id}"

    def workspace_by_label(self):
        rc, out, _err = self.run(["herdr", "workspace", "list"], tag="cleanup:find-workspace", timeout=30, cleanup=True)
        if rc != 0:
            raise RuntimeError(f"workspace list rc={rc}")
        for row in walk(json.loads(out or "{}")):
            if row.get("label") == self.workspace_label() and row.get("workspace_id"):
                return row["workspace_id"]
        return None

    def retire_auth_copy(self):
        """Delete the credential copy unless Codex refreshed it in the run: then it may hold the only current refresh
        token (rotation can retire the source's), so it is kept, private, and the operator is told where."""
        copy = self.codex_home / "auth.json"
        if not copy.exists():
            return {"codex_auth_copy": "absent"}
        self.auth_refreshed = file_sha256(copy) != self.auth_digest
        if not self.auth_refreshed:
            copy.unlink()
            return {"codex_auth_copy": "deleted (unchanged)"}
        print(f"wake-e2e: Codex refreshed its tokens during the run; the refreshed credentials are kept at {copy} "
              f"(mode 0600). The source profile's refresh token may have been rotated: copy this file back or log in "
              f"again, then delete it.", file=sys.stderr)
        return {"codex_auth_copy": f"kept (refreshed): {copy}"}

    def workspace_alive(self):
        rc, out, _err = self.run(["herdr", "workspace", "list"], tag="cleanup:verify-workspace", timeout=30,
                                 cleanup=True)
        if rc != 0:
            raise RuntimeError(f"workspace list rc={rc}")
        return any(row.get("workspace_id") == self.workspace for row in walk(json.loads(out or "{}")))

    def owned_processes(self):
        """pids whose command line names this run's private root (the daemon's --state-dir, the shim)."""
        rc, out, err = self.run(["pgrep", "-f", str(self.root)], tag="cleanup:verify-processes", timeout=10,
                                cleanup=True)
        if rc == 1:  # pgrep: no process matched
            return []
        if rc != 0:
            raise RuntimeError(f"pgrep rc={rc} {err.strip()[:100]}")
        return [int(p) for p in out.split() if p.isdigit() and int(p) != os.getpid()]

    def run_rc(self, *argv, timeout=10):
        e = {k: v for k, v in os.environ.items() if k not in CALLER_SESSION_ENV and k != "HERDR_PANE_ID"}
        return self.run([self.bin, "--state-dir", str(self.state), "--host-endpoint", self.endpoint, *argv],
                        env=e, tag="cleanup:" + "-".join(argv), timeout=timeout, cleanup=True)[0]


def kill_group(p, sig):
    """Signal a helper's whole process group (it was started in its own session); a helper already reaped is
    skipped, so a reused pid is never signalled."""
    if p.poll() is not None:
        return
    try:
        os.killpg(p.pid, sig)
    except (ProcessLookupError, PermissionError):
        pass


def reap(p, timeout=5):
    """Collect a killed helper's output with a bound (a descendant outside its group could hold the pipes)."""
    try:
        return p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        return "", ""


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def provision_codex_auth(source, dest):
    """A private regular-file copy (mode 0600) of the source credentials: Codex's file store truncates and rewrites
    auth.json on token refresh, which would follow a symlink into the real profile. Returns the copy's digest, so the
    run can report (without contents) whether Codex refreshed tokens."""
    dest = Path(dest)
    if dest.is_symlink() or dest.exists():
        dest.unlink()
    fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as out, open(source, "rb") as src:
        shutil.copyfileobj(src, out)
    return file_sha256(dest)


def first_id(text, prefix):
    m = re.search(r'"(%s[A-Za-z0-9]{8})"' % prefix, text)
    return m.group(1) if m else None


def walk(x):
    if isinstance(x, dict):
        yield x
        for v in x.values():
            yield from walk(v)
    elif isinstance(x, list):
        for v in x:
            yield from walk(v)


class Recipient:
    def __init__(self, run, name, harness, wake_on, pane, seat):
        self.run, self.name, self.harness, self.wake_on, self.pane, self.seat = run, name, harness, wake_on, pane, seat
        self.last_wake = 0.0
        self.group = None
        self.pair = None

    # ---------- observation ----------
    def status(self):
        return self.run.herdr("agent", "get", self.pane)["result"]["agent"]["agent_status"]

    def screen(self, lines=400):
        data = self.run.herdr("agent", "read", self.pane, "--source", "recent-unwrapped", "--lines", str(lines),
                              check=False)
        if "raw" in data:
            return data["raw"]
        return self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")

    def wait_idle(self, timeout=240):
        end = time.time() + timeout
        while time.time() < end:
            s = self.status()
            if s == "blocked":
                raise RuntimeError(f"{self.name} is blocked at a dialog: {self.tail()}")
            if s in IDLE:
                time.sleep(1.0)
                if self.status() in IDLE:
                    return
            time.sleep(0.5)
        raise RuntimeError(f"{self.name} did not become idle within {timeout}s: {self.tail()}")

    def tail(self):
        text = self.run.herdr("agent", "read", self.pane, "--source", "visible", check=False).get("raw", "")
        return " | ".join(l.strip() for l in text.splitlines() if l.strip())[-400:]

    def watch_for_wake(self, window):
        t0 = time.time()
        while time.time() - t0 < window:
            s = self.status()
            if s not in IDLE:
                return round(time.time() - t0, 2)
            time.sleep(0.25)
        return None

    def on_screen(self, needle, tries=15):
        for _ in range(tries):
            if needle in self.screen():
                return True
            time.sleep(1)
        return False

    # ---------- lifecycle ----------
    def start(self):
        end = time.time() + 30
        while time.time() < end and not re.search(r"[%$#>] ?$", self.run.herdr(
                "pane", "read", self.pane, "--source", "visible", check=False).get("raw", "").rstrip("\n ") + " "):
            time.sleep(0.5)
        self.run.herdr("pane", "run", self.pane, self.run.launch_line(self.harness))
        end = time.time() + 180
        while time.time() < end:
            data = self.run.herdr("agent", "get", self.pane, check=False)
            agent = (data.get("result") or {}).get("agent") or {}
            if agent.get("agent") == self.harness and agent.get("agent_status") in IDLE + ("blocked",):
                break
            time.sleep(1)
        else:
            raise RuntimeError(f"{self.name}: {self.harness} not detected in {self.pane}: {self.tail()}")
        if re.search(r"Not logged in|Please run /login", self.tail()):
            raise RuntimeError(f"{self.name}: {self.harness} is not logged in (check --claude-config-dir)")
        if self.status() == "blocked" or DIALOG.search(self.tail()):
            raise RuntimeError(f"{self.name}: startup dialog (nothing typed): {self.tail()}")
        for _attempt in range(2):  # one retry: a transient model/capacity error ends the first turn early
            reported = re.compile(r"SETUP-DONE wake_on=%s\b" % self.wake_on)
            self.prompt(STANDING.format(coord=self.run.coord_seats(), wake_on=self.wake_on), expect=reported)
            if reported.search(self.screen()):
                break
        else:
            raise RuntimeError(f"{self.name}: agent did not report wake-on {self.wake_on}: {self.tail()}")

    def prompt(self, text, expect=None):
        """Submit a turn and return once it is over: `expect` (text the turn prints) seen, then settled idle.
        A prompt that returns early must never let the next case start while the recipient is still mid-turn."""
        self.run.herdr("agent", "prompt", self.pane, text, "--wait", "--timeout", str(TURN_TIMEOUT_MS),
                       timeout=TURN_TIMEOUT_MS / 1000 + 30, check=False)
        if expect:
            end = time.time() + 120
            while not expect.search(self.screen()) and time.time() < end:
                time.sleep(1)
        self.wait_idle()
        self.settled()

    def settled(self):
        """The daemon's per-seat wake guard is 30 s from its last reservation, which may follow any turn, so the
        next case waits SPACING_S from the moment the recipient last went idle."""
        self.last_wake = time.time()

    # ---------- actions ----------
    def coord(self, *argv, role="main"):
        return json.loads(self.run.ht(*argv, pane=self.run.coords[role]["pane"], coop=role))

    def send(self, mode, body, direct, role="main"):
        argv = ["send"] + (["--seat", self.seat] if direct else [self.group]) + ["--body", body]
        argv += {"lazy": ["--lazy"], "nudge": ["--nudge"], "ack": ["--require-ack", self.seat]}[mode]
        data = self.coord(*argv, role=role)
        mid = (data.get("result") or {}).get("data")
        if not isinstance(mid, str) or not mid:
            raise RuntimeError(f"send returned no message id: {data}")
        return mid

    def pair_thread(self, role="main"):
        """The coordinator's 1:1 pair with this recipient (the direct send made it), or None."""
        for row in walk(self.coord("thread", "list", "--joined", role=role)):
            if str(row.get("topic_data", "")).startswith("1:1") and self.seat in str(row.get("topic_data")):
                return row.get("thread")
        return None

    def joined(self, thread, role="main"):
        data = self.coord("thread", "participants", thread, role=role)
        for row in walk(data):
            if row.get("seat") == self.seat:
                return row.get("joined") is True, row.get("effective_state")
        return False, "absent"

    def receipt(self, mid):
        data = self.coord("delivery", "recipients", mid)
        for row in walk(data):
            if row.get("seat") == self.seat and "effective_status" in row:
                return row["effective_status"]
        return None

    def wakes_since(self, anchor):
        """Wake prompts typed after the last occurrence of `anchor` (this case's own prompt) on screen."""
        text = self.screen(1000)
        at = text.rfind(anchor)
        segment = text[at:] if at >= 0 else ""
        return sum(segment.count(m) for m in WAKE_MARKERS), at >= 0

    def watch_quiet(self, seconds):
        """Seconds until the idle recipient started a turn on its own, or None if it stayed idle throughout."""
        t0 = time.time()
        while time.time() - t0 < seconds:
            if self.status() not in IDLE:
                return round(time.time() - t0, 2)
            time.sleep(0.5)
        return None

    # ---------- busy cases ----------
    def busy_case(self, case):
        action, channel = case.split("/")
        kind = action.split("-")[1]
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)  # spacing ends early when cleanup begins
        self.wait_idle()
        tag = f"{case} {self.run.run_id}"
        # Codex can swallow a prompt submitted right after a turn ends: confirm the busy turn started, retry once,
        # and never measure a "busy" case against an idle recipient.
        for _attempt in range(2):
            self.run.herdr("agent", "prompt", self.pane, BUSY_PROMPT.format(tag=tag))
            end = time.time() + 15
            while self.status() in IDLE and time.time() < end:
                time.sleep(0.25)
            if self.status() not in IDLE:
                break
            time.sleep(2)
        else:
            self.run.record({"case": case, "harness": self.harness, "wake_on": self.wake_on, "recipient": self.name,
                             "result": "BLOCKED", "checks": {"reason": "busy prompt not taken twice"}})
            return
        time.sleep(3)  # well inside `sleep 12`: the agent is running a tool
        busy_at_send = self.status()
        checks = {"busy_at_send": busy_at_send}
        role = "main"
        if kind == "invite" and channel == "1:1":
            # First contact from the fresh coordinator, mid-turn: pair + invitation + lazy opening message.
            role = "fresh"
            ref = {"message": self.send("lazy", f"pair-open-busy {self.run.run_id}", direct=True, role=role)}
            thread = self.pair_thread(role=role)
            ref["thread"] = thread
        elif kind == "invite":
            name = f"wake-e2e-busy-{self.name}-{self.run.run_id}"
            thread = self.coord("thread", "create", "--name", name, "--topic", f"wake-e2e busy test thread for {self.name}",
                                "--goal", "wake e2e test traffic")["result"]["data"]
            self.coord("invite", thread, "--seat", self.seat)
            ref = {"thread": thread}
        else:
            ref = {"message": self.send(kind, f"{case} {self.run.run_id}", direct=channel == "1:1")}
        self.wait_idle()
        # A wake owed after the turn (even one held by the 30 s per-seat guard) would start a turn in here.
        checks["self_started_turn_after_s"] = self.watch_quiet(BUSY_AFTER_S)
        self.wait_idle()
        count, found = self.wakes_since(f"Busy-turn test {tag}")
        checks["wake_prompts_since_busy_prompt"] = count if found else "busy prompt not on screen"
        if kind == "invite":
            ok = False
            for _ in range(10):
                ok, _state = self.joined(thread, role=role) if thread else (False, "no pair thread")
                if ok:
                    break
                time.sleep(1)
            checks["joined"] = ok
            if channel == "1:1":
                checks["opening_message_delivered"] = self.on_screen(ref["message"], tries=5)
        else:
            checks["delivered_in_turn"] = self.on_screen(ref["message"], tries=5)
            if kind == "ack":
                checks["ack"] = self.receipt(ref["message"])
        self.settled()
        passed = (busy_at_send not in IDLE and checks["self_started_turn_after_s"] is None
                  and checks["wake_prompts_since_busy_prompt"] == 0
                  and all(v is True or v == "acknowledged" for k, v in checks.items()
                          if k in ("joined", "opening_message_delivered", "delivered_in_turn", "ack")))
        self.run.record({"case": case, "harness": self.harness, "wake_on": self.wake_on, "recipient": self.name,
                         "expect_wake": False,
                         "woke": checks["self_started_turn_after_s"] is not None
                         or checks["wake_prompts_since_busy_prompt"] != 0, "t_wake_s": None,
                         "hello_sent": False, "checks": checks, "ref": ref,
                         "result": "PASS" if passed else "FAIL", "utc": utc()})

    # ---------- one case ----------
    def case(self, case):
        if case.startswith("busy-"):
            return self.busy_case(case)
        action, channel = case.split("/")
        expect_wake = not (action == "send-lazy" and self.wake_on == "nudge")
        gap = SPACING_S - (time.time() - self.last_wake)
        if gap > 0:
            self.run.stopping.wait(gap)  # spacing ends early when cleanup begins
        self.wait_idle()
        body = f"{case} {self.run.run_id}"
        checks = {}
        if action == "invite" and channel == "group":
            name = f"wake-e2e-{self.name}-{self.run.run_id}"
            data = self.coord("thread", "create", "--name", name, "--topic", f"wake-e2e test thread for {self.name}",
                              "--goal", "wake e2e test traffic")
            self.group = data["result"]["data"]
            self.coord("invite", self.group, "--seat", self.seat)
            ref = {"thread": self.group}
        elif action == "invite":
            # First contact: the direct send creates the pair and its invitation; the lazy body itself never wakes.
            ref = {"message": self.send("lazy", f"pair-open {self.run.run_id}", direct=True)}
            self.pair = "pending"
        else:
            ref = {"message": self.send(action.split("-")[1], body, direct=channel == "1:1")}
        t_wake = self.watch_for_wake(WAKE_WINDOW_S)
        woke = t_wake is not None
        hello = False
        if woke:
            self.last_wake = time.time()
        else:
            checks["late_wake_s"] = self.watch_for_wake(20) if expect_wake else None
            if checks["late_wake_s"] is None:
                hello = True
                self.prompt("hello")
                self.last_wake = time.time()
        self.wait_idle()
        self.settled()
        if action == "invite":
            thread = self.group if channel == "group" else None
            if thread is None:
                thread = self.pair = self.pair_thread()
            ok = False
            for _ in range(20):
                ok, state = self.joined(thread)
                if ok:
                    break
                time.sleep(1)
            checks["joined"] = ok
            if channel == "1:1":
                checks["opening_message_delivered"] = self.on_screen(ref["message"])
        else:
            checks["delivered"] = self.on_screen(ref["message"])
            if action == "send-ack":
                status = None
                for _ in range(20):
                    status = self.receipt(ref["message"])
                    if status == "acknowledged":
                        break
                    time.sleep(1)
                checks["ack"] = status
        verified = all(v is True or v == "acknowledged" for k, v in checks.items() if k != "late_wake_s")
        passed = woke == expect_wake and verified
        row = {"case": case, "harness": self.harness, "wake_on": self.wake_on, "recipient": self.name,
               "expect_wake": expect_wake, "woke": woke, "t_wake_s": t_wake, "hello_sent": hello,
               "checks": checks, "ref": ref, "result": "PASS" if passed else "FAIL", "utc": utc()}
        self.run.record(row)

    def ensure_channel(self, channel):
        """Unmeasured setup when the matching invite case did not run first: the recipient joins the ordinary
        thread / accepts the 1:1 pair, so the send cases test delivery and not first contact."""
        if channel == "group" and not self.group:
            name = f"wake-e2e-{self.name}-{self.run.run_id}"
            self.group = self.coord("thread", "create", "--name", name, "--topic", f"wake-e2e test thread for {self.name}",
                                    "--goal", "wake e2e test traffic")["result"]["data"]
            self.coord("invite", self.group, "--seat", self.seat)
            thread = self.group
        elif channel == "1:1" and not self.pair:
            self.send("nudge", f"pair-setup {self.run.run_id}", direct=True)
            thread = self.pair = self.pair_thread()
        else:
            return
        end = time.time() + 120
        while time.time() < end and not self.joined(thread)[0]:
            if self.status() in IDLE and time.time() > end - 90:
                self.prompt("Handle any pending herdr-threads items as your standing instructions say.")
            time.sleep(2)
        if not self.joined(thread)[0]:
            raise RuntimeError(f"setup: {self.name} did not join the {channel} channel")
        self.wait_idle()
        self.settled()

    def run_cases(self, cases):
        try:
            self.start()
            for case in cases:
                if self.run.stopping.is_set():
                    break
                try:
                    if not case.startswith("invite/") and "invite" not in case.split("/")[0]:
                        self.ensure_channel(case.split("/")[1])
                    self.case(case)
                except Stopped:
                    break
                except Exception as e:  # a driver or agent error fails the case, never passes it
                    self.run.record({"case": case, "harness": self.harness, "wake_on": self.wake_on,
                                     "recipient": self.name, "result": "FAIL", "checks": {"error": str(e)[:400]}})
        except Stopped:
            pass
        except Exception as e:
            for case in cases:
                self.run.record({"case": case, "harness": self.harness, "wake_on": self.wake_on,
                                 "recipient": self.name, "result": "FAIL", "checks": {"error": f"startup: {str(e)[:400]}"}})
        finally:
            try:
                (self.run.ev / f"pane-{self.name}.txt").write_text(self.screen(1000))
            except Stopped:
                pass  # interrupted: no pane capture, the summary marks unfinished cases missing


def record(self, row):
    with self.lock:
        self.results.append(row)
        print(f"[{row['result']:7}] {row['recipient']:9} {row['case']:17} wake_on={row['wake_on']:5} "
              f"expect={row.get('expect_wake')!s:5} woke={row.get('woke')!s:5} t={row.get('t_wake_s')} "
              f"{json.dumps(row.get('checks'))}", flush=True)


Run.record = record


def expected_rows(run, cases):
    return {(r.name, c) for r in getattr(run, "recipients", []) for c in cases}


def summary(run, cases):
    """The table of every collected row. Passes only when each expected recipient x case has exactly one row and it
    is PASS: a missing, duplicated or unexpected row fails the run."""
    order = {c: i for i, c in enumerate(CASES)}
    rows = sorted(run.results, key=lambda r: (order.get(r["case"], 99), r["wake_on"] != "nudge", r["harness"]))
    lines = ["| case | wake-on | harness | expect wake | woke (s) | checks | result |", "|---|---|---|---|---|---|---|"]
    for r in rows:
        woke = f"{r.get('woke')} ({r.get('t_wake_s')})" if r.get("woke") else str(r.get("woke"))
        lines.append(f"| {r['case']} | {r['wake_on']} | {r['harness']} | {r.get('expect_wake')} | {woke} | "
                     f"{json.dumps(r.get('checks'))} | {r['result']} |")
    expected = expected_rows(run, cases)
    seen = [(r["recipient"], r["case"]) for r in rows]
    missing = sorted(expected - set(seen))
    extra = sorted({k for k in seen if k not in expected or seen.count(k) > 1})
    passed = sum(r["result"] == "PASS" for r in rows)
    lines.append(f"\n{passed}/{len(expected)} expected rows passed ({len(rows)} collected)")
    if missing:
        lines.append(f"missing rows: {missing}")
    if extra:
        lines.append(f"unexpected or duplicated rows: {extra}")
    run.table = "\n".join(lines)
    run.completeness = {"expected": len(expected), "collected": len(rows), "missing": missing, "extra": extra}
    print("\n" + run.table, flush=True)
    return bool(expected) and not missing and not extra and passed == len(rows)


def write_evidence(run, rc):
    """report.md / results.json, written after cleanup so they carry its outcome and the final exit code."""
    facts = dict(getattr(run, "facts", {}), finished_utc=utc(), exit_code=rc)
    if run.auth_digest:
        copy = run.codex_home / "auth.json"
        refreshed = run.auth_refreshed
        if refreshed is None and copy.exists():  # --keep: the copy was not retired
            refreshed = file_sha256(copy) != run.auth_digest
        facts["codex_auth_refreshed_in_run"] = refreshed
    with run.lock:  # workers left running after an interrupt may still append
        results = list(run.results)
    doc = {"facts": facts, "completeness": getattr(run, "completeness", None), "cleanup": run.cleanup_report,
           "results": results}
    (run.ev / "results.json").write_text(json.dumps(doc, indent=2))
    (run.ev / "report.md").write_text(
        f"# wake e2e {run.run_id}\n\n```json\n{json.dumps(facts, indent=2)}\n```\n\n{getattr(run, 'table', '')}\n\n"
        f"cleanup: `{json.dumps(run.cleanup_report)}`\n\nexit code: {rc}\n")


def main(argv=None):
    ap = argparse.ArgumentParser(prog="wake-e2e", description=__doc__.splitlines()[0])
    ap.add_argument("--bin", default=str(REPO / "bin" / "herdr-threads"), help="herdr-threads binary under test")
    ap.add_argument("--harness", default="claude,codex", help="comma-separated: claude,codex")
    ap.add_argument("--cases", default=",".join(CASES), help="comma-separated subset of: " + ", ".join(CASES))
    ap.add_argument("--evidence-dir", help="default: <run root>/evidence")
    ap.add_argument("--keep", action="store_true", help="leave the workspace and daemon running for inspection")
    ap.add_argument("--claude-model", help="passed as --model (default: the account default)")
    ap.add_argument("--claude-permission-mode", default="auto")
    ap.add_argument("--claude-config-dir", default=os.environ.get("CLAUDE_CONFIG_DIR"),
                    help="the real, logged-in Claude config dir the agents use (default $CLAUDE_CONFIG_DIR; new Herdr "
                         "panes do not inherit it)")
    ap.add_argument("--codex-model", help="passed as -m (default: the profile default)")
    ap.add_argument("--codex-args", default="--approve-for-me",
                    help="extra Codex TUI flags (sandbox/approval), default %(default)r")
    ap.add_argument("--codex-auth-home", help="Codex home whose auth.json is copied (private, mode 0600) into "
                                              "the scratch CODEX_HOME "
                                              "(default $CODEX_HOME or ~/.codex)")
    ap.add_argument("--self-test", action="store_true",
                    help="check the driver's credential, cleanup and completeness paths (no harnesses) and exit")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    args.harness = [h for h in args.harness.split(",") if h]
    cases = [c for c in args.cases.split(",") if c]
    unknown = [c for c in cases if c not in CASES] + [h for h in args.harness if h not in ("claude", "codex")]
    if unknown:
        ap.error(f"unknown case/harness: {unknown}")
    def terminate(signum, _frame):
        raise KeyboardInterrupt(f"signal {signum}")
    saved = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
    signal.signal(signal.SIGTERM, terminate)  # SIGTERM takes the same cleanup path as Ctrl-C
    run = Run(args)
    rc = EXIT_FAIL
    try:
        run.preflight()
        run.scratch()
        run.layout()
        workers = [threading.Thread(target=r.run_cases, args=(cases,), name=r.name, daemon=True)
                   for r in run.recipients]
        run.workers = workers  # registered before starting; drain skips any that never started
        for w in workers:
            if run.stopping.is_set():
                break
            w.start()
        while any(w.is_alive() for w in workers):  # short joins keep Ctrl-C / SIGTERM responsive
            for w in workers:
                if w.ident is not None:
                    w.join(timeout=0.5)
        rc = EXIT_OK if summary(run, cases) else EXIT_FAIL
    except Refused as e:
        print(f"wake-e2e: refused: {e}", file=sys.stderr)
        rc = EXIT_REFUSED
    except BaseException as e:  # including KeyboardInterrupt: cleanup still runs and the run fails
        print(f"wake-e2e: aborted: {e!r}", file=sys.stderr)
        rc = EXIT_FAIL
    finally:
        # Cleanup runs to the end: a second Ctrl-C or SIGTERM must not cut off the remaining steps.
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            problems = run.cleanup()
        except BaseException as e:
            problems = [f"cleanup raised: {e!r}"]
            run.cleanup_report = {"problems": problems}
        if problems:
            print("wake-e2e: cleanup incomplete: " + "; ".join(problems), file=sys.stderr)
            rc = EXIT_FAIL
        write_evidence(run, rc)
        print(f"evidence: {run.ev} (exit {rc})", flush=True)
        for sig, handler in saved.items():
            signal.signal(sig, handler)
    return rc


SELF_TEST = Path(__file__).resolve().parent / "wake_e2e_selftest.py"


def self_test():
    """The driver's own failure paths, with fakes for Herdr and the daemon (scripts/wake_e2e_selftest.py)."""
    import unittest
    sys.path.insert(0, str(SELF_TEST.parent))
    suite = unittest.defaultTestLoader.loadTestsFromName(SELF_TEST.stem)
    return EXIT_OK if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else EXIT_FAIL


if __name__ == "__main__":
    sys.exit(main())
