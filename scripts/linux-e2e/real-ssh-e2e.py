#!/usr/bin/env python3
"""Pre-release real-ssh two-node check: do two nodes link over the DEFAULT ssh transport (no sync.dial_command)?

  scripts/linux-e2e/run real-ssh [common options] [-- DRIVER ARGS...]     # the usual way (builds the binary, then this)
  scripts/linux-e2e/real-ssh-e2e.py --image IMAGE --bin LINUX_BINARY [--platform linux/arm64|linux/amd64]
                                    [--cases ROW,...] [--evidence-dir DIR] [--keep] [--docker DOCKER]
  scripts/linux-e2e/real-ssh-e2e.py --self-test                          # no Docker: fake docker and ssh on PATH

Every other test replaces the transport with sync.dial_command; this is the one check that runs the product's own
`ssh -o BatchMode=yes ... -- <dest> <remote command>` argv against a real sshd and the default remote command
(`command -v herdr-threads`, then `$HOME/.local/bin/herdr-threads`; peer-link design, TransportCommand::build).

Two variants, each on its own private Docker network with two containers of the runner image (A: the dialing node,
B: sshd with key-only auth as user `tester`, herdr-threads installed on B only as the variant says), each node with
its own headless Herdr server and daemon:
  local-bin   herdr-threads only at ~/.local/bin on B, which is not on sshd's non-interactive PATH
  on-path     herdr-threads only at /usr/local/bin on B, which is
Rows (variant/case), all required:
  local-bin/no-key-backoff         BatchMode=yes with no accepted key -> backoff/stopped with the ssh error as
                                   last_error, reached within a bound (no prompt hang)
  local-bin/default-command        default remote command finds ~/.local/bin/herdr-threads: the target links
  local-bin/converge               both nodes hold the same vector of origin logs
  local-bin/send-ack               a person seat on A sends --require-ack to a seat on B; B displays it; A sees the ACK
  local-bin/daemon-stop-reaps-ssh  `daemon stop` on A kills and reaps the ssh child (no process, no zombie)
  local-bin/no-leaks               no herdr-threads, ssh or pump process is left in either container
  on-path/default-command          default remote command finds herdr-threads on PATH: the target links
  on-path/converge                 the vectors converge
  on-path/explicit-remote-command  `peer add --remote-command C` runs C on B (a wrapper that leaves a marker) and links
  on-path/no-leaks                 as above
  cleanup/containers               every container and network this run created is gone
Exit codes: 0 every row present and passed and cleanup verified; 1 a row failed or is missing, or cleanup left
anything; 2 environment refusal (no Docker, no image, no binary). Evidence (report.md, results.json, host.log,
daemon logs) goes to --evidence-dir. A pre-release check run by a person; never in CI.
Python 3.9+, standard library only. Nothing outside Docker objects is touched; keys are generated inside the
containers and the real user config is never read or written.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

EXIT_OK, EXIT_FAIL, EXIT_REFUSED = 0, 1, 2
LABEL = "dev.herdr-threads.real-ssh-e2e"
HOME = "/home/tester"
SOCKET = f"{HOME}/run/h.sock"
USER = "tester"
LINK_WAIT_S = 90
NOKEY_WAIT_S = 60
CONVERGE_WAIT_S = 90
SEAT_WAIT_S = 90
REAP_WAIT_S = 30
# The documented defaults (peer-link design, TransportCommand::build): the driver probes with the same options.
SSH_OPTIONS = ["-o", "BatchMode=yes", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3",
               "-o", "ControlMaster=no", "-o", "ControlPath=none", "-o", "ConnectTimeout=20", "-T"]
DEFAULT_REMOTE_COMMAND = 'exec "$(command -v herdr-threads || echo "$HOME/.local/bin/herdr-threads")" peer --stdio'
WRAPPER = f"{HOME}/ht-wrapper.sh"
MARKER = f"{HOME}/wrapper-used"
EXPLICIT_REMOTE_COMMAND = f"exec {WRAPPER}"
# Capabilities sshd needs as root in a container that drops all of them (privilege separation, chroot, chown).
SSHD_CAPS = ["CHOWN", "DAC_OVERRIDE", "FOWNER", "SETGID", "SETUID", "SYS_CHROOT", "KILL", "NET_BIND_SERVICE"]
# Process lines that must not outlive a variant (matched against `ps -eo pid,stat,args`).
LEAK_PATTERN = re.compile(r"herdr-threads|herdr server|ssh -o BatchMode|peer --stdio|sshd: tester")
SSHD_SETUP = r"""set -e
mkdir -p /run/sshd /etc/ssh/sshd_config.d
ssh-keygen -A >/dev/null
# Unlock tester for key-only login (a '!' password locks sshd out with UsePAM no); the image has no usermod.
sed -i -E 's/^(tester:)[^:]*:/\1*:/' /etc/shadow
printf '%s\n' 'PasswordAuthentication no' 'KbdInteractiveAuthentication no' 'PubkeyAuthentication yes' \
  'UsePAM no' 'PermitRootLogin no' 'AllowUsers tester' > /etc/ssh/sshd_config.d/10-ht.conf
install -d -m 700 -o tester -g tester /home/tester/.ssh
exec /usr/sbin/sshd -D -e
"""

ROWS = ["local-bin/no-key-backoff", "local-bin/default-command", "local-bin/converge", "local-bin/send-ack",
        "local-bin/daemon-stop-reaps-ssh", "local-bin/no-leaks",
        "on-path/default-command", "on-path/converge", "on-path/explicit-remote-command", "on-path/no-leaks",
        "cleanup/containers"]


class Refused(Exception):
    pass


class Interrupted(BaseException):
    def __init__(self, signum):
        super().__init__(signum)
        self.signum = signum


# ---------- argv construction (unit-tested by --self-test) ----------
def labels(run_id):
    return ["--label", f"{LABEL}=1", "--label", f"{LABEL}.run={run_id}"]


def network_create_argv(docker, net, run_id):
    return [docker, "network", "create", *labels(run_id), net]


def node_run_argv(docker, role, name, net, image, run_id, platform=None):
    """`docker run` for node A (the dialer: unprivileged, sleeps) or B (sshd as root with only the capabilities
    privilege separation needs). Both: --init, labelled, no published ports, bounded life."""
    argv = [docker, "run", "-d", "--init", "--name", name, "--network", net, "--hostname", name, *labels(run_id),
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges"]
    if platform:
        argv += ["--platform", platform]
    if role == "B":
        for cap in SSHD_CAPS:
            argv += ["--cap-add", cap]
        return argv + ["--user", "root", image, "bash", "-c", SSHD_SETUP]
    return argv + [image, "sleep", "3600"]


def ssh_probe_argv(dest, command):
    """What the driver runs on A to look at B the way the daemon does: the product's ssh options, `--` before the
    destination, the command as one string for the remote login shell."""
    return ["ssh", *SSH_OPTIONS, "--", dest, command]


def walk(x):
    if isinstance(x, dict):
        yield x
        for v in x.values():
            yield from walk(v)
    elif isinstance(x, list):
        for v in x:
            yield from walk(v)


def parse_json(text):
    try:
        return json.loads(text)
    except ValueError:
        for line in reversed(text.strip().splitlines()):
            try:
                return json.loads(line)
            except ValueError:
                continue
    return None


def target_rows(doc, dest):
    return [r for r in walk(doc) if r.get("ssh_dest") == dest and "state" in r]


def log_vector(doc):
    """The set of (node, incarnation, counter, head) of every origin log a `sync status --json` reply lists."""
    return {(r["node"], r["incarnation"], r["counter"], r.get("head"))
            for r in walk(doc) if {"node", "incarnation", "counter", "head"} <= set(r)}


def leaked_processes(ps_text):
    """Lines of `ps -eo pid,stat,args` output that name a process this run starts and must stop."""
    out = []
    for line in ps_text.splitlines():
        if LEAK_PATTERN.search(line) and "ps -eo" not in line and "grep" not in line:
            out.append(line.strip())
    return out


def zombies(ps_text):
    return [ln.strip() for ln in ps_text.splitlines() if re.match(r"\s*\d+\s+Z", ln)]


def check_rows(expected, results):
    """(ok, problems): every expected row present exactly once and passed; an unknown row is a problem too."""
    problems = []
    seen = {}
    for r in results:
        seen[r["row"]] = seen.get(r["row"], 0) + 1
    for row in expected:
        if row not in seen:
            problems.append(f"missing row: {row}")
    for row, n in seen.items():
        if row not in expected:
            problems.append(f"unexpected row: {row}")
        elif n > 1:
            problems.append(f"duplicate row: {row}")
    for r in results:
        if not r.get("passed"):
            problems.append(f"failed row: {r['row']}: {r.get('detail', '')}")
    return not problems, problems


# ---------- docker ----------
class Docker:
    """Thin wrapper over the docker CLI: every call is logged to evidence/host.log; created objects are recorded
    (before the attempt, so an uncertain create is still cleaned) for cleanup."""

    def __init__(self, binary, evidence, run_id):
        self.bin = binary
        self.evidence = Path(evidence)
        self.run_id = run_id
        self.containers = []
        self.networks = []
        self.log = self.evidence / "host.log"

    def run(self, argv, input_bytes=None, timeout=120, check=False):
        with self.log.open("a", encoding="utf-8", errors="replace") as f:
            f.write("$ " + " ".join(shlex.quote(a) if len(a) < 200 else "<script>" for a in argv) + "\n")
        try:
            p = subprocess.run(argv, input=input_bytes, capture_output=True, timeout=timeout)
            rc, out, err = p.returncode, p.stdout.decode(errors="replace"), p.stderr.decode(errors="replace")
        except subprocess.TimeoutExpired:
            rc, out, err = 124, "", f"timed out after {timeout}s"
        with self.log.open("a", encoding="utf-8", errors="replace") as f:
            f.write(f"  rc={rc}\n" + "".join(f"  | {ln}\n" for ln in (out + err).splitlines()[-30:]))
        if check and rc != 0:
            raise RuntimeError(f"{' '.join(argv[:4])}: rc={rc} {(err or out).strip()[:300]}")
        return rc, out, err

    def d(self, *args, **kw):
        return self.run([self.bin, *args], **kw)

    def create_network(self, net):
        self.networks.append(net)
        self.run(network_create_argv(self.bin, net, self.run_id), check=True)

    def start_node(self, role, name, net, image, platform):
        self.containers.append(name)
        self.run(node_run_argv(self.bin, role, name, net, image, self.run_id, platform), check=True)

    def cleanup(self):
        """Remove everything this run created; returns the list of problems (empty when verified gone)."""
        problems = []
        for c in reversed(self.containers):
            self.d("rm", "-f", "-v", c)
        for n in reversed(self.networks):
            self.d("network", "rm", n)
        rc, out, _ = self.d("ps", "-a", "-q", "--filter", f"label={LABEL}.run={self.run_id}")
        if rc != 0 or out.split():
            problems.append(f"containers left: {out.split() or 'docker ps failed'}")
        rc, out, _ = self.d("network", "ls", "-q", "--filter", f"label={LABEL}.run={self.run_id}")
        if rc != 0 or out.split():
            problems.append(f"networks left: {out.split() or 'docker network ls failed'}")
        return problems


class Node:
    def __init__(self, docker, role, name):
        self.docker, self.role, self.name = docker, role, name
        self.pane = None
        self.seat = None

    def sh(self, script, user=USER, env=None, input_bytes=None, timeout=60):
        argv = [self.docker.bin, "exec", "-i", "-u", user]
        for k, v in (env or {}).items():
            argv += ["-e", f"{k}={v}"]
        return self.docker.run(argv + [self.name, "bash", "-c", script], input_bytes=input_bytes, timeout=timeout)

    def put(self, dest, data, user=USER, mode="755"):
        q = shlex.quote
        script = (f"set -e; umask 022; mkdir -p $(dirname {q(dest)}); cat > {q(dest)}.tmp; "
                  f"chmod {mode} {q(dest)}.tmp; mv {q(dest)}.tmp {q(dest)}")
        rc, _, err = self.sh(script, user=user, input_bytes=data)
        if rc != 0:
            raise RuntimeError(f"{self.name}: put {dest}: {err.strip()[:200]}")

    def ht(self, *argv, human=False, pane=None, timeout=60):
        """The node's own herdr-threads against its own daemon and Herdr server; (rc, parsed JSON or None, text)."""
        env = {"HERDR_SOCKET_PATH": SOCKET}
        if pane:
            env["HERDR_PANE_ID"] = pane
        full = ["herdr-threads"] + (["human"] if human else []) + ["--host-endpoint", SOCKET, "--json", *argv]
        rc, out, err = self.sh(" ".join(shlex.quote(a) for a in full), env=env, timeout=timeout)
        return rc, parse_json(out), out + err

    def start_herdr(self):
        rc, _, err = self.sh(f"mkdir -p {HOME}/run; HERDR_SOCKET_PATH={SOCKET} setsid nohup herdr server </dev/null "
                             f">{HOME}/run/herdr.log 2>&1 & echo $! > {HOME}/run/herdr.pid; "
                             f"for i in $(seq 150); do [ -S {SOCKET} ] && exit 0; sleep 0.1; done; exit 1")
        if rc != 0:
            raise RuntimeError(f"{self.name}: herdr server did not come up: {err.strip()[:200]}")

    def stop_herdr(self):
        self.sh(f"[ -f {HOME}/run/herdr.pid ] && kill $(cat {HOME}/run/herdr.pid) 2>/dev/null; sleep 1; true")

    def make_seat(self):
        env = {"HERDR_SOCKET_PATH": SOCKET}
        rc, out, err = self.sh(f"herdr workspace create --label ht-real-ssh --cwd {HOME} --no-focus", env=env)
        doc = parse_json(out)
        pane = next((r["pane_id"] for r in walk(doc) if "pane_id" in r), None) if doc else None
        if rc != 0 or not pane:
            raise RuntimeError(f"{self.name}: workspace create: {(err or out).strip()[:200]}")
        rc, doc, text = self.ht("me", "init", human=True, pane=pane)
        seat = next((m.group(1) for m in [re.search(r'"(s[A-Za-z0-9]{11})"', text)] if m), None)
        if rc != 0 or not seat:
            raise RuntimeError(f"{self.name}: me init: {text.strip()[:200]}")
        self.pane, self.seat = pane, seat

    def ps(self):
        return self.sh("ps -eo pid,stat,args")[1]

    def daemon_logs(self):
        return self.sh(f"for f in $(find {HOME} -name daemon.log -size +0 2>/dev/null); do echo \"== $f\"; "
                       f"tail -n 300 \"$f\"; done")[1]


# ---------- the driver ----------
class Driver:
    def __init__(self, args):
        self.args = args
        self.run_id = uuid.uuid4().hex[:8]
        self.evidence = Path(args.evidence_dir) if args.evidence_dir else (
            Path(tempfile.gettempdir()) / "ht-real-ssh" / self.run_id)
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.docker = Docker(args.docker, self.evidence, self.run_id)
        self.results = []
        self.problems = []

    def row(self, name, passed, detail=""):
        self.results.append({"row": name, "passed": bool(passed), "detail": detail})
        print(f"  {'PASS' if passed else 'FAIL'} {name}{': ' + detail if detail and not passed else ''}", flush=True)

    def wanted(self, name):
        return not self.args.cases or name in self.args.cases

    def poll(self, what, timeout, probe, every=1.0):
        end = time.time() + timeout
        last = None
        while time.time() < end:
            last = probe()
            if last:
                return last
            time.sleep(every)
        return None

    # -- setup --
    def authorize(self, a, b):
        """A generates a key (once) and B accepts it; B's host key goes to A's known_hosts."""
        rc, out, err = a.sh(f"set -e; umask 077; mkdir -p ~/.ssh; [ -f ~/.ssh/id_ed25519 ] || "
                            f"ssh-keygen -q -t ed25519 -N '' -f ~/.ssh/id_ed25519; cat ~/.ssh/id_ed25519.pub")
        if rc != 0:
            raise RuntimeError(f"ssh-keygen on A: {err.strip()[:200]}")
        rc, _, err = b.sh("umask 077; mkdir -p ~/.ssh; cat >> ~/.ssh/authorized_keys", input_bytes=out.encode())
        if rc != 0:
            raise RuntimeError(f"authorize key on B: {err.strip()[:200]}")

    def trust_host(self, a, b):
        ok = self.poll("sshd up", 30, lambda: a.sh(f"ssh-keyscan -t ed25519 {b.name} 2>/dev/null | grep -q ssh-ed25519")[0] == 0)
        if not ok:
            raise RuntimeError("sshd on B never answered ssh-keyscan")
        rc, _, err = a.sh(f"set -e; umask 077; mkdir -p ~/.ssh; ssh-keyscan -t ed25519 {b.name} 2>/dev/null >> ~/.ssh/known_hosts")
        if rc != 0:
            raise RuntimeError(f"known_hosts: {err.strip()[:200]}")

    def bring_up(self, variant, binary):
        net = f"ht-rs-{self.run_id}-{variant}"
        a_name, b_name = f"ht-rs-{self.run_id}-{variant}-a", f"ht-rs-{self.run_id}-{variant}-b"
        d = self.docker
        d.create_network(net)
        d.start_node("B", b_name, net, self.args.image, self.args.platform)
        d.start_node("A", a_name, net, self.args.image, self.args.platform)
        a, b = Node(d, "A", a_name), Node(d, "B", b_name)
        a.put(f"{HOME}/.local/bin/herdr-threads", binary)
        if variant == "local-bin":
            b.put(f"{HOME}/.local/bin/herdr-threads", binary)
        else:
            b.put("/usr/local/bin/herdr-threads", binary, user="root")
        for n in (a, b):
            # The state dir an installed plugin leaves behind: the CLI's default (and peer --stdio
            # on B over ssh) uses it only when it exists.
            rc, _, err = n.sh(f"mkdir -p {HOME}/.local/state/herdr/plugins/herdr-threads")
            if rc != 0:
                raise RuntimeError(f"{n.name}: state dir: {err.strip()[:200]}")
            n.start_herdr()
            rc, _, text = n.ht("daemon", "ensure")
            if rc != 0:
                raise RuntimeError(f"{n.name}: daemon ensure: {text.strip()[:200]}")
        self.trust_host(a, b)
        return a, b

    # -- probes --
    def link_state(self, a, dest):
        rc, doc, _ = a.ht("peer", "list")
        rows = target_rows(doc, dest) if doc else []
        return rows[0] if rows else None

    def wait_linked(self, a, dest, timeout=LINK_WAIT_S):
        return self.poll("linked", timeout, lambda: (r := self.link_state(a, dest)) and r["state"] == "linked" and r)

    def vectors(self, node):
        rc, doc, _ = node.ht("sync", "status")
        return log_vector(doc) if doc else set()

    def wait_converged(self, a, b):
        def probe():
            va, vb = self.vectors(a), self.vectors(b)
            nodes = {v[0] for v in va}
            return (va, vb) if va == vb and len(nodes) >= 2 else None
        return self.poll("converged", CONVERGE_WAIT_S, probe, every=2.0)

    def ssh(self, a, b, command, timeout=60):
        return a.sh(" ".join(shlex.quote(x) for x in ssh_probe_argv(f"{USER}@{b.name}", command)), timeout=timeout)

    def leak_row(self, variant, nodes):
        for n in nodes:
            n.ht("daemon", "stop")
            n.stop_herdr()
        time.sleep(3)
        leaks = {}
        for n in nodes:
            found = leaked_processes(n.ps())
            if found:
                leaks[n.name] = found
        self.row(f"{variant}/no-leaks", not leaks, f"leaked: {leaks}")

    def collect(self, variant, nodes):
        for n in nodes:
            (self.evidence / f"{variant}-{n.role}-daemon.log").write_text(n.daemon_logs())
            (self.evidence / f"{variant}-{n.role}-ps.txt").write_text(n.ps())

    # -- variants --
    def variant_local_bin(self, binary):
        v = "local-bin"
        a, b = self.bring_up(v, binary)
        dest = f"{USER}@{b.name}"
        try:
            # BatchMode with no accepted key: the dial must fail fast, not hang on a prompt.
            if self.wanted(f"{v}/no-key-backoff"):
                rc, doc, text = a.ht("peer", "add", dest, human=True)
                t0 = time.time()
                row = self.poll("failed dial", NOKEY_WAIT_S, lambda: (r := self.link_state(a, dest))
                                and r["state"] in ("backoff", "stopped") and r.get("last_error") and r)
                err = (row or {}).get("last_error") or ""
                ok = bool(row) and re.search(r"permission denied|publickey", err, re.I) is not None
                self.row(f"{v}/no-key-backoff", rc == 0 and ok,
                         f"peer add rc={rc}; row={row}; after {time.time() - t0:.0f}s (bound {NOKEY_WAIT_S}s)")
            self.authorize(a, b)
            if self.wanted(f"{v}/default-command"):
                rc, out, _ = self.ssh(a, b, "command -v herdr-threads")
                rc2, _, _ = self.ssh(a, b, f"test -x $HOME/.local/bin/herdr-threads")
                pre = rc != 0 and not out.strip() and rc2 == 0
                a.ht("peer", "add", dest, human=True)  # re-adding resets the failed target (the operator's "changed")
                row = self.wait_linked(a, dest) if pre else None
                self.row(f"{v}/default-command", pre and bool(row),
                         f"precondition (not on PATH, present at ~/.local/bin): {pre}; target: {self.link_state(a, dest)}")
            if self.wanted(f"{v}/converge"):
                self.row(f"{v}/converge", bool(self.wait_converged(a, b)),
                         f"A={sorted(self.vectors(a))} B={sorted(self.vectors(b))}")
            if self.wanted(f"{v}/send-ack"):
                self.send_ack(v, a, b)
            if self.wanted(f"{v}/daemon-stop-reaps-ssh"):
                self.stop_reaps(v, a)
            self.leak_row(v, [a, b])
        finally:
            self.collect(v, [a, b])

    def variant_on_path(self, binary):
        v = "on-path"
        a, b = self.bring_up(v, binary)
        dest = f"{USER}@{b.name}"
        try:
            self.authorize(a, b)
            if self.wanted(f"{v}/default-command"):
                rc, out, _ = self.ssh(a, b, "command -v herdr-threads")
                rc2, _, _ = self.ssh(a, b, "test ! -e $HOME/.local/bin/herdr-threads")
                pre = rc == 0 and out.strip().endswith("herdr-threads") and rc2 == 0
                rc, _, text = a.ht("peer", "add", dest, human=True)
                row = self.wait_linked(a, dest) if pre and rc == 0 else None
                self.row(f"{v}/default-command", pre and bool(row),
                         f"precondition (on PATH: {out.strip()!r}, none at ~/.local/bin): {pre}; target: {self.link_state(a, dest)}")
            if self.wanted(f"{v}/converge"):
                self.row(f"{v}/converge", bool(self.wait_converged(a, b)),
                         f"A={sorted(self.vectors(a))} B={sorted(self.vectors(b))}")
            if self.wanted(f"{v}/explicit-remote-command"):
                b.put(WRAPPER, f"#!/bin/sh\necho used >> {MARKER}\nexec herdr-threads peer --stdio\n".encode())
                b.sh(f"rm -f {MARKER}")
                rc, _, text = a.ht("peer", "add", "--remote-command", EXPLICIT_REMOTE_COMMAND, dest, human=True)
                used = self.poll("wrapper used", LINK_WAIT_S, lambda: b.sh(f"test -s {MARKER}")[0] == 0)
                row = self.wait_linked(a, dest) if used else None
                self.row(f"{v}/explicit-remote-command", rc == 0 and bool(used) and bool(row),
                         f"peer add rc={rc}; wrapper marker on B: {bool(used)}; target: {self.link_state(a, dest)}")
            self.leak_row(v, [a, b])
        finally:
            self.collect(v, [a, b])

    def send_ack(self, v, a, b):
        try:
            a.make_seat()
            b.make_seat()
        except RuntimeError as e:
            self.row(f"{v}/send-ack", False, str(e))
            return
        body = f"real-ssh-{self.run_id}"
        sent = {}

        def send():
            rc, doc, text = a.ht("send", "--seat", b.seat, "--body", body, "--require-ack", b.seat,
                                 human=True, pane=a.pane)
            data = (doc or {}).get("result", {}).get("data") if isinstance(doc, dict) else None
            if rc == 0 and isinstance(data, str) and data:
                sent["id"] = data
                return data
            sent["error"] = text.strip()[:200]
        mid = self.poll("B's seat visible on A", SEAT_WAIT_S, send, every=3.0)
        if not mid:
            self.row(f"{v}/send-ack", False, f"send never accepted: {sent.get('error')}")
            return

        def shown():  # reading B's inbox displays (and so ACKs) the message
            rc, doc, text = b.ht("inbox", human=False, pane=b.pane)
            return mid in text or body in text

        arrived = self.poll("message on B", SEAT_WAIT_S, shown, every=3.0)

        def status():
            rc, doc, _ = a.ht("delivery", "recipients", mid, human=True)
            for r in walk(doc):
                if r.get("seat") == b.seat and "effective_status" in r:
                    return r["effective_status"] if r["effective_status"] in ("acknowledged", "displayed", "read") else None
        got = self.poll("ACK on A", SEAT_WAIT_S, status, every=3.0) if arrived else None
        self.row(f"{v}/send-ack", bool(arrived) and bool(got), f"message {mid} on B: {bool(arrived)}; A sees: {got}")

    def stop_reaps(self, v, a):
        def children():
            return [ln.split()[0] for ln in a.sh("ps -eo pid,args | grep '[s]sh -o BatchMode'")[1].splitlines() if ln.strip()]
        before = children()
        a.ht("daemon", "stop")
        gone = self.poll("ssh child gone", REAP_WAIT_S,
                         lambda: not children() and not [z for z in zombies(a.ps()) if "ssh" in z])
        self.row(f"{v}/daemon-stop-reaps-ssh", bool(before) and bool(gone),
                 f"ssh children before stop: {before}; after: {children()}; zombies: {zombies(a.ps())}")

    # -- run --
    def execute(self):
        binary = Path(self.args.bin).read_bytes()
        self.variant_local_bin(binary)
        self.variant_on_path(binary)

    def finish(self, interrupted=None):
        problems = self.docker.cleanup()
        self.row("cleanup/containers", not problems, "; ".join(problems))
        expected = [r for r in ROWS if self.wanted(r) or r == "cleanup/containers"]
        ok, missing = check_rows(expected, self.results)
        rc = EXIT_OK if ok and not interrupted else EXIT_FAIL
        if interrupted:
            rc = 128 + interrupted
        (self.evidence / "results.json").write_text(json.dumps(
            {"run_id": self.run_id, "exit": rc, "rows": self.results, "problems": missing}, indent=2) + "\n")
        lines = [f"# real-ssh two-node check {self.run_id}", "", "| row | result | detail |", "|---|---|---|"]
        lines += [f"| {r['row']} | {'PASS' if r['passed'] else 'FAIL'} | {r['detail'][:200].replace('|', '/')} |"
                  for r in self.results]
        lines += ["", *[f"- {p}" for p in missing]]
        (self.evidence / "report.md").write_text("\n".join(lines) + "\n")
        print(f"evidence: {self.evidence} (exit {rc})", flush=True)
        for p in missing:
            print(f"  {p}", flush=True)
        return rc


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--image", help="runner image (scripts/linux-e2e/run real-ssh passes the one it built)")
    ap.add_argument("--bin", help="Linux herdr-threads binary for the image's architecture")
    ap.add_argument("--platform", help="docker --platform for the containers, e.g. linux/arm64")
    ap.add_argument("--docker", default="docker")
    ap.add_argument("--cases", type=lambda s: s.split(","), help="comma-separated rows (partial run: not a gate)")
    ap.add_argument("--evidence-dir")
    args = ap.parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        return self_test()
    if not (args.image and args.bin):
        ap.error("--image and --bin are required")
    unknown = [c for c in args.cases or [] if c not in ROWS]
    if unknown:
        ap.error(f"unknown rows: {', '.join(unknown)}")
    if shutil.which(args.docker) is None:
        print(f"real-ssh-e2e: refused: {args.docker} not found", file=sys.stderr)
        return EXIT_REFUSED
    if not Path(args.bin).is_file():
        print(f"real-ssh-e2e: refused: --bin {args.bin} is not a file", file=sys.stderr)
        return EXIT_REFUSED
    d = Driver(args)
    if d.docker.run([args.docker, "image", "inspect", args.image])[0] != 0:
        print(f"real-ssh-e2e: refused: image {args.image} not found", file=sys.stderr)
        return EXIT_REFUSED

    def interrupt(signum, _frame):
        raise Interrupted(signum)
    for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(s, interrupt)
    interrupted = None
    try:
        d.execute()
    except Interrupted as e:
        interrupted = e.signum
        print(f"real-ssh-e2e: interrupted ({e.signum}); cleaning up", file=sys.stderr)
    except Exception as e:  # noqa: BLE001 - reported as a failed run, then cleanup
        print(f"real-ssh-e2e: failed: {e}", file=sys.stderr)
        d.problems.append(str(e))
    finally:
        for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(s, signal.SIG_IGN)
        rc = d.finish(interrupted)
    return EXIT_FAIL if d.problems and rc == EXIT_OK else rc


# ---------- --self-test ----------
FAKE_DOCKER = r'''#!/bin/sh
# Fake docker for --self-test: state in $FAKE_STATE (one object per line). `exec` runs the command locally.
echo "$@" >> "$FAKE_STATE/calls"
case "$1" in
  network) case "$2" in create) for a in "$@"; do last=$a; done
                                echo "net $last" >> "$FAKE_STATE/objs"; echo id ;;
                        rm) [ -n "$FAKE_STUCK" ] || grep -v "^net $3\$" "$FAKE_STATE/objs" > "$FAKE_STATE/o2"; mv "$FAKE_STATE/o2" "$FAKE_STATE/objs" 2>/dev/null ;;
                        ls) cat "$FAKE_STATE/objs" 2>/dev/null | sed -n 's/^net //p' ;;
          esac ;;
  run) shift; while [ "$1" != "--name" ]; do shift; done; echo "ctr $2" >> "$FAKE_STATE/objs"; echo id ;;
  rm) for a in "$@"; do last=$a; done
      [ -n "$FAKE_STUCK" ] || { grep -v "^ctr $last\$" "$FAKE_STATE/objs" > "$FAKE_STATE/o2"; mv "$FAKE_STATE/o2" "$FAKE_STATE/objs"; } ;;
  ps) sed -n 's/^ctr //p' "$FAKE_STATE/objs" ;;
  exec) shift; while [ "$1" != "bash" ] && [ "$1" != "sh" ]; do shift; done; shift; exec bash "$@" ;;
  *) exit 0 ;;
esac
'''
FAKE_SSH = '#!/bin/sh\nprintf "%s\\n" "$@" > "$FAKE_STATE/ssh-argv"\necho "/usr/local/bin/herdr-threads"\n'


def self_test():
    import unittest

    class T(unittest.TestCase):
        def setUp(self):
            self.tmp = Path(tempfile.mkdtemp(prefix="ht-rs-selftest."))
            self.addCleanup(shutil.rmtree, self.tmp, True)
            bindir = self.tmp / "bin"
            bindir.mkdir()
            for name, text in (("docker", FAKE_DOCKER), ("ssh", FAKE_SSH)):
                (bindir / name).write_text(text)
                (bindir / name).chmod(0o755)
            self.state = self.tmp / "state"
            self.state.mkdir()
            (self.state / "objs").write_text("")
            env = {"FAKE_STATE": str(self.state), "PATH": f"{bindir}:{os.environ['PATH']}"}
            self.saved = {k: os.environ.get(k) for k in ("FAKE_STATE", "PATH", "FAKE_STUCK")}
            os.environ.update(env)
            os.environ.pop("FAKE_STUCK", None)
            self.addCleanup(self.restore)

        def restore(self):
            for k, v in self.saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

        def test_node_argv(self):
            a = node_run_argv("docker", "A", "n-a", "net", "img", "r1", "linux/arm64")
            b = node_run_argv("docker", "B", "n-b", "net", "img", "r1")
            for argv in (a, b):
                self.assertIn("--init", argv)
                self.assertEqual(argv[argv.index("--network") + 1], "net")
                self.assertIn(f"{LABEL}.run=r1", argv)
                self.assertNotIn("-p", argv)
                self.assertNotIn("-v", argv)
            self.assertEqual(a[a.index("--platform") + 1], "linux/arm64")
            self.assertEqual(a[-2:], ["sleep", "3600"])
            self.assertEqual(a[a.index("--cap-drop") + 1], "ALL")
            self.assertNotIn("--cap-add", a)
            self.assertEqual(b[b.index("--user") + 1], "root")
            self.assertEqual({b[i + 1] for i, w in enumerate(b) if w == "--cap-add"}, set(SSHD_CAPS))
            self.assertIn("PasswordAuthentication no", b[-1])
            self.assertIn("exec /usr/sbin/sshd -D -e", b[-1])

        def test_default_ssh_argv(self):
            argv = ssh_probe_argv("tester@b", DEFAULT_REMOTE_COMMAND)
            dash = argv.index("--")
            self.assertEqual(argv[dash + 1:], ["tester@b", DEFAULT_REMOTE_COMMAND])
            self.assertEqual(argv[:dash], ["ssh", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=15", "-o",
                                           "ServerAliveCountMax=3", "-o", "ControlMaster=no", "-o",
                                           "ControlPath=none", "-o", "ConnectTimeout=20", "-T"])
            self.assertTrue(DEFAULT_REMOTE_COMMAND.startswith('exec "$(command -v herdr-threads || echo '))
            self.assertTrue(DEFAULT_REMOTE_COMMAND.endswith('$HOME/.local/bin/herdr-threads")" peer --stdio'))

        def test_fake_ssh_on_path_receives_the_argv(self):
            d = Docker("docker", self.tmp, "r2")
            node = Node(d, "A", "n-a")
            rc, out, _ = node.sh(" ".join(shlex.quote(x) for x in ssh_probe_argv("tester@b", "command -v herdr-threads")))
            self.assertEqual(rc, 0)
            self.assertEqual(out.strip(), "/usr/local/bin/herdr-threads")
            got = (self.state / "ssh-argv").read_text().splitlines()
            self.assertEqual(got, ssh_probe_argv("tester@b", "command -v herdr-threads")[1:])

        def test_row_completeness(self):
            full = [{"row": r, "passed": True} for r in ROWS]
            self.assertTrue(check_rows(ROWS, full)[0])
            ok, problems = check_rows(ROWS, full[:-1])
            self.assertFalse(ok)
            self.assertEqual(problems, ["missing row: cleanup/containers"])
            bad = full[:-1] + [{"row": "cleanup/containers", "passed": False, "detail": "x"}]
            self.assertFalse(check_rows(ROWS, bad)[0])
            self.assertFalse(check_rows(ROWS, full + [full[0]])[0])
            self.assertFalse(check_rows(ROWS, full + [{"row": "extra/row", "passed": True}])[0])

        def test_parsers(self):
            doc = {"result": {"data": {"rows": [{"ssh_dest": "t@b", "state": "linked", "last_error": None}]}}}
            self.assertEqual(target_rows(doc, "t@b")[0]["state"], "linked")
            self.assertEqual(target_rows(doc, "t@c"), [])
            status = {"logs": [{"node": "n1", "incarnation": 1, "counter": 3, "head": "h"}], "links": []}
            self.assertEqual(log_vector(status), {("n1", 1, 3, "h")})
            self.assertEqual(parse_json('noise\n{"a": 1}\n'), {"a": 1})
            ps = "  PID STAT COMMAND\n 12 S ssh -o BatchMode=yes -- t@b x\n 13 Z [ssh] <defunct>\n 14 S sleep 3600\n"
            self.assertEqual(len(leaked_processes(ps)), 1)
            self.assertEqual(len(zombies(ps)), 1)
            self.assertEqual(leaked_processes(" 1 S sleep 3600\n 2 S ps -eo pid,stat,args\n"), [])

        def test_cleanup_bookkeeping(self):
            d = Docker("docker", self.tmp, "r3")
            d.create_network("net-r3")
            d.start_node("B", "n-b", "net-r3", "img", None)
            d.start_node("A", "n-a", "net-r3", "img", None)
            self.assertEqual(d.containers, ["n-b", "n-a"])
            self.assertEqual(d.networks, ["net-r3"])
            self.assertEqual(d.cleanup(), [])
            self.assertEqual((self.state / "objs").read_text().strip(), "")
            calls = (self.state / "calls").read_text()
            self.assertIn("rm -f -v n-a", calls)
            self.assertIn("network rm net-r3", calls)

        def test_cleanup_reports_leftovers(self):
            d = Docker("docker", self.tmp, "r4")
            d.create_network("net-r4")
            d.start_node("A", "n-a4", "net-r4", "img", None)
            os.environ["FAKE_STUCK"] = "1"
            problems = d.cleanup()
            self.assertEqual(len(problems), 2, problems)
            self.assertIn("n-a4", problems[0])
            self.assertIn("net-r4", problems[1])

        def test_exit_code_follows_rows_and_cleanup(self):
            class Args:
                evidence_dir = str(self.tmp / "ev")
                docker = "docker"
                cases = None
            d = Driver(Args)
            for r in ROWS[:-1]:
                d.row(r, True)
            self.assertEqual(d.finish(), EXIT_OK)
            d2 = Driver(Args)
            for r in ROWS[:-2]:
                d2.row(r, True)  # one row missing
            self.assertEqual(d2.finish(), EXIT_FAIL)
            self.assertIn("missing row: on-path/no-leaks", (d2.evidence / "report.md").read_text())
            d3 = Driver(Args)
            for r in ROWS[:-1]:
                d3.row(r, True)
            d3.docker.create_network("net-r5")
            os.environ["FAKE_STUCK"] = "1"
            self.assertEqual(d3.finish(), EXIT_FAIL)  # cleanup incomplete
            self.assertTrue((d3.evidence / "results.json").exists())

    return EXIT_OK if unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(T)).wasSuccessful() else EXIT_FAIL


if __name__ == "__main__":
    sys.exit(main())
