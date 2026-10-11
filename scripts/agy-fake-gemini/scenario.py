"""Scenario format ``agy-fake-gemini/1``: validation, matching, templates and
request-body helpers for the fake Gemini server (test-infra spec D1/D2).

Pure and stdlib-only (Python 3.9 compatible). The only I/O is ``load``.
"""
from __future__ import annotations

import copy
import json
import re

FORMAT = "agy-fake-gemini/1"

# The D4 delivery header line (delivery-policy spec DP2).
# The thread and sender ids may be followed by their quoted names (delivery/frame.rs ``label``); the
# names are skipped, so group 3 and 4 stay the ids and group 5 the marker tail.
_NAME = r'(?: "(?:[^"\\]|\\.)*")?'
FRAME_HEADER = re.compile(r'^\[herdr-threads\] (\S+) (\S+) in (\S+)' + _NAME + r' from (\S+)' + _NAME + r'(.*):$', re.M)

FRAME_KEYS = ("id", "thread", "kind", "sender")
_PLACEHOLDER = re.compile(r"\{(var|frame)\.([A-Za-z0-9_]+)\}")
_HOOK_PREFIX = "The following system message was injected by a PreInvocation hook:\n"
_SYSTEM_MESSAGE = re.compile(r"<SYSTEM_MESSAGE>\n(.*?)\n</SYSTEM_MESSAGE>", re.S)
_USER_REQUEST = re.compile(r"<USER_REQUEST>\n(.*?)\n</USER_REQUEST>", re.S)

DEFAULT_REPLY = {"text": "DONE-default"}
DEFAULT_AUX_TEXT = "Fake title"


class ScenarioError(ValueError):
    """A scenario violates the format; the message starts with a JSON path."""


def _fail(path, msg):
    raise ScenarioError("%s: %s" % (path, msg))


def _is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def _expect_dict(v, path):
    if not isinstance(v, dict):
        _fail(path, "expected an object")
    return v


def _check_keys(obj, allowed, path):
    for key in obj:
        if key not in allowed:
            _fail("%s.%s" % (path, key), "unknown key")


def _str_list(v, path):
    if not isinstance(v, list):
        _fail(path, "expected a list of strings")
    for i, item in enumerate(v):
        if not isinstance(item, str):
            _fail("%s[%d]" % (path, i), "expected a string")
    return list(v)


def _validate_call(obj, path, allow_extra=()):
    allowed = ("call", "args", "id") + tuple(allow_extra)
    _check_keys(obj, allowed, path)
    if not isinstance(obj.get("call"), str):
        _fail(path + ".call", "expected a string")
    args = obj.get("args", {})
    if not isinstance(args, dict):
        _fail(path + ".args", "expected an object")
    out = {"call": obj["call"], "args": copy.deepcopy(args)}
    if "id" in obj:
        if not isinstance(obj["id"], str):
            _fail(path + ".id", "expected a string")
        out["id"] = obj["id"]
    return out


def _validate_reply(r, path):
    _expect_dict(r, path)
    _check_keys(r, ("text", "call", "calls", "error", "args", "id", "finish",
                    "chunks", "chunk_interval_ms"), path)
    kinds = [k for k in ("text", "call", "calls", "error") if k in r]
    if len(kinds) != 1:
        _fail(path, "exactly one of text/call/calls/error is required (got %s)"
              % (", ".join(kinds) if kinds else "none"))
    kind = kinds[0]
    out = {}
    if kind == "text":
        if not isinstance(r["text"], str):
            _fail(path + ".text", "expected a string")
        out["text"] = r["text"]
    elif kind == "call":
        out.update(_validate_call(r, path, allow_extra=("finish", "chunks", "chunk_interval_ms")))
    elif kind == "calls":
        calls = r["calls"]
        if not isinstance(calls, list) or not calls:
            _fail(path + ".calls", "expected a non-empty list")
        out["calls"] = [_validate_call(_expect_dict(c, "%s.calls[%d]" % (path, i)),
                                       "%s.calls[%d]" % (path, i))
                        for i, c in enumerate(calls)]
    else:
        err = _expect_dict(r["error"], path + ".error")
        _check_keys(err, ("http", "status", "message"), path + ".error")
        for key in ("http", "status", "message"):
            if key not in err:
                _fail(path + ".error." + key, "required")
        if not _is_int(err["http"]) or not 400 <= err["http"] <= 599:
            _fail(path + ".error.http", "expected an integer 400-599")
        for key in ("status", "message"):
            if not isinstance(err[key], str):
                _fail(path + ".error." + key, "expected a string")
        out["error"] = dict(err)
    if kind not in ("call",):
        for key in ("args", "id"):
            if key in r:
                _fail("%s.%s" % (path, key), "only valid with a single call reply")
    finish = r.get("finish", "STOP")
    if not isinstance(finish, str):
        _fail(path + ".finish", "expected a string")
    out["finish"] = finish
    if "chunks" in r:
        if kind != "text":
            _fail(path + ".chunks", "only valid with a text reply")
        if not _is_int(r["chunks"]) or r["chunks"] < 1:
            _fail(path + ".chunks", "expected an integer >= 1")
        out["chunks"] = r["chunks"]
    if "chunk_interval_ms" in r:
        if not _is_int(r["chunk_interval_ms"]) or r["chunk_interval_ms"] < 0:
            _fail(path + ".chunk_interval_ms", "expected an integer >= 0")
        out["chunk_interval_ms"] = r["chunk_interval_ms"]
    return out


def _validate_step(step, path, seen):
    _expect_dict(step, path)
    _check_keys(step, ("label", "match", "delay_ms", "bind", "reply"), path)
    label = step.get("label")
    if not isinstance(label, str) or not label:
        _fail(path + ".label", "required non-empty string")
    if label in seen:
        _fail(path + ".label", "duplicate label %r" % label)
    seen.add(label)
    match = _expect_dict(step.get("match", {}), path + ".match")
    _check_keys(match, ("contains", "not_contains"), path + ".match")
    out = {
        "label": label,
        "match": {
            "contains": _str_list(match.get("contains", []), path + ".match.contains"),
            "not_contains": _str_list(match.get("not_contains", []), path + ".match.not_contains"),
        },
    }
    delay = step.get("delay_ms", 0)
    if not _is_int(delay) or delay < 0:
        _fail(path + ".delay_ms", "expected an integer >= 0")
    out["delay_ms"] = delay
    if "bind" in step:
        bind = _expect_dict(step["bind"], path + ".bind")
        _check_keys(bind, ("marker", "index"), path + ".bind")
        if len(bind) != 1:
            _fail(path + ".bind", "exactly one of marker/index is required")
        if "marker" in bind and not isinstance(bind["marker"], str):
            _fail(path + ".bind.marker", "expected a string")
        if "index" in bind and not _is_int(bind["index"]):
            _fail(path + ".bind.index", "expected an integer")
        out["bind"] = dict(bind)
    else:
        out["bind"] = None
    if "reply" not in step:
        _fail(path + ".reply", "required")
    out["reply"] = _validate_reply(step["reply"], path + ".reply")
    return out


def validate(obj):
    """Return a normalized copy of a scenario or raise ``ScenarioError``."""
    _expect_dict(obj, "$")
    _check_keys(obj, ("format", "name", "aux_text", "default", "vars", "steps"), "$")
    if obj.get("format") != FORMAT:
        _fail("$.format", "must equal %r" % FORMAT)
    if not isinstance(obj.get("name"), str):
        _fail("$.name", "required string")
    aux = obj.get("aux_text", DEFAULT_AUX_TEXT)
    if not isinstance(aux, str):
        _fail("$.aux_text", "expected a string")
    default = _validate_reply(obj.get("default", copy.deepcopy(DEFAULT_REPLY)), "$.default")
    variables = _expect_dict(obj.get("vars", {}), "$.vars")
    for key, value in variables.items():
        if not isinstance(value, str):
            _fail("$.vars.%s" % key, "expected a string")
    if "steps" not in obj:
        _fail("$.steps", "required")
    steps = obj["steps"]
    if not isinstance(steps, list):
        _fail("$.steps", "expected a list")
    seen = set()
    return {
        "format": FORMAT,
        "name": obj["name"],
        "aux_text": aux,
        "default": default,
        "vars": dict(variables),
        "steps": [_validate_step(s, "$.steps[%d]" % i, seen) for i, s in enumerate(steps)],
    }


def load(path):
    """Read ``path`` as JSON and validate it."""
    with open(path, "r", encoding="utf-8") as fh:
        try:
            obj = json.load(fh)
        except ValueError as exc:
            raise ScenarioError("$: not valid JSON (%s)" % exc)
    return validate(obj)


def empty_scenario():
    """The scenario used when no ``scenario.json`` exists."""
    return validate({"format": FORMAT, "name": "", "steps": []})


class Cursor:
    """Consumption state of one scenario. Callers serialize access."""

    def __init__(self, scenario):
        self.scenario = scenario
        self._consumed = []
        self.unmatched = 0

    def select(self, texts):
        """First unconsumed step whose ``match`` holds, marked consumed; ``None`` = default."""
        for step in self.scenario["steps"]:
            if step["label"] in self._consumed:
                continue
            match = step["match"]
            if all(s in texts for s in match["contains"]) and not any(
                    s in texts for s in match["not_contains"]):
                self._consumed.append(step["label"])
                return step
        self.unmatched += 1
        return None

    def consumed_labels(self):
        return list(self._consumed)

    def remaining_labels(self):
        return [s["label"] for s in self.scenario["steps"] if s["label"] not in self._consumed]


# ---------------------------------------------------------------- frames, templates

def frames(text):
    """Every D4 delivery header in ``text`` (marker is the header's tail)."""
    return [{"kind": m.group(1), "id": m.group(2), "thread": m.group(3),
             "sender": m.group(4), "marker": m.group(5)}
            for m in FRAME_HEADER.finditer(text)]


def bind_frame(found, bind):
    """Pick the frame a step binds to; ``None`` when there is none."""
    if not found:
        return None
    if not bind:
        return found[-1]
    if "marker" in bind:
        for frame in found:
            if bind["marker"] in frame["marker"]:
                return frame
        return None
    idx = bind["index"]
    if -len(found) <= idx < len(found):
        return found[idx]
    return None


def render(reply, variables, frame):
    """Substitute ``{var.*}``/``{frame.*}`` in every string of ``reply``.

    Returns ``(reply, None)`` or, for an unresolved placeholder,
    ``({"text": "FAKE-GEMINI-TEMPLATE-ERROR <placeholder>"}, <placeholder>)``.
    """
    bad = []

    def sub(match):
        ns, name = match.group(1), match.group(2)
        if ns == "var" and name in variables:
            return variables[name]
        if ns == "frame" and frame is not None and name in FRAME_KEYS:
            return frame[name]
        bad.append(match.group(0))
        return match.group(0)

    def walk(node):
        if isinstance(node, str):
            return _PLACEHOLDER.sub(sub, node)
        if isinstance(node, list):
            return [walk(x) for x in node]
        if isinstance(node, dict):
            return {k: walk(v) for k, v in node.items()}
        return node

    out = walk(reply)
    if bad:
        return {"text": "FAKE-GEMINI-TEMPLATE-ERROR " + bad[0]}, bad[0]
    return out, None


# ---------------------------------------------------------------- request-body helpers

def _content_parts(body):
    contents = body.get("contents") if isinstance(body, dict) else None
    for content in contents if isinstance(contents, list) else []:
        parts = content.get("parts") if isinstance(content, dict) else None
        for part in parts if isinstance(parts, list) else []:
            if isinstance(part, dict):
                yield part


def _content_texts(body):
    return [p["text"] for p in _content_parts(body) if isinstance(p.get("text"), str)]


def request_texts(body):
    """Text of every content part joined with newlines (never ``systemInstruction``)."""
    return "\n".join(_content_texts(body))


def roles(body):
    contents = body.get("contents") if isinstance(body, dict) else None
    return [c.get("role") for c in contents if isinstance(c, dict)] if isinstance(contents, list) else []


def system_messages(body):
    out = []
    for text in _content_texts(body):
        for m in _SYSTEM_MESSAGE.finditer(text):
            inner = m.group(1)
            if inner.startswith(_HOOK_PREFIX):
                inner = inner[len(_HOOK_PREFIX):]
            out.append(inner)
    return out


def user_requests(body):
    return [m.group(1) for text in _content_texts(body) for m in _USER_REQUEST.finditer(text)]


def _named(body, key):
    out = []
    for part in _content_parts(body):
        item = part.get(key)
        if isinstance(item, dict):
            out.append({"name": item.get("name"), "id": item.get("id")})
    return out


def function_calls(body):
    return _named(body, "functionCall")


def function_responses(body):
    return _named(body, "functionResponse")
