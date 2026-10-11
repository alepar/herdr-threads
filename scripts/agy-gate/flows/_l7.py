"""Helpers for the L7 wake flow; not a flow itself (the leading underscore keeps ``gate.discover`` out).

A wake is a ``<USER_REQUEST>`` the gate did not type (test-infra spec D7, row ``wake``). The fake
server logs every ``<USER_REQUEST>`` of a request's whole history, so the count of untyped entries
in the longest such list is monotonic, and a wake is a rise of that count over a baseline.
"""
from __future__ import annotations

import sys
import time
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l5  # noqa: E402
import _l6  # noqa: E402
import observe  # noqa: E402

SCEN = GATE_DIR / "scenarios"
QUIET_WINDOW = 15  # seconds a "no wake" check watches (spec D7 (b), (d))
WAKE_WINDOW = 60  # seconds a wake may take (spec D7 (a), (e))


class Typed(object):
    """Every prompt the gate submitted, so a ``<USER_REQUEST>`` it did not type is exactly a wake."""

    def __init__(self):
        self.texts = []

    def prompt(self, pane, text):
        self.texts.append(text.strip())
        return pane.prompt(text)

    def submit_draft(self, pane, text):
        """Enter on a draft the gate typed with ``pane.draft``: from now on it counts as typed."""
        self.texts.append(text.strip())
        return pane.keys("Enter")

    def untyped(self, request):
        return untyped_entries(request, self.texts)


def untyped_entries(request, typed):
    """The request's ``<USER_REQUEST>`` entries that contain none of the ``typed`` prompts."""
    return [u for u in (request or {}).get("user_requests") or []
            if not any(t and t in u for t in typed)]


def untyped_count(world, typed):
    """The most untyped entries any main request has carried so far."""
    return max([len(untyped_entries(r, typed.texts)) for r in observe.main_requests(world)] or [0])


def first_wake(world, typed, baseline):
    """The first main request whose untyped entries exceed ``baseline`` (the wake's request), or None."""
    for request in observe.main_requests(world):
        if len(typed.untyped(request)) > baseline:
            return request
    return None


def await_wake(world, typed, baseline, timeout=WAKE_WINDOW):
    return observe.wait_until(lambda: first_wake(world, typed, baseline), timeout)


def quiet_window(world, typed, seconds=QUIET_WINDOW):
    """Watch ``seconds``; ``(main requests added, untyped entries added)`` over it."""
    requests, untyped = len(observe.main_requests(world)), untyped_count(world, typed)
    deadline = time.monotonic() + seconds * observe.TIMEOUT_SCALE
    while time.monotonic() < deadline:
        time.sleep(0.5)
        if len(observe.main_requests(world)) != requests:
            break
    return len(observe.main_requests(world)) - requests, untyped_count(world, typed) - untyped


def frames_of(request):
    """Message ids framed in any of the request's system messages."""
    return _l6.frame_ids_in(_l6.system_texts(request))


def agent_row_live(pane):
    """True while the pane's live region shows an ``Agent(<name>)`` row (the composer is Unreadable)."""
    return any(line.strip().lstrip("●").strip().startswith("Agent(") for line in pane.read_detection().splitlines())


def load(world, name, **variables):
    scenario = _l6.load_scenario(SCEN / (name + ".json"))
    scenario["vars"] = dict(scenario.get("vars", {}), cwd=str(world.ws), **variables)
    world.backend.load(scenario)
    return scenario


await_step = _l5.await_step


def body_json(world, request):
    """The saved request body of ``request`` (None when it was not saved or does not parse)."""
    import json
    rel = (request or {}).get("body")
    try:
        return json.loads((Path(world.ctl) / rel).read_text())
    except (OSError, TypeError, ValueError):
        return None


def body_frames(world, request):
    """Message ids framed anywhere in the request body, including inside a ``functionResponse``.

    A ``PreInvocation`` injection after a tool call rides in the tool result's ``output`` (genuine agy
    1.3.2), where the fake server's ``system_messages`` (text parts only) does not see it."""
    return frame_ids_in_body(body_json(world, request))


def strings_of(node):
    """Every string in a decoded JSON value, keys excluded."""
    if isinstance(node, str):
        yield node
    elif isinstance(node, list):
        for item in node:
            for text in strings_of(item):
                yield text
    elif isinstance(node, dict):
        for item in node.values():
            for text in strings_of(item):
                yield text


def frame_ids_in_body(body):
    return _l6.frame_ids_in(list(strings_of(body)))
