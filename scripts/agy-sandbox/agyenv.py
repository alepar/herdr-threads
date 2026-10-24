"""Isolated agy world for the native gate (test-infra spec D4, D5, D6, D8).

One ``World`` is one root under ``/private/tmp/ih.XXXXXX`` holding a private Herdr
session, a herdr-threads daemon, a model backend, a sandboxed copy of the genuine agy
binary behind a wrapper, and an isolated HOME. Everything it starts is owner-tagged
(``HT_TEST_OWNER``, ``HT_LEAK_RUN_ID``) and recorded for the ``isolated-herdr.sh`` reaper.

Python 3.9 compatible, standard library only.
"""
from __future__ import annotations

import json
import os
import pwd
import shlex
import shutil
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
IH_LIB = REPO / "scripts" / "lib" / "isolated-herdr.sh"
LEAK_CHECK = REPO / "scripts" / "check-no-leaked-processes"
FAKE_GEMINI = REPO / "scripts" / "agy-fake-gemini"
PROFILE_TEMPLATE = HERE / "loopback.sb.in"
SANDBOX_EXEC = "/usr/bin/sandbox-exec"
# Linux: bubblewrap gives agy its own network namespace (loopback only), a read-only system, writes only under the
# root and the real config directories hidden; socat carries its loopback port to the relay outside (spec D4).
BWRAP = "/usr/bin/bwrap"
SOCAT = "/usr/bin/socat"
HIDDEN_DIRS = (".gemini", ".claude", ".codex", ".aisw", ".hermes")
RELAY_SOCKET = "relay.sock"

for _dir in (FAKE_GEMINI,):  # sibling directories are imported by path insertion
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

REAPER_ROOT = Path("/tmp")


class SafetyError(AssertionError):
    """A guard refused an action that could touch a resource the world does not own."""


class WorldError(RuntimeError):
    """The world could not be built or a Herdr/agent operation failed."""


AGENT_ENV_PREFIXES = ("CLAUDE", "CODEX", "HERMES", "GEMINI", "ANTIGRAVITY", "AGY_", "AISW", "ANTHROPIC",
                      "OPENAI", "OPENROUTER", "PI_CODING_AGENT", "OPENCODE")


def scrub_agent_env(env):
    """Drop agent-identity and credential variables the invoking shell may carry.

    ``CLAUDECODE`` would make ``me init`` refuse (it never acts as an agent), and none of these may
    reach Herdr, its panes or the model backend."""
    return {k: v for k, v in env.items() if not k.startswith(AGENT_ENV_PREFIXES)}


def real_home():
    """The invoking user's real home, from the passwd database (never from $HOME)."""
    return pwd.getpwuid(os.getuid()).pw_dir


# ---------------------------------------------------------------- pure rendering

def render_profile(root, real_home_dir, template=None):
    text = (Path(template) if template else PROFILE_TEMPLATE).read_text()
    return text.replace("<REAL_HOME>", str(real_home_dir)).replace("<ROOT>", str(root))


def sandbox_kind(platform_name=None):
    """``sandbox-exec`` on macOS, ``bwrap`` on Linux, else None (no sandbox: refused)."""
    name = platform_name or sys.platform
    return "sandbox-exec" if name == "darwin" else "bwrap" if name.startswith("linux") else None


def sandbox_refusal(platform_name=None, exists=os.path.exists):
    """Why agy cannot be sandboxed on this host (a message), or None."""
    kind = sandbox_kind(platform_name)
    if kind is None:
        return "needs macOS (sandbox-exec) or Linux (bubblewrap); found %s" % (platform_name or sys.platform)
    if kind == "bwrap":
        missing = [tool for tool in (BWRAP, SOCAT) if not exists(tool)]
        if missing:
            return "Linux needs %s for the agy sandbox" % " and ".join(missing)
    return None


def hidden_dirs(real_home_dir):
    """The real config directories that exist (bwrap can mount over an existing directory only)."""
    return [str(Path(real_home_dir) / d) for d in HIDDEN_DIRS if (Path(real_home_dir) / d).is_dir()]


def sandbox_line(root, clone_dir, port, platform_name=None, hidden=()):
    if sandbox_kind(platform_name) == "sandbox-exec":
        return '%s -f "%s/loopback.sb" "%s/agy" "$@"' % (SANDBOX_EXEC, root, clone_dir)
    tmpfs = "".join(' --tmpfs "%s"' % d for d in hidden)
    # No pid namespace: the hooks report agy's pid as its liveness, which must be the host's pid. So agy is not
    # exec'd: the shell stops the in-namespace socat when agy ends.
    inner = ("%s TCP-LISTEN:%s,bind=127.0.0.1,fork,reuseaddr UNIX-CONNECT:%s/%s & s=$!; "
             "trap \"kill $s\" EXIT HUP INT TERM; \"$0\" \"$@\"" % (SOCAT, port, root, RELAY_SOCKET))
    return ('%s --die-with-parent --unshare-net --ro-bind / / --dev-bind /dev /dev --proc /proc'
            ' --bind "%s" "%s"%s \\\n  -- /bin/sh -c \'%s\' "%s/agy" "$@"'
            % (BWRAP, root, root, tmpfs, inner, clone_dir))


def render_wrapper(root, clone_dir, port, token, owner, run_id, platform_name=None, hidden=()):
    """The ``/bin/sh`` wrapper the pane runs as ``agy`` (spec D4).

    ``HERDR_PLUGIN_STATE_DIR`` is an addition to the D4 text: it is the documented way (instance
    resolution step 2) for hooks and the model's ``herdr-threads`` calls to find the root's state
    directory, which is not at Herdr's default plugin path."""
    root = str(root)
    return (
        "#!/bin/sh\n"
        'exec /usr/bin/env -i PATH="%(r)s/bin:/usr/bin:/bin:/usr/sbin:/sbin" HOME="%(r)s/home" TMPDIR="%(r)s/tmp" \\\n'
        '  XDG_CONFIG_HOME="%(r)s/cfg" XDG_CACHE_HOME="%(r)s/cache" XDG_DATA_HOME="%(r)s/data" XDG_STATE_HOME="%(r)s/st" \\\n'
        '  HERDR_PLUGIN_STATE_DIR="%(r)s/ht" \\\n'
        '  GEMINI_API_KEY="%(token)s" GOOGLE_GEMINI_BASE_URL="http://127.0.0.1:%(port)s" \\\n'
        '  TERM="${TERM:-xterm-256color}" LANG=en_US.UTF-8 NO_COLOR=1 \\\n'
        '  HERDR_ENV="$HERDR_ENV" HERDR_PANE_ID="$HERDR_PANE_ID" HERDR_SOCKET_PATH="$HERDR_SOCKET_PATH" \\\n'
        '  HT_TEST_OWNER="%(owner)s" HT_LEAK_RUN_ID="%(run)s" '
        '${HERDR_THREADS_AGY_OPTS+HERDR_THREADS_AGY_OPTS="$HERDR_THREADS_AGY_OPTS"} \\\n'
        '  %(sandbox)s\n'
    ) % {"r": root, "token": token, "port": port, "owner": owner, "run": run_id,
         "sandbox": sandbox_line(root, str(clone_dir), port, platform_name, hidden)}


def settings_dict(root):
    return {"altScreenMode": "never", "modelProvider": "gemini",
            "trustedWorkspaces": [str(Path(root) / "ws")]}


def seed_settings(root):
    """Write the seed ``settings.json`` (exactly three keys) and return its path."""
    path = Path(root) / "home" / ".gemini" / "antigravity-cli" / "settings.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(settings_dict(root), separators=(",", ":")))
    return path


def check_home_inside(env, root):
    """Refuse an environment whose HOME is missing or outside the root (spec D5)."""
    home = env.get("HOME")
    if not home:
        raise SafetyError("refusing an environment with no HOME")
    home_path = os.path.realpath(home)
    root_path = os.path.realpath(str(root))
    if home_path != root_path and not home_path.startswith(root_path + os.sep):
        raise SafetyError("refusing HOME=%s outside the world root %s" % (home, root))


def sha512_file(path):
    import hashlib
    digest = hashlib.sha512()
    with open(str(path), "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


# ---------------------------------------------------------------- process helpers

def ih_env(owner, run_id=None, extra=None):
    env = scrub_agent_env(os.environ)
    env["HT_TEST_OWNER"] = str(owner)
    env["IH_LIB"] = str(IH_LIB)
    if run_id:
        env["HT_LEAK_RUN_ID"] = str(run_id)
    if extra:
        env.update(extra)
    return env


def ih(fn, *args, owner, run_id=None, check=True, timeout=60):
    """Call one function of ``scripts/lib/isolated-herdr.sh`` and return the CompletedProcess."""
    result = subprocess.run(
        ["bash", "-c", 'source "$IH_LIB" && "$@"', "bash", fn] + [str(a) for a in args],
        env=ih_env(owner, run_id), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, universal_newlines=True, timeout=timeout)
    if check and result.returncode != 0:
        raise WorldError("%s %s failed (rc=%s): %s" % (fn, " ".join(map(str, args)), result.returncode,
                                                       (result.stderr or result.stdout).strip()[:400]))
    return result


def spawn_owned(argv, env, owner, ensure_reaper=True, reaper_root=REAPER_ROOT, **popen_kwargs):
    """Start ``argv`` in its own session and record its group as ``<reaper>/g.<pgid>``.

    This is the convention ``ih_recorded_groups`` reaps, the one ``test_support::spawn::spawn_owned``
    writes: if the owner dies, the reaper stops the group. Nothing is started any other way."""
    reaper_dir = Path(reaper_root) / ("ih-reaper.%s" % owner)
    if ensure_reaper:
        ih("ih_reaper_ensure", owner, owner=owner, check=False)
    env = dict(env)
    env["HT_TEST_OWNER"] = str(owner)
    popen_kwargs.setdefault("stdin", subprocess.DEVNULL)
    proc = subprocess.Popen(argv, env=env, start_new_session=True, **popen_kwargs)
    pgid = os.getpgid(proc.pid)
    reaper_dir.mkdir(parents=True, exist_ok=True)
    (reaper_dir / ("g.%d" % pgid)).write_text("%d\n" % proc.pid)
    return proc


def stop_group(proc, timeout=5.0):
    """TERM a spawn_owned process group, KILL after ``timeout``; always reaps."""
    try:
        pgid = os.getpgid(proc.pid)
    except OSError:
        pgid = None
    for sig, wait in ((signal.SIGTERM, timeout), (signal.SIGKILL, 5.0)):
        if proc.poll() is not None:
            break
        try:
            if pgid:
                os.killpg(pgid, sig)
            else:
                proc.send_signal(sig)
        except OSError:
            break
        try:
            proc.wait(wait)
        except subprocess.TimeoutExpired:
            pass
    try:
        proc.wait(1)
    except subprocess.TimeoutExpired:
        pass


def copy_tree(src, dst):
    """Merge ``src`` into ``dst`` (3.9 has ``dirs_exist_ok``, kept explicit for clarity)."""
    for path in Path(src).rglob("*"):
        target = Path(dst) / path.relative_to(src)
        if path.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        elif path.is_file():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(str(path), str(target))


def wait_until(pred, timeout, step=0.1):
    deadline = time.monotonic() + timeout
    while True:
        value = pred()
        if value:
            return value
        if time.monotonic() >= deadline:
            return value
        time.sleep(step)


def check_leaks(run_id, grace=15, script=None, runner=None):
    """Run ``check-no-leaked-processes --run-id``; return ``(ok, output)``."""
    cmd = [str(script or LEAK_CHECK), "--run-id", str(run_id), "--grace", str(grace)]
    result = (runner or subprocess.run)(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, universal_newlines=True,
                                        timeout=grace + 60)
    return result.returncode == 0, (result.stdout or "")


# ---------------------------------------------------------------- model backends

class ModelBackend(object):
    """Protocol: ``start(world) -> (base_url, token)``, ``stop()``, ``requests()``, ``load(scenario)``."""

    def start(self, world):
        raise NotImplementedError

    def stop(self):
        raise NotImplementedError

    def requests(self):
        raise NotImplementedError

    def load(self, scenario):
        raise NotImplementedError


class FakeGeminiBackend(ModelBackend):
    """Task 3's scripted server, spawned with ``--ctl <root>/ctl --owner-pid <gate pid>``."""

    def __init__(self, server_py=None, python=None):
        self.server_py = Path(server_py or (FAKE_GEMINI / "server.py"))
        self.python = python or sys.executable
        self.proc = None
        self.ctl = None

    def start(self, world):
        self.ctl = Path(world.ctl)
        self.ctl.mkdir(parents=True, exist_ok=True)
        log = open(str(self.ctl / "server.log"), "ab")
        self.proc = spawn_owned(
            [self.python, "-I", "-B", str(self.server_py), "--ctl", str(self.ctl),
             "--owner-pid", str(world.owner_pid)],
            world.base_env(), world.owner_pid, stdout=log, stderr=log)
        log.close()
        wait_until(lambda: (self.ctl / "ready").exists() or self.proc.poll() is not None,
                        20 * world.timeout_scale)
        if not (self.ctl / "ready").exists():
            raise WorldError("fake Gemini server did not become ready (rc=%s)" % self.proc.poll())
        port = (self.ctl / "port").read_text().strip()
        return "http://127.0.0.1:%s" % port, "fake-" + uuid.uuid4().hex

    def stop(self):
        if self.proc is not None:
            stop_group(self.proc)
            self.proc = None

    def load(self, scenario):
        import scenario as scenario_mod
        scenario_mod.validate(scenario)
        tmp = self.ctl / (".scenario.%s.tmp" % uuid.uuid4().hex)
        tmp.write_text(json.dumps(scenario))
        os.replace(str(tmp), str(self.ctl / "scenario.json"))

    def requests(self):
        path = self.ctl / "requests.jsonl"
        if not path.exists():
            return []
        out = []
        for line in path.read_text().splitlines():
            line = line.strip()
            if line:
                out.append(json.loads(line))
        return out


# ---------------------------------------------------------------- world

class Sender(object):
    """A plain shell pane enrolled with ``me init`` as a human seat (carries no agent marker)."""

    def __init__(self, world, pane_id):
        self.world = world
        self.pane_id = pane_id

    def ht(self, argv, check=True):
        return self.world.ht(argv, pane=self.pane_id, check=check)


ONBOARDING_FOOTER = "? for shortcuts"
SURVEY = "How's the CLI experience so far"


class AgyPane(object):
    def __init__(self, world, pane_id):
        self.world = world
        self.pane_id = pane_id

    def prompt(self, text, wait=False, timeout=30):
        argv = ["agent", "prompt", self.pane_id, text]
        if wait:
            argv += ["--wait", "--timeout", str(int(timeout * 1000 * self.world.timeout_scale))]
        return self.world.herdr(*argv, timeout=timeout * self.world.timeout_scale + 15)

    def keys(self, *keys):
        return self.world.herdr("agent", "send-keys", self.pane_id, *keys)

    def draft(self, text):
        """Type text into the composer without pressing enter."""
        return self.world.herdr("pane", "send-text", self.pane_id, text)

    def read_detection(self):
        result = self.world.herdr("agent", "read", self.pane_id, "--source", "detection", check=False)
        return result.stdout if result.returncode == 0 else ""

    def read_pane(self, lines=200):
        result = self.world.herdr("pane", "read", self.pane_id, "--source", "recent-unwrapped",
                                  "--lines", str(lines), check=False)
        return result.stdout if result.returncode == 0 else ""

    def agent_info(self):
        """``result.agent`` of ``herdr agent get`` (``{"agent": "agy", "agent_status": ...}``) or None."""
        result = self.world.herdr("agent", "get", self.pane_id, check=False)
        if result.returncode != 0:
            return None
        try:
            info = json.loads(result.stdout)["result"]["agent"]
        except (ValueError, KeyError, TypeError):
            return None
        return info if isinstance(info, dict) else None

    def agent_name(self):
        info = self.agent_info()
        return info.get("agent") if info else None

    def agent_status(self):
        info = self.agent_info()
        return info.get("agent_status") if info else None

    def wait(self, state, timeout=60):
        """True when the agent reaches ``state`` (``idle`` also accepts Herdr's ``done``)."""
        states = ["--until", state]
        if state == "idle":
            states += ["--until", "done"]
        scaled = timeout * self.world.timeout_scale
        result = self.world.herdr("agent", "wait", self.pane_id, *states, "--timeout",
                                  str(int(scaled * 1000)), check=False, timeout=scaled + 15)
        return result.returncode == 0

    def dismiss_onboarding(self, limit=10):
        """Walk a first-run wizard (color scheme, then Terms of Service) until the prompt footer shows.

        The genuine agy 1.3.2 shows it on every fresh HOME and Herdr reports the wizard as ``idle``, so a
        prompt typed into it is lost. Recognises only the wizard pages (never the trust prompt). Returns
        the pages it dismissed (empty when there was none)."""
        pages = []
        for _ in range(limit):
            tail = "\n".join(self.read_pane(80).splitlines()[-30:])
            if ONBOARDING_FOOTER in tail:
                break
            if "Choose your color scheme" in tail:
                pages.append("color")
                self.keys("Enter")
            elif "Terms of Service" in tail and "[Done]" in tail:
                pages.append("terms")
                self.keys("Down")
                self.keys("Right")
                self.keys("Enter")
            elif "Done" in tail and "Navigate" in tail:
                pages.append("confirm")
                self.keys("Enter")
            else:
                break
            time.sleep(1.5)
        return pages

    def dismiss_survey(self):
        """Skip agy's feedback survey (``[0] Skip``), which appears after an executor error and swallows
        the next typed prompt. True when it was showing."""
        if SURVEY not in "\n".join(self.read_pane(80).splitlines()[-20:]):
            return False
        self.keys("0")
        time.sleep(1)
        return True

    def exit(self):
        """``/exit`` bounded by 5 s; the world's teardown sweeps whatever survives."""
        self.world.herdr("agent", "prompt", self.pane_id, "/exit", check=False, timeout=5)
        wait_until(lambda: not self.world.agy_running(), 5)

    def transcripts(self):
        data = self.world.agy_data
        found = []
        for sub in ("brain", "conversations"):
            base = data / sub
            if base.is_dir():
                found += [p for p in base.rglob("*.jsonl") if p.is_file()]
        return sorted(found)


def find_key(data, names):
    """First value found under any of ``names`` in nested JSON, depth first."""
    if isinstance(data, dict):
        for name in names:
            if name in data and not isinstance(data[name], (dict, list)):
                return data[name]
        for value in data.values():
            hit = find_key(value, names)
            if hit is not None:
                return hit
    elif isinstance(data, list):
        for value in data:
            hit = find_key(value, names)
            if hit is not None:
                return hit
    return None


class World(object):
    def __init__(self, run_id, owner_pid, herdr_threads_bin, backend, timeout_scale=1.0,
                 agy_bin=None, artifact_dir=None, keep_on_fail=False):
        self.run_id = run_id
        self.owner_pid = owner_pid
        self.herdr_threads_bin = str(herdr_threads_bin)
        self.backend = backend
        self.timeout_scale = timeout_scale
        self.agy_bin = agy_bin or os.environ.get("HT_AGY_BIN")
        self.artifact_dir = Path(artifact_dir) if artifact_dir else None
        self.keep_on_fail = keep_on_fail
        self.root = None
        self.checks = []
        self.panes = []
        self.stage_count = 0
        self.workspace = None
        self.failed = False
        self.base_url = None
        self.token = None
        self.agy_info = {}
        self.bridge = None

    # -- paths
    @property
    def home(self):
        return self.root / "home"

    @property
    def ws(self):
        return self.root / "ws"

    @property
    def ctl(self):
        return self.root / "ctl"

    @property
    def state(self):
        return self.root / "ht"

    @property
    def agy_data(self):
        return self.home / ".gemini" / "antigravity-cli"

    @property
    def socket(self):
        return self.root / "h.sock"

    # -- environment
    def base_env(self, extra=None):
        env = scrub_agent_env(os.environ)
        env["HT_TEST_OWNER"] = str(self.owner_pid)
        env["HT_LEAK_RUN_ID"] = str(self.run_id)
        if extra:
            env.update(extra)
        return env

    def herdr_env(self, extra=None):
        """The private Herdr environment (``ih_env_args``): inherited ``HERDR_*`` dropped."""
        env = {k: v for k, v in self.base_env().items() if not k.startswith("HERDR_")}
        r = str(self.root)
        env.update({"HOME": r + "/home", "XDG_CONFIG_HOME": r + "/cfg", "XDG_STATE_HOME": r + "/st",
                    "XDG_RUNTIME_DIR": r + "/rt", "HERDR_CONFIG_PATH": r + "/cfg/herdr.toml",
                    "HERDR_SOCKET_PATH": r + "/h.sock"})
        if extra:
            env.update(extra)
        return env

    def herdr(self, *argv, check=True, timeout=30):
        env = self.herdr_env()
        check_home_inside(env, self.root)
        result = subprocess.run(["herdr"] + [str(a) for a in argv], env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                universal_newlines=True, timeout=timeout)
        if check and result.returncode != 0:
            raise WorldError("herdr %s rc=%s: %s" % (" ".join(map(str, argv)), result.returncode,
                                                     (result.stderr or result.stdout).strip()[:300]))
        return result

    def herdr_json(self, *argv, **kwargs):
        result = self.herdr(*argv, **kwargs)
        try:
            return json.loads(result.stdout) if result.stdout.strip() else None
        except ValueError:
            return None

    def ht(self, argv, pane=None, check=True, env=None, timeout=60, human=False):
        """Run the herdr-threads CLI against this world (never the user's home or state).

        ``human=True`` inserts the ``human`` namespace that person/operator commands (``me init``)
        require."""
        full_env = self.herdr_env()
        full_env["HERDR_THREADS_TEST_OWNER_PID"] = str(self.owner_pid)
        if pane:
            full_env["HERDR_PANE_ID"] = pane
            full_env["HERDR_ENV"] = "1"
        if env:
            full_env.update(env)
        check_home_inside(full_env, self.root)
        cmd = [self.herdr_threads_bin] + (["human"] if human else [])
        cmd += ["--state-dir", str(self.state), "--host-endpoint", str(self.socket)]
        cmd += [str(a) for a in argv]
        result = subprocess.run(cmd, env=full_env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, universal_newlines=True, timeout=timeout)
        if check and result.returncode != 0:
            raise WorldError("herdr-threads %s rc=%s: %s" % (" ".join(map(str, argv)), result.returncode,
                                                             (result.stderr or result.stdout).strip()[:400]))
        return result

    # -- lifecycle
    def __enter__(self):
        try:
            self._build()
        except BaseException:
            self.failed = True
            self._teardown()
            raise
        return self

    def __exit__(self, exc_type, exc, tb):
        if exc_type is not None:
            self.failed = True
        self._teardown()
        return False

    def _build(self):
        if not self.agy_bin or not os.path.isfile(self.agy_bin):
            raise WorldError("HT_AGY_BIN is not a file: %r" % (self.agy_bin,))
        created = ih("ih_root_new", owner=self.owner_pid, run_id=self.run_id).stdout.strip()
        self.root = Path(os.path.realpath(created))
        for sub in ("home", "tmp", "ws", "ctl", "ht", "bin", "agybin", "artifacts", "cfg", "st", "rt",
                    "cache", "data"):
            (self.root / sub).mkdir(parents=True, exist_ok=True)
        self.state.chmod(0o700)
        node = self.state / "node"
        node.mkdir(mode=0o700)
        (node / "settings.json").write_text('{"sync":{"discover":false}}\n')
        (node / "settings.json").chmod(0o600)
        seed_settings(self.root)
        # Every pane shell (a launch or handoff target types `agy` by name) finds the wrapper first.
        (self.root / "home" / ".zshenv").write_text('export PATH=%s:"$PATH"\n' % shlex.quote(str(self.root / "bin")))
        if sandbox_kind() == "sandbox-exec":
            (self.root / "loopback.sb").write_text(render_profile(self.root, real_home()))
        ih("ih_start", self.root, "agy-gate", owner=self.owner_pid, run_id=self.run_id, timeout=60)
        link = self.root / "bin" / "herdr-threads"
        link.symlink_to(os.path.realpath(self.herdr_threads_bin))
        self.base_url, self.token = self.backend.start(self)
        if sandbox_kind() == "bwrap":
            self._start_bridge()
        self._stage()
        self.ht(["daemon", "ensure"], timeout=90)
        data = self.herdr_json("workspace", "create", "--cwd", str(self.ws), "--label", "agy-gate",
                               "--no-focus")
        self.workspace = find_key(data, ("workspace_id",))
        if not self.workspace:
            raise WorldError("workspace create returned no workspace id: %r" % (data,))

    def _start_bridge(self):
        """Linux: the host end of agy's loopback port, a unix socket in the root forwarded to the backend."""
        sock = self.root / RELAY_SOCKET
        argv = [SOCAT, "UNIX-LISTEN:%s,fork,unlink-early" % sock, "TCP:127.0.0.1:%s" % self.base_url.rsplit(":", 1)[1]]
        self.bridge = spawn_owned(argv, self.base_env(), self.owner_pid, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not wait_until(sock.exists, 10 * self.timeout_scale):
            raise WorldError("relay bridge socket %s never appeared" % sock)

    def setup_agy(self, permissions=True):
        """Run ``herdr-threads setup agy`` inside the isolated HOME (never the real one).

        ``permissions`` passes ``--with-permissions``: agy's allow member is installed only with
        consent, so a plain ``setup agy`` (``permissions=False``) adds it only when a grant is recorded.

        Runs in the ``human`` namespace: since v0.6.0 an agent-actor call of an escalating command must
        start with that word, and ``World.ht`` always puts ``--state-dir`` first. The gate acts as the
        operator of its isolated world.
        """
        argv = ["setup", "agy"] + (["--with-permissions"] if permissions else [])
        return self.ht(argv, check=False, human=True)

    def unsetup_agy(self):
        """``herdr-threads unsetup agy`` in the isolated HOME, as the operator (``human`` namespace)."""
        return self.ht(["unsetup", "agy"], check=False, human=True)

    def _stage(self):
        """Clone the binary to ``agybin/<n>/agy`` (``cp -c``, else copy) and write the wrapper."""
        self.stage_count += 1
        clone_dir = self.root / "agybin" / str(self.stage_count)
        clone_dir.mkdir(parents=True)
        target = clone_dir / "agy"
        if subprocess.run(["cp", "-c", self.agy_bin, str(target)], stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL).returncode != 0:
            shutil.copyfile(self.agy_bin, str(target))
        os.chmod(str(target), 0o755)
        wrapper = render_wrapper(self.root, clone_dir, self.base_url.rsplit(":", 1)[1], self.token,
                                 self.owner_pid, self.run_id, hidden=hidden_dirs(real_home()))
        tmp = self.root / "bin" / (".agy.%d.tmp" % self.stage_count)
        tmp.write_text(wrapper)
        os.chmod(str(tmp), 0o755)
        os.replace(str(tmp), str(self.root / "bin" / "agy"))
        self.agy_info = {"sha512": sha512_file(target), "size": target.stat().st_size,
                         "inode": target.stat().st_ino, "stage": self.stage_count}
        return target

    def restage_agy(self):
        """New clone directory and a replaced wrapper: both inode and mtime change (spec D4)."""
        time.sleep(0.05)  # distinct mtimes even on coarse clocks
        return self._stage()

    def agy_version(self):
        out = subprocess.run([str(self.root / "agybin" / str(self.stage_count) / "agy"), "--version"],
                             stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             universal_newlines=True, timeout=30, env={"HOME": str(self.home), "PATH": "/usr/bin:/bin"})
        return out.stdout.strip()

    # -- panes
    def _new_tab(self, label):
        data = self.herdr_json("tab", "create", "--workspace", self.workspace, "--cwd", str(self.ws),
                               "--label", label, "--no-focus")
        pane = None
        if isinstance(data, dict):
            pane = find_key((data.get("result") or {}).get("root_pane") or {}, ("pane_id",))
        pane = pane or find_key(data, ("pane_id",))
        if not pane:
            raise WorldError("tab create returned no pane id: %r" % (data,))
        return pane

    def start_agy(self, args=(), pane=None):
        """Run the wrapper in a pane and wait for Herdr to report the agent ``idle``."""
        pane = pane or self._new_tab("agy-%d" % (len(self.panes) + 1))
        argv = [str(self.root / "bin" / "agy")] + [str(a) for a in args]
        command = "PATH=%s:\"$PATH\" %s" % (shlex.quote(str(self.root / "bin")),
                                            " ".join(shlex.quote(a) for a in argv))
        self.herdr("pane", "run", pane, command)
        agy_pane = AgyPane(self, pane)
        self.panes.append(agy_pane)
        deadline = time.monotonic() + 60 * self.timeout_scale
        while True:  # `agent wait` fails with agent_not_found until Herdr has detected the TUI
            result = self.herdr("agent", "wait", pane, "--until", "idle", "--until", "done",
                                "--until", "blocked", "--timeout", str(int(60000 * self.timeout_scale)),
                                check=False, timeout=70 * self.timeout_scale)
            if result.returncode == 0 or "agent_not_found" not in (result.stderr + result.stdout) \
                    or time.monotonic() >= deadline:
                break
            time.sleep(1)
        if result.returncode != 0:
            raise WorldError("agy in pane %s never became idle: %s" % (pane, (result.stderr or result.stdout).strip()[:300]))
        pages = agy_pane.dismiss_onboarding()
        if pages:
            self.artifact("onboarding-pages.txt", ",".join(pages))
        # Herdr reports `idle` before the composer takes input: keystrokes typed at once lose their first
        # characters (genuine agy 1.3.2 on a restart in a used HOME), so wait for the footer and let it settle.
        wait_until(lambda: ONBOARDING_FOOTER in agy_pane.read_pane(80), 30 * self.timeout_scale, step=0.5)
        time.sleep(1.5)
        return agy_pane

    def agy_running(self):
        out = subprocess.run(["ps", "axeww", "-o", "pid=,command="], stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, universal_newlines=True).stdout
        needle = str(self.root) + "/agybin/"
        return any(needle in line and "sandbox-exec" not in line and "bwrap" not in line
                   and "/bin/sh -c" not in line for line in out.splitlines())

    def sender(self):
        pane = self._new_tab("sender")
        self.ht(["me", "init"], pane=pane, human=True)
        return Sender(self, pane)

    # -- recording
    def check(self, name, ok, detail=""):
        self.checks.append({"name": name, "ok": bool(ok), "detail": str(detail)})
        return bool(ok)

    def artifact(self, name, data):
        path = self.root / "artifacts" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(data, bytes):
            path.write_bytes(data)
        elif isinstance(data, str):
            path.write_text(data)
        else:
            path.write_text(json.dumps(data, indent=2, sort_keys=True))
        return path

    # -- teardown (spec D8): panes, backend, daemon, root
    def _teardown(self):
        if self.root is None:
            return
        previous = {}
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            try:
                previous[sig] = signal.signal(sig, signal.SIG_IGN)
            except ValueError:
                pass  # not the main thread
        try:
            for pane in reversed(self.panes):
                try:
                    pane.exit()
                except Exception:
                    pass
            if self.bridge is not None:
                try:
                    stop_group(self.bridge)
                except Exception:
                    pass
            try:
                self.backend.stop()
            except Exception:
                pass
            try:
                self.ht(["daemon", "stop"], check=False, timeout=20)
            except Exception:
                pass
            if self.artifact_dir is not None and (self.root / "artifacts").is_dir():
                try:
                    copy_tree(self.root / "artifacts", self.artifact_dir)
                except Exception:
                    pass
            fn = "ih_keep" if (self.failed and self.keep_on_fail) else "ih_teardown"
            ih(fn, self.root, owner=self.owner_pid, run_id=self.run_id, check=False, timeout=90)
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
            self.root_kept = self.failed and self.keep_on_fail
