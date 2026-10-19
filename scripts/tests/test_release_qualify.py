"""Orchestration contracts with subprocesses replaced at the command boundary."""
import importlib.machinery
import importlib.util
import tempfile
import contextlib
import io
import shutil
import unittest
from pathlib import Path
from unittest.mock import patch

DRIVER = Path(__file__).resolve().parents[1] / 'release-qualify'


class Qualification(unittest.TestCase):
    def load(self):
        self.assertTrue(DRIVER.exists(), 'qualification driver is missing')
        loader = importlib.machinery.SourceFileLoader('qualify', str(DRIVER))
        spec = importlib.util.spec_from_loader(loader.name, loader)
        module = importlib.util.module_from_spec(spec)
        loader.exec_module(module)
        return module

    def run_driver(self, fail=(), self_test=True):
        module = self.load()
        calls = []
        def run(argv, **kwargs):
            calls.append((argv, kwargs['env']))
            return type('Result', (), {'returncode': int(any(x in argv for x in fail))})()
        with tempfile.TemporaryDirectory() as evidence, patch.object(module.subprocess, 'run', run), contextlib.redirect_stdout(io.StringIO()):
            rc = module.qualify(Path(evidence), self_test)
            summary = (Path(evidence) / 'summary.json').read_text()
        self.assertTrue(all(not Path(e['TMPDIR']).exists() for _, e in calls))
        return calls, rc, summary

    def test_order_isolation_and_self_test(self):
        calls, rc, _ = self.run_driver()
        self.assertEqual(rc, 0)
        commands = [argv for argv, _ in calls]
        suite = next(i for i, a in enumerate(commands) if 'nextest' in a and '--no-run' not in a)
        build = commands.index(['nice', 'scripts/build.sh'])
        self.assertLess(suite, build)
        self.assertEqual(sum('nextest' in a and '--no-run' not in a for a in commands), 1)
        self.assertFalse(any('scripts/wake-e2e-herdr' in a for a in commands))
        self.assertTrue(any('scripts/herdr-release-smoke' in a for a in commands))
        self.assertTrue(any('installer_terminal_test.py' in ' '.join(a) for a in commands))
        envs = {e['HT_LEAK_RUN_ID']: e for _, e in calls}
        self.assertGreaterEqual(len(envs), 5)
        self.assertEqual(len(envs), len({e['TMPDIR'] for e in envs.values()}))
        self.assertEqual(len(envs), len({e['HERDR_PLUGIN_STATE_DIR'] for e in envs.values()}))
        self.assertEqual(len(envs), len({e['HT_HERDR_RELEASE_CACHE'] for e in envs.values()}))

    def test_failures_aggregate_and_suite_always_checks_leaks(self):
        calls, rc, summary = self.run_driver(('nextest', 'tests/release/installer_alias_test.py'))
        self.assertEqual(rc, 1)
        self.assertIn('FAIL', summary)
        self.assertTrue(any('scripts/check-no-leaked-processes' in a for a, _ in calls))
        self.assertTrue(any('tests/release/installer_terminal_test.py' in a for a, _ in calls))

    def test_live_wake_uses_newest_after_self_test(self):
        calls, rc, _ = self.run_driver(self_test=False)
        self.assertEqual(rc, 0)
        commands = [a for a, _ in calls]
        wake = next(a for a in commands if 'scripts/wake-e2e-herdr' in a)
        self.assertEqual(wake[1], self.load().newest_release(DRIVER.parent.parent / 'tests/herdr-releases.tsv'))
        self.assertLess(commands.index(['python3', 'scripts/wake-e2e.py', '--self-test']), commands.index(wake))

    def test_failed_candidate_blocks_live_wake_and_binary_installers(self):
        calls, rc, summary = self.run_driver(('scripts/build.sh',), self_test=False)
        commands = [a for a, _ in calls]
        self.assertEqual(rc, 1)
        self.assertFalse(any('scripts/wake-e2e-herdr' in a for a in commands))
        self.assertFalse(any('tests/release/installer_terminal_test.py' in a for a in commands))
        self.assertFalse(any('tests/release/install_test.sh' in a for a in commands))
        self.assertTrue(any('--check-upstream' in a for a in commands))
        self.assertTrue(any('tests/release/installer_alias_test.py' in a for a in commands))
        self.assertIn('BLOCKED', summary)

    def test_cleanup_error_preserves_summary(self):
        module = self.load()
        def run(argv, **kwargs):
            return type('Result', (), {'returncode': 0})()
        created = []
        original_temp = module.tempfile.mkdtemp
        original_remove = shutil.rmtree
        def temp(**kwargs):
            root = original_temp(**kwargs)
            created.append(root)
            return root
        with tempfile.TemporaryDirectory() as evidence:
            with patch.object(module.tempfile, 'mkdtemp', temp), patch.object(module.subprocess, 'run', run), patch.object(module.shutil, 'rmtree', side_effect=OSError('cleanup denied')), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(module.qualify(Path(evidence), True), 1)
                self.assertIn('cleanup denied', (Path(evidence) / 'summary.json').read_text())
        for root in created:
            original_remove(root)

    def test_branch_exception_is_reported(self):
        module = self.load()
        def run(argv, **kwargs):
            if '--check-upstream' in argv:
                raise RuntimeError('injected branch exception')
            return type('Result', (), {'returncode': 0})()
        with tempfile.TemporaryDirectory() as evidence, patch.object(module.subprocess, 'run', run), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(module.qualify(Path(evidence), True), 1)
            self.assertIn('injected branch exception', (Path(evidence) / 'summary.json').read_text())

    def test_newest_is_numeric_not_file_order(self):
        module = self.load()
        with tempfile.TemporaryDirectory() as root:
            table = Path(root) / 'releases.tsv'
            table.write_text('# header\n0.10.0\tasset\tdigest\n0.9.9\tasset\tdigest\n')
            self.assertEqual(module.newest_release(table), '0.10.0')


if __name__ == '__main__':
    unittest.main()
