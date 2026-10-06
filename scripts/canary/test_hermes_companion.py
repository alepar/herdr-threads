"""Synthetic executable driver checks. These never execute installed Hermes."""
import importlib.util
import json
import os
from pathlib import Path
import sys
_saved = list(sys.path)
HERE = Path(__file__).resolve().parent
sys.path[:] = [p for p in sys.path if os.path.realpath(p or '.') != str(HERE)]
sys.modules.pop('bisect', None)
import hashlib
import subprocess
import tempfile
import unittest
sys.path[:] = _saved
ROOT = HERE.parents[1]

def load(name):
    spec = importlib.util.spec_from_file_location('hermes_test_' + name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
runner, manifest = load('run'), load('manifest')
IDENTITY = json.loads((HERE / 'testdata/hermes-native-api.json').read_text())['identity']
IDENTITY['key'] = 'build:' + hashlib.sha256(json.dumps(dict(IDENTITY, schema_version=1), sort_keys=True,
    separators=(',', ':')).encode()).hexdigest()

def adapter():
    events = [dict(event='pre_llm_call', milestone='qualified_turn', always_send=False),
              dict(event='post_tool_call', milestone='qualified_post_tool', always_send=False)]
    return dict(id='hermes', display_name='Hermes', host_kinds=['hermes'], setup_scopes=[],
        legacy_contract_id=None, contracts=[dict(domain=d, origin=o, id=i, events=events,
        required_milestones=['qualified_turn', 'qualified_post_tool']) for d, o, i in [
            ('native_callback', 'native_shape_observation', '1111111111111111'),
            ('bridge_envelope', 'bridge_envelope', '2222222222222222')]],
        canary_strategy=dict(kind='exact_runtime', candidate_kind='exact_build', npm_package=None,
            model_key_env=None, companion='scripts/canary/adapters/hermes.py', artifact_schema_version=1))

def result():
    return dict(schema_version=1, harness='hermes', attempt='try1', identity=IDENTITY,
        evidence_stage='no_model', outcome='inconclusive', reason='synthetic fixture', domains=[])

class HermesCompanion(unittest.TestCase):
    def test_exact_commit_build_generic_schema_and_manifest_final_incomplete(self):
        a, r = adapter(), result()
        self.assertEqual(runner.validate_result(json.dumps(r).encode(), a, 'try1', 'no_model')['identity'], IDENTITY)
        row = dict(harness='hermes', identity=IDENTITY, domain='native_callback', origin='native_shape_observation',
            contract_id='1111111111111111', status='verified', evidence_stage='no_model', source='canary',
            required_milestones=['qualified_turn', 'qualified_post_tool'],
            successful_milestones=['qualified_turn', 'qualified_post_tool'], broken_event=None, broken_field=None,
            supported_since=None, issue_url=None, last_seen_at=1)
        baseline = dict(runtime_contracts={'hermes': a['contracts']}, runtime_rows=[row])
        manifest.validate_runtime(baseline)
        output = {}
        entry = dict(harness='hermes', identity_key=IDENTITY['key'])
        complete = dict(r, outcome='complete', domains=[dict(domain=c['domain'], origin=c['origin'], contract_id=c['id'],
            outcome='compatible', successful_milestones=c['required_milestones'], violations=[]) for c in a['contracts']])
        manifest.add_runtime(output, baseline, {'adapters': [a]}, [(entry, complete), (entry, r)], '2026-10-06T00:00:00Z')
        self.assertEqual(output['runtime_rows'], baseline['runtime_rows'])
        self.assertEqual(output['runtime_rows'][0]['identity']['source'], 'commit-build')
        for source in ['commit-Build', 'commit-build-other', 'commit-build\n']:
            bad = json.loads(json.dumps(r)); bad['identity']['source'] = source
            with self.assertRaises(ValueError): runner.validate_result(json.dumps(bad).encode(), a, 'try1', 'no_model')

    def test_hermes_companion_explicit_runtime_no_model_native_load_and_missing_input_inconclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            index = dict(schema_version=1, attempts=[])
            got, _ = runner.invoke(adapter(), ROOT, '/bin/false', out, 'missing', 'no_model', {}, index)
            self.assertEqual(got['outcome'], 'inconclusive')
            self.assertEqual(got['reason'], 'explicit runtime input required')
            self.assertIsNone(got['identity'])
            fixture = out / 'producer.py'
            doc = dict(schema_version=1, provenance='synthetic_fixture', identity=IDENTITY,
                       native_loaded=True, domains=adapter()['contracts'])
            fixture.write_text('import json\nprint(' + repr(json.dumps(doc)) + ')\n')
            command = out / 'runtime.json'
            command.write_text(json.dumps(dict(schema_version=1, producer='synthetic_fixture',
                argv=[sys.executable, str(fixture)], timeout_seconds=2)))
            got, _ = runner.invoke(adapter(), ROOT, '/bin/false', out, 'fixture', 'no_model', {}, index, command)
            self.assertEqual(got['identity'], IDENTITY)
            self.assertEqual(got['outcome'], 'inconclusive')
            self.assertEqual(runner.verified_domains(got, adapter()), [])
            self.assertIn('fixture', got['reason'])
            runner.validate_index(json.dumps(index).encode(), out, [adapter()])
            block = runner.run_strategy(adapter(), out / 'strategy', '/bin/false', ROOT, runtime_command=command)
            self.assertEqual(block['status'], 'inconclusive')
            self.assertEqual(block['identity'], IDENTITY)

    def test_malformed_input_and_identity_stage_domain_cannot_credit_native(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            for n, raw in enumerate(['{}', '{"schema_version":1,"schema_version":1}', 'x',
                                    json.dumps(dict(schema_version=1, producer='fake_native_manager', argv=['/bin/false'], timeout_seconds=1))]):
                path = out / ('input' + str(n)); path.write_text(raw)
                got, _ = runner.invoke(adapter(), ROOT, '/bin/false', out, 'bad' + str(n), 'no_model', {},
                                       dict(schema_version=1, attempts=[]), path)
                self.assertEqual(got['outcome'], 'inconclusive')
                self.assertIsNone(got['identity'])
                self.assertEqual(got['domains'], [])
        for field, value in [('harness', 'codex'), ('attempt', 'other'), ('evidence_stage', 'live')]:
            bad = dict(result(), **{field: value})
            with self.assertRaises(ValueError): runner.validate_result(json.dumps(bad).encode(), adapter(), 'try1', 'no_model')
        bad = dict(result(), domains=[dict(domain='fake', origin='native_shape_observation', contract_id='1111111111111111',
            successful_milestones=[], violations=[], outcome='inconclusive')])
        with self.assertRaises(ValueError): runner.validate_result(json.dumps(bad).encode(), adapter(), 'try1', 'no_model')

    def test_timeout_and_output_overflow_stop_owned_descendants(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            pidfile = out / 'pid'
            fixture = out / 'hung.py'
            fixture.write_text('import subprocess,sys,time,signal\np=subprocess.Popen([sys.executable,"-c","import time;time.sleep(30)"])\n'
                'def stop(*a):\n p.wait(timeout=1)\n open(' + repr(str(out / 'stopped')) + ',"w").write("reaped")\n sys.exit(0)\n'
                'signal.signal(signal.SIGTERM,stop)\nopen(' + repr(str(pidfile)) + ',"w").write(str(p.pid))\ntime.sleep(30)\n')
            command = out / 'runtime.json'
            command.write_text(json.dumps(dict(schema_version=1, producer='synthetic_fixture',
                argv=[sys.executable, str(fixture)], timeout_seconds=.3)))
            got, _ = runner.invoke(adapter(), ROOT, '/bin/false', out, 'timeout', 'no_model', {},
                                   dict(schema_version=1, attempts=[]), command, timeout=4)
            self.assertEqual(got['outcome'], 'inconclusive')
            self.assertIn('timeout', got['reason'])
            pid = int(pidfile.read_text())
            self.assertGreater(pid, 0)
            self.assertEqual((out / 'stopped').read_text(), 'reaped')
            fixture.write_text('print("private body" * 10000)\n')
            got, work = runner.invoke(adapter(), ROOT, '/bin/false', out, 'overflow', 'no_model', {},
                                     dict(schema_version=1, attempts=[]), command)
            self.assertEqual(got['outcome'], 'inconclusive')
            self.assertNotIn('private body', (work / 'result.json').read_text())

    def test_private_completed_gate_source_fixture_is_exact_and_bounded(self):
        spec = importlib.util.spec_from_file_location('hermes_driver_test', HERE / 'adapters/hermes.py')
        driver = importlib.util.module_from_spec(spec); spec.loader.exec_module(driver)
        import time
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / 'state'; state.mkdir(mode=0o700)
            gates = state / 'harness' / 'evidence-v2'; gates.mkdir(mode=0o700, parents=True)
            gates.parent.chmod(0o700)
            paths = driver.gate_paths({'identity': IDENTITY, 'state_root': str(state)}, adapter(), 'fixture-session')
            now = int(time.time() * 1000)
            for path, key, c in paths:
                doc = dict(version=2, key=key, verified=True, milestones=c['required_milestones'], heartbeat_at_ms=now, sent=[])
                path.write_text(json.dumps(doc)); path.chmod(0o600)
            domains = driver.completed_gates(paths, now)
            self.assertEqual([d['domain'] for d in domains], ['native_callback', 'bridge_envelope'])
            # These are synthetic client-output fixtures, never native acceptance.
            self.assertEqual(runner.verified_domains(dict(result(), domains=domains), adapter()), [])
            path, key, c = paths[0]
            original = json.loads(path.read_text())
            for field, value in [('verified', False), ('milestones', ['qualified_turn']), ('heartbeat_at_ms', now - 1),
                                 ('key', dict(key, runtime=None)), ('sent', ['violation'])]:
                path.write_text(json.dumps(dict(original, **{field: value})))
                with self.assertRaises(ValueError): driver.completed_gates(paths, now)
            path.write_text(json.dumps(original)); path.chmod(0o644)
            with self.assertRaises(ValueError): driver.completed_gates(paths, now)
            path.chmod(0o600); path.unlink(); path.symlink_to(paths[1][0])
            with self.assertRaises(OSError): driver.completed_gates(paths, now)

    def test_existing_owned_generation_requires_native_fingerprint_and_unchanged_settings(self):
        spec = importlib.util.spec_from_file_location('hermes_driver_assets_test', HERE / 'adapters/hermes.py')
        driver = importlib.util.module_from_spec(spec); spec.loader.exec_module(driver)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); home = root / 'home with spaces'; home.mkdir(mode=0o700)
            assets = home / 'plugins' / 'herdr-threads'; assets.mkdir(mode=0o700, parents=True)
            lock = home / 'plugins' / '.herdr-threads-operation.lock'; lock.touch(mode=0o600)
            state = root / 'state'; setup = state / 'setup' / 'hermes'; setup.mkdir(mode=0o700, parents=True)
            token = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
            data = dict(physical_home=str(home), state_root=str(state), installation_token=token,
                        binary='/bin/false', host_endpoint=str(root / 'host.sock'))
            settings = dict(schema_version=1, bridge_schema_version=1, installation_token=token,
                            rust_executable='/bin/false', state_root=str(state), host_endpoint=data['host_endpoint'])
            content = {'__init__.py': b'# fixture only', 'plugin.yaml': b'# fixture only',
                       'bridge_config.json': json.dumps(settings).encode()}
            records = {}
            for name, raw in content.items():
                path = assets / name; path.write_bytes(raw); path.chmod(0o600)
                records[name] = dict(digest='sha256:' + hashlib.sha256(raw).hexdigest(), prior=None, bytes=list(raw))
            m = dict(schema_version=1, home=str(home), installation_token=token, phase='complete', stage_name=None,
                lock_inode=[lock.stat().st_dev, lock.stat().st_ino], directory_inode=[assets.stat().st_dev, assets.stat().st_ino], assets=records)
            manifest_path = setup / ('sha256:' + hashlib.sha256(str(home).encode()).hexdigest() + '.json')
            manifest_path.write_text(json.dumps(m)); manifest_path.chmod(0o600)
            self.assertEqual(driver.owned_generation(data)[0], assets)
            (assets / '__init__.py').write_text('# altered')
            with self.assertRaises(ValueError): driver.owned_generation(data)
            (assets / '__init__.py').write_bytes(content['__init__.py'])
            with self.assertRaises(ValueError): driver.owned_generation(dict(data, binary='/bin/true'))
            m['phase'] = 'preparing'; manifest_path.write_text(json.dumps(m))
            with self.assertRaises(ValueError): driver.owned_generation(data)

    def test_source_stage_and_forged_native_producer_never_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory); script = out / 'fixture.py'; marker = out / 'ran'
            script.write_text('open(' + repr(str(marker)) + ',"w").write("ran")\nprint("{}")\n')
            path = out / 'runtime.json'
            path.write_text(json.dumps(dict(schema_version=1, producer='synthetic_fixture', argv=[sys.executable, str(script)], timeout_seconds=1)))
            got, _ = runner.invoke(adapter(), ROOT, '/bin/false', out, 'source', 'source_captured', {}, dict(schema_version=1, attempts=[]), path)
            self.assertEqual(got['outcome'], 'inconclusive'); self.assertFalse(marker.exists())
            script.write_text('print(' + repr(json.dumps(dict(schema_version=1, provenance='official_native_selective', identity=IDENTITY,
                native_loaded=True, domains=[]))) + ')\n')
            got, _ = runner.invoke(adapter(), ROOT, '/bin/false', out, 'forged', 'no_model', {}, dict(schema_version=1, attempts=[]), path)
            self.assertIsNone(got['identity']); self.assertEqual(got['domains'], [])

if __name__ == '__main__': unittest.main()
