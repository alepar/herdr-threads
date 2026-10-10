#!/usr/bin/env python3
"""Scripted OpenAI-compatible SSE upstream for relay tests (spec D10).

    fake_upstream.py --ctl DIR [--port 0] [--owner-pid PID]

Writes ``DIR/port``, reads ``DIR/script.json`` on every request:

    {"format": "agy-relay-fake-upstream/1",
     "replies": [{"text": "...", "chunks": 2, "cost": 0.001},
                 {"tool_calls": [{"name": "run_command", "arguments": {...}}]},
                 {"error": {"http": 500, "message": "boom"}}],
     "default": {"text": "done"}}

Replies are consumed in order; ``default`` answers once they run out. Strings
may use ``{frame.id}``, ``{frame.thread}``, ``{frame.kind}`` and
``{frame.sender}``, taken from the last herdr-threads frame header found in the
request's message texts. Every received body is appended to
``DIR/requests.jsonl``.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import signal
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FORMAT = "agy-relay-fake-upstream/1"
# Own copy of the D4 frame header regex (L3 does not import from L1).
HEADER_RE = re.compile(r"^\[herdr-threads\] (\S+) (\S+) in (\S+) from (\S+)(.*):$")
_TEMPLATE_RE = re.compile(r"\{frame\.([a-z]+)\}")
_FRAME_FIELDS = {"kind": 1, "id": 2, "thread": 3, "sender": 4}


class TemplateError(Exception):
    pass


def _message_texts(body):
    for msg in body.get("messages") or []:
        content = msg.get("content") if isinstance(msg, dict) else None
        if isinstance(content, str):
            yield content
        elif isinstance(content, list):
            for part in content:
                if isinstance(part, dict) and isinstance(part.get("text"), str):
                    yield part["text"]


def last_frame(body):
    found = None
    for text in _message_texts(body):
        for line in text.splitlines():
            match = HEADER_RE.match(line)
            if match:
                found = match
    return found


def render(value, frame):
    if isinstance(value, str):
        def sub(m):
            field = m.group(1)
            if field not in _FRAME_FIELDS:
                raise TemplateError("unknown template field frame.%s" % field)
            if frame is None:
                raise TemplateError("no frame header in request for {frame.%s}" % field)
            return frame.group(_FRAME_FIELDS[field])
        return _TEMPLATE_RE.sub(sub, value)
    if isinstance(value, list):
        return [render(v, frame) for v in value]
    if isinstance(value, dict):
        return {k: render(v, frame) for k, v in value.items()}
    return value


def _split(text, n):
    n = max(1, int(n))
    if not text:
        return [""]
    size = max(1, -(-len(text) // n))
    return [text[i:i + size] for i in range(0, len(text), size)]


def _chunk(delta, finish=None, usage=None):
    obj = {"id": "gen-fake", "object": "chat.completion.chunk", "created": 0,
           "model": "google/gemini-2.5-flash",
           "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
    if usage is not None:
        obj["usage"] = usage
    return obj


def build_stream(reply, call_serial):
    """Return the SSE body (bytes) for a text or tool_calls reply."""
    frames = [": OPENROUTER PROCESSING", "", "data: " + json.dumps(_chunk({"role": "assistant", "content": ""})), ""]
    finish = "stop"
    if "tool_calls" in reply:
        finish = "tool_calls"
        for index, call in enumerate(reply["tool_calls"]):
            args = json.dumps(call.get("arguments") or {})
            half = max(1, len(args) // 2)
            cid = "fake_call_%d_%d" % (call_serial, index)
            pieces = [
                {"tool_calls": [{"index": index, "id": cid, "type": "function",
                                 "function": {"name": call["name"], "arguments": ""}}]},
                {"tool_calls": [{"index": index, "function": {"arguments": args[:half]}}]},
                {"tool_calls": [{"index": index, "function": {"arguments": args[half:]}}]},
            ]
            for delta in pieces:
                frames += ["data: " + json.dumps(_chunk(delta)), ""]
    else:
        for piece in _split(reply.get("text", ""), reply.get("chunks", 1)):
            frames += ["data: " + json.dumps(_chunk({"content": piece})), ""]
    usage = {"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110}
    if isinstance(reply.get("cost"), (int, float)):
        usage["cost"] = reply["cost"]
    frames += ["data: " + json.dumps(_chunk({}, finish=finish)), "",
               "data: " + json.dumps({"id": "gen-fake", "object": "chat.completion.chunk", "created": 0,
                                      "model": "google/gemini-2.5-flash", "choices": [], "usage": usage}), "",
               "data: [DONE]", "", ""]
    return "\n".join(frames).encode("utf-8")


class FakeUpstream(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, ctl_dir, port=0):
        self.ctl_dir = str(ctl_dir)
        self.lock = threading.Lock()
        self.served = 0  # replies consumed
        self.received = 0  # bodies received
        super().__init__(("127.0.0.1", port), _Handler)

    def next_reply(self):
        with self.lock:
            script = {}
            path = os.path.join(self.ctl_dir, "script.json")
            if os.path.exists(path):
                with open(path) as fh:
                    script = json.load(fh)
                if script.get("format") != FORMAT:
                    raise ValueError("script.json format must be %r" % FORMAT)
            replies = script.get("replies") or []
            index = self.served
            self.served += 1
            if index < len(replies):
                return replies[index], index
            return script.get("default") or {"text": "done"}, index

    def record(self, raw):
        with self.lock:
            self.received += 1
            with open(os.path.join(self.ctl_dir, "requests.jsonl"), "a") as fh:
                try:
                    fh.write(json.dumps(json.loads(raw.decode("utf-8"))) + "\n")
                except ValueError:
                    fh.write(json.dumps({"_raw": raw.decode("utf-8", "replace")}) + "\n")


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def _send_json(self, status, obj):
        data = json.dumps(obj).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        self.server.record(raw)
        try:
            body = json.loads(raw.decode("utf-8"))
            reply, serial = self.server.next_reply()
            frame = last_frame(body)
            reply = render(reply, frame)
        except TemplateError as exc:
            self._send_json(500, {"error": {"message": "template_error: %s" % exc}})
            return
        except (ValueError, OSError) as exc:
            self._send_json(500, {"error": {"message": "fake upstream: %s" % exc}})
            return
        if "error" in reply:
            err = reply["error"]
            self._send_json(int(err.get("http", 500)), {"error": {"message": err.get("message", "error")}})
            return
        data = build_stream(reply, serial)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)
        self.wfile.flush()


def make_fake_upstream(ctl_dir, port=0):
    os.makedirs(str(ctl_dir), exist_ok=True)
    return FakeUpstream(ctl_dir, port)


def _watch_owner(pid, server):
    while True:
        time.sleep(0.25)
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            break
        except PermissionError:
            continue
    threading.Thread(target=server.shutdown, daemon=True).start()


def main(argv=None):
    ap = argparse.ArgumentParser(description="scripted OpenAI-compatible SSE upstream")
    ap.add_argument("--ctl", required=True)
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--owner-pid", type=int)
    args = ap.parse_args(argv)
    server = make_fake_upstream(args.ctl, args.port)
    tmp = os.path.join(args.ctl, "port.tmp")
    with open(tmp, "w") as fh:
        fh.write(str(server.server_address[1]) + "\n")
    os.replace(tmp, os.path.join(args.ctl, "port"))
    signal.signal(signal.SIGTERM, lambda *_: threading.Thread(target=server.shutdown, daemon=True).start())
    if args.owner_pid:
        threading.Thread(target=_watch_owner, args=(args.owner_pid, server), daemon=True).start()
    print("fake-upstream listening on 127.0.0.1:%d" % server.server_address[1], flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
