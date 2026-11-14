#!/usr/bin/env python3
"""Private rig for the Quick start recording: its own Herdr server, herdr-threads daemon and scratch HOME.

  rig.py up TOKEN_FILE        start everything, print the root (TOKEN_FILE: a mode-0600 CLAUDE_CODE_OAUTH_TOKEN)
  rig.py env ROOT -- CMD...   run a command in the rig's environment, e.g.  rig.py env ROOT -- herdr tab list
  rig.py down ROOT            stop the server and daemon, kill what names the root, remove it

Nothing here reads or writes the real Herdr server, the real daemon or the real Claude config.
"""
import json
import os
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HT = os.path.realpath(os.environ.get("HT_BIN") or shutil.which("herdr-threads") or sys.exit("herdr-threads not on PATH"))
HERDR = shutil.which("herdr") or sys.exit("herdr not on PATH")
DROP_PREFIXES = ("HERDR_", "CLAUDE", "CODEX", "HC_", "TMUX", "ANTHROPIC", "AISW")
DROP = ("AI_AGENT", "HERMES_SESSION_ID", "ANTIGRAVITY_CONVERSATION_ID", "ANTIGRAVITY_AGENT", "PI_CODING_AGENT",
        "OPENCODE", "PROMPT_COMMAND")


def rig_env(root, token=None):
    root = Path(root)
    env = {k: v for k, v in os.environ.items() if not (k.startswith(DROP_PREFIXES) or k in DROP)}
    env.update(
        HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "cfg"), XDG_STATE_HOME=str(root / "st"),
        XDG_RUNTIME_DIR=str(root / "rt"), XDG_CACHE_HOME=str(root / "cache"), XDG_DATA_HOME=str(root / "data"),
        HERDR_SOCKET_PATH=str(root / "h.sock"), HERDR_CONFIG_PATH=str(root / "cfg/herdr.toml"),
        CLAUDE_CONFIG_DIR=str(root / "home/.claude"),
        CLAUDE_CODE_PLUGIN_DIRS=str(root / "state/claude-mod/herdr-threads"),
        PATH=f"{root / 'bin'}:{os.environ.get('PATH', '')}",
        PS1=r"\[\e[1;36m\]you $ \[\e[0m\]", TERM="xterm-256color", HOSTNAME="demo", USER="you", LOGNAME="you",
        DISABLE_AUTOUPDATER="1", CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1",
    )
    if token is None and (root / "token").exists():
        token = (root / "token").read_text().strip()
    if token:
        env["CLAUDE_CODE_OAUTH_TOKEN"] = token
    return env


def sh(argv, env, timeout=120, check=True):
    out = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=timeout)
    if check and out.returncode != 0:
        raise RuntimeError(f"{' '.join(argv[:4])}: rc={out.returncode} {out.stderr.strip()[:600]} {out.stdout.strip()[:300]}")
    return out.stdout


def up(token_file):
    token = Path(token_file).read_text().strip()
    root = Path(tempfile.mkdtemp(prefix="hqs.", dir="/tmp"))
    os.chmod(root, 0o700)
    for sub in ("cfg", "st", "rt", "bin", "cache", "data", "home", "home/demo", "home/.claude", "state", "state/node"):
        (root / sub).mkdir(mode=0o700, parents=True, exist_ok=True)
    (root / "token").write_text(token + "\n")
    (root / "token").chmod(0o600)
    (root / "cfg/herdr.toml").write_text("onboarding = false\n[ui]\nsidebar_start_collapsed = true\nsidebar_collapsed_mode = \"hidden\"\n")
    q = shlex.quote
    shim = root / "bin/herdr-threads"
    shim.write_text("#!/bin/sh\npin_state=1 pin_host=1\nfor word in \"$@\"; do\n  case \"$word\" in\n"
                    "    --state-dir|--state-dir=*) pin_state=0 ;;\n    --host-endpoint|--host-endpoint=*) pin_host=0 ;;\n"
                    "  esac\ndone\n"
                    f"if [ \"$pin_host\" = 1 ]; then set -- --host-endpoint {q(str(root / 'h.sock'))} \"$@\"; fi\n"
                    f"if [ \"$pin_state\" = 1 ]; then set -- --state-dir {q(str(root / 'state'))} \"$@\"; fi\n"
                    f"exec {q(HT)} \"$@\"\n")
    shim.chmod(0o700)
    (root / "state/node/settings.json").write_text('{"sync":{"discover":false}}\n')
    (root / "state/node/settings.json").chmod(0o600)
    env = rig_env(root, token)
    with (root / "server.out").open("wb") as log:
        server = subprocess.Popen([HERDR, "server"], env=env, stdin=subprocess.DEVNULL, stdout=log,
                                  stderr=subprocess.STDOUT, start_new_session=True, cwd=str(root / "home/demo"))
    (root / "server.pid").write_text(f"{server.pid}\n")
    deadline = time.time() + 15
    while not (root / "h.sock").exists():
        if server.poll() is not None or time.time() > deadline:
            raise RuntimeError("private herdr server did not start: " + (root / "server.out").read_text()[-500:])
        time.sleep(0.1)
    pin = ["--state-dir", str(root / "state"), "--host-endpoint", str(root / "h.sock")]
    sh([HT, "daemon", "ensure", *pin], env)
    sh([HT, "setup", "claude", "--with-permissions", *pin], env)
    cfg = root / "home/.claude"
    demo = str(root / "home/demo")
    (cfg / ".claude.json").write_text(json.dumps(
        {"hasCompletedOnboarding": True, "projects": {demo: {"hasTrustDialogAccepted": True}}}))
    (cfg / ".claude.json").chmod(0o600)
    settings_path = cfg / "settings.json"
    settings = json.loads(settings_path.read_text()) if settings_path.exists() else {}
    perms = settings.setdefault("permissions", {})
    perms["allow"] = sorted(set(perms.get("allow", [])) | {"Bash", "WebFetch", "WebSearch"})
    settings["model"] = "sonnet"
    settings["spinnerTipsEnabled"] = False
    settings_path.write_text(json.dumps(settings, indent=2))
    skill = cfg / "skills/herdr-threads"
    skill.mkdir(parents=True, exist_ok=True)
    (skill / "SKILL.md").write_text(sh([HT, "skill"], env))
    print(root)


def down(root):
    root = Path(root)
    if root.parent != Path("/tmp") or not root.name.startswith("hqs."):
        sys.exit("not a rig root")
    env = rig_env(root)
    pin = ["--state-dir", str(root / "state"), "--host-endpoint", str(root / "h.sock")]
    for argv in ([HT, "daemon", "stop", *pin], [HERDR, "server", "stop"]):
        try:
            sh(argv, env, timeout=30, check=False)
        except Exception as error:  # still kill by root below
            print(f"down: {error}", file=sys.stderr)
    time.sleep(2)
    for _ in range(3):
        listing = subprocess.run(["ps", "axeww", "-o", "pid=", "-o", "command="], capture_output=True, text=True,
                                 errors="replace").stdout
        for line in listing.splitlines():
            pid, _, command = line.strip().partition(" ")
            if f"{root}/" in command and pid.isdigit() and int(pid) != os.getpid():
                try:
                    os.kill(int(pid), signal.SIGKILL)
                except ProcessLookupError:
                    pass
        time.sleep(1)
    subprocess.run(["rm", "-rf", str(root)], check=True)
    print("down", root)


if __name__ == "__main__":
    if sys.argv[1] == "up":
        up(sys.argv[2])
    elif sys.argv[1] == "down":
        down(sys.argv[2])
    elif sys.argv[1] == "env":
        os.execvpe(sys.argv[4], sys.argv[4:], rig_env(sys.argv[2]))
