"""Offline tests for budget.py, relay.py and fake_upstream.py.

Everything runs against 127.0.0.1 only; no key and no network are needed.
"""
from __future__ import annotations

import contextlib
import http.client
import io
import json
import os
import pwd
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import budget as budget_mod  # noqa: E402
import fake_upstream  # noqa: E402
import relay  # noqa: E402
import translate  # noqa: E402

RELAY_PY = os.path.join(HERE, "relay.py")
MAIN_PATH = "/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse"
GEN_PATH = "/v1beta/models/gemini-3.8-flash:generateContent"
AUX_PATH = "/v1beta/models/gemini-3.1-flash-lite-preview:streamGenerateContent?alt=sse"
TOKEN = "t" * 64
FAKE_KEY = "sk-or-v1-abc123"


def user(text):
    return {"role": "user", "parts": [{"text": text}]}


def simple_body(text="hello", **extra):
    body = {"contents": [user(text)], "generationConfig": {"maxOutputTokens": 65536}}
    body.update(extra)
    return body


def sse_events(data):
    events = []
    for block in data.split(b"\r\n\r\n"):
        block = block.strip()
        if block.startswith(b"data:"):
            events.append(json.loads(block[5:].decode("utf-8")))
    return events


def event_text(events):
    return "".join(p.get("text", "") for e in events for c in e.get("candidates", [])
                   for p in c.get("content", {}).get("parts", []))


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="agyrelay-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.ctl = os.path.join(self.tmp, "ctl")
        self.run_dir = os.path.join(self.tmp, "run")

    def serve(self, server):
        t = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        t.start()

        def stop():
            server.shutdown()
            server.server_close()
            t.join(5)
        self.addCleanup(stop)
        return server

    def start_fake(self, replies=None, default=None):
        self.fake = self.serve(fake_upstream.make_fake_upstream(self.ctl))
        self.set_script(replies, default)
        self.upstream_url = "http://127.0.0.1:%d/api/v1/chat/completions" % self.fake.server_address[1]

    def set_script(self, replies=None, default=None):
        script = {"format": fake_upstream.FORMAT, "replies": replies or []}
        if default is not None:
            script["default"] = default
        with open(os.path.join(self.ctl, "script.json"), "w") as fh:
            json.dump(script, fh)

    def start_relay(self, upstream_url=None, key=None, forward_aux=False, **budget_kw):
        self.budget = budget_mod.Budget(self.run_dir, **budget_kw)
        self.relay = self.serve(relay.make_relay(self.run_dir, upstream_url or self.upstream_url,
                                                 self.budget, TOKEN, key=key, forward_aux=forward_aux))
        self.port = self.relay.server_address[1]

    def request(self, method, path, body=None, headers=None, token=True, raw=None):
        hdrs = dict(headers or {})
        if token is True:
            hdrs.setdefault("x-goog-api-key", TOKEN)
        elif token:
            hdrs["x-goog-api-key"] = token
        data = raw if raw is not None else (json.dumps(body).encode("utf-8") if body is not None else None)
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=15)
        try:
            conn.request(method, path, body=data, headers=hdrs)
            resp = conn.getresponse()
            return resp.status, dict(resp.getheaders()), resp.read()
        finally:
            conn.close()

    def post(self, path, body=None, **kw):
        return self.request("POST", path, body, **kw)

    def budget_json(self):
        with open(os.path.join(self.run_dir, "budget.json")) as fh:
            return json.load(fh)

    def upstream_requests(self):
        path = os.path.join(self.ctl, "requests.jsonl")
        if not os.path.exists(path):
            return []
        with open(path) as fh:
            return [json.loads(line) for line in fh if line.strip()]

    def relay_log(self):
        with open(os.path.join(self.run_dir, "relay.jsonl")) as fh:
            return [json.loads(line) for line in fh if line.strip()]


class BudgetUnit(Base):
    def test_forty_reservations_then_refused(self):
        b = budget_mod.Budget(self.run_dir)
        for i in range(40):
            ok, worst, reason = b.reserve(100, 1024)
            self.assertTrue(ok, "reservation %d" % (i + 1))
            self.assertIsNone(reason)
            b.settle(0.0001, worst)
        ok, _, reason = b.reserve(100, 1024)
        self.assertFalse(ok)
        self.assertTrue(reason)
        st = self.budget_json()
        self.assertEqual((st["requests"], st["exhausted"], st["refusals"]), (40, True, 1))

    def test_cost_ceiling_refuses_when_worst_case_would_pass(self):
        b = budget_mod.Budget(self.run_dir)
        ok, worst, _ = b.reserve(400_000, 1024)
        self.assertTrue(ok)
        b.settle(0.48, worst)
        ok, worst2, reason = b.reserve(400_000, 1024)
        self.assertGreater(0.48 + worst2, 0.50)
        self.assertFalse(ok)
        self.assertIn("cost", reason)
        st = self.budget_json()
        self.assertEqual(st["requests"], 1)
        self.assertTrue(st["exhausted"])
        # a small request that still fits is accepted
        ok, _, _ = b.reserve(0, 1)
        self.assertTrue(ok)

    def test_worst_case_formula(self):
        self.assertAlmostEqual(budget_mod.worst_case(300, 1000), 100 * 0.30e-6 + 1000 * 2.50e-6)

    def test_budget_json_persisted_before_reserve_returns(self):
        b = budget_mod.Budget(self.run_dir)
        self.assertEqual(self.budget_json()["requests"], 0)
        ok, worst, _ = b.reserve(10, 10)
        self.assertTrue(ok)
        st = self.budget_json()  # read before settle
        self.assertEqual(st["requests"], 1)
        self.assertEqual(st["cost_usd"], 0.0)
        self.assertEqual(sorted(st), sorted(["max_requests", "max_cost_usd", "requests", "cost_usd",
                                             "cost_estimated", "exhausted", "refusals",
                                             "reserved_usd"]))
        self.assertFalse(os.path.exists(os.path.join(self.run_dir, "budget.json.tmp")))

    def test_settle_with_and_without_cost(self):
        b = budget_mod.Budget(self.run_dir)
        _, worst, _ = b.reserve(300, 100)
        b.settle(0.002, worst)
        st = self.budget_json()
        self.assertAlmostEqual(st["cost_usd"], 0.002)
        self.assertFalse(st["cost_estimated"])
        _, worst, _ = b.reserve(300, 100)
        b.settle(None, worst)
        st = self.budget_json()
        self.assertAlmostEqual(st["cost_usd"], 0.002 + worst)
        self.assertTrue(st["cost_estimated"])

    def test_limits_cannot_exceed_ceilings_but_can_be_lowered(self):
        with self.assertRaises(ValueError):
            budget_mod.Budget(self.run_dir, max_requests=41)
        with self.assertRaises(ValueError):
            budget_mod.Budget(self.run_dir, max_cost_usd=0.51)
        with self.assertRaises(ValueError):
            budget_mod.Budget(self.run_dir, max_requests=-1)
        b = budget_mod.Budget(self.run_dir, max_requests=3, max_cost_usd=0.1)
        self.assertEqual([b.reserve(1, 1)[0] for _ in range(4)], [True, True, True, False])
        st = self.budget_json()
        self.assertEqual((st["max_requests"], st["max_cost_usd"]), (3, 0.1))

    def test_concurrent_reservations_never_exceed_limit(self):
        b = budget_mod.Budget(self.run_dir, max_requests=10)
        results = []

        def go():
            results.append(b.reserve(10, 10)[0])
        threads = [threading.Thread(target=go) for _ in range(30)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(results.count(True), 10)
        self.assertEqual(self.budget_json()["requests"], 10)

    def test_concurrent_cost_reservations_never_exceed_the_ceiling(self):
        w = budget_mod.worst_case(1000, 100)
        b = budget_mod.Budget(self.run_dir, max_cost_usd=w * 3 + w / 2)
        results = []

        def go():
            results.append(b.reserve(1000, 100)[0])
        threads = [threading.Thread(target=go) for _ in range(20)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(results.count(True), 3)
        st = self.budget_json()
        self.assertAlmostEqual(st["reserved_usd"], 3 * w)
        self.assertEqual(st["requests"], 3)
        self.assertLessEqual(st["cost_usd"] + st["reserved_usd"], st["max_cost_usd"] + 1e-12)

    def test_settle_releases_the_reservation(self):
        w = budget_mod.worst_case(1000, 100)
        b = budget_mod.Budget(self.run_dir, max_cost_usd=w * 1.5)
        ok, worst, _ = b.reserve(1000, 100)
        self.assertTrue(ok)
        self.assertFalse(b.reserve(1000, 100)[0])  # the reservation still holds the room
        b.settle(0.0001, worst)
        st = self.budget_json()
        self.assertEqual(st["reserved_usd"], 0)
        self.assertEqual(st["cost_usd"], 0.0001)
        self.assertTrue(b.reserve(1000, 100)[0])  # passes against the actual cost


class KeyAndRedaction(Base):
    def write_env(self, text):
        pw = os.path.join(self.tmp, "pw")
        os.makedirs(os.path.join(pw, ".hermes"), exist_ok=True)
        with open(os.path.join(pw, ".hermes", ".env"), "w") as fh:
            fh.write(text)
        return pw

    def test_read_key_forms(self):
        for text in ("OPENROUTER_API_KEY=sk-or-v1-aaa\n",
                     "export OPENROUTER_API_KEY=sk-or-v1-aaa\n",
                     "OTHER=1\nOPENROUTER_API_KEY='sk-or-v1-aaa'\n",
                     'OPENROUTER_API_KEY="sk-or-v1-aaa" # comment\n'):
            with self.subTest(text=text):
                self.assertEqual(budget_mod.read_key(self.write_env(text)), "sk-or-v1-aaa")

    def test_read_key_rejections_never_leak_value(self):
        for text in ("", "OTHER=sk-or-v1-zzz\n", "OPENROUTER_API_KEY=notakey-secret99\n",
                     "OPENROUTER_API_KEY=sk-or-v1-a sk-or-v1-b\n", "OPENROUTER_API_KEY='sk-or-unclosed\n",
                     "#OPENROUTER_API_KEY=sk-or-v1-aaa\n"):
            with self.subTest(text=text):
                with self.assertRaises(RuntimeError) as ctx:
                    budget_mod.read_key(self.write_env(text))
                self.assertNotIn("secret99", str(ctx.exception))
                self.assertNotIn("sk-or-v1", str(ctx.exception))

    def test_read_key_missing_file(self):
        with self.assertRaises(RuntimeError):
            budget_mod.read_key(os.path.join(self.tmp, "nowhere"))

    def test_read_key_ignores_environment(self):
        pw = self.write_env("OPENROUTER_API_KEY=sk-or-v1-fromfile\n")
        with mock.patch.dict(os.environ, {"OPENROUTER_API_KEY": "sk-or-v1-fromenv"}):
            self.assertEqual(budget_mod.read_key(pw), "sk-or-v1-fromfile")
        empty = os.path.join(self.tmp, "pw2")
        os.makedirs(os.path.join(empty, ".hermes"))
        with open(os.path.join(empty, ".hermes", ".env"), "w") as fh:
            fh.write("NOTHING=1\n")
        with mock.patch.dict(os.environ, {"OPENROUTER_API_KEY": "sk-or-v1-fromenv"}):
            with self.assertRaises(RuntimeError):
                budget_mod.read_key(empty)

    def test_redact(self):
        text = "k=sk-or-v1-abc_DEF-123 and /tmp/x/run/file; sk-or-"
        out = budget_mod.redact(text, "/tmp/x")
        self.assertEqual(out, "k=sk-or-REDACTED and <root>/run/file; sk-or-")
        self.assertNotIn("abc_DEF", out)


class RelayEndToEnd(Base):
    def test_streaming_text_in_order_with_translation_visible_upstream(self):
        self.start_fake([{"text": "Hello brave new world", "chunks": 4, "cost": 0.001}])
        self.start_relay()
        body = {
            "systemInstruction": {"parts": [{"text": "A"}, {"text": "B"}]},
            "contents": [
                user("go"),
                {"role": "model", "parts": [{"text": "hidden", "thought": True, "thoughtSignature": "SIGX"},
                                            {"functionCall": {"name": "run_command", "args": {"c": "ls"},
                                                              "id": "call-1"}}]},
                {"role": "model", "parts": [{"functionResponse": {"name": "run_command", "id": "call-1",
                                                                  "response": {"output": "files"}}}]},
            ],
            "generationConfig": {"maxOutputTokens": 65536, "thinkingConfig": {"thinkingBudget": 4000}},
        }
        status, headers, data = self.post(MAIN_PATH, body)
        self.assertEqual(status, 200)
        self.assertIn("text/event-stream", headers["Content-Type"])
        self.assertNotIn("Content-Length", headers)
        events = sse_events(data)
        texts = [e["candidates"][0]["content"]["parts"][0]["text"] for e in events[:-1]]
        self.assertEqual("".join(texts), "Hello brave new world")
        self.assertEqual(len(texts), 4)
        self.assertEqual(events[-1]["candidates"][0]["finishReason"], "STOP")
        self.assertEqual(events[-1]["usageMetadata"]["totalTokenCount"], 110)
        self.assertTrue(data.startswith(b"data: ") and data.endswith(b"\r\n\r\n"))

        sent = self.upstream_requests()
        self.assertEqual(len(sent), 1)
        up = sent[0]
        self.assertEqual(up["messages"][0], {"role": "system", "content": "A\n\nB"})
        self.assertEqual([m["role"] for m in up["messages"]], ["system", "user", "assistant", "tool"])
        self.assertEqual(up["messages"][3]["tool_call_id"], "call-1")
        self.assertEqual(up["messages"][2]["tool_calls"][0]["id"], "call-1")
        self.assertNotIn("hidden", json.dumps(up))
        self.assertNotIn("SIGX", json.dumps(up))
        self.assertEqual(up["model"], "google/gemini-2.5-flash")
        self.assertIs(up["stream"], True)
        self.assertEqual(up["stream_options"], {"include_usage": True})
        self.assertEqual(up["usage"], {"include": True})
        self.assertEqual(up["reasoning"], {"enabled": False})
        self.assertEqual(up["provider"], {"max_price": {"prompt": 0.3, "completion": 2.5},
                                          "require_parameters": True})
        self.assertEqual(up["max_tokens"], 1024)
        self.assertNotIn(TOKEN, json.dumps(up))
        st = self.budget_json()
        self.assertEqual(st["requests"], 1)
        self.assertAlmostEqual(st["cost_usd"], 0.001)
        self.assertFalse(st["cost_estimated"])

    def test_tool_call_reply_becomes_function_call_with_fresh_id(self):
        self.start_fake([{"tool_calls": [{"name": "run_command", "arguments": {"CommandLine": "echo hi"}},
                                         {"name": "other", "arguments": {"a": 1}}]}])
        self.start_relay()
        status, _, data = self.post(MAIN_PATH, simple_body())
        self.assertEqual(status, 200)
        events = sse_events(data)
        calls = [p["functionCall"] for e in events for p in e["candidates"][0]["content"]["parts"]
                 if "functionCall" in p]
        self.assertEqual([c["name"] for c in calls], ["run_command", "other"])
        self.assertEqual(calls[0]["args"], {"CommandLine": "echo hi"})
        self.assertEqual(calls[1]["args"], {"a": 1})
        self.assertEqual(len({c["id"] for c in calls}), 2)
        for c in calls:
            self.assertTrue(c["id"].startswith("ht-"))
            self.assertFalse(c["id"].startswith("fake_call"))
        self.assertEqual(events[-1]["candidates"][0]["finishReason"], "STOP")
        rec = self.relay_log()[0]
        self.assertEqual([c["name"] for c in rec["tool_calls"]], ["run_command", "other"])

    def test_streaming_is_incremental(self):
        gate = threading.Event()
        first_seen = []

        class Slow(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Connection", "close")
                self.end_headers()

                def chunk(delta, finish=None):
                    return b"data: " + json.dumps({"choices": [{"index": 0, "delta": delta,
                                                                "finish_reason": finish}]}).encode() + b"\n\n"
                self.wfile.write(chunk({"content": "FIRST"}))
                self.wfile.flush()
                first_seen.append(gate.wait(10))
                self.wfile.write(chunk({"content": "SECOND"}) + chunk({}, "stop") + b"data: [DONE]\n\n")
                self.wfile.flush()

        slow = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Slow))
        self.start_relay(upstream_url="http://127.0.0.1:%d/x" % slow.server_address[1])
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=8)
        self.addCleanup(conn.close)
        conn.request("POST", MAIN_PATH, body=json.dumps(simple_body()), headers={"x-goog-api-key": TOKEN})
        resp = conn.getresponse()
        line = b""
        while not line.startswith(b"data:"):
            line = resp.readline()  # would time out if the relay buffered the stream
        self.assertEqual(event_text(sse_events(line + b"\r\n\r\n")), "FIRST")
        gate.set()
        rest = resp.read()
        self.assertEqual(event_text(sse_events(rest)), "SECOND")
        self.assertEqual(first_seen, [True])

    def test_parallel_requests_never_overrun_the_cost_ceiling(self):
        n, m = 3, 9
        gate = threading.Event()
        lock = threading.Lock()
        seen = []

        class Block(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                with lock:
                    seen.append(1)
                gate.wait(15)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Connection", "close")
                self.end_headers()
                ev = {"choices": [{"index": 0, "delta": {"content": "ok"}, "finish_reason": "stop"}],
                      "usage": {"prompt_tokens": 1, "completion_tokens": 1, "cost": 0.0001}}
                self.wfile.write(b"data: " + json.dumps(ev).encode() + b"\n\ndata: [DONE]\n\n")
                self.wfile.flush()

        self.addCleanup(gate.set)
        blocked = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Block))
        translated = translate.gemini_to_openai(simple_body())
        input_bytes = len(json.dumps(translated, ensure_ascii=False).encode("utf-8"))
        w = budget_mod.worst_case(input_bytes, translated["max_tokens"])
        self.start_relay(upstream_url="http://127.0.0.1:%d/x" % blocked.server_address[1],
                         max_cost_usd=w * n + w / 2)
        statuses = []

        def fire():
            statuses.append(self.post(MAIN_PATH, simple_body())[0])
        threads = [threading.Thread(target=fire) for _ in range(m)]
        for t in threads:
            t.start()
        deadline = time.time() + 10
        while time.time() < deadline and (len(seen) < n or statuses.count(429) < m - n):
            time.sleep(0.02)
        self.assertEqual(len(seen), n)  # at most N reached the blocked upstream
        self.assertEqual(statuses.count(429), m - n)
        gate.set()
        for t in threads:
            t.join(15)
        self.assertEqual(sorted(statuses), [200] * n + [429] * (m - n))
        st = self.budget_json()
        self.assertLessEqual(st["cost_usd"], st["max_cost_usd"])
        self.assertEqual(st["reserved_usd"], 0)
        self.assertEqual(st["requests"], n)
        self.assertEqual(len(seen), n)

    def test_non_streaming_generate_content_is_aggregated(self):
        self.start_fake([{"text": "abcdef", "chunks": 3, "cost": 0.0004}])
        self.start_relay()
        status, headers, data = self.post(GEN_PATH, simple_body())
        self.assertEqual(status, 200)
        self.assertEqual(headers["Content-Type"], "application/json")
        obj = json.loads(data)
        self.assertEqual(obj["candidates"][0]["content"]["parts"], [{"text": "abcdef"}])
        self.assertEqual(obj["candidates"][0]["finishReason"], "STOP")
        self.assertEqual(obj["usageMetadata"]["promptTokenCount"], 100)

    def test_request_limit_refuses_third_before_any_upstream_call(self):
        self.start_fake(default={"text": "ok", "cost": 0.0001})
        self.start_relay(max_requests=2)
        self.assertEqual(self.post(MAIN_PATH, simple_body())[0], 200)
        self.assertEqual(self.post(MAIN_PATH, simple_body())[0], 200)
        status, _, data = self.post(MAIN_PATH, simple_body())
        self.assertEqual(status, 429)
        err = json.loads(data)["error"]
        self.assertEqual((err["code"], err["status"], err["message"]),
                         (429, "RESOURCE_EXHAUSTED", "agy-relay budget exhausted"))
        self.assertEqual(self.fake.received, 2)
        self.assertEqual(len(self.upstream_requests()), 2)
        st = self.budget_json()
        self.assertEqual((st["requests"], st["exhausted"], st["refusals"]), (2, True, 1))
        log = self.relay_log()
        self.assertEqual([r["refused"] for r in log], [False, False, True])
        self.assertEqual(log[2]["status"], 429)

    def test_cost_ceiling_refuses_before_upstream_call(self):
        self.start_fake(default={"text": "ok", "cost": 0.001})
        self.start_relay(max_cost_usd=0.003)  # worst case for one 1024-token request is ~0.0026
        self.assertEqual(self.post(MAIN_PATH, simple_body())[0], 200)
        status, _, data = self.post(MAIN_PATH, simple_body())
        self.assertEqual(status, 429)
        self.assertEqual(json.loads(data)["error"]["status"], "RESOURCE_EXHAUSTED")
        self.assertEqual(self.fake.received, 1)

    def test_budget_persisted_before_each_forward(self):
        seen = []
        run_dir = self.run_dir

        class Spy(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                with open(os.path.join(run_dir, "budget.json")) as fh:
                    seen.append(json.load(fh)["requests"])
                self.send_response(200)
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(b"data: [DONE]\n\n")

        spy = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Spy))
        self.start_relay(upstream_url="http://127.0.0.1:%d/x" % spy.server_address[1])
        self.post(MAIN_PATH, simple_body())
        self.post(MAIN_PATH, simple_body())
        self.assertEqual(seen, [1, 2])

    def test_missing_or_wrong_token_is_403_and_query_key_works(self):
        self.start_fake(default={"text": "ok"})
        self.start_relay()
        for kw in ({"token": False}, {"token": "wrong"}, {"token": TOKEN[:-1]}, {"token": TOKEN + "x"}):
            status, _, data = self.post(MAIN_PATH, simple_body(), **kw)
            self.assertEqual(status, 403, kw)
            self.assertEqual(json.loads(data)["error"]["status"], "PERMISSION_DENIED")
        self.assertEqual(self.request("GET", "/v1beta/models", token=False)[0], 403)
        self.assertEqual(self.request("POST", "/v1beta/models/m:countTokens", {}, token=False)[0], 403)
        self.assertEqual(self.fake.received, 0)
        self.assertEqual(self.budget_json()["requests"], 0)
        status, _, _ = self.post(MAIN_PATH + "&key=" + TOKEN, simple_body(), token=False)
        self.assertEqual(status, 200)
        status, _, _ = self.request("GET", "/v1beta/models?key=" + TOKEN, token=False)
        self.assertEqual(status, 200)
        self.assertEqual(self.post(MAIN_PATH + "&key=wrong", simple_body(), token=False)[0], 403)

    def test_local_routes_cost_nothing(self):
        self.start_fake(default={"text": "ok"})
        self.start_relay()
        status, _, data = self.request("GET", "/v1beta/models")
        self.assertEqual(status, 200)
        self.assertIn("models/gemini-3.8-flash", [m["name"] for m in json.loads(data)["models"]])
        status, _, data = self.post("/v1beta/models/gemini-3.8-flash:countTokens", raw=b"x" * 80)
        self.assertEqual((status, json.loads(data)), (200, {"totalTokens": 20}))
        status, _, data = self.post(AUX_PATH, simple_body("title please"))
        self.assertEqual(status, 200)
        self.assertEqual(event_text(sse_events(data)), "agy smoke")
        status, _, data = self.post("/v1beta/models/gemini-3.1-flash-lite-preview:generateContent",
                                    simple_body())
        self.assertEqual(json.loads(data)["candidates"][0]["content"]["parts"][0]["text"], "agy smoke")
        self.assertEqual(self.fake.received, 0)
        st = self.budget_json()
        self.assertEqual((st["requests"], st["cost_usd"], st["refusals"]), (0, 0.0, 0))
        self.assertEqual([r["kind"] for r in self.relay_log()], ["models", "count", "aux_local", "aux_local"])

    def test_forward_aux_forwards_and_counts(self):
        self.start_fake(default={"text": "TITLE", "cost": 0.0001})
        self.start_relay(forward_aux=True)
        status, _, data = self.post(AUX_PATH, simple_body("title"))
        self.assertEqual(status, 200)
        self.assertEqual(event_text(sse_events(data)), "TITLE")
        self.assertEqual(self.fake.received, 1)
        self.assertEqual(self.budget_json()["requests"], 1)
        self.assertEqual(self.relay_log()[0]["kind"], "main")

    def test_unknown_route_and_bad_json(self):
        self.start_fake(default={"text": "ok"})
        self.start_relay()
        self.assertEqual(self.post("/v1beta/models/m:somethingElse", {})[0], 404)
        status, _, data = self.post(MAIN_PATH, raw=b"{not json")
        self.assertEqual((status, json.loads(data)["error"]["status"]), (400, "INVALID_ARGUMENT"))
        self.assertEqual(self.fake.received, 0)
        self.assertEqual(self.budget_json()["requests"], 0)

    def test_upstream_errors_are_mapped_and_still_count(self):
        self.start_fake([{"error": {"http": 500, "message": "provider down"}},
                         {"error": {"http": 429, "message": "slow down"}},
                         {"error": {"http": 401, "message": "bad key"}}])
        self.start_relay()
        got = []
        for _ in range(3):
            status, _, data = self.post(MAIN_PATH, simple_body())
            got.append((status, json.loads(data)["error"]))
        self.assertEqual([(s, e["status"], e["message"]) for s, e in got], [
            (500, "UNAVAILABLE", "provider down"), (429, "RESOURCE_EXHAUSTED", "slow down"),
            (401, "PERMISSION_DENIED", "bad key")])
        st = self.budget_json()
        self.assertEqual(st["requests"], 3)  # a sent request always counts
        self.assertTrue(st["cost_estimated"])
        self.assertGreater(st["cost_usd"], 0)
        self.assertEqual([r["upstream_status"] for r in self.relay_log()], [500, 429, 401])

    def test_unreachable_upstream_is_502_and_counts(self):
        import socket
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        self.start_relay(upstream_url="http://127.0.0.1:%d/x" % port)
        status, _, data = self.post(MAIN_PATH, simple_body())
        self.assertEqual((status, json.loads(data)["error"]["status"]), (502, "UNAVAILABLE"))
        self.assertEqual(self.budget_json()["requests"], 1)

    def test_redirects_are_not_followed(self):
        hits = {"target": 0}

        class Target(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                hits["target"] += 1
                self.send_response(200)
                self.end_headers()

        target = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Target))
        location = "http://127.0.0.1:%d/elsewhere" % target.server_address[1]

        class Redirect(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(307)
                self.send_header("Location", location)
                self.send_header("Content-Length", "0")
                self.end_headers()

        redirect = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Redirect))
        self.start_relay(upstream_url="http://127.0.0.1:%d/x" % redirect.server_address[1])
        status, _, data = self.post(MAIN_PATH, simple_body())
        self.assertEqual(hits["target"], 0)
        self.assertEqual(status, 502)
        self.assertEqual(json.loads(data)["error"]["status"], "UNAVAILABLE")
        self.assertEqual(self.budget_json()["requests"], 1)

    def test_no_retry_after_upstream_failure(self):
        self.start_fake([{"error": {"http": 500, "message": "x"}}], default={"text": "second"})
        self.start_relay()
        self.assertEqual(self.post(MAIN_PATH, simple_body())[0], 500)
        self.assertEqual(self.fake.received, 1)

    def test_oversized_input_is_400_and_not_counted(self):
        self.start_fake(default={"text": "ok"})
        self.start_relay()
        big = simple_body("x" * (budget_mod.MAX_INPUT_BYTES + 10))
        status, _, data = self.post(MAIN_PATH, big)
        err = json.loads(data)["error"]
        self.assertEqual((status, err["status"], err["message"]),
                         (400, "INVALID_ARGUMENT", "agy-relay input too large"))
        self.assertEqual(self.fake.received, 0)
        st = self.budget_json()
        self.assertEqual((st["requests"], st["refusals"], st["exhausted"]), (0, 0, False))
        self.assertEqual(self.relay_log()[0]["status"], 400)
        # just under the limit still goes through
        ok_body = simple_body("y" * (budget_mod.MAX_INPUT_BYTES - 1000))
        self.assertEqual(self.post(MAIN_PATH, ok_body)[0], 200)

    def test_relay_log_keys_and_contents(self):
        self.start_fake([{"tool_calls": [{"name": "run_command", "arguments": {"CommandLine": "ls"}}],
                          "cost": 0.002}, {"text": "T" * 5000}])
        self.start_relay()
        decl = {"functionDeclarations": [{"name": "run_command", "description": "d",
                                          "parameters": {"type": "OBJECT", "properties": {}}}]}
        self.post(MAIN_PATH, simple_body("one", tools=[decl]))
        self.post(MAIN_PATH, simple_body("two"))
        first, second = self.relay_log()
        keys = {"seq", "t", "path", "kind", "status", "upstream_status", "model", "input_bytes", "roles",
                "tool_names_count", "tool_calls", "text_trimmed", "usage", "cost_usd", "cost_estimated",
                "refused"}
        self.assertTrue(keys <= set(first), keys - set(first))
        self.assertEqual((first["seq"], second["seq"]), (1, 2))
        self.assertEqual((first["kind"], first["status"], first["upstream_status"]), ("main", 200, 200))
        self.assertEqual(first["model"], "gemini-3.8-flash")
        self.assertEqual(first["roles"], ["user"])
        self.assertEqual(first["tool_names_count"], 1)
        self.assertEqual(first["tool_calls"][0]["name"], "run_command")
        self.assertIn("ls", first["tool_calls"][0]["args_trimmed"])
        self.assertEqual((first["cost_usd"], first["cost_estimated"], first["refused"]), (0.002, False, False))
        self.assertGreater(first["input_bytes"], 0)
        self.assertEqual(len(second["text_trimmed"]), 2048)
        self.assertTrue(second["cost_estimated"])
        self.assertNotIn("Authorization", json.dumps(self.relay_log()))

    def test_redaction_in_log_and_run_dir(self):
        reply_text = "key is %s and root is %s" % (FAKE_KEY, self.tmp)
        self.start_fake([{"text": reply_text},
                         {"tool_calls": [{"name": "run_command",
                                          "arguments": {"CommandLine": "echo " + FAKE_KEY}}]}])
        self.start_relay(key="sk-or-v1-livekey999")
        self.post(MAIN_PATH, simple_body("my key is " + FAKE_KEY))
        self.post(MAIN_PATH, simple_body("again"))
        with open(os.path.join(self.run_dir, "relay.jsonl")) as fh:
            raw = fh.read()
        self.assertIn("sk-or-REDACTED", raw)
        self.assertIn("<root>", raw)
        self.assertNotIn(self.tmp, raw)
        for dirpath, _, files in os.walk(self.run_dir):
            for name in files:
                with open(os.path.join(dirpath, name), "rb") as fh:
                    content = fh.read()
                self.assertNotIn(b"abc123", content, name)
                self.assertNotIn(b"livekey999", content, name)

    def test_live_key_sent_upstream_as_bearer_only(self):
        auth = []

        class Spy(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                auth.append((self.headers.get("Authorization"), self.headers.get("x-goog-api-key")))
                self.send_response(200)
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(b"data: [DONE]\n\n")

        spy = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Spy))
        url = "http://127.0.0.1:%d/x" % spy.server_address[1]
        self.start_relay(upstream_url=url, key="sk-or-v1-livekey999")
        self.post(MAIN_PATH, simple_body())
        self.assertEqual(auth, [("Bearer sk-or-v1-livekey999", None)])

    def test_fake_mode_sends_placeholder_bearer(self):
        auth = []

        class Spy(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                auth.append(self.headers.get("Authorization"))
                self.send_response(200)
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(b"data: [DONE]\n\n")

        spy = self.serve(ThreadingHTTPServer(("127.0.0.1", 0), Spy))
        self.start_relay(upstream_url="http://127.0.0.1:%d/x" % spy.server_address[1])
        self.post(MAIN_PATH, simple_body())
        self.assertEqual(auth, ["Bearer fake-upstream"])

    def test_frame_templates_flow_through_the_chain(self):
        self.start_fake([{"tool_calls": [{"name": "run_command",
                                          "arguments": {"CommandLine": "herdr-threads ack {frame.id}"}}]},
                         {"text": "{frame.kind}/{frame.thread}/{frame.sender}/{frame.id}"}])
        self.start_relay()
        frames = ("[herdr-threads] message m-1 in th-1 from alice (ack required):\nfirst body\n\n"
                  "[herdr-threads] reply m-2 in th-2 from bob:\nsecond body")
        body = {"systemInstruction": {"parts": [{"text": "sys"}]},
                "contents": [user("noise"), user(frames)]}
        _, _, data = self.post(MAIN_PATH, body)
        call = [p["functionCall"] for e in sse_events(data) for p in e["candidates"][0]["content"]["parts"]
                if "functionCall" in p][0]
        self.assertEqual(call["args"]["CommandLine"], "herdr-threads ack m-2")  # last header wins
        _, _, data = self.post(MAIN_PATH, body)
        self.assertEqual(event_text(sse_events(data)), "reply/th-2/bob/m-2")


class FakeUpstreamDirect(Base):
    def raw_post(self, body):
        conn = http.client.HTTPConnection("127.0.0.1", self.fake.server_address[1], timeout=10)
        try:
            conn.request("POST", "/api/v1/chat/completions", body=json.dumps(body))
            resp = conn.getresponse()
            return resp.status, resp.read()
        finally:
            conn.close()

    def test_sse_shape_for_text_and_tool_calls(self):
        self.start_fake([{"text": "abcdef", "chunks": 3, "cost": 0.5},
                         {"tool_calls": [{"name": "f", "arguments": {"k": "v"}}]},
                         {"text": "no cost"}])
        status, data = self.raw_post({"messages": []})
        self.assertEqual(status, 200)
        lines = data.decode().split("\n")
        self.assertEqual(lines[0], ": OPENROUTER PROCESSING")
        items = list(translate.parse_sse(data.splitlines(keepends=True)))
        self.assertEqual(items[-1], "[DONE]")
        self.assertEqual(items[0]["choices"][0]["delta"]["role"], "assistant")
        contents = [i["choices"][0]["delta"].get("content") for i in items[1:-3]]
        self.assertEqual(contents, ["ab", "cd", "ef"])
        self.assertEqual(items[-3]["choices"][0]["finish_reason"], "stop")
        self.assertEqual(items[-2]["usage"], {"prompt_tokens": 100, "completion_tokens": 10,
                                              "total_tokens": 110, "cost": 0.5})
        status, data = self.raw_post({"messages": []})
        items = list(translate.parse_sse(data.splitlines(keepends=True)))
        deltas = [i["choices"][0]["delta"] for i in items[1:-3]]
        self.assertEqual(len(deltas), 3)  # name/id chunk and two argument halves
        self.assertEqual(deltas[0]["tool_calls"][0]["function"]["name"], "f")
        joined = "".join(d["tool_calls"][0]["function"]["arguments"] for d in deltas)
        self.assertEqual(json.loads(joined), {"k": "v"})
        self.assertNotEqual(deltas[1]["tool_calls"][0]["function"]["arguments"], joined)
        self.assertEqual(items[-3]["choices"][0]["finish_reason"], "tool_calls")
        status, data = self.raw_post({"messages": []})
        items = list(translate.parse_sse(data.splitlines(keepends=True)))
        self.assertNotIn("cost", items[-2]["usage"])

    def test_replies_consumed_in_order_then_default_and_bodies_recorded(self):
        self.start_fake([{"text": "one"}, {"text": "two"}], default={"text": "dflt"})
        got = []
        for i in range(4):
            _, data = self.raw_post({"messages": [{"role": "user", "content": "q%d" % i}]})
            tr = translate.StreamTranslator()
            text = ""
            for item in translate.parse_sse(data.splitlines(keepends=True)):
                if item != "[DONE]":
                    text += event_text(tr.feed(item))
            got.append(text)
        self.assertEqual(got, ["one", "two", "dflt", "dflt"])
        self.assertEqual([r["messages"][0]["content"] for r in self.upstream_requests()],
                         ["q0", "q1", "q2", "q3"])

    def test_default_when_no_script(self):
        self.fake = self.serve(fake_upstream.make_fake_upstream(self.ctl))
        _, data = self.raw_post({"messages": []})
        self.assertIn(b'"content": "done"', data)

    def test_template_errors_and_content_parts(self):
        self.start_fake([{"text": "{frame.id}"}, {"text": "{frame.nope}"}, {"text": "{frame.id}"}])
        status, data = self.raw_post({"messages": [{"role": "user", "content": "no header here"}]})
        self.assertEqual(status, 500)
        self.assertIn("template_error", data.decode())
        header = "[herdr-threads] message id-9 in t-1 from s-1 (x):"
        status, data = self.raw_post({"messages": [{"role": "user", "content": [{"type": "text", "text": header}]}]})
        self.assertEqual(status, 500)
        self.assertIn("frame.nope", data.decode())
        status, data = self.raw_post({"messages": [{"role": "user", "content": [{"type": "text", "text": header}]}]})
        self.assertEqual(status, 200)
        self.assertIn("id-9", data.decode())

    def test_wrong_script_format_is_reported(self):
        self.start_fake()
        with open(os.path.join(self.ctl, "script.json"), "w") as fh:
            json.dump({"format": "other/9", "replies": []}, fh)
        status, data = self.raw_post({"messages": []})
        self.assertEqual(status, 500)
        self.assertIn("format", data.decode())

    def test_header_regex_groups(self):
        m = fake_upstream.last_frame({"messages": [{"role": "user", "content":
                                                    "x\n[herdr-threads] message abc in thr from snd (ack):\nbody"}]})
        self.assertEqual((m.group(1), m.group(2), m.group(3), m.group(4)), ("message", "abc", "thr", "snd"))
        self.assertIsNone(fake_upstream.last_frame({"messages": [{"role": "user", "content": "plain"}]}))


class CliGuards(Base):
    def run_cli(self, args, env_extra=None, drop=()):
        env = {k: v for k, v in os.environ.items() if k not in drop}
        env["HOME"] = self.tmp
        env.update(env_extra or {})
        return subprocess.run([sys.executable, "-B", RELAY_PY] + args, env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, universal_newlines=True, timeout=30)

    def assert_refused(self, proc, needle=None):
        self.assertEqual(proc.returncode, 2, proc.stderr)
        if needle:
            self.assertIn(needle, proc.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.run_dir, "port")))
        self.assertFalse(os.path.exists(os.path.join(self.run_dir, "local-token")))

    def test_live_refuses_under_ci_variables(self):
        for var, value in (("CI", "1"), ("GITHUB_ACTIONS", "true")):
            with self.subTest(var=var):
                proc = self.run_cli(["--run-dir", self.run_dir, "--live"],
                                    env_extra={var: value}, drop=("CI", "GITHUB_ACTIONS"))
                self.assert_refused(proc, "CI")

    def test_live_refuses_off_darwin(self):
        env = {k: v for k, v in os.environ.items() if k not in ("CI", "GITHUB_ACTIONS")}
        with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(relay, "platform", lambda: "linux"):
            with mock.patch.object(budget_mod, "read_key", side_effect=AssertionError("must not read key")):
                with self.assertRaises(SystemExit) as ctx, contextlib.redirect_stderr(io.StringIO()):
                    relay.main(["--run-dir", self.run_dir, "--live"])
        self.assertEqual(ctx.exception.code, 2)
        self.assertFalse(os.path.exists(self.run_dir))

    def test_live_reads_key_from_passwd_home_not_home_env(self):
        env = {k: v for k, v in os.environ.items() if k not in ("CI", "GITHUB_ACTIONS")}
        env["HOME"] = self.tmp
        seen = []

        def fake_read(pw_dir):
            seen.append(pw_dir)
            raise RuntimeError("stop here")
        with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(relay, "platform", lambda: "darwin"):
            with mock.patch.object(budget_mod, "read_key", fake_read):
                with self.assertRaises(SystemExit) as ctx, contextlib.redirect_stderr(io.StringIO()):
                    relay.main(["--run-dir", self.run_dir, "--live"])
        self.assertEqual(ctx.exception.code, 2)
        self.assertEqual(seen, [pwd.getpwuid(os.getuid()).pw_dir])
        self.assertNotEqual(seen[0], self.tmp)

    def test_ceilings_cannot_be_raised(self):
        for flag, value in (("--max-requests", "41"), ("--max-cost-usd", "0.51"), ("--max-requests", "-1")):
            with self.subTest(flag=flag, value=value):
                proc = self.run_cli(["--run-dir", self.run_dir, "--fake-upstream", "http://127.0.0.1:9",
                                     flag, value])
                self.assert_refused(proc)

    def test_fake_upstream_must_be_loopback(self):
        for url in ("http://example.com/", "https://127.0.0.1:9/", "http://127.0.0.1/",
                    "http://127.0.0.1.evil.example:80/", "http://127.0.0.1:80@evil.example:80/",
                    "http://0.0.0.0:9/", "ftp://127.0.0.1:9/"):
            with self.subTest(url=url):
                self.assert_refused(self.run_cli(["--run-dir", self.run_dir, "--fake-upstream", url]))

    def test_one_of_live_or_fake_upstream_required(self):
        self.assertEqual(self.run_cli(["--run-dir", self.run_dir]).returncode, 2)


class CliLifecycle(Base):
    def spawn_relay(self, extra=()):
        env = {k: v for k, v in os.environ.items() if k not in ("CI", "GITHUB_ACTIONS")}
        env["HOME"] = self.tmp
        out = open(os.path.join(self.tmp, "relay.out"), "w")
        self.addCleanup(out.close)
        proc = subprocess.Popen([sys.executable, "-B", RELAY_PY, "--run-dir", self.run_dir,
                                 "--fake-upstream", "http://127.0.0.1:9"] + list(extra),
                                env=env, stdout=out, stderr=subprocess.STDOUT)
        self.addCleanup(self.reap, proc)
        deadline = time.time() + 15
        port_file = os.path.join(self.run_dir, "port")
        while time.time() < deadline and not os.path.exists(port_file):
            self.assertIsNone(proc.poll(), "relay exited early")
            time.sleep(0.05)
        self.assertTrue(os.path.exists(port_file))
        return proc

    @staticmethod
    def reap(proc):
        if proc.poll() is None:
            proc.kill()
        proc.wait(10)

    def test_files_token_mode_and_authenticated_models_route(self):
        self.spawn_relay()
        token_path = os.path.join(self.run_dir, "local-token")
        mode = stat.S_IMODE(os.stat(token_path).st_mode)
        self.assertEqual(mode, 0o600)
        with open(token_path) as fh:
            token = fh.read().strip()
        self.assertRegex(token, r"^[0-9a-f]{64}$")
        with open(os.path.join(self.run_dir, "port")) as fh:
            port = int(fh.read())
        self.assertEqual(stat.S_IMODE(os.stat(os.path.join(self.run_dir, "budget.json")).st_mode) & 0o022, 0)
        for hdrs, want in (({"x-goog-api-key": token}, 200), ({}, 403)):
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
            conn.request("GET", "/v1beta/models", headers=hdrs)
            self.assertEqual(conn.getresponse().status, want)
            conn.close()
        self.assertEqual(self.budget_json_cli()["max_requests"], 40)

    def budget_json_cli(self):
        with open(os.path.join(self.run_dir, "budget.json")) as fh:
            return json.load(fh)

    def test_sigterm_exits_zero(self):
        proc = self.spawn_relay()
        proc.send_signal(signal.SIGTERM)
        self.assertEqual(proc.wait(10), 0)

    def test_exits_when_owner_pid_disappears(self):
        owner = subprocess.Popen(["sleep", "120"])
        self.addCleanup(self.reap, owner)
        proc = self.spawn_relay(["--owner-pid", str(owner.pid)])
        time.sleep(0.5)
        self.assertIsNone(proc.poll())
        owner.kill()
        owner.wait(10)
        self.assertEqual(proc.wait(5), 0)

    def test_lowered_flags_reach_budget_file(self):
        self.spawn_relay(["--max-requests", "5", "--max-cost-usd", "0.1"])
        st = self.budget_json_cli()
        self.assertEqual((st["max_requests"], st["max_cost_usd"]), (5, 0.1))


class FakeUpstreamCli(Base):
    def test_cli_writes_port_and_exits_with_owner(self):
        owner = subprocess.Popen(["sleep", "120"])
        self.addCleanup(lambda: (owner.poll() is None and owner.kill(), owner.wait(10)))
        os.makedirs(self.ctl)
        proc = subprocess.Popen([sys.executable, "-B", os.path.join(HERE, "fake_upstream.py"), "--ctl", self.ctl,
                                 "--owner-pid", str(owner.pid)], stdout=subprocess.DEVNULL)
        self.addCleanup(lambda: (proc.poll() is None and proc.kill(), proc.wait(10)))
        port_file = os.path.join(self.ctl, "port")
        deadline = time.time() + 15
        while time.time() < deadline and not os.path.exists(port_file):
            time.sleep(0.05)
        with open(port_file) as fh:
            self.assertGreater(int(fh.read()), 0)
        owner.kill()
        owner.wait(10)
        self.assertEqual(proc.wait(5), 0)


if __name__ == "__main__":
    unittest.main()
