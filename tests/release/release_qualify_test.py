#!/usr/bin/env python3
"""Offline release routing checks; all qualification subprocesses are mocked."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
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
    def wake_command(self, queue_proof=False, install_enabled=False):
        with tempfile.TemporaryDirectory(prefix='ht-release-routing-') as temp:
            evidence = Path(temp) / 'evidence'
            with mock.patch.object(release, 'operator_programmatic_wake', return_value=install_enabled), \
                    mock.patch.object(release.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
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


class OperatorQueueMode(unittest.TestCase):
    def configured(self, raw):
        with tempfile.TemporaryDirectory(prefix='ht-mode-test-') as temp:
            node=Path(temp)/'node';node.mkdir()
            if raw is not None:(node/'settings.json').write_bytes(raw)
            report={'doctor':{'node_dir':str(node),'daemon':{'state':'not_running'},'harness_states':{'warning':'unrelated'}}}
            with mock.patch.object(release,'_doctor_report',return_value=report):
                return release.operator_programmatic_wake()

    def test_missing_file_key_off_and_true(self):
        for raw,expected in [(None,False),(b'{}',False),(b'{"programmatic_wake":false}',False),(b'{"programmatic_wake":true}',True)]:
            self.assertIs(self.configured(raw),expected)

    def test_invalid_setting_and_unresolved_probe_do_not_silently_downgrade(self):
        for raw in [b'[]',b'{bad',b'{"programmatic_wake":1}',b'{"programmatic_wake":"on"}',b' '*4097]:
            with self.subTest(raw=raw[:40]),self.assertRaises(RuntimeError):self.configured(raw)
        for report in [{},{'doctor':None},{'doctor':[]},{'doctor':{'node_dir':'relative'}},{'doctor':{'node_dir':[]}}]:
            with mock.patch.object(release,'_doctor_report',return_value=report),self.assertRaises(RuntimeError):
                release.operator_programmatic_wake()

    def test_duplicate_mode_keys_are_ambiguous_not_last_value_wins(self):
        with self.assertRaises(RuntimeError):
            self.configured(b'{"programmatic_wake":true,"programmatic_wake":false}')
        with self.assertRaises(RuntimeError):
            release._doctor_report([sys.executable,'-c',
                'import sys;sys.stdout.write(' + repr('{"doctor":{"node_dir":"/first","node_dir":"/second"}}') + ')'])

    def test_selected_install_on_routes_required_queue_proof(self):
        with mock.patch.object(sys,'platform','linux'):
            argv,_=QueueProofRouting().wake_command(install_enabled=True)
        self.assertIn('--queue-proof',argv)

    def qualification(self, explicit=False, self_test=False):
        with tempfile.TemporaryDirectory(prefix='ht-mode-route-') as temp, \
                mock.patch.object(release.subprocess,'run',return_value=subprocess.CompletedProcess([],0)) as run, \
                mock.patch.object(release,'operator_programmatic_wake',side_effect=RuntimeError('unresolved')) as probe, \
                mock.patch.object(sys,'platform','linux'),contextlib.redirect_stdout(io.StringIO()):
            result=release.qualify(Path(temp)/'evidence',self_test,queue_proof=explicit)
            commands=[call.args[0] for call in run.call_args_list]
            return result,commands,probe.call_count

    def test_explicit_proof_bypasses_operator_probe(self):
        result,commands,count=self.qualification(explicit=True)
        self.assertEqual(result,0);self.assertEqual(count,0)
        self.assertTrue(any(argv[0]=='scripts/wake-e2e-herdr' and '--queue-proof' in argv for argv in commands))

    def test_self_test_skips_real_operator_probe(self):
        result,commands,count=self.qualification(self_test=True)
        self.assertEqual(result,0);self.assertEqual(count,0)
        self.assertFalse(any(argv[0]=='scripts/wake-e2e-herdr' for argv in commands))

    def test_unresolved_mode_blocks_authenticated_wake(self):
        result,commands,count=self.qualification()
        self.assertEqual(result,1);self.assertEqual(count,1)
        self.assertFalse(any(argv[0]=='scripts/wake-e2e-herdr' for argv in commands))

    def test_probe_execution_and_output_are_bounded(self):
        with mock.patch.object(release,'DOCTOR_OUTPUT_LIMIT',128),self.assertRaisesRegex(RuntimeError,'bound'):
            release._doctor_report([sys.executable,'-c',"print('x'*200)"])
        with mock.patch.object(release,'DOCTOR_PROBE_TIMEOUT',0.05),self.assertRaisesRegex(RuntimeError,'timed out'):
            release._doctor_report([sys.executable,'-c','import time;time.sleep(10)'])

    def test_valid_canonical_report_does_not_require_healthy_exit(self):
        report=release._doctor_report([sys.executable,'-c','import json;print(json.dumps({"doctor":{"node_dir":"/valid"}}));raise SystemExit(1)'])
        self.assertEqual(report['doctor']['node_dir'],'/valid')

    def test_non_linux_ordinary_mode_never_probes_operator(self):
        with tempfile.TemporaryDirectory(prefix='ht-native-mode-') as temp, \
                mock.patch.object(release.subprocess,'run',return_value=subprocess.CompletedProcess([],0)), \
                mock.patch.object(release,'operator_programmatic_wake',side_effect=AssertionError('unexpected probe')) as probe, \
                mock.patch.object(sys,'platform','darwin'),contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(release.qualify(Path(temp)/'evidence',False),0)
            probe.assert_not_called()


if __name__ == '__main__':
    unittest.main()
