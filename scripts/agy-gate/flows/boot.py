"""``boot``: the infrastructure self-proof flow (test-infra spec D7). No setup.

Prompt "hi"; the scenario replies BOOT-OK. Checks Herdr detects agent ``agy``, exactly one
main request reached the fake server, BOOT-OK is on the pane, and the sandbox denies the
real ``~/.gemini`` and a non-loopback connect with EPERM.
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import agyenv  # noqa: E402
import observe  # noqa: E402
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "boot.json"
CONNECT = ("import errno,socket,sys\n"
           "s=socket.socket(); s.settimeout(5)\n"
           "try:\n    s.connect(('192.0.2.1',443))\n"
           "except OSError as e:\n    print(errno.errorcode.get(e.errno,'?')); sys.exit(0)\n"
           "print('CONNECTED')\n")


def sandboxed(world, argv, profile=None, timeout=30):
    profile = str(profile or world.root / "loopback.sb")
    return subprocess.run([agyenv.SANDBOX_EXEC, "-f", profile] + argv, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True,
                          timeout=timeout)


def eperm(result):
    return result.returncode != 0 and "Operation not permitted" in result.stderr


def sandbox_self_test(world):
    """The rendered profile denies the real ~/.gemini and any non-loopback connect with EPERM.

    A missing real ~/.gemini yields ENOENT before the sandbox is consulted, so the rule is also
    proven against a sentinel ``.gemini`` under a stand-in home inside the root."""
    gemini = Path(agyenv.real_home()) / ".gemini"
    denied = sandboxed(world, ["/bin/ls", str(gemini)])
    absent = not gemini.exists()
    world.check("sandbox denies the real ~/.gemini", eperm(denied) or (absent and denied.returncode != 0),
                "exists=%s rc=%s stderr=%s" % (not absent, denied.returncode, denied.stderr.strip()[:200]))
    fake_home = world.root / "fakehome"
    (fake_home / ".gemini").mkdir(parents=True, exist_ok=True)
    (fake_home / ".gemini" / "sentinel").write_text("x")
    profile = world.root / "sentinel.sb"
    profile.write_text(agyenv.render_profile(world.root, fake_home))
    sentinel = sandboxed(world, ["/bin/ls", str(fake_home / ".gemini")], profile=profile)
    world.check("sandbox profile denies a present .gemini with EPERM", eperm(sentinel),
                "rc=%s stderr=%s" % (sentinel.returncode, sentinel.stderr.strip()[:200]))
    probe = sandboxed(world, ["/usr/bin/python3", "-I", "-c", CONNECT])
    world.check("sandbox denies a non-loopback connect with EPERM",
                probe.stdout.strip() == "EPERM",
                "stdout=%s stderr=%s" % (probe.stdout.strip(), probe.stderr.strip()[:200]))


def run(world):
    scenario = json.loads(SCENARIO.read_text())
    world.backend.load(scenario)
    pane = world.start_agy()
    detected = pane.agent_name()
    world.check("herdr detects agent agy", detected == "agy", "agent=%r" % (detected,))
    pane.prompt("hi")
    shown = observe.wait_until(lambda: "BOOT-OK" in pane.read_pane(), 30)
    pane.wait("idle", 30)
    world.artifact("pane-final.txt", pane.read_pane())
    world.check("BOOT-OK appears in the pane", bool(shown))
    mains = observe.main_requests(world)
    world.check("exactly one main request reached the fake server", len(mains) == 1,
                "main requests=%d" % len(mains))
    clean, detail = observe.backend_clean(world)
    world.check("no unmatched or template-error request", clean, detail)
    sandbox_self_test(world)
    world.artifact("requests.json", world.backend.requests())


FLOW = Flow(name="boot", requires=("ht-poh.6.1",), run=run)
