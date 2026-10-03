#!/bin/sh
# Isolated clean-install lifecycle validation for the native Herdr package.
#
# usage: scripts/validate-package.sh [SOURCE_REPOSITORY [REF]]
#
# Publishes the committed REF (default HEAD) of SOURCE_REPOSITORY (default:
# this checkout) to a private bare repository, then drives the pinned Herdr
# 0.9.1 CLI against a private `herdr server` whose HOME, XDG directories,
# config and socket all live under one fresh /private/tmp directory. A private
# git `insteadOf` rule makes `herdr plugin install alepar/herdr-threads`
# clone that private repository, so Herdr itself performs the clean checkout,
# the locked release build and the registration. The shared Herdr server is
# never contacted, stopped or restarted; the private server is stopped only
# by signalling the process this script started. Prints
# PACKAGE_VALIDATION_PASS and a JSON evidence summary on success.
set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd -P)
source_repository=${1:-$(CDPATH='' cd "$script_dir/.." && pwd -P)}
source_ref=${2:-HEAD}
exec python3 - "$source_repository" "$source_ref" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

SOURCE = Path(sys.argv[1]).resolve()
REF = sys.argv[2]
PINNED_HERDR_SHA256 = "5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de"
PLUGIN = "herdr-threads"
OWNER_REPO = "alepar/herdr-threads"
PROMPT = "Enter to refresh, q to quit:"
evidence = {"source": str(SOURCE), "checks": []}


def fail(message):
    raise AssertionError(message)


def check(name, condition, detail=""):
    if not condition:
        fail(f"{name}: {detail}")
    evidence["checks"].append(name)


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


herdr = shutil.which("herdr")
if not herdr:
    fail("Herdr 0.9.1 is required on PATH")
herdr = str(Path(herdr).resolve())
if subprocess.check_output([herdr, "--version"], text=True).strip() != "herdr 0.9.1":
    fail("this validation requires Herdr 0.9.1")
if sha256(herdr) != PINNED_HERDR_SHA256:
    fail("installed Herdr differs from the pinned 0.9.1 binary")
evidence["herdr"] = {"path": herdr, "version": "0.9.1", "sha256": PINNED_HERDR_SHA256}

real_home = Path(os.path.expanduser("~"))
# The shared Herdr config whose registry and plugin directories must not change.
# HT_PACKAGE_SHARED_HERDR_CONFIG exists only to demonstrate that this check
# fails when the watched directory changes; the gate never sets it.
shared_config = Path(os.environ.get("HT_PACKAGE_SHARED_HERDR_CONFIG", str(real_home / ".config/herdr")))
shared_registry = shared_config / "plugins.json"


def shared_snapshot():
    # The user's Herdr registry and top-level plugin directories: neither may change.
    plugins = shared_config / "plugins"
    listing = sorted(str(p.relative_to(plugins)) for p in plugins.glob("*/*")) if plugins.is_dir() else []
    registry = sha256(shared_registry) if shared_registry.exists() else None
    return {"registry_sha256": registry, "plugin_dirs": listing}


shared_before = shared_snapshot()
source_commit = subprocess.check_output(["git", "-C", str(SOURCE), "rev-parse", "--verify", REF + "^{commit}"],
                                        text=True).strip()
evidence["source_commit"] = source_commit
evidence["shared_config"] = str(shared_config)

# Short and unique: sockets live here and macOS limits sockaddr_un paths.
root = Path(tempfile.mkdtemp(prefix="htpv-", dir="/private/tmp"))
home, config, state, runtime = (root / name for name in ("home", "cfg", "st", "rt"))
for directory in (home, config, state, runtime):
    directory.mkdir(mode=0o700)
(config / "herdr.toml").write_text("")
socket = root / "h.sock"
foreign_target = root / "foreign-cargo-target"
environment = {key: value for key, value in os.environ.items()
               if not key.startswith("HERDR_") and not key.startswith("GIT_")}
environment.update({
    "HOME": str(home),
    "CLAUDE_CONFIG_DIR": str(home / ".claude"), "CODEX_HOME": str(home / ".codex"),
    "XDG_CONFIG_HOME": str(config), "XDG_STATE_HOME": str(state), "XDG_RUNTIME_DIR": str(runtime),
    "HERDR_CONFIG_PATH": str(config / "herdr.toml"), "HERDR_SOCKET_PATH": str(socket),
    # The private HOME must still find the user's Rust toolchain (read-only use).
    "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(real_home / ".rustup")),
    "CARGO_HOME": os.environ.get("CARGO_HOME", str(real_home / ".cargo")),
    # A hostile inherited target dir: the package build must not follow it.
    "CARGO_TARGET_DIR": str(foreign_target),
    "GIT_CONFIG_NOSYSTEM": "1",
})
server = None
handoff_server = None
started_daemons = set()


def run(argv, *, expected=0, timeout=60, env=None, cwd=None, input=None):
    result = subprocess.run(argv, env=environment if env is None else env, cwd=cwd, input=input,
                            text=True, capture_output=True, timeout=timeout)
    if expected is not None and result.returncode != expected:
        fail(f"{argv!r}: exit {result.returncode} (expected {expected}); "
             f"stdout={result.stdout[-3000:]!r}; stderr={result.stderr[-3000:]!r}")
    return result


def herdr_json(*args, timeout=60):
    output = run([herdr, *args], timeout=timeout).stdout
    value = json.loads(output)
    if "error" in value:
        fail(f"herdr {' '.join(args)}: {value['error']}")
    return value["result"]


def registry():
    path = config / "herdr/plugins.json"
    return json.loads(path.read_text()) if path.exists() else None


def install(ref=None):
    argv = [herdr, "plugin", "install", OWNER_REPO, "--yes"]
    if ref:
        argv[4:4] = ["--ref", ref]
    return run(argv, expected=None, timeout=900)


def action(name, timeout=60):
    """Invoke an installed action through Herdr and return its finished log."""
    invoked = herdr_json("plugin", "action", "invoke", name)
    log_id = invoked["log"]["log_id"]
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        logs = herdr_json("plugin", "log", "list", "--plugin", PLUGIN, "--limit", "50")["logs"]
        entry = next((log for log in logs if log["log_id"] == log_id), None)
        if entry and entry["status"] != "running":
            return entry
        time.sleep(0.2)
    fail(f"action {name} did not finish")


def daemons():
    listing = run(["ps", "-axo", "pid=,command="]).stdout
    found = {}
    for line in listing.splitlines():
        pid, _, command = line.strip().partition(" ")
        if " daemon run " in command and str(root) in command:
            found[int(pid)] = command.strip()
    return found


def wait_gone(pid, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if pid not in daemons():
            return True
        time.sleep(0.1)
    return False


def field(text, name):
    match = re.search(rf'^{re.escape(name)}: "?([^"\n]*)"?$', text, re.M)
    return match.group(1) if match else None


def pane_exists(pane):
    listed = herdr_json("pane", "list")["panes"]
    return any(item["pane_id"] == pane for item in listed)


def wait_pane(pane, predicate, timeout=20):
    deadline = time.monotonic() + timeout
    text = ""
    while time.monotonic() < deadline:
        read = run([herdr, "pane", "read", pane, "--source", "recent-unwrapped", "--lines", "400"], expected=None)
        text = read.stdout
        if predicate(text):
            return text
        time.sleep(0.2)
    fail(f"pane {pane} never matched; last text={text[-2000:]!r}")


server_starts = 0


def server_owns_socket():
    return f"api socket: {socket}" in (root / f"server-{server_starts}.out").read_text(errors="replace")


def start_server():
    """Start the private Herdr server; each start logs to its own file."""
    global server, server_starts
    server_starts += 1
    log_path = root / f"server-{server_starts}.out"
    with log_path.open("wb") as server_log:
        server = subprocess.Popen([herdr, "server"], env=environment, stdin=subprocess.DEVNULL,
                                  stdout=server_log, stderr=server_log, start_new_session=True)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if socket.exists() and "api socket:" in log_path.read_text(errors="replace"):
            return
        if server.poll() is not None:
            fail(f"private Herdr server exited {server.returncode}: {log_path.read_text(errors='replace')}")
        time.sleep(0.1)
    fail("private Herdr server did not create its socket")


def private_handoff_servers():
    """Handoff-import servers spawned by the private server (argv names the private config dir)."""
    listing = run(["ps", "-axo", "pid=,command="]).stdout
    found = {}
    for line in listing.splitlines():
        pid, _, command = line.strip().partition(" ")
        if " server --handoff-import " in command and str(config) in command:
            found[int(pid)] = command.strip()
    return found


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


def stop_server():
    """Stop only the server this script started (or its handoff successor); never `herdr server stop`."""
    global server, handoff_server
    if server is None and handoff_server is None:
        return
    if server is not None:
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)
        server = None
    if handoff_server is not None:
        # Detached by Herdr, so not our child: signal it and poll for exit.
        for sig, wait in ((signal.SIGTERM, 10), (signal.SIGKILL, 5)):
            try:
                os.kill(handoff_server, sig)
            except ProcessLookupError:
                break
            deadline = time.monotonic() + wait
            while pid_alive(handoff_server) and time.monotonic() < deadline:
                time.sleep(0.1)
            if not pid_alive(handoff_server):
                break
        handoff_server = None
    deadline = time.monotonic() + 10
    while socket.exists() and time.monotonic() < deadline:
        time.sleep(0.1)


def restart_server():
    stop_server()
    start_server()
    check(f"restarted private server {server_starts} owns the private socket", server_owns_socket())


def startup_logs(timeout=60):
    """Wait until every startup-sourced log for this plugin has finished; return them."""
    deadline = time.monotonic() + timeout
    entries = []
    while time.monotonic() < deadline:
        logs = herdr_json("plugin", "log", "list", "--plugin", PLUGIN, "--limit", "50")["logs"]
        entries = [log for log in logs if log.get("event") == "startup"]
        if entries and all(log["status"] != "running" for log in entries):
            # Herdr runs startup hooks once, synchronously at server start; a
            # second pass would already be listed, but settle briefly anyway.
            time.sleep(1)
            logs = herdr_json("plugin", "log", "list", "--plugin", PLUGIN, "--limit", "50")["logs"]
            return [log for log in logs if log.get("event") == "startup"]
        time.sleep(0.2)
    fail(f"startup hooks did not finish: {entries!r}")


try:
    # -- Private source repository with main, broken and update refs ----------
    bare = root / "repos" / OWNER_REPO.split("/")[0] / (OWNER_REPO.split("/")[1] + ".git")
    bare.parent.mkdir(parents=True)
    run(["git", "init", "-q", "--bare", str(bare)])
    (bare.parent / OWNER_REPO.split("/")[1]).symlink_to(bare.name)
    run(["git", "-C", str(SOURCE), "push", "-q", str(bare), f"{source_commit}:refs/heads/main"],
        env=dict(environment, HOME=str(real_home)) | {"GIT_CONFIG_NOSYSTEM": "1"})
    run(["git", "--git-dir", str(bare), "symbolic-ref", "HEAD", "refs/heads/main"])
    run(["git", "config", "--global", f"url.file://{root}/repos/.insteadOf", "https://github.com/"])
    work = root / "work"
    run(["git", "clone", "-q", str(bare), str(work)])
    identity = ["-c", "user.name=package validation", "-c", "user.email=package-validation@invalid"]
    run(["git", "-C", str(work), "checkout", "-q", "-b", "broken"])
    with (work / "src/main.rs").open("a") as broken:
        broken.write('\ncompile_error!("package validation: deliberately failed build");\n')
    run(["git", "-C", str(work), *identity, "commit", "-qam", "Deliberately failed build"])
    run(["git", "-C", str(work), "checkout", "-q", "-b", "update", "main"])
    cargo_toml = (work / "Cargo.toml").read_text()
    old_version = re.search(r'(?m)^version = "(\d+)\.(\d+)\.(\d+)"$', cargo_toml)
    old_version_text = ".".join(old_version.groups())
    new_version = f"{old_version.group(1)}.{old_version.group(2)}.{int(old_version.group(3)) + 1}"
    (work / "Cargo.toml").write_text(cargo_toml.replace(old_version.group(0), f'version = "{new_version}"', 1))
    lock = (work / "Cargo.lock").read_text()
    locked_entry = f'name = "{PLUGIN}"\nversion = "{old_version_text}"'
    check("update fixture edits the locked root package", locked_entry in lock)
    (work / "Cargo.lock").write_text(lock.replace(locked_entry, f'name = "{PLUGIN}"\nversion = "{new_version}"', 1))
    run(["git", "-C", str(work), *identity, "commit", "-qam", "Package validation source update"])
    run(["git", "-C", str(work), "push", "-q", "origin", "broken", "update"])
    update_commit = run(["git", "-C", str(work), "rev-parse", "update"]).stdout.strip()
    evidence["versions"] = {"installed": old_version_text, "update": new_version, "update_commit": update_commit}

    # -- Private Herdr server --------------------------------------------------
    start_server()
    check("private server owns the private socket", server_owns_socket())
    check("private registry starts empty", registry() is None)

    # -- 1. A failed build never registers -------------------------------------
    failed = install("broken")
    check("failed build: install exits nonzero", failed.returncode != 0, failed.stdout[-2000:])
    combined = failed.stdout + failed.stderr
    check("failed build: Herdr reports the build failure",
          "deliberately failed build" in combined and "Plugin was not installed." in combined, combined[-2000:])
    check("failed build: no registry entry", not registry())
    check("failed build: no managed checkout left",
          not any((config / "herdr/plugins").glob("github/*")) and not any((config / "herdr/plugins").glob(".tmp-install-*")))

    # -- 2. Clean install: Herdr clones, builds (locked, release) and registers -
    installed = install()
    check("clean install exits zero", installed.returncode == 0, installed.stdout[-2000:] + installed.stderr[-2000:])
    entries = registry()
    check("clean install registers exactly this plugin", entries and len(entries) == 1 and entries[0]["plugin_id"] == PLUGIN,
          repr(entries))
    entry = entries[0]
    plugin_root = Path(entry["plugin_root"])
    check("clean install resolves the published commit", entry["source"]["resolved_commit"] == source_commit, repr(entry["source"]))
    check("installed plugin is private and enabled", entry["enabled"] and str(plugin_root).startswith(str(config)))
    binary = plugin_root / "bin" / PLUGIN
    check("build installed the shipped executable", binary.is_file() and os.access(binary, os.X_OK))
    check("shipped executable reports the source version",
          run([str(binary), "--version"]).stdout == f"{PLUGIN} {old_version_text}\n")
    check("build ignored the inherited CARGO_TARGET_DIR", not foreign_target.exists())
    listed = {item["action_id"] for item in herdr_json("plugin", "action", "list", "--plugin", PLUGIN)["actions"]}
    check("installed action list", listed == {"health", "doctor", "ensure", "stop", "view"}, repr(listed))
    installed_sha = sha256(binary)

    # -- 3. Installed actions run the shipped executable in the Herdr context --
    plugin_state = state / "herdr/plugins" / PLUGIN
    not_running = action("health")
    check("registration alone does not start the daemon",
          not_running["exit_code"] == 3 and "daemon is not running" in not_running["stderr"], repr(not_running))

    # -- 3a. Herdr runs the [[startup]] entry when the server starts -----------
    # Restart only the private server: Herdr 0.9.1 runs every enabled plugin's
    # startup commands once, right after the server is ready.
    restart_server()
    started = startup_logs()
    check("startup entry ran exactly once", len(started) == 1, repr(started))
    startup = started[0]
    # Behavioral checks first, so a miswired entry fails on what it does, not
    # only on the literal manifest command.
    check("startup entry exits 0", startup["status"] != "running" and startup["exit_code"] == 0, repr(startup))
    running = daemons()
    started_daemons.update(running)
    check("startup started exactly one daemon", len(running) == 1, repr(running))
    (daemon_pid, daemon_command), = running.items()
    expected_command = f"{binary} daemon run --state-dir {plugin_state} --host-endpoint {socket}"
    check("startup daemon is the shipped executable with Herdr's state and socket", daemon_command == expected_command,
          f"{daemon_command!r} != {expected_command!r}")
    check("startup reports a serving daemon",
          field(startup["stdout"], "state") in ("healthy", "degraded"), repr(startup))
    # The literal manifest command is pinned by the deterministic
    # manifest::manifest_matches_pinned_091_documented_argv_shape test; the gate
    # checks only what the entry does.
    boot_one = field(startup["stdout"], "boot_id")
    evidence["startup"] = {"exit_code": startup["exit_code"], "command": startup["command"],
                           "daemon_command": daemon_command}

    ensured = action("ensure")
    check("ensure action succeeds", ensured["exit_code"] == 0, repr(ensured))
    check("ensure reports a serving daemon", field(ensured["stdout"], "state") in ("healthy", "degraded"), ensured["stdout"])
    check("ensure reuses the startup daemon", field(ensured["stdout"], "boot_id") == boot_one and boot_one,
          f"{ensured['stdout']!r} vs startup boot {boot_one!r}")
    check("ensure after startup keeps exactly one daemon", daemons() == {daemon_pid: expected_command}, repr(daemons()))
    instance = field(ensured["stdout"], "instance_id")
    health = action("health")
    check("health action succeeds against the same daemon",
          health["exit_code"] == 0 and field(health["stdout"], "boot_id") == boot_one, repr(health))
    doctor = action("doctor")
    check("doctor action succeeds", doctor["exit_code"] == 0, repr(doctor))
    check("doctor action reports a concise serving-daemon verdict",
          field(doctor["stdout"], "doctor") in ("ok", "degraded")
          and field(doctor["stdout"], "daemon") in ("healthy", "degraded")
          and "details: herdr-threads doctor --debug" in doctor["stdout"], doctor["stdout"])
    doctor_debug = run([str(binary), "--state-dir", str(plugin_state), "--host-endpoint", str(socket),
                        "doctor", "--debug"]).stdout
    for line in (f"state_dir: {plugin_state}", f"host_endpoint: {socket} (present)",
                 f"version: {old_version_text}", "daemon.version_matches: true"):
        check(f"doctor debug reports {line.split(':')[0]}", line in doctor_debug, doctor_debug)
    evidence["daemon"] = {"state": field(ensured["stdout"], "state"), "command": daemon_command}
    evidence["doctor_result"] = field(doctor["stdout"], "doctor")

    # -- 3b. Live handoff reruns the startup entry against the serving daemon -
    # `herdr server live-handoff` (also what `herdr update --handoff` sends)
    # spawns a detached `herdr server --handoff-import`, which runs every
    # startup command again once it owns the sockets
    # (src/server/headless/bootstrap.rs:197 in Herdr 0.9.1). The daemon started
    # above must survive the handoff and be reused, not replaced or duplicated.
    old_server = server
    run([herdr, "server", "live-handoff"], timeout=60)
    handed_off_at_ms = int(time.time() * 1000)
    # Precondition (Herdr behaviour, not a package claim): the original server
    # exits once the successor owns the sockets.
    try:
        old_server.wait(timeout=15)
    except subprocess.TimeoutExpired:
        fail("live handoff precondition: the original private server did not exit within 15 s")
    server = None
    successors = private_handoff_servers()
    check("live handoff: exactly one private handoff-import server", len(successors) == 1, repr(successors))
    (handoff_server, _), = successors.items()
    check("live handoff: the successor owns the private socket",
          socket.exists() and "logs" in herdr_json("plugin", "log", "list", "--plugin", PLUGIN, "--limit", "1"))
    handoff_started = startup_logs()
    check("live handoff reran the startup entry exactly once", len(handoff_started) == 1, repr(handoff_started))
    handoff_startup = handoff_started[0]
    check("handoff startup entry exits 0", handoff_startup["exit_code"] == 0, repr(handoff_startup))
    check("handoff startup reuses the serving daemon", field(handoff_startup["stdout"], "boot_id") == boot_one,
          f"{handoff_startup['stdout']!r} vs startup boot {boot_one!r}")
    check("handoff keeps exactly the startup daemon", daemons() == {daemon_pid: expected_command}, repr(daemons()))
    handoff_health = action("health")
    check("health after handoff reaches the same daemon",
          handoff_health["exit_code"] == 0 and field(handoff_health["stdout"], "boot_id") == boot_one,
          repr(handoff_health))
    # The successor is a new server process, hence a new host incarnation (the
    # daemon's peer pid/start-time witness). Host-backed calls fail with
    # stale_host_observation until the observation worker has published and
    # reconciled a snapshot from the successor; wait for that, then step 4's
    # first seat resolve is the host-backed proof.
    deadline = time.monotonic() + 30
    reconciled_at = None
    while time.monotonic() < deadline:
        # Explicit context flags: step 4 is the first call that relies on
        # Herdr's plugin environment (HERDR_PLUGIN_STATE_DIR) to find the daemon.
        probe = run([str(binary), "--state-dir", str(plugin_state), "--host-endpoint", str(socket),
                     "daemon", "health"], expected=None)
        value = field(probe.stdout, "last_reconciliation_at") if probe.returncode == 0 else None
        if value and value.isdigit() and int(value) > handed_off_at_ms:
            reconciled_at = int(value)
            break
        time.sleep(0.5)
    check("daemon reconciles against the handoff successor", reconciled_at is not None,
          f"last health: {probe.stdout[-1500:]!r} {probe.stderr[-500:]!r}")
    evidence["handoff"] = {"reconciled_after_ms": reconciled_at - handed_off_at_ms,
                           "startup_exit_code": handoff_startup["exit_code"],
                           "startup_boot_id": field(handoff_startup["stdout"], "boot_id"),
                           "daemon_pid_unchanged": daemons() == {daemon_pid: expected_command}}

    # -- 4. Durable mail data through the shipped CLI --------------------------
    workspace = herdr_json("workspace", "create", "--cwd", str(root), "--label", "package-validation", "--no-focus")
    seat_pane = workspace["root_pane"]["pane_id"]
    cli_env = dict(environment, HERDR_PLUGIN_STATE_DIR=str(plugin_state))
    resolved = run([str(binary), "seat", "resolve", "--pane", seat_pane, "--new-seat", "--operator"], env=cli_env)
    seat = field(resolved.stdout, "value")
    check("operator resolves a fresh seat", bool(seat and re.fullmatch(r"s[0-9A-Za-z]{8}", seat)), resolved.stdout)
    cooperative = ["--cooperative-seat", seat, "--cooperative-target", seat_pane,
                   "--cooperative-harness", "codex", "--cooperative-role", "top-level"]
    run([str(binary), "check-in", "--lifecycle-event", f"package-validation-{uuid.uuid4()}", *cooperative],
        env=dict(cli_env, HERDR_PANE_ID=seat_pane))
    topic = f"package validation survives update {uuid.uuid4().hex[:8]}"
    created = run([str(binary), "thread", "create", "--topic", topic, *cooperative],
                  env=dict(cli_env, HERDR_PANE_ID=seat_pane))
    thread = field(created.stdout, "value")
    check("seat creates a durable thread", bool(thread and re.fullmatch(r"t[0-9A-Za-z]{8}", thread)), created.stdout)

    # -- 5. The operator view stays readable until q ---------------------------
    opened = action("view")
    check("view action succeeds", opened["exit_code"] == 0, repr(opened))
    view_pane = json.loads(opened["stdout"])["result"]["plugin_pane"]["pane"]["pane_id"]
    first = wait_pane(view_pane, lambda text: PROMPT in text and thread in text)
    check("view renders the thread topic", topic in first, first[-2000:])
    time.sleep(3)
    check("view pane is still open after 3s", pane_exists(view_pane))
    run([herdr, "pane", "send-text", view_pane, "x"])
    run([herdr, "pane", "send-keys", view_pane, "enter"])
    wait_pane(view_pane, lambda text: "Press Enter to refresh or q to quit." in text)
    check("unrelated input keeps the view open", pane_exists(view_pane))
    refreshes = first.count(PROMPT)
    run([herdr, "pane", "send-keys", view_pane, "enter"])
    wait_pane(view_pane, lambda text: text.count("Press Enter to refresh or q to quit.") >= 1 and text.count(PROMPT) > refreshes + 1)
    check("Enter refreshes the view in place", pane_exists(view_pane))
    run([herdr, "pane", "send-text", view_pane, "q"])
    run([herdr, "pane", "send-keys", view_pane, "enter"])
    deadline = time.monotonic() + 10
    while pane_exists(view_pane) and time.monotonic() < deadline:
        time.sleep(0.1)
    check("q closes the view pane", not pane_exists(view_pane))

    # -- 6. A failed rebuild leaves the working install untouched --------------
    registry_before = (config / "herdr/plugins.json").read_bytes()
    failed_update = install("broken")
    check("failed rebuild exits nonzero", failed_update.returncode != 0)
    check("failed rebuild leaves the registry byte-identical",
          (config / "herdr/plugins.json").read_bytes() == registry_before)
    check("failed rebuild leaves the shipped executable", sha256(binary) == installed_sha)
    still = action("health")
    check("failed rebuild leaves the daemon serving",
          still["exit_code"] == 0 and field(still["stdout"], "boot_id") == boot_one, repr(still))

    # -- 7. Source update while the older daemon runs --------------------------
    updated = install("update")
    check("source update install exits zero", updated.returncode == 0, updated.stdout[-2000:] + updated.stderr[-2000:])
    entries = registry()
    check("update replaces the single registration",
          len(entries) == 1 and entries[0]["source"]["resolved_commit"] == update_commit, repr(entries))
    binary = Path(entries[0]["plugin_root"]) / "bin" / PLUGIN
    check("updated executable reports the new version",
          run([str(binary), "--version"]).stdout == f"{PLUGIN} {new_version}\n")
    check("the older daemon is still running", daemon_pid in daemons())
    # Restart only the private server again: the updated startup entry must hit
    # the version-mismatch path and never start a second writer.
    restart_server()
    update_started = startup_logs()
    check("updated startup entry ran exactly once", len(update_started) == 1, repr(update_started))
    update_startup = update_started[0]
    check("updated startup reports the version mismatch",
          update_startup["exit_code"] == 3 and "(daemon_version_mismatch)" in update_startup["stderr"]
          and old_version_text in update_startup["stderr"], repr(update_startup))
    check("updated startup did not start a second writer", set(daemons()) == {daemon_pid}, repr(daemons()))
    evidence["update_startup"] = {"exit_code": update_startup["exit_code"]}
    mismatch = action("ensure")
    check("ensure reports the version mismatch",
          mismatch["exit_code"] == 3 and "(daemon_version_mismatch)" in mismatch["stderr"]
          and old_version_text in mismatch["stderr"], repr(mismatch))
    check("ensure did not start a second writer", set(daemons()) == {daemon_pid})
    doctor_mismatch = action("doctor")
    check("doctor reports the version mismatch",
          doctor_mismatch["exit_code"] == 3
          and field(doctor_mismatch["stdout"], "doctor") == "daemon_version_mismatch",
          repr(doctor_mismatch))
    stopped_old = action("stop")
    check("updated stop reaches the older owner",
          stopped_old["exit_code"] == 0 and "stop_accepted" in stopped_old["stdout"]
          and field(stopped_old["stdout"], "boot_id") == boot_one, repr(stopped_old))
    check("older daemon exited", wait_gone(daemon_pid))
    restarted = action("ensure")
    check("ensure starts the updated daemon", restarted["exit_code"] == 0, repr(restarted))
    check("updated daemon reports the new version", field(restarted["stdout"], "software_version") == new_version)
    check("updated daemon keeps the instance", field(restarted["stdout"], "instance_id") == instance)
    check("updated daemon is a new boot", field(restarted["stdout"], "boot_id") != boot_one)
    new_daemons = daemons()
    started_daemons.update(new_daemons)
    view = run([str(binary), "view", "--once"], env=cli_env).stdout
    check("thread survives the source update", thread in view and topic in view.replace("\n", ""), view[-3000:])
    seats = run([str(binary), "seat", "list"], env=cli_env).stdout
    check("seat survives the source update", seat in seats, seats)

    # -- 8. Explicit shutdown ----------------------------------------------------
    stopped = action("stop")
    check("stop action succeeds", stopped["exit_code"] == 0 and "stop_accepted" in stopped["stdout"], repr(stopped))
    check("updated daemon exited", all(wait_gone(pid) for pid in new_daemons))
    after = action("health")
    check("health reports not running after stop", after["exit_code"] == 3, repr(after))
    check("no private daemon remains", not daemons())
    started_daemons.clear()

    # -- 9. Uninstall (transition to the link step; Herdr behaviour, no claim) --
    # The link step then reads the thread written before the uninstall.
    run([herdr, "plugin", "uninstall", PLUGIN])

    # -- 10. Local development link: build first, because link never builds ----
    checkout = root / "linkdev"
    run(["git", "clone", "-q", "--branch", "main", str(bare), str(checkout)])
    linked_binary = checkout / "bin" / PLUGIN
    run([herdr, "plugin", "link", str(checkout)])
    entries = registry()
    check("link registers the checkout in place",
          entries and len(entries) == 1 and Path(entries[0]["plugin_root"]).resolve() == checkout.resolve(), repr(entries))
    check("link does not build", not linked_binary.exists())
    unbuilt = action("health")
    check("unbuilt link fails loudly", unbuilt["exit_code"] not in (0, 3) and "bin/herdr-threads" in unbuilt["stderr"],
          repr(unbuilt))
    unbuilt_ensure = action("ensure")
    check("unbuilt link ensure fails and starts nothing",
          unbuilt_ensure["exit_code"] not in (0, 3) and not daemons(), repr(unbuilt_ensure))
    run([str(checkout / "scripts/build.sh")], cwd=str(checkout), timeout=900)
    check("local build ignored the inherited CARGO_TARGET_DIR", not foreign_target.exists())
    check("local build installs the checkout executable",
          run([str(linked_binary), "--version"]).stdout == f"{PLUGIN} {old_version_text}\n")
    linked_ensure = action("ensure")
    check("linked ensure succeeds", linked_ensure["exit_code"] == 0, repr(linked_ensure))
    linked = daemons()
    started_daemons.update(linked)
    linked_command = f"{linked_binary.resolve()} daemon run --state-dir {plugin_state} --host-endpoint {socket}"
    check("linked daemon is the checkout executable with Herdr's state and socket",
          list(linked.values()) == [linked_command], f"{linked!r} != {linked_command!r}")
    check("linked daemon keeps the instance", field(linked_ensure["stdout"], "instance_id") == instance)
    linked_health = action("health")
    check("linked health reaches the same daemon", linked_health["exit_code"] == 0
          and field(linked_health["stdout"], "boot_id") == field(linked_ensure["stdout"], "boot_id"), repr(linked_health))
    view = run([str(linked_binary), "view", "--once"], env=cli_env).stdout
    check("linked checkout reads the preserved thread", thread in view, view[-3000:])
    linked_stop = action("stop")
    check("linked stop succeeds", linked_stop["exit_code"] == 0 and "stop_accepted" in linked_stop["stdout"],
          repr(linked_stop))
    check("linked daemon exited", all(wait_gone(pid) for pid in linked))
    started_daemons.clear()
    run([herdr, "plugin", "unlink", PLUGIN])
    evidence["result"] = "pass"
finally:
    if started_daemons or ((server is not None or handoff_server is not None) and daemons()):
        for pid in daemons():
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
    stop_server()
    for pid in private_handoff_servers():
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    shared_after = shared_snapshot()
    if shared_after != shared_before:
        evidence["result"] = "fail"
        print(f"shared Herdr registry changed: {shared_before!r} -> {shared_after!r}", file=sys.stderr)
    if os.environ.get("HT_PACKAGE_KEEP"):
        print(f"kept {root}", file=sys.stderr)
    else:
        shutil.rmtree(root, ignore_errors=True)

check("shared Herdr registry and plugin directories unchanged", shared_after == shared_before)
print(json.dumps(evidence, indent=2))
print("PACKAGE_VALIDATION_PASS")
PY
