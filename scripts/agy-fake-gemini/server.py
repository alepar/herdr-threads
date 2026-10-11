#!/usr/bin/env python3
"""Scripted Gemini-native HTTP server for the agy test infrastructure (spec D2).

    server.py --ctl DIR [--port 0] [--host 127.0.0.1] [--owner-pid PID]

Control files live in DIR: ``scenario.json`` (read, hot-swappable),
``requests.jsonl`` and ``bodies/`` (request log), ``cursor.json``, ``port`` and
``ready``. Python 3 stdlib only, 3.9 compatible, loopback bind only.
"""
from __future__ import annotations

import argparse
import http.server
import json
import os
import signal
import sys
import threading
import time
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import scenario as sc  # noqa: E402

BODY_CAP = 1024 * 1024
LOOPBACK = "127.0.0.1"
USAGE = {"promptTokenCount": 32, "candidatesTokenCount": 2, "totalTokenCount": 34}
MODELS = {"models": [{"name": "models/gemini-3.8-flash",
                      "supportedGenerationMethods": ["generateContent"]}]}


def _atomic_write(path, text):
    tmp = "%s.tmp.%d.%d" % (path, os.getpid(), threading.get_ident())
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write(text)
    os.replace(tmp, path)


def _split_text(text, n):
    """``n`` nearly equal non-empty pieces (fewer when the text is shorter)."""
    n = max(1, min(n, len(text))) if text else 1
    base, extra = divmod(len(text), n)
    out, pos = [], 0
    for i in range(n):
        size = base + (1 if i < extra else 0)
        out.append(text[pos:pos + size])
        pos += size
    return out


def _call_part(call):
    return {"functionCall": {"name": call["call"], "args": call.get("args", {}),
                             "id": call.get("id") or str(uuid.uuid4())}}


def _envelope(parts, finish=None):
    cand = {"content": {"role": "model", "parts": parts}, "index": 0}
    out = {"candidates": [cand], "modelVersion": "fake"}
    if finish is not None:
        cand["finishReason"] = finish
        out["usageMetadata"] = dict(USAGE)
    return out


def reply_events(reply):
    """The response objects for a (rendered) non-error reply, one per SSE event."""
    finish = reply.get("finish", "STOP")
    if "text" in reply:
        pieces = _split_text(reply["text"], reply.get("chunks", 1))
        return [_envelope([{"text": p}], finish if i == len(pieces) - 1 else None)
                for i, p in enumerate(pieces)]
    if "call" in reply:
        return [_envelope([_call_part(reply)], finish)]
    return [_envelope([_call_part(c) for c in reply["calls"]], finish)]


def merge_events(events):
    """One JSON object for the non-streaming route."""
    if len(events) == 1:
        return events[0]
    parts = []
    for ev in events:
        parts.extend(ev["candidates"][0]["content"]["parts"])
    merged = _merge_text(parts)
    return _envelope(merged, events[-1]["candidates"][0]["finishReason"])


def _merge_text(parts):
    text = "".join(p["text"] for p in parts if "text" in p)
    return [{"text": text}]


class State:
    """Everything the handlers share; ``lock`` serializes cursor, seq and log."""

    def __init__(self, ctl_dir):
        self.ctl = os.path.abspath(ctl_dir)
        os.makedirs(os.path.join(self.ctl, "bodies"), exist_ok=True)
        self.lock = threading.Lock()
        self.seq = 0
        self.sig = "unset"
        self.scenario_error = None
        self.cursor = sc.Cursor(sc.empty_scenario())

    def _refresh(self):
        """Reload ``scenario.json`` when its (inode, mtime) changed; reset the cursor."""
        path = os.path.join(self.ctl, "scenario.json")
        try:
            fh = open(path, "rb")
        except OSError:
            sig, raw = None, None
        else:
            with fh:
                st = os.fstat(fh.fileno())
                sig, raw = (st.st_ino, st.st_mtime_ns), fh.read()
        if sig == self.sig:
            return
        self.sig = sig
        self.scenario_error = None
        if raw is None:
            self.cursor = sc.Cursor(sc.empty_scenario())
            return
        try:
            try:
                obj = json.loads(raw.decode("utf-8"))
            except ValueError as exc:
                raise sc.ScenarioError("$: not valid JSON (%s)" % exc)
            self.cursor = sc.Cursor(sc.validate(obj))
        except sc.ScenarioError as exc:
            self.scenario_error = str(exc)
            self.cursor = sc.Cursor(sc.empty_scenario())

    def _write_cursor(self):
        cur = self.cursor
        _atomic_write(os.path.join(self.ctl, "cursor.json"), json.dumps({
            "scenario": cur.scenario["name"] or None,
            "consumed": cur.consumed_labels(),
            "remaining": cur.remaining_labels(),
            "unmatched": cur.unmatched,
        }))

    def handle(self, kind, path, model, raw):
        """Log an arriving request and decide its reply.

        Returns ``(plan, step)``; ``plan`` is a dict with ``reply`` (rendered) and
        ``delay_ms``, or ``error`` for an HTTP error to send.
        """
        try:
            body = json.loads(raw.decode("utf-8")) if raw else {}
        except ValueError:
            body = {}
        if not isinstance(body, dict):
            body = {}
        with self.lock:
            self.seq += 1
            seq = self.seq
            record = {"seq": seq, "t": time.time(), "path": path, "model": model,
                      "kind": kind, "step": None, "unmatched": False,
                      "template_error": None}
            plan = {"reply": None, "delay_ms": 0, "error": None}
            if kind in ("main", "aux"):
                self._refresh()
            if kind == "main":
                if self.scenario_error is not None:
                    record["scenario_error"] = self.scenario_error
                    plan["error"] = {"http": 500, "status": "INTERNAL",
                                     "message": "FAKE-GEMINI-SCENARIO-ERROR " + self.scenario_error}
                else:
                    texts = sc.request_texts(body)
                    step = self.cursor.select(texts)
                    scn = self.cursor.scenario
                    reply = scn["default"] if step is None else step["reply"]
                    frame = sc.bind_frame(sc.frames(texts), step["bind"] if step else None)
                    reply, bad = sc.render(reply, scn["vars"], frame)
                    record["step"] = step["label"] if step else None
                    record["unmatched"] = step is None
                    record["template_error"] = bad
                    plan["delay_ms"] = step["delay_ms"] if step else 0
                    if "error" in reply:
                        plan["error"] = reply["error"]
                    else:
                        plan["reply"] = reply
            elif kind == "aux":
                if self.scenario_error is not None:
                    record["scenario_error"] = self.scenario_error
                plan["reply"] = {"text": self.cursor.scenario["aux_text"]}
            if kind in ("main", "aux", "count"):
                record["roles"] = sc.roles(body)
                record["system_messages"] = sc.system_messages(body)
                record["user_requests"] = sc.user_requests(body)
                record["function_calls"] = sc.function_calls(body)
                record["function_responses"] = sc.function_responses(body)
            else:
                record.update({"roles": [], "system_messages": [], "user_requests": [],
                               "function_calls": [], "function_responses": []})
            if kind == "models":
                record["body"] = None
            else:
                rel = "bodies/%06d.json" % seq
                with open(os.path.join(self.ctl, rel), "wb") as fh:
                    fh.write(raw[:BODY_CAP])
                record["body"] = rel
                if len(raw) > BODY_CAP:
                    record["body_truncated"] = True
            with open(os.path.join(self.ctl, "requests.jsonl"), "a", encoding="utf-8") as fh:
                fh.write(json.dumps(record) + "\n")
                fh.flush()
            if kind == "main":
                self._write_cursor()
        return plan


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    # -- plumbing
    def _json(self, status, obj):
        data = json.dumps(obj).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)
        self.close_connection = True

    def _error(self, err):
        self._json(err["http"], {"error": {"code": err["http"], "message": err["message"],
                                           "status": err["status"]}})

    def _stream(self, events, interval_ms):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        for i, ev in enumerate(events):
            if i and interval_ms:
                time.sleep(interval_ms / 1000.0)
            self.wfile.write(b"data: " + json.dumps(ev).encode("utf-8") + b"\r\n\r\n")
            self.wfile.flush()

    @staticmethod
    def _model(path):
        if "/models/" not in path:
            return None
        return path.split("/models/", 1)[1].split(":", 1)[0] or None

    # -- verbs
    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if not path.startswith("/v1beta/models"):
            return self._json(404, {"error": {"code": 404, "message": "not found",
                                              "status": "NOT_FOUND"}})
        self.server.state.handle("models", path, None, b"")
        self._json(200, MODELS)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length > 0 else b""
        path = self.path.split("?", 1)[0]
        state = self.server.state
        model = self._model(path)
        if path.endswith(":countTokens"):
            state.handle("count", path, model, raw)
            return self._json(200, {"totalTokens": 32})
        stream = path.endswith(":streamGenerateContent")
        if not (stream or path.endswith(":generateContent")):
            return self._json(404, {"error": {"code": 404, "message": "not found",
                                              "status": "NOT_FOUND"}})
        kind = "aux" if model and "flash-lite" in model else "main"
        plan = state.handle(kind, path, model, raw)
        if plan["delay_ms"]:
            time.sleep(plan["delay_ms"] / 1000.0)
        if plan["error"]:
            return self._error(plan["error"])
        reply = plan["reply"]
        events = reply_events(reply)
        if stream:
            self._stream(events, reply.get("chunk_interval_ms", 0))
        else:
            self._json(200, merge_events(events))


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def handle_error(self, request, client_address):
        if isinstance(sys.exc_info()[1], (ConnectionError, BrokenPipeError)):
            return
        super().handle_error(request, client_address)


def make_server(ctl_dir, port=0):
    """A server bound to loopback; ``serve_forever`` is the caller's job."""
    state = State(ctl_dir)
    srv = Server((LOOPBACK, port), Handler)
    srv.state = state
    return srv


def _pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ctl", required=True)
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--host", default=LOOPBACK)
    ap.add_argument("--owner-pid", type=int, default=None)
    args = ap.parse_args(argv)
    if args.host != LOOPBACK:
        sys.stderr.write("agy-fake-gemini: refusing --host %s (loopback %s only)\n"
                         % (args.host, LOOPBACK))
        return 2
    srv = make_server(args.ctl, args.port)
    ctl = srv.state.ctl
    for name in ("ready", "port"):
        try:
            os.unlink(os.path.join(ctl, name))
        except OSError:
            pass
    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    _atomic_write(os.path.join(ctl, "port"), str(srv.server_port))
    thread = threading.Thread(target=srv.serve_forever, args=(0.05,), daemon=True)
    thread.start()
    with open(os.path.join(ctl, "ready"), "w"):
        pass
    while not stop.wait(0.1):
        if args.owner_pid is not None and not _pid_alive(args.owner_pid):
            break
    srv.shutdown()
    srv.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
