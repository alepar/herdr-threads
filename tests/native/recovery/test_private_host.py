"""Safety-guard tests for the host-recovery suite. No Herdr server, daemon or model is started."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from private_host import CommandLog, PrivateHerdr, SafetyError, TestDaemon, pid_alive

HERDR = shutil.which("herdr")


@unittest.skipUnless(HERDR, "herdr not on PATH")
class GuardTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="htrt-", dir="/private/tmp"))
        self.log = CommandLog(self.root / "commands.jsonl")
        self.host = PrivateHerdr(self.root, self.log)
        self.children = []

    def tearDown(self):
        for child in self.children:
            if child.poll() is None:
                child.kill()
                child.wait()
        self.log.close()
        shutil.rmtree(self.root)

    def spawn_sleep(self):
        child = subprocess.Popen(["sleep", "30"], start_new_session=True)
        self.children.append(child)
        return child

    def test_environment_drops_inherited_herdr_context(self):
        inherited = {"HERDR_SOCKET_PATH": "/shared/herdr.sock", "HERDR_PANE_ID": "w9:p9", "HERDR_ENV": "1"}
        with mock.patch.dict(os.environ, inherited):
            env = self.host.environment()
        self.assertEqual(env["HERDR_SOCKET_PATH"], str(self.root / "h.sock"))
        self.assertEqual({k for k in env if k.startswith("HERDR_")}, {"HERDR_SOCKET_PATH", "HERDR_CONFIG_PATH"})
        self.assertTrue(env["HOME"].startswith(str(self.root)))

    def test_api_refuses_a_socket_outside_the_run_directory(self):
        self.host.socket = Path("/private/tmp/not-this-run.sock")
        with self.assertRaises(SafetyError):
            self.host.api("ping")

    def test_stop_refuses_a_process_that_is_not_the_private_server(self):
        child = self.spawn_sleep()
        self.host.process = child
        with self.assertRaises(SafetyError):
            self.host.stop()
        self.assertTrue(pid_alive(child.pid))
        self.host.process = None

    def test_kill_daemon_refuses_a_pid_whose_argv_is_not_this_runs_daemon(self):
        child = self.spawn_sleep()
        instance = self.root / "s" / "instances" / "x"
        instance.mkdir(parents=True)
        (instance / "locator").write_text(str(self.host.socket))
        (instance / "endpoint.json").write_text(json.dumps({"pid": child.pid, "boot_id": "b"}))
        daemon = TestDaemon("/bin/sleep", self.root / "s", self.host, self.log)
        with self.assertRaises(SafetyError):
            daemon.kill_daemon()
        self.assertTrue(pid_alive(child.pid))
        self.assertEqual(daemon.killed, [])

    def test_instance_lookup_requires_the_private_host_locator(self):
        instance = self.root / "s" / "instances" / "y"
        instance.mkdir(parents=True)
        (instance / "locator").write_text("/shared/herdr.sock")
        daemon = TestDaemon("/bin/sleep", self.root / "s", self.host, self.log)
        with self.assertRaises(AssertionError):
            daemon.instance_dir()


if __name__ == "__main__":
    unittest.main()
