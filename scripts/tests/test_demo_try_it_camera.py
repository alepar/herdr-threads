"""Synthetic PTY camera ownership and recording-boundary regressions."""
import importlib.util
from argparse import Namespace
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]


class CameraTests(unittest.TestCase):
    def run_camera(self, action, setup_failure=False):
        with tempfile.TemporaryDirectory(prefix="ht-try-it.", dir="/private/tmp") as directory:
            root = Path(directory)
            client = root / "herdr"
            client.write_text("#!/usr/bin/env python3\nimport os,signal,time\n"
                              "signal.signal(signal.SIGWINCH,lambda *a: print('OWNED width '+str(os.get_terminal_size().columns),flush=True))\n"
                              "print('PRIVATE SETUP',flush=True)\nwhile True:time.sleep(.01)\n")
            client.chmod(0o700)
            if setup_failure:
                (root / "camera.pid").mkdir()
            env = dict(os.environ, PATH=str(root) + ":" + os.environ["PATH"])
            process = subprocess.Popen(["python3", str(SCRIPTS / "demo-try-it-camera.py"),
                                        "--root", str(root), "--timeout", "2"],
                                       env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                until = time.monotonic() + 2
                while not (root / "camera.pid").is_file() and process.poll() is None and time.monotonic() < until:
                    time.sleep(.01)
                if not setup_failure:
                    time.sleep(.1)
                    if action == "stop":
                        (root / "camera-start").touch()
                        time.sleep(.8)
                        (root / "camera-stop").touch()
                    elif action == "malformed":
                        with (root / "control.jsonl").open("a") as stream:
                            stream.write("[]\n")
                _, error = process.communicate(timeout=5)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
            if (root / "camera.pid").is_file():
                with self.assertRaises(ProcessLookupError):
                    os.kill(int((root / "camera.pid").read_text()), 0)
            cast = root / "camera.cast"
            events = [json.loads(line) for line in cast.read_text().splitlines()] if cast.exists() else []
            return process.returncode, error, events

    def test_stop_reaps_client_and_excludes_setup_and_extra_column_frame(self):
        code, error, events = self.run_camera("stop")
        self.assertEqual(code, 0, error.decode())
        output = "".join(event[2] for event in events[1:])
        self.assertIn("OWNED width 160", output)
        self.assertNotIn("PRIVATE SETUP", output)
        self.assertNotIn("width 161", output)

    def test_timeout_reaps_client(self):
        code, error, _ = self.run_camera("timeout")
        self.assertNotEqual(code, 0)
        self.assertIn(b"deadline reached", error)

    def test_bad_control_reaps_client(self):
        code, error, _ = self.run_camera("malformed")
        self.assertNotEqual(code, 0)
        self.assertIn(b"literal key strings", error)

    def test_pid_file_failure_cannot_leave_client_running(self):
        spec = importlib.util.spec_from_file_location("camera", SCRIPTS / "demo-try-it-camera.py")
        camera = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(camera)
        with tempfile.TemporaryDirectory(prefix="ht-try-it.", dir="/private/tmp") as directory:
            root = Path(directory)
            (root / "camera.pid").mkdir()
            client = root / "herdr"
            client.write_text("#!/bin/sh\nexec sleep 30\n")
            client.chmod(0o700)
            owned = []
            original = camera.pty.fork
            def record_fork():
                pid, master = original()
                if pid:
                    owned.append(pid)
                return pid, master
            with patch.dict(os.environ, PATH=str(root) + ":" + os.environ["PATH"]):
                with patch.object(camera.pty, "fork", side_effect=record_fork):
                    with self.assertRaises(IsADirectoryError):
                        camera.capture(Namespace(root=root, timeout=2, cols=160, rows=62))
            self.assertEqual(len(owned),1)
            with self.assertRaises(ProcessLookupError):
                os.kill(owned[0],0)

    def test_settings_copy_preserves_preferences_and_private_trust(self):
        spec = importlib.util.spec_from_file_location("prepare", SCRIPTS / "demo-try-it-prepare.py")
        prepare = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(prepare)
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            root = home / "run"
            for p in [root / "home", root / "codex-home", root / "claude-config", home / "codex", home / "claude"]:
                p.mkdir(parents=True)
            source = home / "codex/config.toml"
            agents = home / "codex/agents"
            agents.mkdir()
            (agents / "reader.toml").write_text('model = "example"\n')
            scripts = home / "codex/scripts"
            scripts.mkdir()
            (scripts / "native.sh").write_text('echo state > ' + str(home / "codex/log") + '\n')
            source.write_text('model = "example"\n[tui]\nstatus_line=["weekly-limit"]\n'
                              '[agents.reader]\nconfig_file=' + json.dumps(str(agents / "reader.toml")) + '\n')
            (home / "codex/hooks.json").write_text(json.dumps({"hooks":{"SessionStart":[{"hooks":[{"command":"bash " + str(scripts / "native.sh")}]}]}}))
            (root / "codex-home/config.toml").write_text('[hooks.state.private]\ntrusted_hash="genuine-native-review"\n')
            (home / "claude/settings.json").write_text('{"effortLevel":"high"}')
            before = source.read_bytes()
            with patch.object(Path, "home", return_value=home):
                prepare.copy_settings(root, home / "codex", home / "claude")
            config = prepare.tomllib.loads((root / "codex-home/config.toml").read_text())
            self.assertEqual(config["model"], "example")
            self.assertEqual(config["tui"]["status_line"], ["weekly-limit"])
            self.assertEqual(config["hooks"]["state"]["private"]["trusted_hash"], "genuine-native-review")
            self.assertEqual(config["agents"]["reader"]["config_file"],str(root / "codex-home/agents/reader.toml"))
            self.assertTrue((root / "codex-home/agents/reader.toml").exists())
            self.assertIn(str(root / "codex-home/scripts/native.sh"),(root / "codex-home/hooks.json").read_text())
            self.assertIn(str(root / "codex-home/log"),(root / "codex-home/scripts/native.sh").read_text())
            self.assertEqual(json.loads((root / "claude-config/settings.json").read_text())["effortLevel"], "high")
            self.assertEqual(source.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
