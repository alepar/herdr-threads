"""Private Herdr host, test daemon and CLI helpers for the host-recovery suite (ht-4is.11.5).

Everything here runs against a private `herdr server` whose HOME, XDG directories, config and API
socket live under one fresh run directory. The shared Herdr server is never contacted: every child
environment drops inherited `HERDR_*` variables and names only the private socket, and the raw API
client refuses any socket outside the run directory. Only processes this suite started are
signalled, and each signal is preceded by an argv/environment ownership check. No model is launched;
panes run stand-in shell commands only.
"""

import hashlib
import json
import os
from pathlib import Path
import signal
import socket as socketlib
import subprocess
import time

PINNED_HERDR_VERSION = "herdr 0.9.1"
PINNED_HERDR_SHA256 = "5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de"
PROTOCOL = 22


class SafetyError(AssertionError):
    """A guard refused an action that could touch a resource this suite does not own."""


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def utc_ms():
    return int(time.time() * 1000)


def process_command(pid):
    result = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True, timeout=10)
    return result.stdout.strip() if result.returncode == 0 else None


def process_environment_mentions(pid, needle):
    """True when the process argv+environment (macOS `ps eww`) contains `needle`."""
    result = subprocess.run(["ps", "eww", "-o", "command=", "-p", str(pid)], capture_output=True, text=True, timeout=10)
    return result.returncode == 0 and needle in result.stdout


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def foreign_processes(root):
    """Every `herdr server` and `herdr-threads daemon run` process that does not name `root`.

    Used before and after a run to prove the shared server and unrelated daemons were untouched.
    Keyed by pid with the kernel start time, so a restart shows up as a changed entry."""
    listing = subprocess.run(["ps", "-axo", "pid=,lstart=,command="], capture_output=True, text=True, timeout=10).stdout
    found = {}
    for line in listing.splitlines():
        parts = line.split(None, 6)
        if len(parts) < 7:
            continue
        pid, started, command = parts[0], " ".join(parts[1:6]), parts[6]
        is_server = "herdr server" in command
        is_daemon = "herdr-threads" in command and " daemon run " in command
        if (is_server or is_daemon) and str(root) not in command:
            if is_server and process_environment_mentions(int(pid), str(root)):
                continue  # our own private server (its argv is bare; the environment names the run root)
            found[pid] = {"started": started, "command": command}
    return found


class CommandLog:
    def __init__(self, path):
        self.path = Path(path)
        self.stream = self.path.open("a", encoding="utf-8")

    def write(self, record):
        record = {"utc_ms": utc_ms(), **record}
        self.stream.write(json.dumps(record, sort_keys=True) + "\n")
        self.stream.flush()

    def close(self):
        self.stream.close()


class PrivateHerdr:
    """One private Herdr 0.9.1 server rooted at `root` (short path: macOS limits socket paths)."""

    def __init__(self, root, log, name="h"):
        self.root = Path(root)
        self.log = log
        self.name = name
        self.home = self.root / f"{name}-home"
        self.config = self.root / f"{name}-cfg"
        self.state = self.root / f"{name}-st"
        self.runtime = self.root / f"{name}-rt"
        self.socket = self.root / f"{name}.sock"
        self.process = None
        self.starts = 0
        herdr = subprocess.run(["which", "herdr"], capture_output=True, text=True).stdout.strip()
        if not herdr:
            raise SafetyError("Herdr 0.9.1 is required on PATH")
        self.herdr = str(Path(herdr).resolve())

    def check_binary(self):
        version = subprocess.run([self.herdr, "--version"], capture_output=True, text=True, timeout=10,
                                 env=self.environment()).stdout.strip()
        digest = sha256(self.herdr)
        return {"path": self.herdr, "version": version, "sha256": digest,
                "pinned": version == PINNED_HERDR_VERSION and digest == PINNED_HERDR_SHA256}

    def environment(self, **extra):
        env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
        env.update({
            "HOME": str(self.home), "XDG_CONFIG_HOME": str(self.config), "XDG_STATE_HOME": str(self.state),
            "XDG_RUNTIME_DIR": str(self.runtime), "HERDR_CONFIG_PATH": str(self.config / "herdr.toml"),
            "HERDR_SOCKET_PATH": str(self.socket),
        })
        env.update(extra)
        return env

    def start(self):
        if self.process is not None:
            raise SafetyError("private server already running")
        for directory in (self.home, self.config, self.state, self.runtime):
            directory.mkdir(mode=0o700, exist_ok=True)
        config_file = self.config / "herdr.toml"
        if not config_file.exists():
            config_file.write_text("")
        self.starts += 1
        log_path = self.root / f"{self.name}-server-{self.starts}.out"
        with log_path.open("wb") as server_log:
            self.process = subprocess.Popen([self.herdr, "server"], env=self.environment(), stdin=subprocess.DEVNULL,
                                            stdout=server_log, stderr=server_log, start_new_session=True)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self.socket.exists() and "api socket:" in log_path.read_text(errors="replace"):
                break
            if self.process.poll() is not None:
                raise SafetyError(f"private Herdr server exited {self.process.returncode}")
            time.sleep(0.1)
        else:
            raise SafetyError("private Herdr server did not create its socket")
        self.log.write({"kind": "server_start", "server": self.name, "pid": self.process.pid, "start": self.starts})
        return self.process.pid

    def stop(self):
        """Signal only the process this object started, after re-checking its identity."""
        if self.process is None:
            return None
        pid = self.process.pid
        if self.process.poll() is None:
            command = process_command(pid)
            if command is None or not command.endswith("herdr server") or not process_environment_mentions(pid, str(self.root)):
                raise SafetyError(f"refusing to signal pid {pid}: not this run's private server ({command!r})")
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        code = self.process.returncode
        self.process = None
        deadline = time.monotonic() + 10
        while self.socket.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        self.log.write({"kind": "server_stop", "server": self.name, "pid": pid, "returncode": code})
        return code

    def api(self, method, params=None, timeout=10, expect_ok=True):
        if self.socket.resolve().parent != self.root.resolve():
            raise SafetyError("API socket is outside the private run directory")
        request = {"id": f"hr-{time.monotonic_ns()}", "method": method, "params": params or {}}
        connection = socketlib.socket(socketlib.AF_UNIX)
        connection.settimeout(timeout)
        try:
            connection.connect(str(self.socket))
            connection.sendall((json.dumps(request) + "\n").encode())
            buffer = b""
            while not buffer.endswith(b"\n"):
                chunk = connection.recv(1 << 16)
                if not chunk:
                    break
                buffer += chunk
        finally:
            connection.close()
        response = json.loads(buffer)
        self.log.write({"kind": "herdr_api", "server": self.name, "method": method, "params": params or {},
                        "ok": "result" in response, "response": response if method != "session.snapshot" else "<snapshot>"})
        if expect_ok and "result" not in response:
            raise AssertionError(f"herdr {method}: {response.get('error')}")
        return response.get("result", response)

    def snapshot(self):
        return self.api("session.snapshot")["snapshot"]

    def panes(self):
        return {pane["pane_id"]: pane for pane in self.snapshot()["panes"]}

    def pane_by_terminal(self, terminal):
        return next((pane for pane in self.snapshot()["panes"] if pane["terminal_id"] == terminal), None)

    def run_in_pane(self, pane, command):
        """Type a stand-in shell command into a pane (never a model)."""
        self.api("pane.send_text", {"pane_id": pane, "text": command})
        self.api("pane.send_keys", {"pane_id": pane, "keys": ["enter"]})

    def pane_text(self, pane, lines=200):
        result = self.api("pane.read", {"pane_id": pane, "source": "recent_unwrapped", "lines": lines})
        read = result.get("read", result)
        return read.get("text", "") if isinstance(read, dict) else str(read)


class TestDaemon:
    """The herdr-threads CLI plus the one daemon this suite ensures for (state_dir, host endpoint)."""

    def __init__(self, binary, state_dir, host, log):
        self.binary = str(Path(binary).resolve())
        self.state_dir = Path(state_dir)
        self.host = host
        self.log = log
        self.killed = []

    def instance_dir(self):
        locators = [path.parent for path in self.state_dir.glob("instances/*/locator")
                    if path.read_text().strip() == str(self.host.socket)]
        if len(locators) != 1:
            raise AssertionError(f"expected one instance for {self.host.socket}, found {locators}")
        return locators[0]

    def endpoint(self):
        return json.loads((self.instance_dir() / "endpoint.json").read_text())

    def run(self, *argv, pane=None, coop=None, json_out=True, timeout=60, operator_note=None):
        env = self.host.environment()
        if pane:
            env["HERDR_PANE_ID"] = pane
            env["HERDR_ENV"] = "1"
        prefix = [self.binary, "--state-dir", str(self.state_dir), "--host-endpoint", str(self.host.socket)]
        if json_out:
            prefix.append("--json")
        if coop:
            seat, target = coop
            prefix += ["--cooperative-seat", seat, "--cooperative-target", target,
                       "--cooperative-harness", "claude", "--cooperative-role", "top-level"]
        full = prefix + list(argv)
        try:
            result = subprocess.run(full, env=env, capture_output=True, text=True, timeout=timeout)
            rc, out, err = result.returncode, result.stdout, result.stderr
        except subprocess.TimeoutExpired as error:
            rc, out, err = "timeout", error.stdout or "", error.stderr or ""
        data = None
        if json_out and rc == 0 and out.strip():
            try:
                data = json.loads(out)["result"]["data"]
            except (ValueError, KeyError, TypeError):
                data = None
        self.log.write({"kind": "cli", "argv": full[1:], "pane": pane, "rc": rc, "stdout": out[-6000:],
                        "stderr": err[-3000:], "note": operator_note})
        return rc, data, out, err

    def ok(self, *argv, **kwargs):
        rc, data, out, err = self.run(*argv, **kwargs)
        if rc != 0:
            raise AssertionError(f"herdr-threads {' '.join(argv)} exited {rc}: {(err or out).strip()[:400]}")
        return data

    def ensure(self):
        return self.ok("daemon", "ensure", timeout=90)

    def health(self):
        rc, data, out, err = self.run("daemon", "health", timeout=30)
        return data if rc == 0 else None

    def seats(self):
        return {item["seat"]: item for item in self.ok("seat", "list", "--limit", "100")["items"]}

    def inspect(self, seat):
        return self.ok("seat", "inspect", seat, "--limit", "100")

    def pending(self, seat):
        return self.ok("pending-receipts", "--seat", seat, "--limit", "100")["items"]

    def warnings(self, seat):
        return self.ok("warnings", "--seat", seat, "--limit", "100")["items"]

    def kill_daemon(self):
        """SIGKILL exactly the daemon this suite ensured, after checking its argv names our state dir."""
        endpoint = self.endpoint()
        pid = int(endpoint["pid"])
        command = process_command(pid)
        expected = f"daemon run --state-dir {self.state_dir} --host-endpoint {self.host.socket}"
        if command is None or not command.startswith(self.binary) or expected not in command:
            raise SafetyError(f"refusing to kill pid {pid}: not this run's test daemon ({command!r})")
        os.kill(pid, signal.SIGKILL)
        deadline = time.monotonic() + 10
        while pid_alive(pid) and time.monotonic() < deadline:
            time.sleep(0.05)
        self.killed.append({"pid": pid, "boot_id": endpoint["boot_id"], "command": command})
        self.log.write({"kind": "daemon_kill", "pid": pid, "boot_id": endpoint["boot_id"], "command": command})
        return pid, endpoint["boot_id"]

    def stop(self):
        rc, data, out, err = self.run("daemon", "stop", timeout=60)
        return rc


def wait_for(predicate, timeout=30, interval=0.5, description="condition"):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(interval)
    raise TimeoutError(f"timed out waiting for {description}; last={last!r}")
