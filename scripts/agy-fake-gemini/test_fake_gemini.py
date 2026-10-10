"""Offline tests for the fake Gemini server and its scenario engine (loopback only)."""
import copy
import http.client
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import scenario as sc  # noqa: E402
import server as srvmod  # noqa: E402

SERVER = os.path.join(HERE, "server.py")
TESTDATA = os.path.join(HERE, "testdata", "agy-request-systemmessage.json")
MAIN = "/v1beta/models/gemini-3.1-pro-preview"
AUX = "/v1beta/models/gemini-3.1-flash-lite-preview"
UUID4 = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")


def scn(steps=None, **extra):
    obj = {"format": sc.FORMAT, "name": "t", "steps": steps if steps is not None else []}
    obj.update(extra)
    return obj


def step(label, reply=None, **kw):
    out = {"label": label, "reply": {"text": "ok-" + label} if reply is None else reply}
    out.update(kw)
    return out


def body_with(*texts):
    return {"contents": [{"role": "user", "parts": [{"text": t} for t in texts]}]}


def load_testdata():
    with open(TESTDATA, encoding="utf-8") as fh:
        return json.load(fh)


class ValidateTests(unittest.TestCase):
    def bad(self, obj, needle):
        with self.assertRaises(sc.ScenarioError) as cm:
            sc.validate(obj)
        self.assertIn(needle, str(cm.exception))

    def test_minimal_and_defaults(self):
        out = sc.validate(scn())
        self.assertEqual(out["aux_text"], "Fake title")
        self.assertEqual(out["default"]["text"], "DONE-default")
        self.assertEqual(out["vars"], {})
        self.assertEqual(out["steps"], [])

    def test_full_example_from_spec(self):
        out = sc.validate(scn([
            step("inv0", {"call": "run_command", "args": {"CommandLine": "x"}},
                 match={"contains": ["a"], "not_contains": ["b"]}, delay_ms=3000,
                 bind={"marker": "ack"}),
            step("inv1", {"text": "OK", "chunks": 3, "chunk_interval_ms": 200}),
        ], aux_text="Gate title", vars={"cwd": "/x"}, default={"text": "D"}))
        self.assertEqual(out["steps"][0]["delay_ms"], 3000)
        self.assertEqual(out["steps"][1]["reply"]["chunks"], 3)
        self.assertEqual(out["steps"][0]["reply"]["finish"], "STOP")

    def test_not_an_object(self):
        self.bad([], "$")

    def test_wrong_format(self):
        o = scn(); o["format"] = "agy-fake-gemini/2"
        self.bad(o, "$.format")

    def test_missing_format(self):
        o = scn(); del o["format"]
        self.bad(o, "$.format")

    def test_name_must_be_str(self):
        o = scn(); o["name"] = 3
        self.bad(o, "$.name")

    def test_aux_text_must_be_str(self):
        self.bad(scn(aux_text=1), "$.aux_text")

    def test_vars_must_map_str_to_str(self):
        self.bad(scn(vars={"a": 1}), "$.vars.a")
        self.bad(scn(vars=[]), "$.vars")

    def test_steps_must_be_list(self):
        self.bad(scn(steps={}), "$.steps")

    def test_unknown_top_level_key(self):
        self.bad(scn(extra=1), "$.extra")

    def test_unknown_step_key(self):
        self.bad(scn([step("a", bogus=1)]), "$.steps[0].bogus")

    def test_unknown_match_key(self):
        self.bad(scn([step("a", match={"regex": []})]), "$.steps[0].match.regex")

    def test_unknown_reply_key(self):
        self.bad(scn([step("a", {"text": "x", "zzz": 1})]), "$.steps[0].reply.zzz")

    def test_label_required_and_unique(self):
        s = step("a"); del s["label"]
        self.bad(scn([s]), "$.steps[0].label")
        self.bad(scn([step("a"), step("a")]), "$.steps[1].label")

    def test_match_lists_of_str(self):
        self.bad(scn([step("a", match={"contains": "x"})]), "$.steps[0].match.contains")
        self.bad(scn([step("a", match={"not_contains": [1]})]), "$.steps[0].match.not_contains[0]")

    def test_delay_ms(self):
        self.bad(scn([step("a", delay_ms=-1)]), "delay_ms")
        self.bad(scn([step("a", delay_ms="1")]), "delay_ms")
        self.bad(scn([step("a", delay_ms=True)]), "delay_ms")

    def test_bind_exactly_one(self):
        self.bad(scn([step("a", bind={})]), "$.steps[0].bind")
        self.bad(scn([step("a", bind={"marker": "m", "index": 0})]), "$.steps[0].bind")
        self.bad(scn([step("a", bind={"marker": 1})]), "bind.marker")
        self.bad(scn([step("a", bind={"index": "0"})]), "bind.index")

    def test_reply_required(self):
        s = step("a"); del s["reply"]
        self.bad(scn([s]), "$.steps[0].reply")

    def test_reply_exactly_one_kind(self):
        self.bad(scn([step("a", {})]), "exactly one")
        self.bad(scn([step("a", {"text": "x", "call": "c"})]), "exactly one")

    def test_text_str(self):
        self.bad(scn([step("a", {"text": 1})]), "reply.text")

    def test_call_rules(self):
        self.bad(scn([step("a", {"call": 1})]), "reply.call")
        self.bad(scn([step("a", {"call": "c", "args": []})]), "reply.args")
        self.bad(scn([step("a", {"call": "c", "id": 3})]), "reply.id")
        out = sc.validate(scn([step("a", {"call": "c"})]))
        self.assertEqual(out["steps"][0]["reply"]["args"], {})

    def test_args_and_id_only_with_call(self):
        self.bad(scn([step("a", {"text": "x", "args": {}})]), "reply.args")
        self.bad(scn([step("a", {"text": "x", "id": "i"})]), "reply.id")

    def test_calls_rules(self):
        self.bad(scn([step("a", {"calls": []})]), "reply.calls")
        self.bad(scn([step("a", {"calls": ["x"]})]), "reply.calls[0]")
        self.bad(scn([step("a", {"calls": [{"call": "c", "zz": 1}]})]), "reply.calls[0].zz")
        self.bad(scn([step("a", {"calls": [{"args": {}}]})]), "reply.calls[0].call")

    def test_error_rules(self):
        ok = {"http": 429, "status": "RESOURCE_EXHAUSTED", "message": "m"}
        sc.validate(scn([step("a", {"error": ok})]))
        for patch, needle in (({"http": 200}, "error.http"), ({"http": 600}, "error.http"),
                              ({"http": "429"}, "error.http"), ({"status": 1}, "error.status"),
                              ({"message": 1}, "error.message"), ({"extra": 1}, "error.extra")):
            e = dict(ok); e.update(patch)
            self.bad(scn([step("a", {"error": e})]), needle)
        e = dict(ok); del e["message"]
        self.bad(scn([step("a", {"error": e})]), "error.message")

    def test_finish_chunks_interval(self):
        self.bad(scn([step("a", {"text": "x", "finish": 1})]), "reply.finish")
        self.bad(scn([step("a", {"text": "x", "chunks": 0})]), "reply.chunks")
        self.bad(scn([step("a", {"call": "c", "chunks": 2})]), "reply.chunks")
        self.bad(scn([step("a", {"text": "x", "chunk_interval_ms": -5})]), "chunk_interval_ms")
        out = sc.validate(scn([step("a", {"text": "x", "finish": "MAX_TOKENS"})]))
        self.assertEqual(out["steps"][0]["reply"]["finish"], "MAX_TOKENS")

    def test_default_reply_validated(self):
        self.bad(scn(default={"text": 1}), "$.default.text")

    def test_load(self):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        p = os.path.join(d, "s.json")
        with open(p, "w") as fh:
            json.dump(scn([step("a")]), fh)
        self.assertEqual(sc.load(p)["steps"][0]["label"], "a")
        with open(p, "w") as fh:
            fh.write("{nope")
        with self.assertRaises(sc.ScenarioError):
            sc.load(p)


class CursorTests(unittest.TestCase):
    def make(self):
        return sc.Cursor(sc.validate(scn([
            step("p0", match={"not_contains": ["CHILD-PROMPT"]}),
            step("c0", match={"contains": ["CHILD-PROMPT"]}),
            step("p1", match={"not_contains": ["CHILD-PROMPT"]}),
        ])))

    def test_child_then_parent(self):
        cur = self.make()
        self.assertEqual(cur.select("CHILD-PROMPT go")["label"], "c0")
        self.assertEqual(cur.select("parent")["label"], "p0")
        self.assertEqual(cur.select("parent again")["label"], "p1")
        self.assertEqual(cur.consumed_labels(), ["c0", "p0", "p1"])
        self.assertEqual(cur.remaining_labels(), [])

    def test_parent_then_child(self):
        cur = self.make()
        self.assertEqual(cur.select("parent")["label"], "p0")
        self.assertEqual(cur.select("CHILD-PROMPT")["label"], "c0")
        self.assertEqual(cur.remaining_labels(), ["p1"])

    def test_default_when_nothing_matches(self):
        cur = self.make()
        self.assertEqual(cur.select("CHILD-PROMPT")["label"], "c0")
        self.assertIsNone(cur.select("CHILD-PROMPT"))
        self.assertEqual(cur.unmatched, 1)
        self.assertEqual(cur.consumed_labels(), ["c0"])

    def test_contains_needs_every_substring(self):
        cur = sc.Cursor(sc.validate(scn([step("a", match={"contains": ["x", "y"]})])))
        self.assertIsNone(cur.select("x alone"))
        self.assertEqual(cur.select("x and y")["label"], "a")

    def test_no_match_matches_anything(self):
        cur = sc.Cursor(sc.validate(scn([step("a")])))
        self.assertEqual(cur.select("")["label"], "a")


HDR2 = ("[herdr-threads] message m1 in t-alpha from alice [ack-required]:\nbody one\n"
        "[herdr-threads] reply m2 in t-beta from bob:\nbody two\n")


class FrameTests(unittest.TestCase):
    def test_frames(self):
        f = sc.frames("noise\n" + HDR2)
        self.assertEqual(len(f), 2)
        self.assertEqual((f[0]["kind"], f[0]["id"], f[0]["thread"], f[0]["sender"]),
                         ("message", "m1", "t-alpha", "alice"))
        self.assertIn("[ack-required]", f[0]["marker"])
        self.assertEqual((f[1]["kind"], f[1]["id"], f[1]["sender"]), ("reply", "m2", "bob"))
        self.assertEqual(f[1]["marker"], "")

    def test_frames_ignores_mid_line_header(self):
        self.assertEqual(sc.frames("see [herdr-threads] message m1 in t from a:"), [])

    def test_bind_frame(self):
        f = sc.frames(HDR2)
        self.assertEqual(sc.bind_frame(f, {"marker": "ack"})["id"], "m1")
        self.assertIsNone(sc.bind_frame(f, {"marker": "nope"}))
        self.assertEqual(sc.bind_frame(f, {"index": 1})["id"], "m2")
        self.assertEqual(sc.bind_frame(f, {"index": 0})["id"], "m1")
        self.assertIsNone(sc.bind_frame(f, {"index": 5}))
        self.assertEqual(sc.bind_frame(f, None)["id"], "m2")
        self.assertIsNone(sc.bind_frame([], None))


class RenderTests(unittest.TestCase):
    FRAME = {"kind": "message", "id": "m1", "thread": "t-a", "sender": "alice", "marker": ""}

    def test_vars_frame_and_nested_args(self):
        reply = {"call": "run", "args": {"cmd": "ack {frame.id} in {frame.thread}",
                                         "nested": {"cwd": "{var.cwd}", "list": ["{frame.sender}", 7]}}}
        out, bad = sc.render(reply, {"cwd": "/w"}, self.FRAME)
        self.assertIsNone(bad)
        self.assertEqual(out["args"]["cmd"], "ack m1 in t-a")
        self.assertEqual(out["args"]["nested"], {"cwd": "/w", "list": ["alice", 7]})
        self.assertEqual(reply["args"]["cmd"], "ack {frame.id} in {frame.thread}")  # input untouched

    def test_kind_placeholder(self):
        out, bad = sc.render({"text": "{frame.kind}"}, {}, self.FRAME)
        self.assertEqual(out["text"], "message")

    def test_unresolved_var(self):
        out, bad = sc.render({"text": "a {var.nope} b"}, {}, self.FRAME)
        self.assertEqual(bad, "{var.nope}")
        self.assertEqual(out, {"text": "FAKE-GEMINI-TEMPLATE-ERROR {var.nope}"})

    def test_frame_placeholder_without_frame(self):
        out, bad = sc.render({"call": "c", "args": {"x": "{frame.id}"}}, {}, None)
        self.assertEqual(bad, "{frame.id}")
        self.assertEqual(out["text"], "FAKE-GEMINI-TEMPLATE-ERROR {frame.id}")

    def test_unknown_frame_key_is_unresolved(self):
        out, bad = sc.render({"text": "{frame.marker}"}, {}, self.FRAME)
        self.assertEqual(bad, "{frame.marker}")

    def test_plain_braces_left_alone(self):
        out, bad = sc.render({"text": '{"a": 1} {x}'}, {}, None)
        self.assertIsNone(bad)
        self.assertEqual(out["text"], '{"a": 1} {x}')


class BodyHelperTests(unittest.TestCase):
    def test_testdata_extraction(self):
        body = load_testdata()
        self.assertEqual(sc.system_messages(body), ["SYS-PROBE"])
        self.assertEqual(sc.user_requests(body), ["hello"])
        self.assertEqual(sc.roles(body), ["user", "user"])

    def test_system_instruction_prose_not_matched(self):
        body = load_testdata()
        self.assertIn("<USER_REQUEST> tags", json.dumps(body["systemInstruction"]))
        self.assertNotIn("<USER_REQUEST> tags", sc.request_texts(body))
        self.assertEqual(len(sc.user_requests(body)), 1)

    def test_system_message_without_hook_prefix_and_multiline(self):
        body = body_with("<SYSTEM_MESSAGE>\nline1\nline2\n</SYSTEM_MESSAGE>")
        self.assertEqual(sc.system_messages(body), ["line1\nline2"])

    def test_function_calls_and_responses(self):
        body = {"contents": [
            {"role": "model", "parts": [{"functionCall": {"name": "a", "args": {}, "id": "i1"}},
                                        {"text": "t"}]},
            {"role": "user", "parts": [{"functionResponse": {"name": "a", "id": "i1",
                                                              "response": {}}}]}]}
        self.assertEqual(sc.function_calls(body), [{"name": "a", "id": "i1"}])
        self.assertEqual(sc.function_responses(body), [{"name": "a", "id": "i1"}])
        self.assertEqual(sc.roles(body), ["model", "user"])

    def test_helpers_tolerate_garbage(self):
        for body in ({}, {"contents": "x"}, {"contents": [None, {"parts": 3}, {"parts": [1]}]}):
            self.assertEqual(sc.system_messages(body), [])
            self.assertEqual(sc.function_calls(body), [])
            self.assertEqual(sc.request_texts(body), "")


class ServerCase(unittest.TestCase):
    def setUp(self):
        self.ctl = tempfile.mkdtemp(prefix="afg.")
        self.addCleanup(shutil.rmtree, self.ctl, True)
        self.srv = srvmod.make_server(self.ctl)
        self.thread = threading.Thread(target=self.srv.serve_forever, args=(0.02,), daemon=True)
        self.thread.start()
        self.addCleanup(self._stop)
        self.port = self.srv.server_port

    def _stop(self):
        self.srv.shutdown()
        self.srv.server_close()
        self.thread.join(5)

    # -- helpers
    def set_scenario(self, obj):
        p = os.path.join(self.ctl, "scenario.json")
        tmp = p + ".tmp"
        with open(tmp, "w") as fh:
            json.dump(obj, fh)
        os.replace(tmp, p)

    def request(self, method, path, body=None, raw=None):
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
            conn.request(method, path, body=data, headers={"Content-Type": "application/json"})
            resp = conn.getresponse()
            return resp.status, dict(resp.getheaders()), resp.read()
        finally:
            conn.close()

    def post(self, path, body=None, **kw):
        return self.request("POST", path, body if body is not None else body_with("hi"), **kw)

    def main_json(self, body=None):
        status, _, data = self.post(MAIN + ":generateContent", body)
        return status, json.loads(data)

    def log(self):
        p = os.path.join(self.ctl, "requests.jsonl")
        if not os.path.exists(p):
            return []
        with open(p) as fh:
            return [json.loads(line) for line in fh if line.strip()]

    def cursor(self):
        with open(os.path.join(self.ctl, "cursor.json")) as fh:
            return json.load(fh)


class RouteTests(ServerCase):
    def test_models_get(self):
        status, hdr, data = self.request("GET", "/v1beta/models?key=x")
        self.assertEqual(status, 200)
        names = [m["name"] for m in json.loads(data)["models"]]
        self.assertIn("models/gemini-3.8-flash", names)
        self.assertEqual(self.log()[0]["kind"], "models")
        self.assertIsNone(self.log()[0]["body"])

    def test_count_tokens(self):
        status, _, data = self.post(MAIN + ":countTokens")
        self.assertEqual((status, json.loads(data)), (200, {"totalTokens": 32}))
        rec = self.log()[0]
        self.assertEqual((rec["kind"], rec["model"]), ("count", "gemini-3.1-pro-preview"))

    def test_unknown_routes_404_and_unlogged(self):
        self.assertEqual(self.request("GET", "/nope")[0], 404)
        self.assertEqual(self.post("/v1beta/models/x:embedContent")[0], 404)
        self.assertEqual(self.log(), [])

    def test_aux_flash_lite_uses_aux_text_and_consumes_nothing(self):
        self.set_scenario(scn([step("s0")], aux_text="Gate title"))
        status, _, data = self.post(AUX + ":streamGenerateContent?alt=sse")
        self.assertEqual(status, 200)
        self.assertIn("Gate title", data.decode())
        rec = self.log()[0]
        self.assertEqual((rec["kind"], rec["step"], rec["unmatched"]), ("aux", None, False))
        status, js = self.main_json()
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["text"], "ok-s0")
        self.assertEqual(self.cursor()["consumed"], ["s0"])

    def test_aux_default_text(self):
        _, _, data = self.post(AUX + ":generateContent")
        self.assertEqual(json.loads(data)["candidates"][0]["content"]["parts"][0]["text"], "Fake title")


class ReplyKindTests(ServerCase):
    def test_text_envelope(self):
        self.set_scenario(scn([step("a", {"text": "hello"})]))
        status, js = self.main_json()
        self.assertEqual(status, 200)
        cand = js["candidates"][0]
        self.assertEqual(cand["content"], {"role": "model", "parts": [{"text": "hello"}]})
        self.assertEqual((cand["finishReason"], cand["index"]), ("STOP", 0))
        self.assertEqual(js["usageMetadata"], {"promptTokenCount": 32, "candidatesTokenCount": 2,
                                               "totalTokenCount": 34})
        self.assertEqual(js["modelVersion"], "fake")

    def test_default_reply_unmatched(self):
        status, js = self.main_json()
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["text"], "DONE-default")
        rec = self.log()[0]
        self.assertTrue(rec["unmatched"])
        self.assertIsNone(rec["step"])
        self.assertEqual(self.cursor()["unmatched"], 1)

    def test_call_without_id_gets_uuid4(self):
        self.set_scenario(scn([step("a", {"call": "run_command", "args": {"x": 1}})]))
        _, js = self.main_json()
        fc = js["candidates"][0]["content"]["parts"][0]["functionCall"]
        self.assertEqual((fc["name"], fc["args"]), ("run_command", {"x": 1}))
        self.assertRegex(fc["id"], UUID4)

    def test_call_with_id(self):
        self.set_scenario(scn([step("a", {"call": "c", "args": {}, "id": "fixed-1"})]))
        _, js = self.main_json()
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["functionCall"]["id"], "fixed-1")

    def test_calls_are_parallel_parts(self):
        self.set_scenario(scn([step("a", {"calls": [{"call": "one", "args": {}},
                                                    {"call": "two", "args": {"k": "v"}, "id": "x2"}]})]))
        _, js = self.main_json()
        parts = js["candidates"][0]["content"]["parts"]
        self.assertEqual([p["functionCall"]["name"] for p in parts], ["one", "two"])
        self.assertEqual(parts[1]["functionCall"]["id"], "x2")
        self.assertRegex(parts[0]["functionCall"]["id"], UUID4)

    def test_finish_override(self):
        self.set_scenario(scn([step("a", {"text": "x", "finish": "MAX_TOKENS"})]))
        _, js = self.main_json()
        self.assertEqual(js["candidates"][0]["finishReason"], "MAX_TOKENS")

    def test_error_reply_json_route(self):
        self.set_scenario(scn([step("a", {"error": {"http": 429, "status": "RESOURCE_EXHAUSTED",
                                                    "message": "slow down"}})]))
        status, js = self.main_json()
        self.assertEqual(status, 429)
        self.assertEqual(js, {"error": {"code": 429, "message": "slow down",
                                        "status": "RESOURCE_EXHAUSTED"}})

    def test_error_reply_stream_route(self):
        self.set_scenario(scn([step("a", {"error": {"http": 500, "status": "INTERNAL", "message": "boom"}})]))
        status, hdr, data = self.post(MAIN + ":streamGenerateContent?alt=sse")
        self.assertEqual(status, 500)
        self.assertEqual(json.loads(data)["error"]["status"], "INTERNAL")


class StreamTests(ServerCase):
    def stream(self, path=MAIN + ":streamGenerateContent?alt=sse", body=None):
        t0 = time.monotonic()
        status, hdr, data = self.post(path, body)
        return status, hdr, data, time.monotonic() - t0

    def test_sse_framing_exact(self):
        self.set_scenario(scn([step("a", {"text": "hi"})]))
        status, hdr, data, _ = self.stream()
        self.assertEqual(status, 200)
        self.assertEqual(hdr["Content-Type"], "text/event-stream")
        self.assertNotIn("Content-Length", hdr)
        self.assertEqual(hdr["Connection"], "close")
        self.assertTrue(data.startswith(b"data: {"))
        self.assertTrue(data.endswith(b"}\r\n\r\n"))
        self.assertEqual(data.count(b"\r\n\r\n"), 1)
        ev = json.loads(data[len(b"data: "):-4])
        self.assertEqual(ev["candidates"][0]["content"]["parts"][0]["text"], "hi")
        self.assertEqual(ev["candidates"][0]["finishReason"], "STOP")

    def test_chunks_in_order_interval_and_last_carries_finish(self):
        self.set_scenario(scn([step("a", {"text": "abcdefgh", "chunks": 3, "chunk_interval_ms": 100})]))
        status, _, data, elapsed = self.stream()
        events = [json.loads(e[len("data: "):]) for e in data.decode().split("\r\n\r\n") if e]
        self.assertEqual(len(events), 3)
        texts = [e["candidates"][0]["content"]["parts"][0]["text"] for e in events]
        self.assertEqual("".join(texts), "abcdefgh")
        self.assertTrue(all(texts))
        self.assertEqual([len(t) for t in texts], [3, 3, 2])
        self.assertGreaterEqual(elapsed, 0.2)
        self.assertEqual(["finishReason" in e["candidates"][0] for e in events], [False, False, True])
        self.assertEqual(["usageMetadata" in e for e in events], [False, False, True])

    def test_chunks_more_than_chars_stay_non_empty(self):
        self.set_scenario(scn([step("a", {"text": "ab", "chunks": 5})]))
        _, _, data, _ = self.stream()
        events = [json.loads(e[len("data: "):]) for e in data.decode().split("\r\n\r\n") if e]
        self.assertEqual([e["candidates"][0]["content"]["parts"][0]["text"] for e in events], ["a", "b"])

    def test_chunks_on_json_route_concatenate(self):
        self.set_scenario(scn([step("a", {"text": "abcd", "chunks": 2})]))
        _, js = self.main_json()
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["text"], "abcd")
        self.assertEqual(js["candidates"][0]["finishReason"], "STOP")

    def test_call_streams_as_one_event(self):
        self.set_scenario(scn([step("a", {"call": "c", "args": {}})]))
        _, _, data, _ = self.stream()
        self.assertEqual(data.count(b"data: "), 1)
        self.assertIn(b"functionCall", data)


class DelayAndLogTests(ServerCase):
    def test_record_written_before_delay(self):
        self.set_scenario(scn([step("slow", delay_ms=800)]))
        result = {}

        def go():
            t0 = time.monotonic()
            result["resp"] = self.post(MAIN + ":generateContent", body_with("USER ask"))
            result["elapsed"] = time.monotonic() - t0

        th = threading.Thread(target=go)
        th.start()
        deadline = time.monotonic() + 5
        while not self.log() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(th.is_alive(), "reply must still be held")
        recs = self.log()
        self.assertEqual(len(recs), 1)
        self.assertEqual(recs[0]["step"], "slow")
        self.assertTrue(os.path.exists(os.path.join(self.ctl, recs[0]["body"])))
        th.join(10)
        self.assertEqual(result["resp"][0], 200)
        self.assertGreaterEqual(result["elapsed"], 0.75)

    def test_record_schema_and_seq(self):
        self.set_scenario(scn([step("a"), step("b")]))
        body = load_testdata()
        self.post(MAIN + ":streamGenerateContent?alt=sse", body)
        self.post(AUX + ":streamGenerateContent?alt=sse", body)
        self.request("GET", "/v1beta/models")
        recs = self.log()
        self.assertEqual([r["seq"] for r in recs], [1, 2, 3])
        self.assertEqual([r["kind"] for r in recs], ["main", "aux", "models"])
        r = recs[0]
        for key in ("seq", "t", "path", "model", "kind", "step", "unmatched", "template_error", "roles",
                    "system_messages", "user_requests", "function_calls", "function_responses", "body"):
            self.assertIn(key, r)
        self.assertEqual(r["path"], MAIN + ":streamGenerateContent")
        self.assertEqual(r["model"], "gemini-3.1-pro-preview")
        self.assertEqual(r["system_messages"], ["SYS-PROBE"])
        self.assertEqual(r["user_requests"], ["hello"])
        self.assertEqual(r["roles"], ["user", "user"])
        self.assertEqual(r["body"], "bodies/000001.json")
        with open(os.path.join(self.ctl, r["body"])) as fh:
            self.assertEqual(json.load(fh), body)
        self.assertEqual(self.cursor(), {"scenario": "t", "consumed": ["a"], "remaining": ["b"],
                                         "unmatched": 0})

    def test_function_activity_logged(self):
        body = {"contents": [
            {"role": "model", "parts": [{"functionCall": {"name": "f", "args": {}, "id": "c1"}}]},
            {"role": "user", "parts": [{"functionResponse": {"name": "f", "id": "c1", "response": {}}}]}]}
        self.main_json(body)
        r = self.log()[0]
        self.assertEqual(r["function_calls"], [{"name": "f", "id": "c1"}])
        self.assertEqual(r["function_responses"], [{"name": "f", "id": "c1"}])

    def test_body_cap(self):
        big = {"contents": [{"role": "user", "parts": [{"text": "x" * (1024 * 1024 + 500)}]}]}
        status, _ = self.main_json(big)
        self.assertEqual(status, 200)
        r = self.log()[0]
        self.assertTrue(r["body_truncated"])
        self.assertEqual(os.path.getsize(os.path.join(self.ctl, r["body"])), 1024 * 1024)

    def test_small_body_not_flagged(self):
        self.main_json()
        self.assertNotIn("body_truncated", self.log()[0])

    def test_non_json_body_does_not_crash(self):
        status, _, data = self.post(MAIN + ":generateContent", raw=b"\xff not json")
        self.assertEqual(status, 200)
        self.assertEqual(self.log()[0]["roles"], [])


class TemplateAndRoutingTests(ServerCase):
    def test_template_error_recorded_and_replied(self):
        self.set_scenario(scn([step("a", {"call": "c", "args": {"id": "{frame.id}"}})]))
        status, js = self.main_json(body_with("no frame header here"))
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["text"],
                         "FAKE-GEMINI-TEMPLATE-ERROR {frame.id}")
        r = self.log()[0]
        self.assertEqual((r["template_error"], r["step"]), ("{frame.id}", "a"))

    def test_templates_resolve_from_frame_and_vars(self):
        self.set_scenario(scn(
            [step("a", {"call": "run", "args": {"cmd": "ack {frame.id} {frame.thread} {var.cwd}"}},
                  bind={"marker": "ack-required"})], vars={"cwd": "/w"}))
        _, js = self.main_json(body_with(HDR2))
        args = js["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]
        self.assertEqual(args["cmd"], "ack m1 t-alpha /w")
        self.assertIsNone(self.log()[0]["template_error"])

    def test_default_bind_is_last_frame(self):
        self.set_scenario(scn([step("a", {"text": "{frame.id}"})]))
        _, js = self.main_json(body_with(HDR2))
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["text"], "m2")

    def test_parent_child_routing_over_http(self):
        self.set_scenario(scn([
            step("p0", match={"not_contains": ["CHILD-PROMPT"]}),
            step("c0", match={"contains": ["CHILD-PROMPT"]})]))
        self.main_json(body_with("CHILD-PROMPT do it"))
        self.main_json(body_with("parent"))
        self.assertEqual([r["step"] for r in self.log()], ["c0", "p0"])

    def test_system_instruction_not_used_for_matching(self):
        self.set_scenario(scn([step("a", match={"contains": ["<USER_REQUEST> tags"]})]))
        self.main_json(load_testdata())
        r = self.log()[0]
        self.assertTrue(r["unmatched"])


class HotSwapTests(ServerCase):
    def test_hot_swap_resets_cursor(self):
        self.set_scenario(scn([step("a0"), step("a1")]))
        self.main_json()
        self.assertEqual(self.cursor()["consumed"], ["a0"])
        self.set_scenario({**scn([step("b0", {"text": "from-b"})]), "name": "B"})
        _, js = self.main_json()
        self.assertEqual(js["candidates"][0]["content"]["parts"][0]["text"], "from-b")
        self.assertEqual(self.cursor(), {"scenario": "B", "consumed": ["b0"], "remaining": [],
                                         "unmatched": 0})
        self.assertEqual([r["step"] for r in self.log()], ["a0", "b0"])

    def test_unchanged_file_keeps_cursor(self):
        self.set_scenario(scn([step("a0"), step("a1")]))
        self.main_json()
        self.main_json()
        self.assertEqual([r["step"] for r in self.log()], ["a0", "a1"])

    def test_scenario_removed_goes_back_to_default(self):
        self.set_scenario(scn([step("a0")]))
        self.main_json()
        os.unlink(os.path.join(self.ctl, "scenario.json"))
        self.main_json()
        self.assertTrue(self.log()[1]["unmatched"])

    def test_invalid_scenario_is_http_500_and_logged(self):
        self.set_scenario({"format": "wrong"})
        status, js = self.main_json()
        self.assertEqual(status, 500)
        self.assertTrue(js["error"]["message"].startswith("FAKE-GEMINI-SCENARIO-ERROR $.format"))
        r = self.log()[0]
        self.assertIn("$.format", r["scenario_error"])
        self.assertIsNone(r["step"])
        # fixing the file recovers
        self.set_scenario(scn([step("ok")]))
        status, js = self.main_json()
        self.assertEqual(status, 200)
        self.assertNotIn("scenario_error", self.log()[1])

    def test_unparseable_scenario_file(self):
        with open(os.path.join(self.ctl, "scenario.json"), "w") as fh:
            fh.write("{broken")
        status, js = self.main_json()
        self.assertEqual(status, 500)
        self.assertIn("FAKE-GEMINI-SCENARIO-ERROR", js["error"]["message"])

    def test_aux_text_picked_up_without_prior_main(self):
        self.set_scenario(scn([], aux_text="Fresh"))
        _, _, data = self.post(AUX + ":generateContent")
        self.assertIn("Fresh", data.decode())


class SubprocessCase(unittest.TestCase):
    def setUp(self):
        self.ctl = tempfile.mkdtemp(prefix="afg.")
        self.addCleanup(shutil.rmtree, self.ctl, True)
        self.procs = []

    def spawn(self, *extra, **kw):
        p = subprocess.Popen([sys.executable, SERVER, "--ctl", self.ctl] + list(extra),
                             start_new_session=True, stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, **kw)
        self.procs.append(p)
        self.addCleanup(self._kill, p)
        return p

    @staticmethod
    def _kill(p):
        if p.poll() is None:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except OSError:
                pass
        p.wait()
        if p.stderr:
            p.stderr.close()

    def wait_ready(self, timeout=10):
        deadline = time.monotonic() + timeout
        while not os.path.exists(os.path.join(self.ctl, "ready")):
            if time.monotonic() > deadline:
                self.fail("server never became ready")
            time.sleep(0.02)

    def wait_exit(self, p, timeout):
        t0 = time.monotonic()
        try:
            rc = p.wait(timeout)
        except subprocess.TimeoutExpired:
            self.fail("process still alive after %.1fs" % timeout)
        return rc, time.monotonic() - t0


class ProcessTests(SubprocessCase):
    def test_port_then_ready_and_serves(self):
        p = self.spawn()
        self.wait_ready()
        with open(os.path.join(self.ctl, "port")) as fh:
            port = int(fh.read())
        self.assertGreater(port, 0)
        self.assertTrue(os.path.isdir(os.path.join(self.ctl, "bodies")))
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        conn.request("POST", MAIN + ":countTokens", body=b"{}")
        self.assertEqual(json.loads(conn.getresponse().read()), {"totalTokens": 32})
        conn.close()
        self.assertIsNone(p.poll())

    def test_binds_loopback_only(self):
        self.spawn()
        self.wait_ready()
        with open(os.path.join(self.ctl, "port")) as fh:
            port = int(fh.read())
        out = subprocess.run(["lsof", "-nP", "-iTCP:%d" % port, "-sTCP:LISTEN"],
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=20,
                             universal_newlines=True).stdout
        if not out.strip():
            self.skipTest("lsof unavailable")
        self.assertIn("127.0.0.1:%d" % port, out)
        self.assertNotIn("*:%d" % port, out)

    def test_sigterm_exits_zero(self):
        p = self.spawn()
        self.wait_ready()
        p.send_signal(signal.SIGTERM)
        rc, _ = self.wait_exit(p, 2)
        self.assertEqual(rc, 0)

    def test_owner_pid_death_stops_server(self):
        owner = subprocess.Popen(["sleep", "30"], start_new_session=True)
        self.addCleanup(self._kill_owner, owner)
        p = self.spawn("--owner-pid", str(owner.pid))
        self.wait_ready()
        time.sleep(0.5)
        self.assertIsNone(p.poll(), "server must outlive a live owner")
        owner.kill()
        owner.wait()
        rc, elapsed = self.wait_exit(p, 3)
        self.assertEqual(rc, 0)
        self.assertLess(elapsed, 1.5)

    @staticmethod
    def _kill_owner(owner):
        if owner.poll() is None:
            owner.kill()
        owner.wait()

    def test_non_loopback_host_refused(self):
        p = self.spawn("--host", "0.0.0.0")
        rc, _ = self.wait_exit(p, 5)
        self.assertEqual(rc, 2)
        self.assertIn("loopback", p.stderr.read().decode())
        self.assertFalse(os.path.exists(os.path.join(self.ctl, "ready")))

    def test_stale_ready_is_removed_on_start(self):
        ready_file = os.path.join(self.ctl, "ready")
        port_file = os.path.join(self.ctl, "port")
        open(ready_file, "w").close()
        past = time.time() - 3600
        os.utime(ready_file, (past, past))
        with open(port_file, "w") as fh:
            fh.write("1")
        self.spawn()
        deadline = time.monotonic() + 10
        while True:
            # A ready file left over from an earlier run keeps its old mtime.
            if os.path.exists(ready_file) and os.stat(ready_file).st_mtime > past + 60:
                break
            if time.monotonic() > deadline:
                self.fail("server never wrote a fresh ready file")
            time.sleep(0.005)
        with open(port_file) as fh:
            self.assertNotEqual(fh.read(), "1")


if __name__ == "__main__":
    unittest.main()
