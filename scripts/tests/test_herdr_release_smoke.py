"""scripts/herdr-release-smoke without network or Herdr: a release download that keeps failing becomes that
version's ERROR row, and the remaining versions, the report and the leak check still run."""
import hashlib
import http.client
import importlib.machinery
import importlib.util
import io
import os
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

SCRIPT = os.path.join(os.path.dirname(__file__), "..", "herdr-release-smoke")
loader = importlib.machinery.SourceFileLoader("herdr_release_smoke", SCRIPT)
spec = importlib.util.spec_from_loader("herdr_release_smoke", loader)
smoke = importlib.util.module_from_spec(spec)
loader.exec_module(smoke)

BODY = b"#!/bin/sh\necho herdr fake\n"
DIGEST = hashlib.sha256(BODY).hexdigest()


class Response(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class ExhaustedDownloads(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.cache = Path(self.tmp.name) / "cache"
        patcher = mock.patch.object(smoke.time, "sleep", lambda s: None)
        patcher.start()
        self.addCleanup(patcher.stop)

    def fetch(self, failure):
        with mock.patch.object(smoke.urllib.request, "urlopen", side_effect=failure):
            return smoke.fetch("9.9.1", "herdr-linux-aarch64", DIGEST, self.cache)

    def test_every_exhausted_download_failure_is_a_runtime_error(self):
        for failure in (http.client.IncompleteRead(b"ab"), urllib.error.URLError("down"),
                        TimeoutError("slow"), ConnectionResetError("reset")):
            with self.subTest(failure=type(failure).__name__):
                with self.assertRaises(RuntimeError) as raised:
                    self.fetch(failure)
                self.assertIn("after 3 attempts", str(raised.exception))
                self.assertEqual(list(self.cache.rglob("*.part")), [])
                self.assertEqual(list(self.cache.rglob("herdr")), [])

    def test_a_version_whose_download_fails_is_an_error_row_and_the_matrix_goes_on(self):
        listed = {"9.9.1": {"herdr-linux-aarch64": DIGEST}, "9.9.2": {"herdr-linux-aarch64": DIGEST}}
        report = Path(self.tmp.name) / "report.md"

        def urlopen(url, timeout):
            if "/v9.9.1/" in url:
                raise http.client.IncompleteRead(b"partial")
            return Response(BODY)

        calls = []

        def run(argv, **kwargs):
            calls.append(argv)
            if argv[0] == "git":
                return SimpleNamespace(returncode=0, stdout="abc1234\n", stderr="")
            if argv[-1] == "--version":
                return SimpleNamespace(returncode=0, stdout="herdr 9.9.2\n", stderr="")
            return SimpleNamespace(returncode=0, stdout="", stderr="")  # cargo build/test, leak check

        argv = ["herdr-release-smoke", "--versions", "9.9.1", "9.9.2", "--cache", str(self.cache),
                "--report", str(report)]
        with mock.patch.object(smoke, "read_list", return_value=listed), \
                mock.patch.object(smoke, "host_asset", return_value="herdr-linux-aarch64"), \
                mock.patch.object(smoke.urllib.request, "urlopen", side_effect=urlopen), \
                mock.patch.object(smoke.subprocess, "run", side_effect=run), \
                mock.patch.object(sys, "argv", argv), \
                mock.patch.object(sys, "stdout", io.StringIO()):
            rc = smoke.main()
        self.assertEqual(rc, 1)
        text = report.read_text()
        self.assertRegex(text, r"\| 9\.9\.1 \| ERROR \|.*after 3 attempts")
        self.assertIn("| 9.9.2 | PASS |", text)
        self.assertIn("Leak check: clean", text)
        self.assertTrue(any("check-no-leaked-processes" in str(a[0]) for a in calls), calls)


if __name__ == "__main__":
    unittest.main()
