"""Tests of the isolated herdr-core helper scripts/lib/isolated-core.sh (ht-kqrt.76)."""
import os
import shutil
import subprocess
import tempfile
import unittest

HELPER = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", "lib", "isolated-core.sh"))


def shellcheck_or_skip(which=shutil.which, env=os.environ):
    """The shellcheck path; skips when absent locally, raises when absent under CI."""
    found = which("shellcheck")
    if found:
        return found
    if env.get("CI"):
        raise RuntimeError("shellcheck is required when CI is set")
    raise unittest.SkipTest("shellcheck not installed")


class IsolatedCoreTests(unittest.TestCase):
    def test_shellcheck_clean(self):
        done = subprocess.run([shellcheck_or_skip(), HELPER], capture_output=True, text=True, check=False)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_shellcheck_required_in_ci(self):
        with self.assertRaises(RuntimeError):
            shellcheck_or_skip(which=lambda _name: None, env={"CI": "true"})
        with self.assertRaises(unittest.SkipTest):
            shellcheck_or_skip(which=lambda _name: None, env={})
        self.assertEqual(shellcheck_or_skip(which=lambda _name: "/x/shellcheck", env={"CI": "true"}), "/x/shellcheck")

    def test_never_calls_hc_server_status(self):
        with open(HELPER, encoding="utf-8") as fh:
            text = fh.read()
        code = [ln for ln in text.splitlines() if not ln.lstrip().startswith("#")]
        self.assertFalse([ln for ln in code if "server status" in ln])

    def test_noop_skip_when_hc_absent(self):
        with tempfile.TemporaryDirectory(prefix="ht-ic-test-") as empty:
            before = set(os.listdir("/tmp"))
            script = f'. "{HELPER}"; ic_start absent-case; echo "rc=$?"'
            done = subprocess.run(
                ["env", "-i", f"PATH={empty}:/usr/bin:/bin", "/bin/sh", "-c", script],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertEqual(done.stdout.splitlines(), ["skipped: no hc: absent-case", "rc=2"])
            self.assertEqual([n for n in set(os.listdir("/tmp")) - before if n.startswith("ic.")], [])


if __name__ == "__main__":
    unittest.main()
