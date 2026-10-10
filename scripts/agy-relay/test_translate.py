"""Offline tests for translate.py (pure functions only)."""
from __future__ import annotations

import copy
import json
import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import translate  # noqa: E402

TESTDATA = os.path.join(HERE, "testdata")


def load(name):
    with open(os.path.join(TESTDATA, name)) as fh:
        return json.load(fh)


def sse_lines(name):
    with open(os.path.join(TESTDATA, name), "rb") as fh:
        return fh.read().splitlines(keepends=True)


def call_turn(cid, name="run_command", args=None):
    fc = {"name": name, "args": args or {"CommandLine": "echo x"}}
    if cid is not None:
        fc["id"] = cid
    return {"role": "model", "parts": [{"functionCall": fc}]}


def resp_turn(cid, name="run_command", role="model", response=None):
    fr = {"name": name, "response": response or {"output": "ok"}}
    if cid is not None:
        fr["id"] = cid
    return {"role": role, "parts": [{"functionResponse": fr}]}


def user(text):
    return {"role": "user", "parts": [{"text": text}]}


class RequestTranslation(unittest.TestCase):
    def test_system_parts_joined(self):
        out = translate.gemini_to_openai({
            "systemInstruction": {"parts": [{"text": "A"}, {"text": "B"}]},
            "contents": [user("hi")]})
        self.assertEqual(out["messages"][0], {"role": "system", "content": "A\n\nB"})
        self.assertEqual(out["messages"][1], {"role": "user", "content": "hi"})

    def test_function_response_in_model_and_user_role_becomes_tool_message(self):
        for role in ("model", "user"):
            with self.subTest(role=role):
                out = translate.gemini_to_openai({"contents": [
                    user("go"), call_turn("c-1"), resp_turn("c-1", role=role)]})
                msgs = out["messages"]
                self.assertEqual([m["role"] for m in msgs], ["user", "assistant", "tool"])
                self.assertEqual(msgs[1]["tool_calls"][0]["id"], "c-1")
                self.assertEqual(msgs[2]["tool_call_id"], "c-1")
                self.assertEqual(json.loads(msgs[2]["content"]), {"output": "ok"})
                self.assertIsNone(msgs[1]["content"])

    def test_recorded_agy_turn_with_role_variants(self):
        reqs = load("agy-native-requests.json")
        body = {"contents": reqs["n03-A.req.json"]["contents"]}
        for variant in ("user", "model"):
            b = copy.deepcopy(body)
            b["contents"][2]["role"] = variant
            msgs = translate.gemini_to_openai(b)["messages"]
            self.assertEqual([m["role"] for m in msgs], ["user", "assistant", "tool"])
            self.assertEqual(msgs[2]["tool_call_id"], "3dbfab59-4678-4ae0-b4c1-fa820e049259")
            self.assertEqual(json.loads(msgs[1]["tool_calls"][0]["function"]["arguments"])["Cwd"], "/ROOT/ws")

    def test_recorded_resumed_turn_merges_consecutive_users(self):
        reqs = load("agy-native-requests.json")
        msgs = translate.gemini_to_openai({"contents": reqs["n04-B.req.json"]["contents"]})["messages"]
        self.assertEqual([m["role"] for m in msgs], ["user", "assistant", "tool", "assistant", "user"])
        self.assertIn("What did the previous command print?", msgs[4]["content"])
        self.assertIn("<SYSTEM_MESSAGE>", msgs[4]["content"])
        self.assertIn("</USER_REQUEST>\n<ADDITIONAL_METADATA>", msgs[4]["content"])
        self.assertEqual(msgs[3]["content"], "HT_TOOL_OK")

    def test_valid_unique_id_kept_verbatim(self):
        out = translate.gemini_to_openai({"contents": [call_turn("abc_DEF-123"), resp_turn("abc_DEF-123")]})
        self.assertEqual(out["messages"][0]["tool_calls"][0]["id"], "abc_DEF-123")

    def test_invalid_id_aliased_and_response_paired_through_alias(self):
        out = translate.gemini_to_openai({"contents": [call_turn("bad id!"), resp_turn("bad id!")]})
        cid = out["messages"][0]["tool_calls"][0]["id"]
        self.assertTrue(re.match(r"^ht_[0-9a-f]{24}$", cid), cid)
        self.assertEqual(out["messages"][1]["tool_call_id"], cid)

    def test_overlong_id_aliased(self):
        long_id = "a" * 65
        out = translate.gemini_to_openai({"contents": [call_turn(long_id), resp_turn(long_id)]})
        self.assertRegex(out["messages"][0]["tool_calls"][0]["id"], r"^ht_[0-9a-f]{24}$")

    def test_duplicate_ids_aliased_distinctly_and_paired_in_order(self):
        out = translate.gemini_to_openai({"contents": [
            call_turn("dup", args={"n": 1}), resp_turn("dup", response={"r": 1}),
            call_turn("dup", args={"n": 2}), resp_turn("dup", response={"r": 2})]})
        msgs = out["messages"]
        self.assertEqual([m["role"] for m in msgs], ["assistant", "tool", "assistant", "tool"])
        ids = [msgs[0]["tool_calls"][0]["id"], msgs[2]["tool_calls"][0]["id"]]
        self.assertNotEqual(ids[0], ids[1])
        for i in ids:
            self.assertRegex(i, r"^ht_[0-9a-f]{24}$")
        self.assertEqual(msgs[1]["tool_call_id"], ids[0])
        self.assertEqual(json.loads(msgs[1]["content"]), {"r": 1})
        self.assertEqual(msgs[3]["tool_call_id"], ids[1])
        self.assertEqual(json.loads(msgs[3]["content"]), {"r": 2})

    def test_no_id_call_gets_position_alias_and_pairs_by_name_earliest_first(self):
        out = translate.gemini_to_openai({"contents": [
            call_turn(None, "a"), call_turn(None, "b"), call_turn(None, "a"),
            resp_turn(None, "a", response={"r": "first"}), resp_turn(None, "b"),
            resp_turn(None, "a", response={"r": "second"})]})
        calls = [m for m in out["messages"] if m["role"] == "assistant"]
        # consecutive assistant turns merge into one message with three calls
        self.assertEqual(len(calls), 1)
        ids = [c["id"] for c in calls[0]["tool_calls"]]
        self.assertEqual(len(set(ids)), 3)
        for i in ids:
            self.assertRegex(i, r"^ht_[0-9a-f]{24}$")
        tools = [m for m in out["messages"] if m["role"] == "tool"]
        self.assertEqual([t["tool_call_id"] for t in tools], [ids[0], ids[1], ids[2]])
        self.assertEqual(json.loads(tools[0]["content"]), {"r": "first"})
        self.assertEqual(json.loads(tools[2]["content"]), {"r": "second"})
        # tool messages sit directly after the assistant message, never merged
        self.assertEqual([m["role"] for m in out["messages"]], ["assistant", "tool", "tool", "tool"])

    def test_no_id_alias_depends_on_position_and_name(self):
        out = translate.gemini_to_openai({"contents": [call_turn(None, "a"), resp_turn(None, "a")]})
        import hashlib
        expect = "ht_" + hashlib.sha256(b"0:a").hexdigest()[:24]
        self.assertEqual(out["messages"][0]["tool_calls"][0]["id"], expect)

    def test_unpaired_response_becomes_user_note(self):
        out = translate.gemini_to_openai({"contents": [user("hi"), resp_turn("ghost", response={"x": 1})]})
        roles = [m["role"] for m in out["messages"]]
        self.assertNotIn("tool", roles)
        note = out["messages"][-1]
        self.assertEqual(note["role"], "user")
        self.assertIn('[agy-relay] unpaired function response run_command: {"x": 1}', note["content"])

    def test_answered_call_is_not_reused(self):
        out = translate.gemini_to_openai({"contents": [
            call_turn("c1"), resp_turn("c1"), resp_turn("c1")]})
        self.assertEqual([m["role"] for m in out["messages"]], ["assistant", "tool", "user"])

    def test_thought_parts_and_signatures_dropped(self):
        out = translate.gemini_to_openai({"contents": [
            user("q"),
            {"role": "model", "parts": [
                {"text": "secret thinking", "thought": True},
                {"text": "answer", "thoughtSignature": "SIG"},
                {"functionCall": {"name": "f", "args": {}, "id": "i1"}, "thoughtSignature": "SIG2"}]}]})
        dumped = json.dumps(out)
        self.assertNotIn("secret thinking", dumped)
        self.assertNotIn("SIG", dumped)
        self.assertNotIn("thought", dumped)
        self.assertEqual(out["messages"][1]["content"], "answer")

    def test_mixed_parts_split_in_order(self):
        out = translate.gemini_to_openai({"contents": [
            user("start"), call_turn("c1"),
            {"role": "user", "parts": [
                {"functionResponse": {"id": "c1", "name": "run_command", "response": {"o": 1}}},
                {"text": "then this"}]}]})
        self.assertEqual([m["role"] for m in out["messages"]], ["user", "assistant", "tool", "user"])
        self.assertEqual(out["messages"][3]["content"], "then this")

    def test_text_then_call_in_one_model_content_is_one_assistant_message(self):
        out = translate.gemini_to_openai({"contents": [
            user("go"),
            {"role": "model", "parts": [{"text": "running"},
                                        {"functionCall": {"name": "f", "args": {"a": 1}, "id": "i"}}]}]})
        msg = out["messages"][1]
        self.assertEqual(msg["content"], "running")
        self.assertEqual(msg["tool_calls"][0]["function"], {"name": "f", "arguments": '{"a": 1}'})
        self.assertEqual(msg["tool_calls"][0]["type"], "function")

    def test_consecutive_same_role_merged_with_blank_line(self):
        out = translate.gemini_to_openai({"contents": [user("one"), user("two"),
                                                       {"role": "model", "parts": [{"text": "a"}]},
                                                       {"role": "model", "parts": [{"text": "b"}]}]})
        self.assertEqual(out["messages"], [{"role": "user", "content": "one\n\ntwo"},
                                           {"role": "assistant", "content": "a\n\nb"}])

    def test_schema_uppercase_types_lowered_recursively(self):
        out = translate.gemini_to_openai({"contents": [user("x")], "tools": [{"functionDeclarations": [{
            "name": "f", "description": "d",
            "parameters": {"type": "OBJECT", "properties": {
                "s": {"type": "STRING"},
                "type": {"type": "ARRAY", "items": {"type": "INTEGER"}}},
                "required": ["s"]}}]}]})
        params = out["tools"][0]["function"]["parameters"]
        self.assertEqual(params["type"], "object")
        self.assertEqual(params["properties"]["s"]["type"], "string")
        self.assertEqual(params["properties"]["type"]["type"], "array")
        self.assertEqual(params["properties"]["type"]["items"]["type"], "integer")
        self.assertEqual(params["required"], ["s"])
        self.assertEqual(out["tools"][0]["type"], "function")
        self.assertEqual(out["tools"][0]["function"]["description"], "d")

    def test_parameters_json_schema_passed_through_unchanged(self):
        decls = load("agy-tool-declarations.json")["selected"]
        body = {"contents": [user("x")], "tools": [{"functionDeclarations": list(decls.values())}]}
        out = translate.gemini_to_openai(body)
        self.assertEqual([t["function"]["name"] for t in out["tools"]], ["run_command", "send_message"])
        for tool, decl in zip(out["tools"], decls.values()):
            self.assertEqual(tool["function"]["parameters"], decl["parametersJsonSchema"])
            self.assertEqual(tool["function"]["description"], decl["description"])
        self.assertEqual(out["tools"][0]["function"]["parameters"]["type"], "object")

    def test_tool_config_modes(self):
        for mode, want in (("AUTO", "auto"), ("ANY", "required"), ("NONE", "none")):
            out = translate.gemini_to_openai({"contents": [user("x")],
                                              "toolConfig": {"functionCallingConfig": {"mode": mode}}})
            self.assertEqual(out["tool_choice"], want)
        self.assertNotIn("tool_choice", translate.gemini_to_openai({"contents": [user("x")]}))

    def test_generation_config_mapping(self):
        out = translate.gemini_to_openai({"contents": [user("x")], "generationConfig": {
            "temperature": 0.2, "topP": 0.9, "stopSequences": ["END"], "maxOutputTokens": 65536,
            "thinkingConfig": {"includeThoughts": True, "thinkingBudget": 4000}}})
        self.assertEqual((out["temperature"], out["top_p"], out["stop"]), (0.2, 0.9, ["END"]))
        self.assertEqual(out["max_tokens"], 1024)
        self.assertNotIn("thinking", json.dumps(out).lower())

    def test_max_tokens_below_cap_kept_and_default(self):
        out = translate.gemini_to_openai({"contents": [user("x")], "generationConfig": {"maxOutputTokens": 77}})
        self.assertEqual(out["max_tokens"], 77)
        self.assertEqual(translate.gemini_to_openai({"contents": [user("x")]})["max_tokens"], 1024)
        for key in ("temperature", "top_p", "stop", "tools", "tool_choice"):
            self.assertNotIn(key, translate.gemini_to_openai({"contents": [user("x")]}))

    def test_injectable_alias_generator(self):
        out = translate.gemini_to_openai({"contents": [call_turn("bad id!"), resp_turn("bad id!")]},
                                         new_alias=lambda key: "X" + str(len(key)))
        self.assertEqual(out["messages"][0]["tool_calls"][0]["id"], "X7")

    def test_input_not_mutated(self):
        body = {"contents": [user("a"), call_turn("bad id!"), resp_turn("bad id!")],
                "systemInstruction": {"parts": [{"text": "s"}]}}
        before = copy.deepcopy(body)
        translate.gemini_to_openai(body)
        self.assertEqual(body, before)


def feed_all(tr, name):
    events = []
    for item in translate.parse_sse(sse_lines(name)):
        if item == "[DONE]":
            break
        events += tr.feed(item)
    return events


class StreamTranslation(unittest.TestCase):
    def test_text_stream_events_in_order_and_final_usage(self):
        tr = translate.StreamTranslator()
        events = feed_all(tr, "openrouter-stream-text.sse")
        texts = [e["candidates"][0]["content"]["parts"][0]["text"] for e in events]
        self.assertEqual(texts, ["Shell", " Command Execution"])
        for e in events:
            self.assertEqual(e["candidates"][0]["content"]["role"], "model")
            self.assertNotIn("finishReason", e["candidates"][0])
        final = tr.finish()
        self.assertEqual(len(final), 1)
        self.assertEqual(final[0]["candidates"][0]["finishReason"], "STOP")
        self.assertEqual(final[0]["usageMetadata"], {
            "promptTokenCount": 138, "candidatesTokenCount": 3, "totalTokenCount": 141})
        self.assertEqual(tr.usage()["cost"], 0.0000489)

    def test_toolcall_stream_single_function_call_fresh_id(self):
        tr = translate.StreamTranslator()
        events = feed_all(tr, "openrouter-stream-toolcall.sse")
        calls = [p["functionCall"] for e in events for p in e["candidates"][0]["content"]["parts"]
                 if "functionCall" in p]
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["name"], "run_command")
        self.assertEqual(calls[0]["args"]["CommandLine"], "echo HT_TOOL_OK")
        self.assertEqual(calls[0]["args"]["WaitMsBeforeAsync"], 100)
        self.assertRegex(calls[0]["id"], r"^ht-[0-9a-f-]{36}$")
        self.assertNotIn("VaKfrm9pBpnZWCnC2dt7", json.dumps(events))
        final = tr.finish()
        self.assertEqual(final[0]["candidates"][0]["finishReason"], "STOP")
        self.assertNotIn("functionCall", json.dumps(final))  # emitted exactly once
        self.assertEqual(final[0]["usageMetadata"]["promptTokenCount"], 10355)

    def test_two_tool_calls_get_distinct_ids(self):
        tr = translate.StreamTranslator()
        events = []
        for chunk in [
            {"choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "id": "up1", "function": {"name": "a", "arguments": '{"x":'}},
                {"index": 1, "id": "up2", "function": {"name": "b", "arguments": ""}}]}}]},
            {"choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "1}"}}]}}]},
            {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]},
        ]:
            events += tr.feed(chunk)
        calls = [p["functionCall"] for e in events for p in e["candidates"][0]["content"]["parts"]]
        self.assertEqual([c["name"] for c in calls], ["a", "b"])
        self.assertEqual(calls[0]["args"], {"x": 1})
        self.assertEqual(calls[1]["args"], {})
        self.assertNotEqual(calls[0]["id"], calls[1]["id"])
        for c in calls:
            self.assertTrue(c["id"].startswith("ht-"))
            self.assertNotIn(c["id"], ("up1", "up2"))

    def test_injected_new_id(self):
        counter = iter(["id-1", "id-2"])
        tr = translate.StreamTranslator(new_id=lambda: next(counter))
        events = tr.feed({"choices": [{"index": 0, "delta": {"tool_calls": [
            {"index": 0, "function": {"name": "a", "arguments": ""}},
            {"index": 1, "function": {"name": "b", "arguments": ""}}]}, "finish_reason": "stop"}]})
        ids = [p["functionCall"]["id"] for p in events[0]["candidates"][0]["content"]["parts"]]
        self.assertEqual(ids, ["id-1", "id-2"])

    def test_finish_reason_map(self):
        for upstream, want in (("stop", "STOP"), ("tool_calls", "STOP"), ("length", "MAX_TOKENS"),
                               ("content_filter", "SAFETY")):
            tr = translate.StreamTranslator()
            tr.feed({"choices": [{"index": 0, "delta": {"content": "x"}, "finish_reason": upstream}]})
            self.assertEqual(tr.finish()[0]["candidates"][0]["finishReason"], want, upstream)

    def test_finish_without_usage_has_no_usage_metadata(self):
        tr = translate.StreamTranslator()
        tr.feed({"choices": [{"index": 0, "delta": {"content": "x"}, "finish_reason": "stop"}]})
        self.assertNotIn("usageMetadata", tr.finish()[0])
        self.assertIsNone(tr.usage())

    def test_unfinished_stream_still_flushes_tool_calls_in_finish(self):
        tr = translate.StreamTranslator()
        self.assertEqual(tr.feed({"choices": [{"index": 0, "delta": {"tool_calls": [
            {"index": 0, "function": {"name": "a", "arguments": "{}"}}]}}]}), [])
        final = tr.finish()
        self.assertEqual(final[0]["candidates"][0]["content"]["parts"][0]["functionCall"]["name"], "a")


class SseParsing(unittest.TestCase):
    def test_comments_blank_lines_ignored_done_yielded(self):
        lines = [b": OPENROUTER PROCESSING\n", b"\n", b'data: {"a": 1}\r\n', b"\r\n",
                 b"event: ignored\n", b"data:  [DONE]\n"]
        self.assertEqual(list(translate.parse_sse(lines)), [{"a": 1}, "[DONE]"])

    def test_recorded_stream_ends_with_done(self):
        items = list(translate.parse_sse(sse_lines("openrouter-stream-text.sse")))
        self.assertEqual(items[-1], "[DONE]")
        self.assertEqual(len(items), 5)

    def test_malformed_json_skipped(self):
        self.assertEqual(list(translate.parse_sse([b"data: {nope\n", b'data: {"ok": true}\n'])), [{"ok": True}])


class Aggregate(unittest.TestCase):
    def test_text_events_concatenated_with_finish_and_usage(self):
        tr = translate.StreamTranslator()
        events = feed_all(tr, "openrouter-stream-text.sse") + tr.finish()
        agg = translate.aggregate(events)
        cand = agg["candidates"][0]
        self.assertEqual(cand["content"]["parts"], [{"text": "Shell Command Execution"}])
        self.assertEqual(cand["finishReason"], "STOP")
        self.assertEqual(agg["usageMetadata"]["totalTokenCount"], 141)

    def test_function_call_part_survives(self):
        tr = translate.StreamTranslator()
        events = feed_all(tr, "openrouter-stream-toolcall.sse") + tr.finish()
        parts = translate.aggregate(events)["candidates"][0]["content"]["parts"]
        self.assertEqual([list(p) for p in parts if "functionCall" in p], [["functionCall"]])


class ErrorMapping(unittest.TestCase):
    def test_status_table(self):
        table = {400: "INVALID_ARGUMENT", 401: "PERMISSION_DENIED", 403: "PERMISSION_DENIED",
                 404: "NOT_FOUND", 429: "RESOURCE_EXHAUSTED", 402: "FAILED_PRECONDITION",
                 422: "FAILED_PRECONDITION", 500: "UNAVAILABLE", 502: "UNAVAILABLE", 503: "UNAVAILABLE"}
        for code, status in table.items():
            http, body = translate.map_error(code, json.dumps({"error": {"message": "m%d" % code}}).encode())
            self.assertEqual(http, code)
            self.assertEqual(body, {"error": {"code": code, "message": "m%d" % code, "status": status}})

    def test_non_json_body_used_as_message(self):
        http, body = translate.map_error(500, b"plain failure")
        self.assertEqual(body["error"]["message"], "plain failure")
        http, body = translate.map_error(500, b"")
        self.assertIn("500", body["error"]["message"])

    def test_string_error_field(self):
        _, body = translate.map_error(400, b'{"error": "bad"}')
        self.assertEqual(body["error"]["message"], "bad")

    def test_non_error_status_becomes_502(self):
        http, body = translate.map_error(302, b"")
        self.assertEqual((http, body["error"]["status"]), (502, "UNAVAILABLE"))


class LocalRoutes(unittest.TestCase):
    def test_count_tokens(self):
        self.assertEqual(translate.count_tokens(b"x" * 400), {"totalTokens": 100})
        self.assertEqual(translate.count_tokens(b"abc"), {"totalTokens": 0})

    def test_is_aux(self):
        self.assertTrue(translate.is_aux("gemini-3.1-flash-lite-preview"))
        self.assertFalse(translate.is_aux("gemini-3.8-flash"))
        self.assertFalse(translate.is_aux(""))
        self.assertFalse(translate.is_aux(None))

    def test_aux_and_models(self):
        aux = translate.aux_response()
        self.assertEqual(aux["candidates"][0]["content"]["parts"][0]["text"], "agy smoke")
        self.assertEqual(aux["candidates"][0]["finishReason"], "STOP")
        self.assertEqual(translate.aux_response("t")["candidates"][0]["content"]["parts"][0]["text"], "t")
        names = [m["name"] for m in translate.models_response()["models"]]
        self.assertIn("models/gemini-3.8-flash", names)


if __name__ == "__main__":
    unittest.main()
