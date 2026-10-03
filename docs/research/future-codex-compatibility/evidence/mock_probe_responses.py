#!/usr/bin/env python3
"""Throwaway mock of the OpenAI Responses API used ONLY because the real Codex account was at its
usage limit during the spike (see findings.md). It streams a deliberately slow assistant turn when
the newest user text contains SLOW (40 s of text deltas, 2 s apart) and a one-word answer otherwise,
and logs every request's user-role input texts so the client-side queue/steer behaviour of Codex
can be read off the request sequence.  usage: mock_responses.py <port> <logfile>"""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]); LOG = sys.argv[2]
N = [0]

def log(obj):
    with open(LOG, "a") as f:
        f.write(json.dumps(obj) + "\n")

def user_texts(inp):
    out = []
    for it in inp if isinstance(inp, list) else []:
        if it.get("type") == "message" and it.get("role") == "user":
            for c in it.get("content", []):
                if c.get("type") == "input_text":
                    out.append(c["text"])
    return out

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(404); self.end_headers()
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        if not self.path.endswith("/responses"):
            self.send_response(404); self.end_headers(); return
        req = json.loads(body)
        N[0] += 1; n = N[0]
        if req.get("tools"):
            with open(LOG + ".tools.json", "w") as f:
                json.dump([t for t in req["tools"] if (t.get("name") or "") in ("exec_command", "multi_agent_v1")], f, indent=1)
        texts = [t for t in user_texts(req.get("input")) if not t.startswith("<")]
        log({"t": time.time(), "req": n, "model": req.get("model"), "user_texts": texts,
             "tools": [t.get("name") or t.get("type") for t in req.get("tools", [])],
             "n_outputs": sum(1 for it in req.get("input", []) if it.get("type") == "function_call_output"),
             "tool_outputs": [str(it.get("output", ""))[:3000] for it in req.get("input", [])
                              if it.get("type") == "function_call_output"]})
        slow = bool(texts) and "SLOW" in texts[-1]
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        def ev(name, data):
            self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode()); self.wfile.flush()
        nout, in_tools = 0, False
        for it in req.get("input", []):
            if it.get("type") == "message" and it.get("role") == "user":
                if any(("TOOLS" in c.get("text", "") or c.get("text", "").startswith(("WORKERS", "CHILD")))
                       and not c.get("text", "").startswith("<") for c in it.get("content", [])):
                    in_tools = any("TOOLS" in c.get("text", "") for c in it.get("content", []))
                    nout = 0
            elif it.get("type") == "function_call_output":
                nout += 1
        newest = texts[-1] if texts else ""
        calls = []
        if newest.startswith("PROBE ") and nout == 0 and req.get("tools"):
            calls = [("", "exec_command", {"cmd": newest[6:], "yield_time_ms": 10000})]
        elif newest.startswith("CHILD") and nout == 0 and req.get("tools"):
            who = newest.split()[1]
            calls = [("", "exec_command", {"cmd": f"sleep 3; echo {who}", "yield_time_ms": 10000})]
        elif newest.startswith("WORKERS") and nout == 0 and req.get("tools"):
            calls = [("multi_agent_v1", "spawn_agent", {"message": f"CHILD {w} run the shell command: echo {w}", "model": "gpt-6-luna"}) for w in ("worker-a", "worker-b")]
        if calls:
            ev("response.created", {"type": "response.created", "response": {"id": f"resp_{n}"}})
            for i, (ns, name, args) in enumerate(calls):
                item = {"type": "function_call", "id": f"fc_{n}_{i}", "call_id": f"call_{n}_{i}", "name": name, "arguments": json.dumps(args)}
                if ns: item["namespace"] = ns
                ev("response.output_item.added", {"type": "response.output_item.added", "item": item})
                ev("response.output_item.done", {"type": "response.output_item.done", "item": item})
            ev("response.completed", {"type": "response.completed", "response": {"id": f"resp_{n}", "usage": {"input_tokens": 1, "input_tokens_details": None, "output_tokens": 1, "output_tokens_details": None, "total_tokens": 2}}})
            log({"t": time.time(), "req": n, "done": True, "emitted": [c[1] for c in calls]})
            return
        if in_tools and nout < 3 and req.get("tools"):
            # TOOLS scenario: three sequential exec_command calls (sleep 15 each), then a text reply
            ev("response.created", {"type": "response.created", "response": {"id": f"resp_{n}"}})
            item = {"type": "function_call", "id": f"fc_{n}", "call_id": f"call_{n}", "name": "exec_command",
                    "arguments": json.dumps({"cmd": f"sleep 15; echo step{nout + 1}", "yield_time_ms": 30000})}
            ev("response.output_item.added", {"type": "response.output_item.added", "item": item})
            ev("response.output_item.done", {"type": "response.output_item.done", "item": item})
            ev("response.completed", {"type": "response.completed", "response": {"id": f"resp_{n}", "usage": {"input_tokens": 1, "input_tokens_details": None, "output_tokens": 1, "output_tokens_details": None, "total_tokens": 2}}})
            log({"t": time.time(), "req": n, "done": True, "emitted": "function_call"})
            return
        mid = f"msg_{n}"
        ev("response.created", {"type": "response.created", "response": {"id": f"resp_{n}"}})
        ev("response.output_item.added", {"type": "response.output_item.added", "item": {"type": "message", "role": "assistant", "id": mid, "content": []}})
        full = ""
        steps = 20 if slow else 1
        for i in range(steps):
            d = f"chunk{i} " if slow else "ok"
            full += d
            ev("response.output_text.delta", {"type": "response.output_text.delta", "item_id": mid, "output_index": 0, "content_index": 0, "delta": d})
            if slow: time.sleep(2)
        ev("response.output_item.done", {"type": "response.output_item.done", "item": {"type": "message", "role": "assistant", "id": mid, "content": [{"type": "output_text", "text": full}]}})
        ev("response.completed", {"type": "response.completed", "response": {"id": f"resp_{n}", "usage": {"input_tokens": 1, "input_tokens_details": None, "output_tokens": 1, "output_tokens_details": None, "total_tokens": 2}}})
        log({"t": time.time(), "req": n, "done": True})

ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
