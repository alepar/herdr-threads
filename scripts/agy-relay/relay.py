#!/usr/bin/env python3
"""Gemini-native -> OpenAI/OpenRouter relay with an enforced budget (spec D9, D10).

    relay.py --run-dir DIR (--live | --fake-upstream URL) [--max-requests N]
             [--max-cost-usd X] [--port 0] [--owner-pid PID] [--forward-aux]

Run-dir files: port, local-token (0600), budget.json, relay.jsonl.
"""
from __future__ import annotations

import argparse
import gzip
import hmac
import json
import os
import pwd
import secrets
import signal
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import budget as budget_mod  # noqa: E402
import translate  # noqa: E402

LIVE_URL = "https://openrouter.ai/api/v1/chat/completions"
UPSTREAM_MODEL = "google/gemini-2.5-flash"
UPSTREAM_TIMEOUT = 120
TEXT_TRIM = 2048


def platform():
    """Module-level so tests can pretend to be non-darwin."""
    return sys.platform


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(req.full_url, code, "redirect refused", headers, fp)


def _opener():
    # No proxies from the environment, no redirect following.
    return urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())


def build_upstream_body(translated):
    body = dict(translated)
    body.update({
        "model": UPSTREAM_MODEL,
        "stream": True,
        "stream_options": {"include_usage": True},
        "usage": {"include": True},
        "reasoning": {"enabled": False},
        "max_tokens": min(int(translated.get("max_tokens") or budget_mod.MAX_TOKENS), budget_mod.MAX_TOKENS),
        "provider": {"max_price": {"prompt": 0.3, "completion": 2.5}, "require_parameters": True},
    })
    return body


def _trim(text, limit=TEXT_TRIM):
    return text if len(text) <= limit else text[:limit]


class Relay(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, run_dir, upstream_url, budget, token, key=None, forward_aux=False, port=0):
        self.run_dir = str(run_dir)
        self.root = os.path.dirname(os.path.abspath(self.run_dir))
        self.upstream_url = upstream_url
        self.budget = budget
        self.token = token
        self._key = key
        self.forward_aux = forward_aux
        self._log_lock = threading.Lock()
        self._seq = 0
        super().__init__(("127.0.0.1", port), _Handler)

    def auth_header(self):
        return "Bearer " + self._key if self._key else "Bearer fake-upstream"

    def log(self, record):
        with self._log_lock:
            self._seq += 1
            record = dict(record)
            record["seq"] = self._seq
            record["t"] = round(time.time(), 3)
            line = json.dumps(record, ensure_ascii=False)
            if self._key:
                line = line.replace(self._key, "sk-or-REDACTED")
            line = budget_mod.redact(line, self.root)
            with open(os.path.join(self.run_dir, "relay.jsonl"), "a") as fh:
                fh.write(line + "\n")


def make_relay(run_dir, upstream_url, budget, token, key=None, forward_aux=False):
    os.makedirs(str(run_dir), exist_ok=True)
    return Relay(run_dir, upstream_url, budget, token, key=key, forward_aux=forward_aux)


class _Handler(BaseHTTPRequestHandler):
    server_version = "agy-relay"

    def log_message(self, fmt, *args):  # request lines may carry ?key=
        pass

    # -- plumbing ----------------------------------------------------------
    def _authorized(self, query):
        presented = self.headers.get("x-goog-api-key")
        if presented is None:
            presented = (query.get("key") or [None])[0]
        return presented is not None and hmac.compare_digest(
            presented.encode("utf-8"), self.server.token.encode("utf-8"))

    def _json(self, status, obj):
        data = json.dumps(obj).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)

    def _error(self, status, message, grpc=None):
        message = budget_mod.redact(message, self.server.root)
        code, obj = translate.gemini_error(status, message, grpc)
        self._json(code, obj)
        return code

    def _begin_sse(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

    def _write_event(self, event):
        self.wfile.write(b"data: " + json.dumps(event).encode("utf-8") + b"\r\n\r\n")
        self.wfile.flush()

    def _read_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        if (self.headers.get("Content-Encoding") or "").lower() == "gzip":
            raw = gzip.decompress(raw)
        return raw

    # -- routes ------------------------------------------------------------
    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        if not self._authorized(query):
            self._error(403, "invalid local token")
            return
        if "/models" in url.path and ":" not in url.path:
            self.server.log({"path": url.path, "kind": "models", "status": 200, "refused": False})
            self._json(200, translate.models_response())
            return
        self._error(404, "not found")

    def do_POST(self):
        url = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(url.query)
        if not self._authorized(query):
            self._error(403, "invalid local token")
            return
        try:
            raw = self._read_body()
        except (OSError, ValueError, EOFError):
            self._error(400, "unreadable request body")
            return
        action = url.path.rsplit(":", 1)[-1] if ":" in url.path else ""
        model = ""
        if "/models/" in url.path:
            model = url.path.split("/models/", 1)[1].split(":", 1)[0]
        if action == "countTokens":
            self.server.log({"path": url.path, "kind": "count", "status": 200, "model": model,
                             "input_bytes": len(raw), "refused": False})
            self._json(200, translate.count_tokens(raw))
            return
        if action not in ("streamGenerateContent", "generateContent"):
            self._error(404, "not found")
            return
        streaming = action == "streamGenerateContent"
        try:
            body = json.loads(raw.decode("utf-8"))
            if not isinstance(body, dict):
                raise ValueError("body is not an object")
        except ValueError:
            self.server.log({"path": url.path, "kind": "main", "status": 400, "model": model,
                             "input_bytes": len(raw), "refused": False})
            self._error(400, "request body is not valid JSON")
            return
        if translate.is_aux(model) and not self.server.forward_aux:
            event = translate.aux_response()
            self.server.log({"path": url.path, "kind": "aux_local", "status": 200, "model": model,
                             "input_bytes": len(raw), "text_trimmed": "agy smoke", "refused": False})
            if streaming:
                self._begin_sse()
                self._write_event(event)
            else:
                self._json(200, event)
            return
        self._main(url.path, model, body, streaming)

    def _main(self, path, model, body, streaming):
        srv = self.server
        translated = translate.gemini_to_openai(body)
        input_bytes = len(json.dumps(translated, ensure_ascii=False).encode("utf-8"))
        rec = {
            "path": path, "kind": "main", "model": model, "input_bytes": input_bytes,
            "roles": [m.get("role") for m in translated["messages"]],
            "tool_names_count": len(translated.get("tools") or []),
            "tool_calls": [], "text_trimmed": "", "usage": None, "cost_usd": None,
            "cost_estimated": False, "upstream_status": None, "refused": False,
        }
        if input_bytes > budget_mod.MAX_INPUT_BYTES:
            rec["status"] = 400
            srv.log(rec)
            self._error(400, "agy-relay input too large", "INVALID_ARGUMENT")
            return
        ok, worst, _reason = srv.budget.reserve(input_bytes, translated["max_tokens"])
        if not ok:
            rec["refused"] = True
            rec["status"] = 429
            srv.log(rec)
            self._error(429, "agy-relay budget exhausted", "RESOURCE_EXHAUSTED")
            return
        # From here on the request counts; it is settled and logged exactly once, and always
        # before the last byte of the response is written.
        done = []

        def finalize(status, usage=None):
            if done:
                return
            done.append(True)
            cost = usage.get("cost") if isinstance(usage, dict) else None
            srv.budget.settle(cost, worst)
            rec["status"] = status
            rec["usage"] = usage
            rec["cost_usd"] = cost if isinstance(cost, (int, float)) else worst
            rec["cost_estimated"] = not isinstance(cost, (int, float))
            rec["text_trimmed"] = _trim("".join(text_parts))
            srv.log(rec)

        text_parts = []

        def note(ev):
            for cand in ev.get("candidates") or []:
                for part in (cand.get("content") or {}).get("parts") or []:
                    if part.get("text"):
                        text_parts.append(part["text"])
                    if "functionCall" in part:
                        fc = part["functionCall"]
                        rec["tool_calls"].append({
                            "name": fc.get("name"),
                            "args_trimmed": _trim(json.dumps(fc.get("args", {}), ensure_ascii=False), 512)})

        try:
            req = urllib.request.Request(
                srv.upstream_url,
                data=json.dumps(build_upstream_body(translated)).encode("utf-8"),
                headers={"Content-Type": "application/json", "Accept": "text/event-stream",
                         "Authorization": srv.auth_header()},
                method="POST")
            try:
                resp = _opener().open(req, timeout=UPSTREAM_TIMEOUT)
            except urllib.error.HTTPError as exc:
                rec["upstream_status"] = exc.code
                try:
                    err_body = exc.read()
                except OSError:
                    err_body = b""
                status, obj = translate.map_error(exc.code, err_body)
                obj["error"]["message"] = budget_mod.redact(obj["error"]["message"], srv.root)
                finalize(status)
                self._json(status, obj)
                return
            except (urllib.error.URLError, OSError) as exc:
                finalize(502)
                self._error(502, "upstream unreachable: %s" % type(exc).__name__, "UNAVAILABLE")
                return
            rec["upstream_status"] = resp.status
            tr = translate.StreamTranslator()
            events = []
            try:
                if streaming:
                    self._begin_sse()
                for item in translate.parse_sse(resp):
                    if item == "[DONE]":
                        break
                    if "error" in item and not item.get("choices"):
                        rec["upstream_error"] = _trim(str(item["error"]), 256)
                        break
                    for ev in tr.feed(item):
                        note(ev)
                        events.append(ev)
                        if streaming:
                            self._write_event(ev)
                final = tr.finish()
                for ev in final:
                    note(ev)
                    events.append(ev)
                finalize(200, tr.usage())
                if streaming:
                    for ev in final:
                        self._write_event(ev)
                else:
                    self._json(200, translate.aggregate(events))
            except (BrokenPipeError, ConnectionResetError):
                rec["client_gone"] = True
            except (OSError, ValueError) as exc:
                rec["stream_error"] = type(exc).__name__
            finally:
                resp.close()
                finalize(200, tr.usage())
        finally:
            finalize(500)



# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _die(message):
    sys.stderr.write("agy-relay: %s\n" % message)
    sys.exit(2)


def _loopback_url(url):
    parsed = urllib.parse.urlparse(url)
    return parsed.scheme == "http" and parsed.hostname in ("127.0.0.1", "localhost") \
        and parsed.port is not None


def _write_private(path, text):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.fchmod(fd, 0o600)
        os.write(fd, text.encode("utf-8"))
    finally:
        os.close(fd)


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
    ap = argparse.ArgumentParser(prog="relay.py", description="agy Gemini->OpenAI relay")
    ap.add_argument("--run-dir", required=True)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--live", action="store_true")
    mode.add_argument("--fake-upstream")
    ap.add_argument("--max-requests", type=int, default=budget_mod.MAX_REQUESTS)
    ap.add_argument("--max-cost-usd", type=float, default=budget_mod.MAX_COST_USD)
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--owner-pid", type=int)
    ap.add_argument("--forward-aux", action="store_true")
    args = ap.parse_args(argv)

    if args.live:
        if os.environ.get("CI") or os.environ.get("GITHUB_ACTIONS"):
            _die("--live refuses to run under CI")
        if platform() != "darwin":
            _die("--live is only supported on darwin")
    elif not _loopback_url(args.fake_upstream):
        _die("--fake-upstream must be an http://127.0.0.1:PORT or http://localhost:PORT URL")
    if not (0 <= args.max_requests <= budget_mod.MAX_REQUESTS):
        _die("--max-requests must be 0..%d" % budget_mod.MAX_REQUESTS)
    if not (0 <= args.max_cost_usd <= budget_mod.MAX_COST_USD):
        _die("--max-cost-usd must be 0..%s" % budget_mod.MAX_COST_USD)

    key = None
    if args.live:
        try:
            key = budget_mod.read_key(pwd.getpwuid(os.getuid()).pw_dir)
        except RuntimeError as exc:
            _die(str(exc))
        upstream = LIVE_URL
    else:
        upstream = args.fake_upstream
        if urllib.parse.urlparse(upstream).path in ("", "/"):
            upstream = upstream.rstrip("/") + "/api/v1/chat/completions"

    run_dir = os.path.abspath(args.run_dir)
    os.makedirs(run_dir, exist_ok=True)
    try:
        bud = budget_mod.Budget(run_dir, args.max_requests, args.max_cost_usd)
    except ValueError as exc:
        _die(str(exc))
    token = secrets.token_hex(32)
    server = Relay(run_dir, upstream, bud, token, key=key, forward_aux=args.forward_aux, port=args.port)
    _write_private(os.path.join(run_dir, "local-token"), token + "\n")
    _write_private(os.path.join(run_dir, "port.tmp"), "%d\n" % server.server_address[1])
    os.replace(os.path.join(run_dir, "port.tmp"), os.path.join(run_dir, "port"))
    signal.signal(signal.SIGTERM, lambda *_: threading.Thread(target=server.shutdown, daemon=True).start())
    if args.owner_pid:
        threading.Thread(target=_watch_owner, args=(args.owner_pid, server), daemon=True).start()
    print("agy-relay listening on 127.0.0.1:%d" % server.server_address[1], flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
