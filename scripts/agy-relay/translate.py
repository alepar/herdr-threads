"""Gemini <-> OpenAI chat-completions translation for the agy relay (spec D9).

Pure functions and one small stateful stream translator. No I/O, no globals
mutated, stdlib only, Python 3.9 compatible.
"""
from __future__ import annotations

import hashlib
import json
import re
import uuid

MAX_TOKENS_CAP = 1024
_VALID_ID = re.compile(r"^[A-Za-z0-9_-]{1,64}$")

_FINISH_MAP = {
    "stop": "STOP",
    "tool_calls": "STOP",
    "function_call": "STOP",
    "length": "MAX_TOKENS",
    "content_filter": "SAFETY",
}
_MODE_MAP = {"AUTO": "auto", "ANY": "required", "NONE": "none"}


def _sha_alias(key):
    return "ht_" + hashlib.sha256(key.encode("utf-8")).hexdigest()[:24]


# ---------------------------------------------------------------------------
# Gemini request -> OpenAI request
# ---------------------------------------------------------------------------


def lower_schema_types(node):
    """Return a copy of a schema with every ``type`` value lower-cased."""
    if isinstance(node, dict):
        out = {}
        for key, value in node.items():
            if key == "type" and isinstance(value, str):
                out[key] = value.lower()
            elif key == "type" and isinstance(value, list) and all(isinstance(v, str) for v in value):
                out[key] = [v.lower() for v in value]
            else:
                out[key] = lower_schema_types(value)
        return out
    if isinstance(node, list):
        return [lower_schema_types(v) for v in node]
    return node


def _convert_tools(body):
    tools = []
    for tool in body.get("tools") or []:
        decls = tool.get("functionDeclarations") or tool.get("function_declarations") or []
        for decl in decls:
            fn = {"name": decl.get("name", "")}
            if "description" in decl:
                fn["description"] = decl["description"]
            if "parametersJsonSchema" in decl:
                fn["parameters"] = decl["parametersJsonSchema"]
            elif "parameters" in decl:
                fn["parameters"] = lower_schema_types(decl["parameters"])
            tools.append({"type": "function", "function": fn})
    return tools


def _plan_call_ids(contents, new_alias):
    """Assign an OpenAI id to every functionCall, in request order."""
    calls = []
    for content in contents:
        for part in content.get("parts") or []:
            if isinstance(part, dict) and isinstance(part.get("functionCall"), dict):
                calls.append(part["functionCall"])
    counts = {}
    for call in calls:
        cid = call.get("id")
        if isinstance(cid, str) and cid:
            counts[cid] = counts.get(cid, 0) + 1
    used = set()
    seen = {}
    plan = []
    for position, call in enumerate(calls):
        cid = call.get("id")
        name = call.get("name", "")
        if isinstance(cid, str) and cid:
            if _VALID_ID.match(cid) and counts[cid] == 1:
                out = cid
            else:
                n = seen.get(cid, 0)
                seen[cid] = n + 1
                out = new_alias(cid if n == 0 else "%s#%d" % (cid, n))
        else:
            out = new_alias("%d:%s" % (position, name))
        while out in used:  # keep the emitted ids unique whatever the input
            out = new_alias(out + "+")
        used.add(out)
        plan.append(out)
    return plan


def gemini_to_openai(body, new_alias=None):
    """Translate a Gemini generateContent body to OpenAI chat-completions fields."""
    new_alias = new_alias or _sha_alias
    contents = [c for c in (body.get("contents") or []) if isinstance(c, dict)]
    call_ids = _plan_call_ids(contents, new_alias)
    call_iter = iter(call_ids)

    messages = []
    pending = []  # unanswered calls: dicts {orig, oai, name, msg}
    tool_count = {}  # id(assistant message) -> tool messages already placed after it

    system = body.get("systemInstruction") or body.get("system_instruction")
    if isinstance(system, dict):
        texts = [p["text"] for p in system.get("parts") or []
                 if isinstance(p, dict) and isinstance(p.get("text"), str)]
        joined = "\n\n".join(texts)
        if joined:
            messages.append({"role": "system", "content": joined})

    def push_text(role, text):
        last = messages[-1] if messages else None
        if last is not None and last["role"] == role:
            last["_texts"].append(text)
            return last
        msg = {"role": role, "_texts": [text]}
        if role == "assistant":
            msg["tool_calls"] = []
        messages.append(msg)
        return msg

    def push_assistant_call(call):
        last = messages[-1] if messages else None
        if last is None or last["role"] != "assistant":
            last = {"role": "assistant", "_texts": [], "tool_calls": []}
            messages.append(last)
        return last

    def place_tool(assistant_msg, tool_msg):
        idx = next(i for i, m in enumerate(messages) if m is assistant_msg)
        n = tool_count.get(id(assistant_msg), 0)
        messages.insert(idx + 1 + n, tool_msg)
        tool_count[id(assistant_msg)] = n + 1

    for content in contents:
        role = "assistant" if content.get("role") == "model" else "user"
        for part in content.get("parts") or []:
            if not isinstance(part, dict) or part.get("thought") is True:
                continue
            if isinstance(part.get("functionCall"), dict):
                call = part["functionCall"]
                oai_id = next(call_iter)
                msg = push_assistant_call(call)
                args = call.get("args")
                msg["tool_calls"].append({
                    "id": oai_id,
                    "type": "function",
                    "function": {"name": call.get("name", ""),
                                 "arguments": json.dumps(args if args is not None else {})},
                })
                orig = call.get("id") if isinstance(call.get("id"), str) and call.get("id") else None
                pending.append({"orig": orig, "oai": oai_id, "name": call.get("name", ""), "msg": msg})
            elif isinstance(part.get("functionResponse"), dict):
                resp = part["functionResponse"]
                name = resp.get("name", "")
                rid = resp.get("id") if isinstance(resp.get("id"), str) and resp.get("id") else None
                match = None
                for entry in pending:
                    if (rid is not None and entry["orig"] == rid) or (rid is None and entry["name"] == name):
                        match = entry
                        break
                payload = json.dumps(resp.get("response") if resp.get("response") is not None else {})
                if match is None:
                    push_text("user", "[agy-relay] unpaired function response %s: %s" % (name, payload))
                else:
                    pending.remove(match)
                    place_tool(match["msg"], {"role": "tool", "tool_call_id": match["oai"],
                                              "content": payload})
            elif isinstance(part.get("text"), str):
                if part["text"] != "":
                    push_text(role, part["text"])
            # every other part kind (inlineData, thoughtSignature-only parts, ...) is dropped

    for msg in messages:
        if "_texts" in msg:
            texts = msg.pop("_texts")
            msg["content"] = "\n\n".join(texts) if texts else None
        if msg.get("role") == "assistant" and not msg.get("tool_calls"):
            msg.pop("tool_calls", None)
    messages = [m for m in messages if not (m["role"] == "assistant" and m.get("content") is None
                                            and not m.get("tool_calls"))]

    out = {"messages": messages}
    gen = body.get("generationConfig") or body.get("generation_config") or {}
    max_out = gen.get("maxOutputTokens")
    if isinstance(max_out, int) and not isinstance(max_out, bool) and max_out > 0:
        out["max_tokens"] = min(max_out, MAX_TOKENS_CAP)
    else:
        out["max_tokens"] = MAX_TOKENS_CAP
    if "temperature" in gen:
        out["temperature"] = gen["temperature"]
    if "topP" in gen:
        out["top_p"] = gen["topP"]
    if gen.get("stopSequences"):
        out["stop"] = gen["stopSequences"]

    tools = _convert_tools(body)
    if tools:
        out["tools"] = tools
    cfg = body.get("toolConfig") or body.get("tool_config") or {}
    fcc = cfg.get("functionCallingConfig") or cfg.get("function_calling_config") or {}
    mode = fcc.get("mode")
    if isinstance(mode, str) and mode.upper() in _MODE_MAP:
        out["tool_choice"] = _MODE_MAP[mode.upper()]
    return out


# ---------------------------------------------------------------------------
# OpenAI stream -> Gemini events
# ---------------------------------------------------------------------------


def parse_sse(lines):
    """Yield parsed ``data:`` JSON objects and the string ``"[DONE]"``."""
    for raw in lines:
        line = raw.decode("utf-8", "replace") if isinstance(raw, (bytes, bytearray)) else raw
        line = line.rstrip("\r\n")
        if not line or line.startswith(":"):
            continue
        if not line.startswith("data:"):
            continue
        data = line[5:]
        if data.startswith(" "):
            data = data[1:]
        data = data.strip()
        if data == "[DONE]":
            yield "[DONE]"
            continue
        try:
            obj = json.loads(data)
        except ValueError:
            continue
        if isinstance(obj, dict):
            yield obj


def _default_new_id():
    return "ht-" + str(uuid.uuid4())


def _text_event(text):
    return {"candidates": [{"content": {"role": "model", "parts": [{"text": text}]}, "index": 0}]}


class StreamTranslator(object):
    """Turns OpenAI chat.completion.chunk dicts into Gemini stream events."""

    def __init__(self, new_id=_default_new_id):
        self._new_id = new_id
        self._calls = {}  # index -> {"name": str, "args": str}
        self._calls_emitted = False
        self._finish = None
        self._usage = None

    def usage(self):
        return self._usage

    def _flush_calls(self):
        if self._calls_emitted or not self._calls:
            return []
        self._calls_emitted = True
        parts = []
        for index in sorted(self._calls):
            call = self._calls[index]
            raw = call["args"].strip()
            try:
                args = json.loads(raw) if raw else {}
            except ValueError:
                args = {}
            if not isinstance(args, dict):
                args = {}
            parts.append({"functionCall": {"name": call["name"], "args": args, "id": self._new_id()}})
        return parts

    def feed(self, chunk):
        events = []
        if not isinstance(chunk, dict):
            return events
        if isinstance(chunk.get("usage"), dict):
            self._usage = chunk["usage"]
        for choice in chunk.get("choices") or []:
            if not isinstance(choice, dict) or choice.get("index", 0) != 0:
                continue
            delta = choice.get("delta") or {}
            text = delta.get("content")
            if isinstance(text, str) and text:
                events.append(_text_event(text))
            for tc in delta.get("tool_calls") or []:
                if not isinstance(tc, dict):
                    continue
                slot = self._calls.setdefault(tc.get("index", 0), {"name": "", "args": ""})
                fn = tc.get("function") or {}
                if isinstance(fn.get("name"), str) and fn["name"] and not slot["name"]:
                    slot["name"] = fn["name"]
                if isinstance(fn.get("arguments"), str):
                    slot["args"] += fn["arguments"]
            reason = choice.get("finish_reason")
            if reason and self._finish is None:
                self._finish = reason
                parts = self._flush_calls()
                if parts:
                    events.append({"candidates": [{"content": {"role": "model", "parts": parts},
                                                   "index": 0}]})
        return events

    def finish(self):
        parts = self._flush_calls()
        content = {"role": "model", "parts": parts or [{"text": ""}]}
        reason = _FINISH_MAP.get(self._finish or "stop", "STOP")
        event = {"candidates": [{"content": content, "finishReason": reason, "index": 0}]}
        usage = self._usage
        if isinstance(usage, dict):
            prompt = int(usage.get("prompt_tokens") or 0)
            completion = int(usage.get("completion_tokens") or 0)
            total = usage.get("total_tokens")
            event["usageMetadata"] = {
                "promptTokenCount": prompt,
                "candidatesTokenCount": completion,
                "totalTokenCount": int(total) if total is not None else prompt + completion,
            }
        return [event]


def aggregate(events):
    """One non-streaming Gemini response from a list of stream events."""
    parts = []
    finish = None
    usage = None
    for event in events:
        for cand in event.get("candidates") or []:
            for part in (cand.get("content") or {}).get("parts") or []:
                if "text" in part and parts and "text" in parts[-1] and len(parts[-1]) == 1 \
                        and len(part) == 1:
                    parts[-1] = {"text": parts[-1]["text"] + part["text"]}
                else:
                    parts.append(dict(part))
            if cand.get("finishReason"):
                finish = cand["finishReason"]
        if event.get("usageMetadata"):
            usage = event["usageMetadata"]
    if not parts:
        parts = [{"text": ""}]
    cand = {"content": {"role": "model", "parts": parts}, "index": 0}
    if finish:
        cand["finishReason"] = finish
    out = {"candidates": [cand]}
    if usage:
        out["usageMetadata"] = usage
    return out


# ---------------------------------------------------------------------------
# Errors and local routes
# ---------------------------------------------------------------------------


def _grpc_status(status):
    if status == 400:
        return "INVALID_ARGUMENT"
    if status in (401, 403):
        return "PERMISSION_DENIED"
    if status == 404:
        return "NOT_FOUND"
    if status == 429:
        return "RESOURCE_EXHAUSTED"
    if 400 <= status < 500:
        return "FAILED_PRECONDITION"
    return "UNAVAILABLE"


def gemini_error(http_status, message, status=None):
    return http_status, {"error": {"code": http_status, "message": message,
                                   "status": status or _grpc_status(http_status)}}


def map_error(status, body_bytes):
    """Map an upstream HTTP error to (http_status, Gemini error body)."""
    message = ""
    try:
        parsed = json.loads(body_bytes.decode("utf-8", "replace"))
        err = parsed.get("error") if isinstance(parsed, dict) else None
        if isinstance(err, dict):
            message = str(err.get("message") or "")
        elif isinstance(err, str):
            message = err
    except ValueError:
        pass
    if not message:
        message = body_bytes.decode("utf-8", "replace").strip()[:500] or "upstream error %d" % status
    http_status = status if status >= 400 else 502
    return gemini_error(http_status, message)


def models_response():
    names = ["gemini-3.8-flash", "gemini-3.1-flash-lite-preview"]
    return {"models": [{"name": "models/" + n, "displayName": n,
                        "supportedGenerationMethods": ["generateContent", "streamGenerateContent",
                                                       "countTokens"]} for n in names]}


def count_tokens(raw_body):
    return {"totalTokens": len(raw_body) // 4}


def aux_response(text="agy smoke"):
    event = _text_event(text)
    event["candidates"][0]["finishReason"] = "STOP"
    event["usageMetadata"] = {"promptTokenCount": 0, "candidatesTokenCount": 0, "totalTokenCount": 0}
    return event


def is_aux(model):
    return "flash-lite" in (model or "")
