"""Helpers shared by the L6 flows (delivery, downgrade); not a flow itself.

Thread and message setup over the herdr-threads CLI, flow-local read-only queries over the daemon
database (test-infra spec D6 observation 1), transcript reads, and the ``hooks.json`` edit that puts
``step_mutator.py`` in front of the ``PreInvocation`` hook (spec D3). The leading underscore keeps
``gate.discover`` from loading this module as a flow.
"""
from __future__ import annotations

import contextlib
import hashlib
import json
import re
import shlex
import sys
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l4  # noqa: E402
import _l5  # noqa: E402
import agyenv  # noqa: E402
import observe  # noqa: E402

MUTATOR = GATE_DIR / "step_mutator.py"
PYTHON = "/usr/bin/python3"
# The D4 frame header (delivery-policy spec DP2), as the fake server and the daemon see it.
_NAME = r'(?: "(?:[^"\\]|\\.)*")?'
HEADER = re.compile(r"^\[herdr-threads\] (\S+) (\S+) in (\S+)" + _NAME + r" from (\S+)" + _NAME + r"(.*):$", re.M)
OVERSIZE = 10 * 1024  # over the 8 KiB page budget, so the frame is truncated

load_scenario = _l5.load_scenario
query = _l5.query


@contextlib.contextmanager
def response_hold(world, label):
    """Keep the scripted response live until all coordinator sends have committed."""
    path = Path(world.ctl) / ("hold-" + hashlib.sha256(label.encode()).hexdigest())
    for suffix in (".entered", ".release", ".expired"):
        Path(str(path) + suffix).unlink(missing_ok=True)
    def assert_held():
        if not observe.wait_until(lambda: Path(str(path) + ".entered").exists(), 30):
            raise RuntimeError("busy response did not enter: " + label)
        if Path(str(path) + ".expired").exists():
            raise RuntimeError("busy response expired before publication completed: " + label)
    try:
        yield assert_held
    finally:
        Path(str(path) + ".release").touch()


def data_of(doc):
    """``result.data`` of a ``--json`` envelope (or the object itself)."""
    if isinstance(doc, dict) and isinstance(doc.get("result"), dict) and "data" in doc["result"]:
        return doc["result"]["data"]
    return doc


def ht_json(world, argv, pane=None, human=False):
    """Run ``herdr-threads --json <argv>``; ``(returncode, decoded data or {})``."""
    result = world.ht(["--json"] + list(argv), pane=pane, check=False, human=human)
    try:
        return result.returncode, data_of(json.loads(result.stdout))
    except ValueError:
        return result.returncode, {"_stdout": result.stdout, "_stderr": result.stderr}


def screen_tail(pane, lines=30):
    return "\n".join(pane.read_pane(80).splitlines()[-lines:])


SURVEY = agyenv.SURVEY


def dismiss_onboarding(pane, limit=10):
    """Walk agy's first-run wizard (see ``AgyPane.dismiss_onboarding``)."""
    return pane.dismiss_onboarding(limit)


def dismiss_survey(pane):
    """Skip agy's feedback survey (see ``AgyPane.dismiss_survey``)."""
    return pane.dismiss_survey()


def start_agy(world, **kwargs):
    """``world.start_agy``, which already walks the first-run wizard."""
    return world.start_agy(**kwargs)


def setup(world):
    """``setup agy --with-permissions`` as the operator of the isolated world (``World.setup_agy``)."""
    return world.setup_agy()


def first_id(data, keys):
    """An id string from a result: the value itself, or the first of ``keys`` (searched depth first)."""
    if isinstance(data, str):
        return data
    for key in keys:
        hit = _l4.find(data, key)
        if isinstance(hit, str):
            return hit
    if isinstance(data, dict) and len(data) == 1:
        return first_id(next(iter(data.values())), keys)
    return None


def create_thread(world, sender, topic):
    rc, data = ht_json(world, ["thread", "create", "--topic", topic], pane=sender.pane_id, human=True)
    world.artifact("thread-create-%s.json" % topic, {"rc": rc, "data": data})
    return first_id(data, ("thread_id", "thread", "id"))


# Codes a refused attribution carries; an accept whose output names one did not run as the agy session.
REFUSAL_CODES = ("caller_not_located", "invalid_request", "missing_session", "no_harness",
                 "unattributed_session", "no_fingerprint", "session_claim_required")


class Accept(object):
    """Outcome of the agy seat's own ``accept``: ``ok``, a ``detail`` line and the raw run_command ``output``."""

    def __init__(self, ok, detail, output=""):
        self.ok, self.detail, self.output = ok, detail, output


def accept_verdict(output, state):
    """``(ok, detail)`` for an accept the agy session ran: exit 0, no refusal code, membership ``joined``."""
    refused = [code for code in REFUSAL_CODES if code in output]
    ran = re.search(r"exited with code 0\b", output) is not None
    ok = ran and not refused and state == "joined"
    return ok, "ran=%s refusals=%s membership=%s output=%r" % (ran, refused, state, output[-300:])


def membership_state(world, seat):
    """``state`` of the seat's membership (``joined``/``invited``/...); None when it has none."""
    rows = query(world, "SELECT state FROM memberships WHERE seat_id='%s'" % seat)
    return rows[0]["state"] if rows else None


def join(world, sender, pane, thread, seat, timeout=45):
    """The sender invites the agy seat and the agy session accepts: ``(invite, Accept)``.

    The invitation wakes the idle agent (``wake-join``); the scripted model answers that wake by running
    ``herdr-threads accept <thread>`` through agy's ``run_command``, so the accept carries the session claim
    (``ANTIGRAVITY_CONVERSATION_ID`` plus the agy ancestor). Calling ``accept`` from the gate process has
    neither, and the product refuses an unattributable caller (agent seats have no pane-context journal)."""
    invite = world.ht(["invite", thread, "--seat", seat], pane=sender.pane_id, check=False, human=True)
    done = _l5.await_step(world, "wake-join-done", timeout) if invite.returncode == 0 else None
    if done is None:
        return invite, Accept(False, "the accept step was not served (invite rc=%s)" % invite.returncode)
    pane.wait("idle", 30)
    output = _l4.last_functionresponse(world, done)
    ok, detail = accept_verdict(output, membership_state(world, seat))
    return invite, Accept(ok, detail, output)


def send(world, sender, thread, body, *flags):
    """Send ``body``; the new message id (None when the send failed)."""
    rc, data = ht_json(world, ["send", thread, "--body", body] + list(flags), pane=sender.pane_id, human=True)
    world.artifact("send-%s.json" % re.sub(r"\W+", "_", body[:12]), {"rc": rc, "data": data})
    return first_id(data, ("message_id", "message", "id")) if rc == 0 else None


def tables(world, *names):
    return {name: query(world, "SELECT * FROM %s" % name) for name in names}


def receipt(world, message_id):
    """``{state, actor, provenance}`` of the one recipient's receipt for ``message_id`` (``{}`` when none).

    ``provenance`` is the ``action_provenance`` of the recorded ACK observation (None while pending)."""
    rows = query(world, "SELECT state, ack_actor_seat_id, ack_observation FROM receipt_state "
                        "WHERE message_id='%s'" % message_id)
    if not rows:
        return {}
    try:
        observation = json.loads(rows[0]["ack_observation"] or "{}")
    except ValueError:
        observation = {}
    return {"state": rows[0]["state"], "actor": rows[0]["ack_actor_seat_id"],
            "provenance": observation.get("action_provenance"), "observation": observation}


def lazy_state(world, message_id):
    """``pending`` or ``displayed`` (a completed lazy delivery); None when the message is not lazy."""
    rows = query(world, "SELECT state FROM lazy_recipients WHERE message_id='%s'" % message_id)
    return rows[0]["state"] if rows else None


def transcript_text(pane):
    """Concatenated text of every transcript file the pane's agy wrote."""
    out, seen = [], set()
    for path in pane.transcripts():
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        if text not in seen:  # brain/ and conversations/ may expose the same file twice
            seen.add(text)
            out.append(text)
    return "\n".join(out)


def system_texts(request):
    return list((request or {}).get("system_messages") or [])


def frame_ids_in(texts):
    ids = []
    for text in texts:
        ids.extend(m.group(2) for m in HEADER.finditer(text))
    return ids


def hooks_path(world):
    return _l4.plugin_dir(world) / "hooks.json"


def install_mutator(world):
    """Prefix the ``PreInvocation`` command with ``step_mutator.py --ctl <ctl> --`` (spec D3).

    Returns the original ``hooks.json`` bytes, to restore byte-identically."""
    path = hooks_path(world)
    original = path.read_bytes()
    doc = json.loads(original.decode("utf-8"))
    prefix = " ".join(shlex.quote(p) for p in (PYTHON, "-I", str(MUTATOR), "--ctl", str(world.ctl), "--"))
    for entries in doc.values():
        for handler in entries["PreInvocation"]:
            handler["command"] = prefix + " " + handler["command"]
    path.write_bytes((json.dumps(doc, separators=(",", ":")) + "\n").encode("utf-8"))
    return original


def restore_hooks(world, original):
    hooks_path(world).write_bytes(original)


def arm(world, mode="abort"):
    fired = Path(world.ctl) / "mutate-fired"
    if fired.exists():
        fired.unlink()
    (Path(world.ctl) / ("mutate-armed" if mode == "abort" else "mutate-armed-ignore")).write_text("")


def fired(world):
    path = Path(world.ctl) / "mutate-fired"
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def doctor_delivery(world):
    """The ``hooks.agy.delivery`` projection of ``doctor --json`` (``{}`` when absent).

    Doctor resolves ``agy`` on ``PATH`` to judge whether a recorded downgrade still applies, so the
    call puts the world's current executable clone first: the identity a running agy's hooks record."""
    clone = world.root / "agybin" / str(world.stage_count)
    env = {"PATH": "%s:%s" % (clone, world.herdr_env().get("PATH", "/usr/bin:/bin"))}
    result = world.ht(["--json", "doctor"], check=False, env=env)
    try:
        data = data_of(json.loads(result.stdout))
    except ValueError:
        return {}
    return _l4.find(data, "delivery") or {}


def state_files(world):
    """Adapter state files (delivery conversations and downgrade record) as ``{relative path: text}``."""
    out = {}
    for base in sorted(world.state.glob("**/adapter-state/agy")):
        for path in sorted(base.rglob("*.json")):
            try:
                out[str(path.relative_to(base))] = path.read_text()
            except OSError:
                pass
    return out


def downgrade_record(world):
    """The parsed ``downgrade.json`` of the adapter state (None when the adapter wrote none)."""
    text = state_files(world).get("downgrade.json")
    try:
        return json.loads(text) if text else None
    except ValueError:
        return None


def frame_in_transcript(text, message_id):
    """True when a frame header line for ``message_id`` is in the transcript text."""
    return re.search(r"\[herdr-threads\] \S+ %s in " % re.escape(message_id), text) is not None


def unconsumed(world, scenario, optional=()):
    """Labels of ``scenario`` steps no main request consumed, minus ``optional``."""
    used = {r.get("step") for r in observe.main_requests(world)}
    return [s["label"] for s in scenario["steps"] if s["label"] not in used and s["label"] not in optional]
