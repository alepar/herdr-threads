import http.server, json, os, sys, threading, time
CTL = sys.argv[1]
lock = threading.Lock()
def pop():
    q = os.path.join(CTL, 'queue.json')
    try: items = json.load(open(q))
    except Exception: items = []
    if not items: return {"text": "DONE-default"}
    head = items.pop(0); json.dump(items, open(q, 'w')); return head
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *a): pass
    def send(self, obj, stream):
        data = json.dumps(obj).encode()
        if stream: data = b"data: " + data + b"\r\n\r\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream" if stream else "application/json")
        self.send_header("Content-Length", str(len(data))); self.send_header("Connection", "close"); self.end_headers()
        self.wfile.write(data)
    def do_GET(self):
        with lock:
            open(os.path.join(CTL,'requests.jsonl'),'a').write(json.dumps({"t":time.time(),"method":"GET","path":self.path.split('?')[0]})+"\n")
        self.send({"models":[{"name":"models/gemini-3.8-flash","supportedGenerationMethods":["generateContent"]}]}, False)
    def do_POST(self):
        n = int(self.headers.get('Content-Length','0')); raw = self.rfile.read(n)
        try: body = json.loads(raw)
        except Exception: body = {"_raw": raw[:2000].decode(errors='replace')}
        path = self.path.split('?')[0]
        stream = 'streamGenerateContent' in path
        with lock:
            if ':countTokens' in path: resp = None
            elif 'flash-lite' in path: item = {"text": "Probe title"}
            else: item = pop()
            if ':countTokens' in path:
                open(os.path.join(CTL,'requests.jsonl'),'a').write(json.dumps({"t":time.time(),"path":path})+"\n")
                self.send({"totalTokens": 32}, False); return
            open(os.path.join(CTL,'requests.jsonl'),'a').write(json.dumps({"t":time.time(),"path":path,"reply":item,"body":body})+"\n")
        if 'sleep' in item: time.sleep(item['sleep'])
        if 'call' in item:
            parts = [{"functionCall": {"name": item['call'], "args": item.get('args', {})}}]
        else:
            parts = [{"text": item['text']}]
        self.send({"candidates":[{"content":{"role":"model","parts":parts},"finishReason":"STOP","index":0}],
                   "usageMetadata":{"promptTokenCount":32,"candidatesTokenCount":2,"totalTokenCount":34},"modelVersion":"fake"}, stream)
s = http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[2]) if len(sys.argv)>2 else 0), H)
open(os.path.join(CTL,'port'),'w').write(str(s.server_port))
s.serve_forever()
