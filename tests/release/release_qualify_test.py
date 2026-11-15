#!/usr/bin/env python3
"""Offline release routing checks; all qualification subprocesses are mocked."""
import contextlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
loader = importlib.machinery.SourceFileLoader('release_qualify', str(ROOT / 'scripts/release-qualify'))
spec = importlib.util.spec_from_loader(loader.name, loader)
release = importlib.util.module_from_spec(spec)
loader.exec_module(release)


class QueueProofRouting(unittest.TestCase):
    def wake_command(self, queue_proof=False):
        with tempfile.TemporaryDirectory(prefix='ht-release-routing-') as temp:
            evidence = Path(temp) / 'evidence'
            with mock.patch.object(release.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
                with contextlib.redirect_stdout(io.StringIO()):
                    if queue_proof:
                        result = release.qualify(evidence, False, queue_proof=True)
                    else:
                        result = release.qualify(evidence, False)
            self.assertEqual(result, 0)
            commands = [call.args[0] for call in run.call_args_list]
            wakes = [argv for argv in commands if argv[0] == 'scripts/wake-e2e-herdr']
            self.assertEqual(len(wakes), 1)
            self.assertFalse(any("--queue-proof" in argv for argv in commands if argv is not wakes[0]))
            return wakes[0], evidence

    def test_linux_flag_forwards_only_to_authenticated_wake(self):
        with mock.patch.object(sys, 'platform', 'linux'):
            argv, evidence = self.wake_command(queue_proof=True)
        self.assertEqual(argv, ['scripts/wake-e2e-herdr', release.newest_release(ROOT / 'tests/herdr-releases.tsv'),
                                '--evidence-dir', str(evidence / 'wake'), '--queue-proof'])

    def test_unflagged_command_unchanged_on_linux_and_macos(self):
        for platform in ('linux', 'darwin'):
            with self.subTest(platform=platform), mock.patch.object(sys, 'platform', platform):
                argv, evidence = self.wake_command()
            self.assertEqual(argv, ['scripts/wake-e2e-herdr', release.newest_release(ROOT / 'tests/herdr-releases.tsv'),
                                    '--evidence-dir', str(evidence / 'wake')])

    def test_linux_cli_selects_queue_proof(self):
        with mock.patch.object(sys, 'platform', 'linux'), \
                mock.patch.object(sys, 'argv', ['release-qualify', '--queue-proof', '--evidence-dir', '/unused/evidence']), \
                mock.patch.object(release, 'qualify', return_value=0) as qualify:
            self.assertEqual(release.main(), 0)
        qualify.assert_called_once_with(Path('/unused/evidence'), False, queue_proof=True)

    def test_non_linux_explicit_proof_refuses_before_any_side_effect(self):
        with mock.patch.object(sys, 'platform', 'darwin'), \
                mock.patch.object(sys, 'argv', ['release-qualify', '--queue-proof']), \
                mock.patch.object(release, 'qualify') as qualify, \
                mock.patch.object(release.tempfile, 'mkdtemp') as scratch, \
                mock.patch.object(release.subprocess, 'run') as run, \
                contextlib.redirect_stderr(io.StringIO()) as errors:
            with self.assertRaises(SystemExit) as error:
                release.main()
        self.assertEqual(error.exception.code, 2)
        self.assertIn("--queue-proof requires Linux", errors.getvalue())
        qualify.assert_not_called()
        scratch.assert_not_called()
        run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
