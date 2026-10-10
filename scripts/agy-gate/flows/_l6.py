"""Helpers shared by the L6 flows (delivery, downgrade); not a flow itself.

Thread and message setup over the herdr-threads CLI, flow-local read-only queries over the daemon
database (test-infra spec D6 observation 1), transcript reads, and the ``hooks.json`` edit that puts
``step_mutator.py`` in front of the ``PreInvocation`` hook (spec D3). The leading underscore keeps
``gate.discover`` from loading this module as a flow.
"""
from __future__ import annotations

import json
import re
import shlex
import sys
import time
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l4  # noqa: E402
import _l5  # noqa: E402
import observe  # noqa: E402

MUTATOR = GATE_DIR / "step_mutator.py"
PYTHON = "/usr/bin/python3"
# The D4 frame header (delivery-policy spec DP2), as the fake server and the daemon see it.
_NAME = r'(?: "(?:[^"\\]|\\.)*")?'
HEADER = re.compile(r"^\[herdr-threads\] (\S+) (\S+) in (\S+)" + _NAME + r" from (\S+)" + _NAME + r"(.*):$", re.M)
OVERSIZE = 10 * 1024  # over the 8 KiB page budget, so the frame is truncated

load_scenario = _l5.load_scenario
query = _l5.query


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


def dismiss_onboarding(pane, limit=10):
    """Walk a first-run wizard (color scheme, then Terms of Service) until the prompt footer shows.

    The genuine agy 1.3.2 shows it on every fresh HOME and Herdr reports the wizard as ``idle``, so a
    prompt typed into it is lost. Returns the pages it dismissed (empty when there was none)."""
    pages = []
    for _ in range(limit):
        tail = screen_tail(pane)
        if "? for shortcuts" in tail:
            break
        if "Choose your color scheme" in tail:
            pages.append("color")
            pane.keys("Enter")
        elif "Terms of Service" in tail and "[Done]" in tail:
            pages.append("terms")
            pane.keys("Down")
            pane.keys("Right")
            pane.keys("Enter")
        elif "Done" in tail and "Navigate" in tail:
            pages.append("confirm")
            pane.keys("Enter")
        else:
            break
        time.sleep(1.5)
    return pages


SURVEY = "How's the CLI experience so far"


def dismiss_survey(pane):
    """Skip agy's feedback survey (``[0] Skip``), which appears after an executor error and swallows the
    next typed prompt. True when it was showing."""
    if SURVEY not in screen_tail(pane, 20):
        return False
    pane.keys("0")
    time.sleep(1)
    return True


def start_agy(world, **kwargs):
    """``world.start_agy`` followed by the onboarding walk."""
    pane = world.start_agy(**kwargs)
    pages = dismiss_onboarding(pane)
    if pages:
        world.artifact("onboarding-pages.txt", ",".join(pages))
    return pane


def setup(world):
    """``setup agy --with-permissions`` as a person (an agent caller must write ``setup`` first and is
    refused ``--state-dir`` ahead of it, which ``World.ht`` always does)."""
    return world.ht(["setup", "agy", "--with-permissions"], check=False, human=True)


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


def join(world, sender, pane, thread, seat):
    """The sender invites the agy seat and the agy pane accepts."""
    invite = world.ht(["invite", thread, "--seat", seat], pane=sender.pane_id, check=False, human=True)
    accept = world.ht(["accept", thread], pane=pane.pane_id, check=False)
    return invite, accept


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
