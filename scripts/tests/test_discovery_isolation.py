"""Daemon-free checks for script fixtures' discovery settings."""
import ast
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "scripts/lib/isolated-herdr.sh"


class DiscoveryIsolation(unittest.TestCase):
    def shell(self, code, *args):
        return subprocess.run(["/bin/sh", "-c", '. "$1"; shift; ' + code,
                               "sh", str(LIB), *map(str, args)],
                              env={**os.environ, "HT_SKIP_HERDR_TESTS": "1"},
                              capture_output=True, text=True, check=True)

    def test_private_root_disables_default_plugin_discovery_before_server_start(self):
        root = Path(self.shell('ih_root_new').stdout.strip())
        try:
            settings = json.loads((root / 'st/herdr/plugins/herdr-threads/node/settings.json').read_text())
            self.assertIs(settings['sync']['discover'], False)
            self.assertEqual(stat.S_IMODE((root / 'st/herdr/plugins/herdr-threads/node/settings.json').stat().st_mode), 0o600)
            self.assertFalse((root / 'server.pid').exists())
            for path in ('st/herdr/plugins/herdr-threads', 'st/herdr/plugins/herdr-threads/node'):
                self.assertEqual(stat.S_IMODE((root / path).stat().st_mode), 0o700)
        finally:
            shutil.rmtree(root)

    def test_explicit_state_seed_preserves_other_settings_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp) / 'state'
            node = state / 'node'
            node.mkdir(parents=True)
            settings = node / 'settings.json'
            settings.write_text(json.dumps({'mod_delivery': 'off', 'sync': {'dial_command': 'fixture {dest}'}}))
            self.shell('ih_disable_sync_discovery "$1"; ih_disable_sync_discovery "$1"', state)
            self.assertEqual(stat.S_IMODE(node.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(settings.stat().st_mode), 0o600)
            self.assertEqual(json.loads(settings.read_text()),
                             {'mod_delivery': 'off', 'sync': {'dial_command': 'fixture {dest}', 'discover': False}})

    def test_xdg_cell_preset_seeds_its_explicit_fixture_state(self):
        script = (ROOT / 'scripts/validate-native-demo.sh').read_text()
        begin = script.index('    xdg=$out/home/.local/state/herdr-threads')
        end = script.index('    before=$(cell_versions', begin)
        with tempfile.TemporaryDirectory() as tmp:
            self.shell('out=$1; flags="--state-dir @XDG_STATE@"; ' + script[begin:end], tmp)
            path = Path(tmp) / 'home/.local/state/herdr-threads/node/settings.json'
            self.assertIs(json.loads(path.read_text())['sync']['discover'], False)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_real_ssh_fixture_command_seeds_private_node_before_ensure(self):
        tree = ast.parse((ROOT / 'scripts/linux-e2e/real-ssh-e2e.py').read_text())
        calls = [n for n in ast.walk(tree) if isinstance(n, ast.Call)
                 and isinstance(n.func, ast.Attribute) and n.func.attr == 'sh'
                 and n.args and isinstance(n.args[0], ast.JoinedStr)
                 and 'settings.json' in ast.unparse(n.args[0])]
        self.assertEqual(len(calls), 1)
        with tempfile.TemporaryDirectory() as tmp:
            node = str(Path(tmp) / 'state/node')
            expression = ast.Expression(calls[0].args[0])
            command = eval(compile(expression, '<fixture-command>', 'eval'), {'node': node})
            subprocess.run(['/bin/sh', '-c', command], check=True)
            self.assertEqual(json.loads((Path(node) / 'settings.json').read_text()), {'sync': {'discover': False}})
            self.assertEqual(stat.S_IMODE((Path(node) / 'settings.json').stat().st_mode), 0o600)
            for path in (Path(node), Path(node).parent):
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)

    def test_native_demo_refuses_ambient_discovery_without_changing_supplied_state(self):
        spec = importlib.util.spec_from_file_location('native_demo_isolation', ROOT / 'scripts/validate-native-demo.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp) / 'state'
            state.mkdir()
            driver = object.__new__(module.Driver)
            driver.state = state
            driver.args = SimpleNamespace(state_dir=str(state))
            for value in (None, {'sync': {'discover': True}}):
                node = state / 'node'
                node.mkdir(exist_ok=True)
                path = node / 'settings.json'
                if value is not None:
                    path.write_text(json.dumps(value))
                before = path.read_bytes() if path.exists() else None
                status, detail, _ = driver.s_scratch()
                self.assertEqual(status, module.FAIL)
                self.assertIn('sync.discover=false', detail)
                self.assertEqual(path.read_bytes() if path.exists() else None, before)


if __name__ == '__main__':
    unittest.main()
