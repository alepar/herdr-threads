#!/usr/bin/env python3
"""Local 401 stub for t0.hook-fires (nested spec §D3): `stub_api.py [--lifetime SECONDS]`.

Listens on 127.0.0.1 at an ephemeral port, prints the port on one stdout line (flushed), and answers every
request on every path and method with 401, so a harness started against it fires its SessionStart hook and
then fails to authenticate without a model or a network. Exits after --lifetime seconds (default 120) as a
backstop; harness-canary.sh also stops it through scripts/canary/run.py. Pure stdlib."""
import sys
# scripts/canary/bisect.py would shadow the stdlib `bisect` (http.server -> email.utils -> random imports it on
# Python 3.12, Ubuntu's) when this directory is sys.path[0]; drop it before anything else is imported.
_here = __import__("os").path.dirname(__import__("os").path.abspath(__file__))
sys.path[:] = [p for p in sys.path if p not in ("", _here)]

import argparse, http.server, json, socketserver, threading

BODY = json.dumps({"type": "error", "error": {"type": "authentication_error", "message": "canary stub: invalid"}}).encode()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _answer(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length:
            self.rfile.read(min(length, 1 << 20))
        self.send_response(401)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(BODY)))
        self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(BODY)
        self.close_connection = True

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = do_OPTIONS = _answer

    def log_message(self, *args):
        pass


class LoopbackHTTPServer(http.server.ThreadingHTTPServer):
    def server_bind(self):
        # HTTPServer resolves server_name through getfqdn before startup. This
        # loopback-only fixture has no hostname dependency or virtual hosts.
        socketserver.TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]


def serve(lifetime):
    server = LoopbackHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    print(server.server_address[1], flush=True)
    timer = threading.Timer(lifetime, server.shutdown)
    timer.daemon = True
    timer.start()
    try:
        server.serve_forever()
    finally:
        timer.cancel()
        server.server_close()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lifetime", type=float, default=120.0)
    serve(ap.parse_args(argv).lifetime)
    return 0


if __name__ == "__main__":
    sys.exit(main())
