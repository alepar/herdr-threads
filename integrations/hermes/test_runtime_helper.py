"""Active-entrypoint refusal tests. No installed Hermes or model evidence."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).with_name("runtime_helper.py")
BOOT = "import sys,runpy;sys.path.insert(0,sys.argv[1]);p=sys.argv[2];sys.argv=[p,*sys.argv[3:]];runpy.run_path(p,run_name='__main__')"


class RuntimeHelperTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="hermes runtime Ω ")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.native = self.root / "native source"
        package = self.native / "hermes_cli"
        package.mkdir(parents=True)
        for name in ("__init__", "profiles", "config", "version_info", "plugins"):
            (package / f"{name}.py").write_text("import os,sys\nprint('native-import-attempt',file=sys.stderr)\nfrom pathlib import Path\nPath(os.environ['HOME'],'native-imported').write_text('forbidden')\n")
        self.home = self.root / "custom home"

    def probe(self, profile):
        env = {**os.environ, "HOME": str(self.root), "HERMES_HOME": str(self.home), "PYTHONDONTWRITEBYTECODE": "1", "GIT_OPTIONAL_LOCKS": "0"}
        child = subprocess.run([sys.executable, "-I", "-B", "-c", BOOT, str(self.native), str(HELPER), "--profile", profile], env=env, capture_output=True, timeout=5)
        self.assertEqual(child.returncode, 0, child.stderr.decode())
        self.assertLessEqual(len(child.stdout), 16384)
        self.assertEqual(child.stderr, b"")
        return json.loads(child.stdout)

    def test_active_entry_refuses_before_native_import_or_home_creation(self):
        for profile in ("default", " WoRk ", "missing"):
            with self.subTest(profile=profile):
                result = self.probe(profile)
                self.assertEqual(result["status"], "unsupported")
                self.assertEqual(result["reason"], "native_read_only_boundary_unavailable")
                self.assertIsNone(result["runtime_descriptor"])
                self.assertIsNone(result["callback_timeout_ms"])
                self.assertIsNone(result["api"])
                self.assertEqual(result["evidence_stage"], "unavailable")
                self.assertFalse((self.root / "native-imported").exists())
                self.assertFalse(self.home.exists())
                self.assertFalse(list(self.native.rglob("*.pyc")))

    def test_bad_profile_never_reaches_native_boundary(self):
        for profile in ("", "x" * 257, "x\u0085y"):
            with self.subTest(profile=profile):
                self.assertEqual(self.probe(profile)["reason"], "profile_unavailable")
                self.assertFalse((self.root / "native-imported").exists())


if __name__ == "__main__":
    unittest.main()
