"""Structural unit tests for the validator's model-issued call rule (ht-p03.18, P15/P31/FIX-NOW).

Run from the repository root:  python3 -m unittest discover -s scripts/tests/validator_corpus
"""

import importlib.util
from pathlib import Path
import unittest

REPO = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("demo", REPO / "scripts" / "validate-native-demo.py")
demo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo)

HT = "herdr-threads"

# shell line -> every command it runs after peeling wrappers, as argv lists
PEEL_TABLE = [
    ("herdr-threads ack m", [[HT, "ack", "m"]]),
    ("env herdr-threads ack m", [[HT, "ack", "m"]]),
    ("env -i FOO=1 herdr-threads ack m", [[HT, "ack", "m"]]),
    ("env -u X -- herdr-threads ack m", [[HT, "ack", "m"]]),
    ("FOO=1 BAR=2 herdr-threads ack m", [[HT, "ack", "m"]]),
    ("sudo herdr-threads ack m", [[HT, "ack", "m"]]),
    ("sudo -u root -- herdr-threads ack m", [[HT, "ack", "m"]]),
    ("exec herdr-threads ack m", [[HT, "ack", "m"]]),
    ("command -- herdr-threads ack m", [[HT, "ack", "m"]]),
    ("command -v herdr-threads", []),
    ("nohup herdr-threads ack m &", [[HT, "ack", "m"]]),
    ("timeout 5 herdr-threads ack m", [[HT, "ack", "m"]]),
    ("timeout -s KILL -k 2 10 herdr-threads ack m", [[HT, "ack", "m"]]),
    ("sh -c 'herdr-threads ack m'", [[HT, "ack", "m"]]),
    ('bash -lc "herdr-threads ack m; herdr-threads inbox"', [[HT, "ack", "m"], [HT, "inbox"]]),
    ("env sudo -u root nohup timeout 3 herdr-threads ack m", [[HT, "ack", "m"]]),
    ("echo `herdr-threads ack m`", [["echo", ""], [HT, "ack", "m"]]),
    ("echo $(herdr-threads ack m)", [["echo", ""], [HT, "ack", "m"]]),
    ('echo "$(herdr-threads ack m)"', [["echo", ""], [HT, "ack", "m"]]),
    ("echo '$(herdr-threads ack m)'", [["echo", "$(herdr-threads ack m)"]]),
    ('echo "herdr-threads" ack', [["echo", HT, "ack"]]),
    ("cd /p && herdr-threads ack m", [["cd", "/p"], [HT, "ack", "m"]]),
    ("herdr-threads inbox | head -5", [[HT, "inbox"], ["head", "-5"]]),
    ("sh -c \"bash -c 'herdr-threads ack m'\"", [[HT, "ack", "m"]]),
    ("/opt/bin/herdr-threads ack m", [["/opt/bin/herdr-threads", "ack", "m"]]),
    ("eval 'herdr-threads ack m'", [[HT, "ack", "m"]]),
    ("env -S 'herdr-threads ack m'", [[HT, "ack", "m"]]),
    ("xargs -n1 herdr-threads ack", [[HT, "ack"]]),
    ("sh script.sh", [["sh", "script.sh"]]),
    ('"$(which herdr-threads)" ack m', [[HT, "ack", "m"]]),
    ("for m in a b; do herdr-threads ack $m; done", [["for", "m", "in", "a", "b"], [HT, "ack", "$m"], ["done"]]),
]


class PeelWrappers(unittest.TestCase):
    def test_table_has_at_least_fifteen_lines(self):
        self.assertGreaterEqual(len(PEEL_TABLE), 15)

    def test_peeled_commands(self):
        for line, expected in PEEL_TABLE:
            with self.subTest(line=line):
                self.assertEqual(demo.peel_wrappers(line), expected)

    def test_word_list_is_joined_first(self):
        self.assertEqual(demo.peel_wrappers(["env", "FOO=1", "herdr-threads", "ack", "a b"]), [[HT, "ack", "a b"]])

    def test_empty_and_none(self):
        self.assertEqual(demo.peel_wrappers(""), [])
        self.assertEqual(demo.peel_wrappers(None), [])

    def test_unbalanced_quotes_do_not_raise(self):
        self.assertEqual(demo.peel_wrappers("herdr-threads ack 'm"), [[HT, "ack", "m"]])
        self.assertEqual(demo.peel_wrappers('echo "$(herdr-threads ack m'), [["echo", ""], [HT, "ack", "m"]])


class NestedAgentCli(unittest.TestCase):
    def test_nested_model_clis(self):
        for argv, nested in ((["claude", "-p", "x"], True), (["claude", "--print", "x"], True), (["/usr/bin/claude", "-p"], True),
                             (["codex", "exec", "x"], True), (["codex", "--no-daemon", "exec", "x"], True),
                             (["claude", "--resume"], False), (["codex", "resume"], False), (["echo", "claude", "-p"], False),
                             (["claude", "--", "-p"], False), ([], False)):
            with self.subTest(argv=argv):
                self.assertEqual(demo.is_nested_agent_cli(argv), nested)


def call(command, **over):
    return {"command": command, **over}


class ModelIssuedCalls(unittest.TestCase):
    def calls(self, command, root="s1", **over):
        return demo.model_issued_herdr_calls(call(command, **over), root_session=root)

    def test_direct_and_wrapped_calls_count(self):
        for command in ("herdr-threads ack m", "env herdr-threads ack m", "timeout 5 sudo herdr-threads ack m", "sh -c 'herdr-threads ack m'",
                        "x=$(herdr-threads inbox)"):
            with self.subTest(command=command):
                self.assertEqual(len(self.calls(command)), 1)

    def test_a_subagents_call_never_counts(self):
        self.assertEqual(self.calls("herdr-threads ack m", sidechain=True), [])
        self.assertEqual(self.calls("herdr-threads ack m", session="child-session"), [])
        self.assertEqual(len(self.calls("herdr-threads ack m", session="s1")), 1)

    def test_nested_claude_or_codex_is_not_the_roots_call(self):
        self.assertEqual(self.calls('claude -p "$(herdr-threads ack m)"'), [])
        self.assertEqual(self.calls("codex exec \"`herdr-threads ack m`\""), [])
        self.assertEqual(self.calls("sh -c 'claude -p x \"$(herdr-threads ack m)\"'"), [])
        self.assertEqual(len(self.calls("claude -p x; herdr-threads ack m")), 1)  # a separate command of the same line

    def test_quoted_literal_and_lookups_are_not_calls(self):
        # FIX-NOW: the old text scan matched `echo "herdr-threads" ack` as a mutating call.
        for command in ('echo "herdr-threads" ack', "echo 'herdr-threads ack m'", "which herdr-threads", "command -v herdr-threads",
                        "type herdr-threads", "grep herdr-threads log", "herdr-threadsX ack"):
            with self.subTest(command=command):
                self.assertEqual(self.calls(command), [])
        self.assertFalse(demo.mutating_call('echo "herdr-threads" ack'))
        self.assertFalse(demo.invokes_herdr_threads('echo "herdr-threads" ack'))

    def test_verbs_and_mutation_follow_the_peeled_command(self):
        self.assertEqual(demo.herdr_threads_verbs("sudo -u root herdr-threads --state-dir /s thread create --topic t"), ["thread create"])
        self.assertTrue(demo.mutating_call("env X=1 nohup herdr-threads send t --body b"))
        self.assertFalse(demo.mutating_call("env herdr-threads inbox"))

    def test_root_ack_needs_a_model_issued_ack_naming_the_message(self):
        ack = demo.Driver.is_root_ack
        self.assertTrue(ack(("c", "env herdr-threads ack msg-1", False), "msg-1"))
        self.assertFalse(ack(("c", "env herdr-threads ack msg-2", False), "msg-1"))
        self.assertFalse(ack(("c", "herdr-threads ack msg-1", True), "msg-1"))
        self.assertFalse(ack(("c", 'echo "herdr-threads ack msg-1"', False), "msg-1"))
        self.assertFalse(ack(("c", "herdr-threads body msg-1", False), "msg-1"))


class CodexBinEnvTest(unittest.TestCase):
    """HT_CODEX_BIN is a codex-only default: it must not break Claude runs, but an explicit flag still must."""

    def parse(self, harness, *extra, env=None):
        import os
        from unittest import mock
        with mock.patch.dict(os.environ, {"HT_CODEX_BIN": "/bin/sh"} if env is None else env, clear=False):
            return demo.parse(["--harness", harness, "--dry-run", "--bin", "/bin/sh", *extra])

    def test_env_ignored_for_claude(self):
        self.assertIsNone(self.parse("claude").codex_bin)

    def test_env_used_for_codex(self):
        self.assertEqual(self.parse("codex").codex_bin, "/bin/sh")

    def test_explicit_flag_rejected_for_claude(self):
        with self.assertRaises(SystemExit):
            self.parse("claude", "--codex-bin", "/bin/sh")


if __name__ == "__main__":
    unittest.main()
