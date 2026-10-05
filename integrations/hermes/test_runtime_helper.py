"""Synthetic source-shaped entry tests; no installed native acceptance."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).with_name("runtime_helper.py").resolve()
BOOT = "import sys,runpy;sys.path.insert(0,{root!r});import hermes_bootstrap;runpy.run_module('trace',run_name='__main__',alter_sys=True)"


class RuntimeHelperTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"), prefix="helper Ω ")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.native = self.root / "native source"
        self.selected = self.root / "dependencies" / "site-packages"
        self.selected.mkdir(parents=True)
        package = self.native / "hermes_cli"
        package.mkdir(parents=True)
        (package / "__init__.py").write_text("")
        (self.native / "hermes_bootstrap.py").write_text(
            "import os,sys\nfrom pathlib import Path\n_root=Path(__file__).resolve().parent\n_pm_repair=False\n_launch_python=None\n"
            "sys.path[:]=[str(_root),os.environ['FIXTURE_DEP'],*sys.path[1:]]\n"
            "os.environ['PYTHONPATH']=os.pathsep.join(sys.path[:2])\n")
        (self.native / "hermes_constants.py").write_text(
            "import os\nfrom pathlib import Path\ndef get_hermes_home(): return Path(os.environ['HERMES_HOME'])\n")
        (package / "profiles.py").write_text(
            "import os\nfrom pathlib import Path\ndef normalize_profile_name(p): return p.strip().lower()\n"
            "def validate_profile_name(p):\n if p not in ('default','work'): raise ValueError('private')\n"
            "def resolve_profile_env(p):\n p=normalize_profile_name(p);validate_profile_name(p)\n"
            " root=Path(os.environ['FIXTURE_HOME']);home=root if p=='default' else root/'profiles'/p\n"
            " if not home.is_dir(): raise FileNotFoundError('private')\n return str(home)\n")
        (package / "version_info.py").write_text(
            "from types import SimpleNamespace\ndef get_version_info():\n return SimpleNamespace(source='git',base_version='0.21.5',derived_version='0.21.5+1.g1234567',commit='1234567890abcdef1234567890abcdef12345678',dirty=False,distance=1)\n")
        (package / "config.py").write_text(
            "import os\nclass FailedConfigRead(dict): pass\ndef load_config_readonly():\n"
            " value={'plugins':{'enabled':['herdr-threads'],'disabled':[]}}\n"
            " return FailedConfigRead(value) if os.environ.get('FIXTURE_FAILED') else value\n")
        self.home = self.root / "home with Ω"
        (self.home / "profiles" / "work").mkdir(parents=True)

    def probe(self, profile="default", **extra):
        env = {"HOME": str(self.root), "HERMES_HOME": str(self.home),
               "FIXTURE_HOME": str(self.home), "FIXTURE_DEP": str(self.selected),
               "PYTHONDONTWRITEBYTECODE": "1", **extra}
        env["HERDR_HERMES_INSPECTION_SCOPE"] = json.dumps({
            "interpreter": str(Path(sys.executable).resolve()), "source_root": str(self.native),
            "profile": profile, "home": str(self.home if profile == 'default' else self.home / 'profiles' / profile)})
        child = subprocess.run([sys.executable, "-I", "-c", BOOT.format(root=str(self.native)),
                                "--count", "--no-report", str(HELPER), "--profile", profile],
                               env=env, capture_output=True, timeout=5)
        self.assertEqual(child.returncode, 0, child.stderr.decode())
        self.assertLessEqual(len(child.stdout), 16384)
        return json.loads(child.stdout)

    def test_official_profile_result_preserves_selected_home_and_config_observation(self):
        # A helper that stays unsupported, uses default for named selection, or
        # falsely labels selection as activation fails these independent facts.
        for profile in ('default', 'work'):
            with self.subTest(profile=profile):
                result = self.probe(profile)
                self.assertEqual(result['status'], 'observed')
                want = self.home if profile == 'default' else self.home / 'profiles' / profile
                self.assertEqual(result['home'], str(want))
                self.assertEqual(result['physical_home'], str(want.resolve()))
                self.assertEqual(result['config_quality'], 'successful')
                self.assertEqual(result['enabled'], ['herdr-threads'])
                self.assertIsNone(result['fallback_kind'])
                self.assertEqual(result['evidence_stage'], 'startup_profile_observation')
                self.assertFalse(list((self.native / 'hermes_cli').rglob('*.pyc')))

    def test_failed_config_has_no_values_or_inferred_fallback(self):
        result = self.probe(FIXTURE_FAILED='1')
        self.assertEqual(result.get('config_quality'), 'failed_config_read')
        self.assertIsNone(result['enabled'])
        self.assertIsNone(result['disabled'])
        self.assertIsNone(result['fallback_kind'])

    def test_missing_profile_refuses_without_default_fallback(self):
        result = self.probe('missing')
        self.assertEqual(result['status'], 'unavailable')
        self.assertIsNone(result['home'])

    def test_mixed_loaded_origin_refuses_before_config(self):
        boot = self.native / 'hermes_bootstrap.py'
        with boot.open('a') as stream:
            stream.write("import types\nm=types.ModuleType('hermes_cli.config');m.__file__='/foreign/config.py';m.__spec__=types.SimpleNamespace(origin=m.__file__);sys.modules[m.__name__]=m\n")
        self.assertEqual(self.probe()['status'], 'unavailable')

    def test_early_external_and_relaunch_mismatched_shapes_refuse(self):
        boot = self.native / 'hermes_bootstrap.py'
        original = boot.read_text()
        for amendment in ("_pm_repair=True\n", "_launch_python=Path('/other/interpreter')\n", "os.environ['PYTHONPATH']='unbound'\n",
                          "sys.path[1]='/external/owner'\n"):
            with self.subTest(shape=amendment):
                boot.write_text(original + amendment)
                self.assertEqual(self.probe()['status'], 'unavailable')
        boot.write_text(original)
        self.assertEqual(self.probe()['status'], 'observed')

    def test_malformed_config_projection_is_unknown_without_raw_values(self):
        config = self.native / 'hermes_cli' / 'config.py'
        config.write_text("class FailedConfigRead(dict): pass\ndef load_config_readonly(): return {'plugins':{'enabled':'private secret','disabled':[]}}\n")
        result = self.probe()
        self.assertEqual(result['status'], 'observed')
        self.assertEqual(result['config_quality'], 'unknown')
        self.assertIsNone(result['enabled'])
        self.assertNotIn('private secret', json.dumps(result))


if __name__ == '__main__':
    unittest.main()
