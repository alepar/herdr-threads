"""Hard budget, key reading and redaction for the agy relay (spec D10)."""
from __future__ import annotations

import json
import os
import re
import shlex
import threading

MAX_REQUESTS = 40
MAX_COST_USD = 0.50
MAX_INPUT_BYTES = 400_000
MAX_TOKENS = 1024
PRICE_IN = 0.30e-6
PRICE_OUT = 2.50e-6

_KEY_RE = re.compile(r"sk-or-[A-Za-z0-9_-]+")


def worst_case(input_bytes, max_tokens):
    return input_bytes / 3 * PRICE_IN + max_tokens * PRICE_OUT


def _fsync_dir(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class Budget(object):
    """Per-run request and cost ceiling, persisted to ``DIR/budget.json``."""

    def __init__(self, run_dir, max_requests=MAX_REQUESTS, max_cost_usd=MAX_COST_USD):
        if not isinstance(max_requests, int) or isinstance(max_requests, bool) \
                or max_requests < 0 or max_requests > MAX_REQUESTS:
            raise ValueError("max_requests must be an integer in 0..%d" % MAX_REQUESTS)
        if not isinstance(max_cost_usd, (int, float)) or isinstance(max_cost_usd, bool) \
                or not (0 <= max_cost_usd <= MAX_COST_USD):
            raise ValueError("max_cost_usd must be in 0..%s" % MAX_COST_USD)
        self.run_dir = str(run_dir)
        self.path = os.path.join(self.run_dir, "budget.json")
        self._lock = threading.Lock()
        os.makedirs(self.run_dir, exist_ok=True)
        self._state = {
            "max_requests": max_requests,
            "max_cost_usd": float(max_cost_usd),
            "requests": 0,
            "cost_usd": 0.0,
            "cost_estimated": False,
            "exhausted": False,
            "refusals": 0,
        }
        self._persist()

    def snapshot(self):
        with self._lock:
            return dict(self._state)

    def _persist(self):
        tmp = self.path + ".tmp"
        with open(tmp, "w") as fh:
            json.dump(self._state, fh, indent=2)
            fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, self.path)
        _fsync_dir(self.run_dir)

    def reserve(self, input_bytes, max_tokens):
        """Return (ok, worst, reason). A True result is already persisted."""
        worst = worst_case(input_bytes, min(max_tokens, MAX_TOKENS))
        with self._lock:
            st = self._state
            reason = None
            if st["requests"] >= st["max_requests"]:
                reason = "request ceiling reached"
            elif st["cost_usd"] + worst > st["max_cost_usd"]:
                reason = "cost ceiling would be exceeded"
            if reason is not None:
                st["exhausted"] = True
                st["refusals"] += 1
                self._persist()
                return False, worst, reason
            st["requests"] += 1
            self._persist()
            return True, worst, None

    def settle(self, cost_usd, worst):
        with self._lock:
            st = self._state
            if isinstance(cost_usd, (int, float)) and not isinstance(cost_usd, bool) and cost_usd >= 0:
                st["cost_usd"] += float(cost_usd)
            else:
                st["cost_usd"] += float(worst)
                st["cost_estimated"] = True
            self._persist()


def read_key(pw_dir):
    """Read OPENROUTER_API_KEY from ``<pw_dir>/.hermes/.env`` only (never os.environ)."""
    path = os.path.join(pw_dir, ".hermes", ".env")
    try:
        with open(path) as fh:
            lines = fh.read().splitlines()
    except OSError as exc:
        raise RuntimeError("OpenRouter key file unreadable: %s" % path) from exc
    for line in lines:
        name, sep, value = line.partition("=")
        if not sep:
            continue
        name = name.strip()
        if name.startswith("export "):
            name = name[len("export "):].strip()
        if name != "OPENROUTER_API_KEY":
            continue
        try:
            parsed = shlex.split(value, comments=True)
        except ValueError:
            continue
        key = parsed[0] if len(parsed) == 1 else ""
        if key.startswith("sk-or-"):
            return key
    raise RuntimeError("no usable OPENROUTER_API_KEY line in %s" % path)


def redact(text, root):
    text = _KEY_RE.sub("sk-or-REDACTED", text)
    if root:
        variants = {str(root), os.path.realpath(str(root))}
        for variant in sorted(variants, key=len, reverse=True):
            if variant and variant != "/":
                text = text.replace(variant, "<root>")
    return text
