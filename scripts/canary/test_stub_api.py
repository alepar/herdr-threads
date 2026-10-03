#!/usr/bin/env python3
"""stub_api.py must start when scripts/canary/bisect.py sits next to it (Linux/py3.12 regression, ht-p03.14.7)."""
import os, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by `random`, hence `tempfile`) when
# `discover -s scripts/canary` puts this directory first on sys.path: import the stdlib ones without it.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import shutil, subprocess, tempfile, unittest  # noqa: E402
sys.path[:] = _saved


class StubStartsBesideBisect(unittest.TestCase):
    def test_prints_port_with_a_poisoned_bisect_next_to_it(self):
        with tempfile.TemporaryDirectory() as d:
            shutil.copy(HERE / "stub_api.py", d)
            # A sibling named like a stdlib module that `random` imports; importing it from here would fail.
            pathlib.Path(d, "bisect.py").write_text("raise ImportError('shadowed stdlib bisect')\n")
            # Put the poisoned dir first on sys.path, as running the script by path does.
            code = ("import faulthandler, sys, runpy; faulthandler.dump_traceback_later(15); "
                    "sys.path.insert(0, %r); sys.argv = ['stub_api.py', '--lifetime', '2']; "
                    "runpy.run_path(%r, run_name='__main__')") % (d, str(pathlib.Path(d, "stub_api.py")))
            try:
                p = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=30)
            except subprocess.TimeoutExpired as error:
                self.fail(f"stub timed out; stdout={error.stdout!r}; stderr={error.stderr!r}")
            first = p.stdout.splitlines()[0] if p.stdout else ""
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertTrue(first.isdigit(), f"no port on stdout; stderr={p.stderr[-400:]!r}")

    def test_loopback_startup_and_lifetime_do_not_resolve_hostnames(self):
        # A local 401 fixture must not wait on ambient DNS before its ready
        # line or lifetime backstop. Reject resolution in the real child.
        code = ("import socket, sys, runpy; "
                "socket.getfqdn = lambda *a: (_ for _ in ()).throw(AssertionError('unexpected hostname lookup')); "
                "sys.argv = ['stub_api.py', '--lifetime', '0.1']; "
                "runpy.run_path(%r, run_name='__main__')") % str(HERE / "stub_api.py")
        p = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=30)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertTrue(p.stdout.strip().isdigit(), p.stdout)

    def test_loopback_server_answers_401_without_hostname_resolution(self):
        code = """
import socket, runpy
socket.getfqdn = lambda *a: (_ for _ in ()).throw(AssertionError('unexpected hostname lookup'))
stub = runpy.run_path(%r, run_name='stub_test')
import http.client, threading
server = stub['LoopbackHTTPServer'](('127.0.0.1', 0), stub['Handler'])
worker = threading.Thread(target=server.serve_forever)
worker.start()
client = http.client.HTTPConnection(*server.server_address, timeout=5)
try:
    client.request('POST', '/v1/messages', body='{}')
    response = client.getresponse()
    assert response.status == 401, response.status
    assert response.getheader('Content-Type') == 'application/json'
    assert response.read() == stub['BODY']
finally:
    client.close()
    server.shutdown()
    worker.join(5)
    server.server_close()
assert not worker.is_alive()
""" % str(HERE / "stub_api.py")
        p = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=30)
        self.assertEqual(p.returncode, 0, p.stderr)


if __name__ == "__main__":
    unittest.main()
