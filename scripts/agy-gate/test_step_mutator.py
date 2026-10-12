"""Offline tests for step_mutator.py (pure mutate() plus the CLI against a tiny fake hook). Python 3.9 safe."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import step_mutator as sm  # noqa: E402

TEXT = ("[herdr-threads] Messages follow.\n\n[herdr-threads] message msg-o1 in t1 from s2:\n  hello\n\n"
        "[herdr-threads] lazy msg-l1 in t1 from s2:\n  passive")
SYSTEM = json.dumps({"injectSteps": [{"systemMessage": {"systemMessage": TEXT}}]}, separators=(",", ":")).encode()
USER = json.dumps({"injectSteps": [{"userMessage": {"userMessage": TEXT}}]}).encode()
ABORT = b'{"injectSteps":[{"toolCall":{"name":"run_command","args":{"CommandLine":"true"}}}]}'


class MutateTests(unittest.TestCase):
    def test_abort_replaces_a_system_message_step_with_the_d3_bytes(self):
        self.assertEqual(sm.mutate(SYSTEM, "abort"), ABORT)

    def test_ignore_replaces_it_with_a_well_formed_unknown_key_step(self):
        out = sm.mutate(SYSTEM, "ignore")
        step = json.loads(out)["injectSteps"][0]
        self.assertEqual(list(step), ["htIgnoredStep"])
        self.assertNotIn("systemMessage", out.decode())

    def test_passthrough_byte_for_byte(self):
        for data in (SYSTEM, b"", b"not json", b"[]", USER, b'{"injectSteps":[]}', b'{"injectSteps":"x"}',
                     b'{"injectSteps":[{"toolCall":{}}]}', b"\xff\xfe"):
            self.assertIs(sm.mutate(data, None), data)
            self.assertEqual(sm.mutate(data, "bogus"), data)
        for data in (b"", b"not json", USER, b'{"injectSteps":[]}', b"\xff\xfe"):
            for mode in ("abort", "ignore"):
                self.assertEqual(sm.mutate(data, mode), data)

    def test_frame_ids_come_from_the_original_stdout(self):
        self.assertEqual(sm.frame_ids(SYSTEM), ["msg-o1", "msg-l1"])
        named = json.dumps({"injectSteps": [{"systemMessage": {"systemMessage":
                            '[herdr-threads] message msg-n1 in tAb1 "a \\"b\\" c" from sCd2 "Ann" [human]:\n  hi'}}]})
        self.assertEqual(sm.frame_ids(named.encode()), ["msg-n1"])
        self.assertEqual(sm.frame_ids(b"junk"), [])
        self.assertEqual(sm.frame_ids(ABORT), [])


class ChildEnvTests(unittest.TestCase):
    def test_agy_parent_directory_leads_path(self):
        env = sm.child_env({"PATH": "/usr/bin:/bin", "X": "1"}, "/r/agybin/2/agy")
        self.assertEqual(env["PATH"], "/r/agybin/2:/usr/bin:/bin")
        self.assertEqual(env["X"], "1")

    def test_other_parents_leave_the_environment_alone(self):
        base = {"PATH": "/usr/bin"}
        for image in (None, "", "/usr/bin/python3", "/bin/sh"):
            self.assertIs(sm.child_env(base, image), base)

    def test_missing_path_is_just_the_agy_directory(self):
        self.assertEqual(sm.child_env({}, "/r/agy")["PATH"], "/r")

    def test_parent_image_is_an_absolute_path_or_none(self):
        image = sm.parent_image(os.getpid())
        self.assertTrue(image is None or os.path.isabs(image), image)
        self.assertIsNone(sm.parent_image(2 ** 22 + 12345))


class CliTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.ctl = Path(self.tmp.name)
        self.hook = self.ctl / "hook.py"
        self.hook.write_text(
            "import sys, pathlib\n"
            "stdin = sys.stdin.buffer.read()\n"
            "pathlib.Path(sys.argv[1]).write_bytes(stdin)\n"
            "sys.stdout.buffer.write(pathlib.Path(sys.argv[2]).read_bytes())\n"
            "sys.exit(int(sys.argv[3]))\n")
        self.out = self.ctl / "out.bin"
        self.seen = self.ctl / "seen.bin"

    def tearDown(self):
        self.tmp.cleanup()

    def run_cli(self, stdout_bytes, code=0, stdin=b"PAYLOAD"):
        self.out.write_bytes(stdout_bytes)
        result = subprocess.run(
            [sys.executable, str(HERE / "step_mutator.py"), "--ctl", str(self.ctl), "--", sys.executable,
             str(self.hook), str(self.seen), str(self.out), str(code)],
            input=stdin, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return result

    def test_unarmed_passes_stdout_stdin_and_exit_code_through(self):
        result = self.run_cli(SYSTEM, code=3)
        self.assertEqual((result.stdout, result.returncode), (SYSTEM, 3))
        self.assertEqual(self.seen.read_bytes(), b"PAYLOAD")
        self.assertFalse((self.ctl / "mutate-fired").exists())

    def test_armed_fires_once_and_records_the_original(self):
        (self.ctl / "mutate-armed").write_text("")
        first = self.run_cli(SYSTEM, code=0)
        self.assertEqual(first.stdout, ABORT)
        self.assertFalse((self.ctl / "mutate-armed").exists())
        record = json.loads((self.ctl / "mutate-fired").read_text())
        self.assertEqual(record["mode"], "abort")
        self.assertEqual(record["frame_ids"], ["msg-o1", "msg-l1"])
        self.assertEqual(record["original_stdout"].encode(), SYSTEM)
        second = self.run_cli(SYSTEM)
        self.assertEqual(second.stdout, SYSTEM)

    def test_ignore_mode_file_selects_the_ignored_step(self):
        (self.ctl / "mutate-armed-ignore").write_text("")
        result = self.run_cli(SYSTEM)
        self.assertEqual(list(json.loads(result.stdout)["injectSteps"][0]), ["htIgnoredStep"])
        self.assertEqual(json.loads((self.ctl / "mutate-fired").read_text())["mode"], "ignore")

    def test_armed_but_nothing_to_mutate_keeps_the_arming_and_passes_through(self):
        (self.ctl / "mutate-armed").write_text("")
        for data in (b"", b"junk", USER):
            result = self.run_cli(data, code=1)
            self.assertEqual((result.stdout, result.returncode), (data, 1))
        self.assertTrue((self.ctl / "mutate-armed").exists())
        self.assertFalse((self.ctl / "mutate-fired").exists())


if __name__ == "__main__":
    unittest.main()
