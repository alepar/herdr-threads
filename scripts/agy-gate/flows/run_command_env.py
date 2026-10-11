"""``run_command_env``: an agy ``run_command`` shell carries the attribution environment (ht-54r.84).

``AgyAdapter::caller_env`` declares that the shells agy's ``run_command`` starts inherit
``ANTIGRAVITY_CONVERSATION_ID`` and have an ``agy``-family process as an ancestor. Nobody had observed that
on the supported path. The genuine agy, driven by a scripted fake upstream, runs three commands in a
real ``run_command`` shell: ``env``, ``echo ppid=$PPID`` and ``herdr-threads inbox``. The
next request carries each output in a ``functionResponse``. The flow checks that the variable names the
presence-lock conversation (and the live binding's native session), that an agy-family process is an
ancestor of the shell, and that the inbox call is attributed and served.

Separate commands, not one ``;``-joined line: the owned allow rule (``command(herdr-threads)``) blocks a
compound command, and separate calls give separate evidence. ``env`` and ``echo`` are outside that rule,
so the flow adds ``command(env)`` and ``command(echo)`` to the isolated settings before agy starts.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l4  # noqa: E402
import _l5  # noqa: E402
import observe  # noqa: E402
from gate import Flow  # noqa: E402

SCENARIO = GATE_DIR / "scenarios" / "run-command-env.json"
EXTRA_ALLOW = ("command(env)", "command(echo)")
VARIABLE = "ANTIGRAVITY_CONVERSATION_ID"
AGY_NAMES = ("agy", "antigravity", "antigravity-cli")
REFUSAL_CODES = ("missing_session", "no_harness", "unattributed_session", "no_fingerprint",
                 "session_claim_required")
MAX_CHAIN = 40


def _strings(node):
    if isinstance(node, str):
        yield node
    elif isinstance(node, dict):
        for value in node.values():
            yield from _strings(value)
    elif isinstance(node, list):
        for value in node:
            yield from _strings(value)


def _responses(node):
    if isinstance(node, dict):
        if isinstance(node.get("functionResponse"), dict):
            yield node["functionResponse"]
        for value in node.values():
            yield from _responses(value)
    elif isinstance(node, list):
        for value in node:
            yield from _responses(value)


def last_functionresponse(world, request):
    """The newest functionResponse of a saved request body, its string values joined by newlines.

    Each follow-up request repeats the earlier responses; the last one is this step's command output.
    """
    try:
        found = list(_responses(json.loads(_l4.body_text(world, request))))
    except ValueError:
        return ""
    return "\n".join(_strings(found[-1].get("response"))) if found else ""


def allow_extra(world):
    doc = _l4.read_settings(world) or {}
    permissions = doc.setdefault("permissions", {})
    allow = permissions.setdefault("allow", [])
    for rule in EXTRA_ALLOW:
        if rule not in allow:
            allow.append(rule)
    _l4.write_settings(world, doc)


def process_row(pid):
    """``(ppid, comm)`` of ``pid`` as the gate sees it, or None."""
    result = subprocess.run(["ps", "-o", "ppid=,comm=", "-p", str(pid)], stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, universal_newlines=True)
    line = result.stdout.strip()
    if result.returncode != 0 or not line:
        return None
    parts = line.split(None, 1)
    if len(parts) != 2 or not parts[0].isdigit():
        return None
    return int(parts[0]), parts[1].strip()


def ancestor_chain(pid):
    """``[(pid, comm), ...]`` from ``pid`` up to the root."""
    chain, seen = [], set()
    while pid and pid not in seen and len(chain) < MAX_CHAIN:
        seen.add(pid)
        row = process_row(pid)
        if row is None:
            break
        chain.append((pid, row[1]))
        pid = row[0]
    return chain


def is_agy_family(comm):
    return Path(comm).name in AGY_NAMES


def parse_ppid(text):
    """The pid in an ``echo ppid=$PPID`` response, or None."""
    match = re.search(r"(?m)^ppid=(\d+)\s*$", text)
    return int(match.group(1)) if match else None


def run(world):
    scenario = _l4.load_into(world, SCENARIO)
    world.check("setup agy succeeds", world.setup_agy().returncode == 0)
    allow_extra(world)
    world.check("env and echo are allowed beside the owned rule",
                all(rule in _l4.allow_members(world) for rule in EXTRA_ALLOW),
                "allow=%s" % _l4.allow_members(world))
    world.artifact("agy-version.txt", world.agy_version())
    pane = world.start_agy()

    pane.prompt("ENV-PROBE run the three commands")
    env_req = _l5.await_step(world, "echo-call", 60)  # the request after the env command
    echo_req = _l5.await_step(world, "inbox-call", 60)
    final = _l5.await_step(world, "env-final", 60)
    world.check("every probe command was followed by a request", all((env_req, echo_req, final)),
                "env=%s echo=%s final=%s" % (bool(env_req), bool(echo_req), bool(final)))
    world.check("agy never blocked on the probe commands", not pane.wait("blocked", 3),
                "screen=%s" % pane.read_pane(40)[-300:])

    env_text = last_functionresponse(world, env_req)
    echo_text = last_functionresponse(world, echo_req)
    inbox_text = last_functionresponse(world, final)
    env_lines = sorted(set(re.findall(r"(?m)^ANTIGRAVITY[A-Z_]*=\S*", env_text)))
    env_lines = [re.sub(r"^(\w*(?:TOKEN|CSRF|SECRET|KEY)\w*)=.*$", r"\1=<redacted>", line) for line in env_lines]
    world.artifact("env-response.txt", "\n".join(env_lines) + "\nother-variables: %d\n"
                   % len(re.findall(r"(?m)^[A-Za-z_][A-Za-z0-9_]*=", env_text)))
    world.artifact("echo-response.txt", echo_text)
    world.artifact("inbox-response.txt", inbox_text)

    ids = _l5.presence_ids(world)
    world.check("exactly one agy presence lock", len(ids) == 1, "ids=%s" % ids)
    seen = re.findall(r"(?m)^" + VARIABLE + r"=(\S+)", env_text)
    world.check("the run_command shell has %s" % VARIABLE, bool(seen), "lines=%s" % env_lines)
    world.check("the variable names the presence-lock conversation", bool(seen) and seen[0] in ids,
                "variable=%s locks=%s" % (seen[:1], ids))
    binding = _l5.live_binding(world)
    world.check("one live agy binding exists", binding is not None, "bindings=%s" % _l5.live_bindings(world))
    world.check("the binding's native session is the variable's id",
                bool(binding) and bool(seen) and binding["native_session"] == seen[0],
                "binding=%s variable=%s" % (binding, seen[:1]))

    parent = parse_ppid(echo_text)
    world.check("the echo response names the shell's parent pid", parent is not None, "echo=%r" % echo_text[-300:])
    chain = ancestor_chain(parent) if parent else []
    world.artifact("ancestor-chain.txt", "\n".join("%d %s" % c for c in chain) + "\n")
    world.check("an agy-family process is an ancestor of the shell",
                any(is_agy_family(comm) for _pid, comm in chain), "chain=%s" % chain)

    served = re.search(r"exited with code 0\b", inbox_text) is not None
    refused = [c for c in REFUSAL_CODES if c in inbox_text]
    world.check("the inbox call exited 0 with no attribution refusal", served and not refused,
                "refusals=%s inbox=%r" % (refused, inbox_text[-400:]))
    world.check("agy returned to idle", pane.wait("idle", 30))
    pane.exit()
    _l4.finish_checks(world, scenario)


FLOW = Flow(name="run_command_env", requires=("ht-poh.4.4",), run=run)
