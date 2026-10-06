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
import sqlite3
import shlex
import urllib.request
import io
import uuid
import subprocess
import tempfile
import unittest
from unittest import mock
from contextlib import contextmanager, ExitStack
from types import ModuleType, SimpleNamespace
import threading
import time
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

@contextmanager
def synthetic_native_fixture(mode='settled'):
    """Fake source/API context only; calls real driver, never installed Hermes.

    All filesystem inputs are private source fixtures. The pending callback is
    an owned thread released/joined here, including assertion/error exits.
    No subprocess, native acceptance artifact, host server or model is created.
    """
    driver = load('adapters/hermes')
    with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
        root = Path(directory).resolve()
        source = root / 'source'; source.mkdir()
        home = root / 'home'; home.mkdir(mode=0o700)
        state = root / 'state'; state.mkdir(mode=0o700)
        assets = home / 'plugins' / 'herdr-threads'; assets.mkdir(mode=0o700, parents=True)
        lock = assets.parent / '.herdr-threads-operation.lock'; lock.touch(mode=0o600)
        setup = state / 'setup' / 'hermes'; setup.mkdir(mode=0o700, parents=True)
        gates = state / 'harness' / 'evidence-v2'; gates.mkdir(mode=0o700, parents=True)
        gates.parent.chmod(0o700)
        selected = root / 'site-packages'; selected.mkdir()
        work = root / 'work'; work.mkdir()
        (work / 'request.json').write_text(json.dumps({'adapter': adapter()}))
        binary = root / 'synthetic-binary'; binary.write_text('# source fixture, never executed'); binary.chmod(0o700)
        endpoint = root / 'synthetic-endpoint'
        path = root / 'runtime.json'
        data = dict(schema_version=1, producer='official_native_selective', timeout_seconds=1,
            profile='fixture', home=str(home), physical_home=str(home), source_root=str(source),
            isolation_root=str(root), state_root=str(state), host_endpoint=str(endpoint), identity=IDENTITY,
            installation_token='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', binary=str(binary),
            argv=[sys.executable, '-I', '-c', 'opaque synthetic source fixture; never executed',
                  '--count', '--no-report', str(driver.HERE), '--native-driver', str(path)])
        path.write_text(json.dumps(data))
        settings = dict(schema_version=1, bridge_schema_version=1, installation_token=data['installation_token'],
            rust_executable=str(binary), state_root=str(state), host_endpoint=str(endpoint))
        contents = {'__init__.py': b'# synthetic plugin source', 'plugin.yaml': b'# synthetic manifest',
                    'bridge_config.json': json.dumps(settings).encode()}
        records = {}
        for name, raw in contents.items():
            asset = assets / name; asset.write_bytes(raw); asset.chmod(0o600)
            records[name] = dict(digest='sha256:' + hashlib.sha256(raw).hexdigest(), prior=None, bytes=list(raw))
        generation = dict(schema_version=1, home=str(home), installation_token=data['installation_token'],
            phase='complete', stage_name=None, lock_inode=[lock.stat().st_dev, lock.stat().st_ino],
            directory_inode=[assets.stat().st_dev, assets.stat().st_ino], assets=records)
        generation_path = setup / ('sha256:' + hashlib.sha256(str(home).encode()).hexdigest() + '.json')
        generation_path.write_text(json.dumps(generation)); generation_path.chmod(0o600)
        modules = {}
        for name in ('hermes_bootstrap', 'hermes_constants', 'hermes_cli', 'hermes_cli.profiles',
                     'hermes_cli.version_info', 'hermes_cli.config', 'hermes_cli.plugins'):
            file = source / (name.replace('.', '/') + '.py'); file.parent.mkdir(exist_ok=True); file.touch()
            module = ModuleType(name); module.__file__ = str(file)
            module.__spec__ = SimpleNamespace(origin=str(file), _initializing=False)
            modules[name] = module
        bootstrap = modules['hermes_bootstrap']
        bootstrap._root = source; bootstrap._pm_repair = False; bootstrap._launch_python = None
        profiles = modules['hermes_cli.profiles']
        profiles.normalize_profile_name = lambda name: name
        profiles.validate_profile_name = lambda name: None
        profiles.resolve_profile_env = lambda name: str(home)
        modules['hermes_constants'].get_hermes_home = lambda: home
        modules['hermes_cli.version_info'].get_version_info = lambda: SimpleNamespace(**{
            k: v for k, v in IDENTITY.items() if k not in ('key', 'release_version')})
        config = modules['hermes_cli.config']
        config.FailedConfigRead = type('SyntheticFailedConfigRead', (), {})
        effective = {'plugins': {'enabled': ['herdr-threads'], 'disabled': []}}
        config.load_config_readonly = lambda: effective
        m = SimpleNamespace(name='herdr-threads', path=assets, kind='standalone', source='user',
            manifest_version=2, portable=False, requires_env=[], requires_plugins=[],
            python_dependencies=[], provides_tools=[], capabilities=[])
        stop, ready = threading.Event(), threading.Event()
        workers, calls, seen_gates = [], [], []
        reader = SimpleNamespace(snapshot=lambda: {'synthetic': True},
            thread=SimpleNamespace(is_alive=lambda: False), bridge=SimpleNamespace(unreaped_child=None))
        slot_name = '_herdr_threads_hermes_reader_schema1'
        slot = ModuleType(slot_name); slot.reader = reader
        plugin = SimpleNamespace(SLOT=slot_name,CHILD_RESTRICTION='cooperative children may read; never check in or ACK')

        def measured_callback(event, kwargs):
            """Explicit source-shaped native API stand-in writes actual instance layout."""
            measurement=data['measurement']
            inst=state/'instances'/hashlib.sha256(os.fsencode(data['host_endpoint'])).hexdigest()
            inst.mkdir(parents=True,exist_ok=True,mode=0o700);inst.parent.chmod(0o700)
            namespace=inst/'namespace'
            namespace.write_text('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');namespace.chmod(0o600)
            ctx=inst/'contexts'/'explicit-source-fixture';ctx.mkdir(parents=True,exist_ok=True,mode=0o700)
            ctx.parent.chmod(0o700);journal=ctx/'context.json'
            doc=json.loads(journal.read_text()) if journal.exists() else dict(version=1,instance=namespace.read_text(),
                seat=measurement['seat'],current=None,pending=None,completed=[],declared_resets=[],prepared_kinds=[])
            session=kwargs['session_id']
            if kwargs.get('parent_session_id'):
                if mode=='child-write':
                    (ctx/'attention.json').write_text('{"synthetic_child_write":true}');(ctx/'attention.json').chmod(0o600)
                return [{'context':plugin.CHILD_RESTRICTION}] if mode!='child-zero' else []
            if event=='on_session_reset':
                if mode=='reset-zero': return []
                doc['declared_resets'].append(dict(harness='Hermes',target=measurement['target'],session=session,
                    event_key='hermes:fixture-reset',process_nonce='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',sequence=3,
                    observed_at_millis=int(time.time()*1000),generation=doc['current']['binding_generation'],consumed=False))
                journal.write_text(json.dumps(doc));journal.chmod(0o600)
                return []
            previous=doc['current'];generation=previous['binding_generation']+1 if previous else 1
            execution=str(uuid.uuid4())
            current=dict(format_version=1,instance=doc['instance'],seat=measurement['seat'],target=measurement['target'],
                harness='Hermes',binding_generation=generation,execution=execution,session={'Native':session},role='TopLevel')
            raw=json.dumps(['pre_llm_call',session,kwargs['turn_id'],None,kwargs['api_request_id']],ensure_ascii=False,separators=(',',':')).encode()
            event_key='hermes:'+hashlib.sha256(raw).hexdigest()
            kind='Clear' if previous else 'Startup'
            if mode=='reset-startup' and previous:kind='Startup'
            request=dict(operation_id=str(uuid.uuid4()),mode='Lifecycle',context=dict(current,binding_generation=generation-1),
                expected_generation=generation-1 if previous else None,event_id=event_key,payload_version=1,payload=[1])
            done=dict(request=request,response=dict(context=current,historical=False,output=[1]),completed_at_millis=int(time.time()*1000))
            if mode=='wrong-event':done['request']['event_id']='hermes:wrong-event'
            doc['completed'].append(done)
            doc['prepared_kinds'].append(dict(event_id=event_key,operation_id=request['operation_id'],
                request_digest='sha256:'+hashlib.sha256(json.dumps([request,kind],ensure_ascii=False,separators=(',',':')).encode()).hexdigest(),kind=kind))
            doc['current']=current
            for hint in doc['declared_resets']: hint['consumed']=True
            journal.write_text(json.dumps(doc));journal.chmod(0o600)
            connection=sqlite3.connect(inst/'threads.sqlite3')
            connection.execute('CREATE TABLE IF NOT EXISTS occupant_bindings (seat_id TEXT,generation INTEGER,target_id TEXT,harness TEXT,native_session TEXT,execution_id TEXT,observation_provenance TEXT,registered_at INTEGER,ended_at INTEGER)')
            connection.execute('UPDATE occupant_bindings SET ended_at=? WHERE ended_at IS NULL',(int(time.time()*1000),))
            connection.execute('INSERT INTO occupant_bindings VALUES (?,?,?,?,?,?,?,?,?)',(measurement['seat'],generation,
                measurement['target'],'hermes',session,execution,'cooperative_top_level',int(time.time()*1000),None))
            if mode=='child-bytebound':
                connection.execute('CREATE TABLE IF NOT EXISTS bounded_fixture (oversized BLOB)')
                connection.execute('INSERT INTO bounded_fixture VALUES (?)',(b'x'*65537,))
            connection.commit();connection.close();(inst/'threads.sqlite3').chmod(0o600)
            if mode=='namespace':namespace.write_text('cccccccc-cccc-4ccc-8ccc-cccccccccccc')
            if mode=='return-zero':return []
            return [{'context':'cooperative private-context-never-public'}]

        class SyntheticManager:
            def __init__(self, *, scope_key):
                if scope_key != str(home): raise AssertionError('wrong selected scope')
                self.home_path = home
                self._hook_timeout_lock = threading.Lock()
                self._hook_running_callbacks = {}; self._hook_abandoned = {}
                self._plugins = {}; self._hooks = {}
                self.unloaded = False
                managers.append(self)
            def _scan_directory(self, parent, kind):
                calls.append('scan')
                if (parent, kind) != (assets.parent, 'user'): raise AssertionError('foreign scan')
                if mode == 'scan': raise ValueError('synthetic_scan')
                return [m]
            def _gate_manifest(self, manifest, disabled, enabled):
                calls.append('gate')
                return mode != 'gate' and manifest.name in enabled and manifest.name not in disabled
            def _warn_python_dependencies(self, manifest): calls.append('dependencies')
            def _validate_plugin_config_schema(self, manifest): calls.append('schema')
            def _load_plugin(self, manifest):
                calls.append('load')
                if mode == 'load': return
                self._plugins['herdr-threads'] = SimpleNamespace(enabled=True, manifest=m, error=None, module=plugin)
                self._hooks = {'pre_llm_call': [self.callback], 'post_tool_call': [self.callback]}
                if 'measurement' in data:self._hooks['on_session_reset']=[self.callback]
            def callback(self, event, kwargs):
                if 'measurement' in data and (event=='on_session_reset' or kwargs.get('parent_session_id')):
                    return
                paths = driver.gate_paths(data, adapter(), kwargs['session_id'])
                milestones = ['qualified_turn'] if event == 'pre_llm_call' else ['qualified_turn', 'qualified_post_tool']
                for n, (gate, key, contract) in enumerate(paths):
                    if mode == 'missing' or (mode == 'partial' and n == 1): continue
                    doc = dict(version=2, key=key, verified=event == 'post_tool_call', milestones=milestones,
                               heartbeat_at_ms=int(time.time() * 1000), sent=[])
                    gate.write_text(json.dumps(doc)); gate.chmod(0o600)
                if event == 'post_tool_call':
                    seen_gates[:] = paths
                    if mode in ('pending', 'running'):
                        ready.set(); stop.wait(2)
            def invoke_hook(self, event, **kwargs):
                calls.append(event)
                if mode == 'dispatch': raise ValueError('synthetic_dispatch')
                for callback in self._hooks[event]:
                    if event == 'post_tool_call' and mode in ('pending', 'running'):
                        key = (event, id(callback), kwargs['session_id']); token = object()
                        self._hook_running_callbacks[key] = token
                        if mode == 'pending': self._hook_abandoned[(event, id(callback))] = {key}
                        def owned_worker():
                            try: callback(event, kwargs)
                            finally:
                                with self._hook_timeout_lock:
                                    self._hook_running_callbacks.pop(key, None)
                                    self._hook_abandoned.clear()
                        worker = threading.Thread(target=owned_worker, name='synthetic-canary-pending')
                        workers.append(worker); worker.start()
                        if not ready.wait(1): raise AssertionError('synthetic output not ready')
                    else: callback(event, kwargs)
                if event == 'post_tool_call':
                    if mode == 'drift': (assets / '__init__.py').write_text('# changed source fixture')
                    if mode == 'missing-bookkeeping': del self._hook_running_callbacks
                    if mode == 'missing-abandoned': del self._hook_abandoned
                    if mode == 'missing-lock': del self._hook_timeout_lock
                    if mode == 'abandoned-only': self._hook_abandoned = {('post_tool_call', 1): {('synthetic',)}}
                    if mode == 'unknown-running': self._hook_running_callbacks = []
                    if mode == 'unknown-bookkeeping': self._hook_abandoned = []
                    if mode == 'contended': self._hook_timeout_lock.acquire()
                if 'measurement' in data and event!='post_tool_call':return measured_callback(event,kwargs)
                return []  # A result/ACK says nothing about callback settlement.
            def unload(self):
                calls.append('unload'); self.unloaded = True
                if mode == 'unload': raise ValueError('synthetic_unload')
                # Explicitly emulate reviewed ledger erasure, without joining worker.
                if hasattr(self, '_hook_running_callbacks'): self._hook_running_callbacks.clear()
                self._hook_abandoned = {}
                self._plugins.clear(); self._hooks.clear()
                return True

        managers = []
        plugins = modules['hermes_cli.plugins']; plugins.PluginManager = SyntheticManager
        @contextmanager
        def selected_scope(value):
            if value != home: raise AssertionError('wrong official profile scope')
            yield
        plugins._plugin_home_scope = selected_scope
        main = ModuleType('__main__'); main.__file__ = str(Path(driver.sysconfig.get_path('stdlib')).resolve() / 'trace.py')
        main.__spec__ = SimpleNamespace(name='trace', origin=main.__file__)
        stack.enter_context(mock.patch.dict(sys.modules, dict(modules, __main__=main, **{slot_name: slot})))
        stack.enter_context(mock.patch.object(sys, 'path', [str(driver.HERE.parent), str(selected), 'retained-tail']))
        stack.enter_context(mock.patch.object(sys, 'argv', data['argv'][-3:]))
        stack.enter_context(mock.patch.object(sys, 'orig_argv', data['argv']))
        stack.enter_context(mock.patch.dict(os.environ, {'PYTHONPATH': os.pathsep.join((str(source), str(selected))),
            'HERDR_HERMES_CANARY_WORK': str(work), 'HOME': str(root), 'HERMES_HOME': str(home)}))
        def synthetic_import(name):
            if name not in modules: raise AssertionError('forbidden installed import: ' + name)
            return modules[name]
        stack.enter_context(mock.patch.object(driver.importlib, 'import_module', side_effect=synthetic_import))
        # The endpoint stat is a labeled fake IPC boundary; no private host runs.
        original_lstat = Path.lstat
        def fixture_lstat(value):
            if value == endpoint: return SimpleNamespace(st_mode=0o140600, st_uid=os.geteuid())
            return original_lstat(value)
        stack.enter_context(mock.patch.object(Path, 'lstat', fixture_lstat))
        fixture = SimpleNamespace(driver=driver, data=data, path=path, modules=modules, calls=calls,
            managers=managers, gates=seen_gates, workers=workers, reader=reader, effective=effective, root=root)
        try:
            yield fixture
        finally:
            stop.set()
            for worker in workers:
                worker.join(timeout=2)
                if worker.is_alive(): raise AssertionError('owned synthetic worker survived')
            for manager in managers:
                if mode == 'contended' and manager._hook_timeout_lock.locked(): manager._hook_timeout_lock.release()

class SyntheticNativeOrchestration(unittest.TestCase):
    """Real production entry/driver tests using only explicitly fake native APIs."""
    def test_opt_in_measurement_reaches_actual_native_driver_without_legacy_promotion(self):
        with synthetic_native_fixture() as f:
            f.data['measurement']=dict(schema_version=1,invocation_id='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
                target='w1:p1',seat='s.fixture',context_marker='cooperative',child_form='explicit_parent_callback')
            f.path.write_text(json.dumps(f.data))
            observation=f.driver.native_driver(f.path)
            self.assertIn('measurement',observation,'actual native driver must expose opt-in separate measurements')
            self.assertEqual(observation['measurement']['scope'],'selective_native_api_invocation_not_model_delivery')
            self.assertNotIn('measurement',dict(result(),domains=observation['domains']))

    def test_opt_in_actual_selective_returns_child_conservation_and_consumed_clear(self):
        with synthetic_native_fixture() as f:
            f.data['measurement']=dict(schema_version=1,invocation_id='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
                target='w1:p1',seat='s.fixture',context_marker='cooperative',child_form='explicit_parent_callback')
            f.path.write_text(json.dumps(f.data))
            observation=f.driver.native_driver(f.path)
            rows=observation['measurement']['rows']
            self.assertEqual([r['verdict'] for r in rows],['PASS','PASS','PASS'])
            public=json.dumps(observation['measurement'])
            for private in ('private-context-never-public','s.fixture','w1:p1',str(f.root)):
                self.assertNotIn(private,public)
            self.assertIn('on_session_reset',f.calls)
            self.assertEqual(sum(call=='pre_llm_call' for call in f.calls),3)

    def test_opt_in_missing_wrong_namespace_child_writes_and_reset_none_are_not_pass(self):
        cases=[('return-zero',0),('wrong-event',0),('namespace',0),('child-zero',1),
               ('child-write',1),('child-bytebound',1),('reset-zero',2),('reset-startup',2)]
        for mode,index in cases:
            with self.subTest(mode=mode),synthetic_native_fixture(mode) as f:
                f.data['measurement']=dict(schema_version=1,invocation_id='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
                    target='w1:p1',seat='s.fixture',context_marker='cooperative',child_form='explicit_parent_callback')
                f.path.write_text(json.dumps(f.data))
                measured=f.driver.native_driver(f.path)['measurement']['rows']
                self.assertNotEqual(measured[index]['verdict'],'PASS')

    def test_opt_in_persist_disabled_child_form_is_explicitly_skipped(self):
        with synthetic_native_fixture() as f:
            f.data['measurement']=dict(schema_version=1,invocation_id='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
                target='w1:p1',seat='s.fixture',context_marker='cooperative',child_form='persist_disabled_no_callback')
            f.path.write_text(json.dumps(f.data))
            measured=f.driver.native_driver(f.path)['measurement']['rows'][1]
            self.assertEqual((measured['verdict'],measured['samples']),('SKIPPED',0))

    def test_opt_in_diagnostic_cannot_be_promoted_by_default_canary_wrapper(self):
        with synthetic_native_fixture() as f:
            f.data['measurement']=dict(schema_version=1,invocation_id='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',
                target='w1:p1',seat='s.fixture',context_marker='cooperative',child_form='explicit_parent_callback')
            f.path.write_text(json.dumps(f.data))
            observation=f.driver.native_driver(f.path)
            fake_engine=SimpleNamespace(_json=runner._json,
                bounded_capture=lambda *a,**kw:(0,json.dumps(observation).encode(),None))
            output=io.StringIO()
            with mock.patch.object(f.driver,'runner',return_value=fake_engine),mock.patch.object(sys,'stdout',output):
                f.driver.main(['--harness','hermes','--attempt','try1','--stage','no_model',
                    '--work-dir',str(f.root/'work'),'--binary',f.data['binary'],'--runtime-command-file',str(f.path)])
            measured=json.loads(output.getvalue())
            self.assertEqual(set(measured),set(result()))
            self.assertEqual((measured['outcome'],measured['domains']),('inconclusive',[]))
            self.assertNotIn('measurement',measured)

    def test_synthetic_pending_worker_with_fresh_domains_refuses_before_unload(self):
        # Removing the pre-unload dispatcher check must turn this refusal into success.
        with synthetic_native_fixture('pending') as f:
            with self.assertRaisesRegex(ValueError, 'dispatcher'):
                f.driver.native_driver(f.path)
            self.assertTrue(f.workers[0].is_alive())
            self.assertEqual(len(f.driver.completed_gates(f.gates, 0)), 2)
            self.assertTrue(f.managers[0].unloaded)
            self.assertEqual(f.managers[0]._hook_running_callbacks, {})

    def test_synthetic_settled_driver_source_fixture_domains_only(self):
        with synthetic_native_fixture() as f:
            observation = f.driver.native_driver(f.path)
            self.assertEqual(observation['identity'], IDENTITY)
            self.assertTrue(observation['native_loaded'])
            self.assertEqual([d['domain'] for d in observation['domains']], ['native_callback', 'bridge_envelope'])
            self.assertEqual(f.calls, ['scan', 'gate', 'dependencies', 'schema', 'load', 'pre_llm_call', 'post_tool_call', 'unload'])
            # Injected production return is synthetic evidence only, never native PASS.
            self.assertEqual(runner.verified_domains(dict(result(), domains=observation['domains']), adapter()), [])

    def test_synthetic_dispatcher_running_unknown_and_contended_refuse_bounded(self):
        # Removing/relaxing the dispatcher snapshot lets complete synthetic gates escape.
        for mode in ('running', 'abandoned-only', 'missing-bookkeeping', 'missing-abandoned',
                     'missing-lock', 'unknown-bookkeeping', 'unknown-running', 'contended'):
            with self.subTest(mode=mode), synthetic_native_fixture(mode) as f:
                started = time.monotonic()
                with self.assertRaisesRegex(ValueError, 'dispatcher'):
                    f.driver.native_driver(f.path)
                self.assertLess(time.monotonic() - started, .8)
                self.assertEqual(len(f.driver.completed_gates(f.gates, 0)), 2)
                self.assertTrue(f.managers[0].unloaded)

    def test_synthetic_dispatcher_missing_lock_and_expired_deadline_refuse(self):
        # Unknown lock state and expired budget cannot establish completion.
        with synthetic_native_fixture() as f:
            observation = f.driver.native_driver(f.path)
            self.assertEqual(len(observation['domains']), 2)
            manager = f.managers[0]
            with self.assertRaisesRegex(ValueError, 'dispatcher'):
                f.driver.require_settled_dispatcher(manager, time.monotonic() - 1)
            with mock.patch.object(f.driver.time, 'monotonic', side_effect=[0, 2]):
                with self.assertRaisesRegex(ValueError, 'dispatcher'):
                    f.driver.require_settled_dispatcher(manager, 1)
            self.assertFalse(manager._hook_timeout_lock.locked())
            del manager._hook_timeout_lock
            with self.assertRaisesRegex(ValueError, 'dispatcher'):
                f.driver.require_settled_dispatcher(manager, time.monotonic() + 1)

    def test_synthetic_config_disabled_unknown_failed_never_load(self):
        # Bypassing successful selected enabled config would incorrectly reach load.
        for mode in ('disabled', 'unselected', 'unknown', 'failed'):
            with self.subTest(mode=mode), synthetic_native_fixture() as f:
                config = f.modules['hermes_cli.config']
                if mode == 'disabled': f.effective['plugins']['disabled'] = ['herdr-threads']
                elif mode == 'unselected': f.effective['plugins']['enabled'] = []
                elif mode == 'unknown': f.effective['plugins'].pop('enabled')
                else: config.load_config_readonly = lambda: config.FailedConfigRead()
                with self.assertRaisesRegex(ValueError, 'config|disabled'):
                    f.driver.native_driver(f.path)
                self.assertNotIn('load', f.calls)
                self.assertNotIn('pre_llm_call', f.calls)
                self.assertTrue(f.managers[0].unloaded)

    def test_synthetic_scan_gate_and_load_failure_unload_and_refuse(self):
        # A failed discovery/gate/registration must never credit output domains.
        for mode, reason in (('scan', 'synthetic_scan'), ('gate', 'disabled'), ('load', 'load')):
            with self.subTest(mode=mode), synthetic_native_fixture(mode) as f:
                with self.assertRaisesRegex(ValueError, reason): f.driver.native_driver(f.path)
                self.assertTrue(f.managers[0].unloaded)
                self.assertNotIn('pre_llm_call', f.calls)
                if mode != 'load': self.assertNotIn('load', f.calls)

    def test_synthetic_dispatch_missing_partial_output_and_generation_drift_refuse(self):
        # Skipping the real completed-gate/generation consumer permits incomplete input.
        for mode, reason in (('dispatch', 'synthetic_dispatch'), ('missing', None),
                             ('partial', None), ('drift', 'timeout_or_drift|generation')):
            with self.subTest(mode=mode), synthetic_native_fixture(mode) as f:
                with self.assertRaises((ValueError, OSError)) as caught: f.driver.native_driver(f.path)
                if reason: self.assertRegex(str(caught.exception), reason)
                self.assertTrue(f.managers[0].unloaded)
                self.assertIn('load', f.calls)

    def test_synthetic_finally_unload_failure_and_surviving_reader_refuse(self):
        # Returning from the success path despite failed cleanup is a bug.
        with synthetic_native_fixture('unload') as f:
            with self.assertRaisesRegex(ValueError, 'synthetic_unload'): f.driver.native_driver(f.path)
            self.assertEqual(len(f.driver.completed_gates(f.gates, 0)), 2)
            self.assertTrue(f.managers[0].unloaded)
        with synthetic_native_fixture() as f:
            f.reader.thread.is_alive = lambda: True
            with self.assertRaisesRegex(ValueError, 'owned_worker_survived'): f.driver.native_driver(f.path)
            self.assertTrue(f.managers[0].unloaded)

    def test_synthetic_reader_wait_uses_existing_deadline_and_finally(self):
        # Waiting without the decreasing deadline hangs a reader lacking observation.
        with synthetic_native_fixture() as f:
            f.data['timeout_seconds'] = .3; f.path.write_text(json.dumps(f.data))
            f.reader.snapshot = lambda: None
            started = time.monotonic()
            with self.assertRaisesRegex(ValueError, 'timeout'): f.driver.native_driver(f.path)
            self.assertLess(time.monotonic() - started, .8)
            self.assertTrue(f.managers[0].unloaded)
            self.assertNotIn('pre_llm_call', f.calls)

    def test_synthetic_entry_origin_spec_and_root_slot_agree(self):
        # Restoring extra sys.path slots or accepting mixed origins breaks binding.
        with synthetic_native_fixture() as f:
            modules, observed = f.driver.native_context(f.data)
            self.assertEqual(observed, IDENTITY)
            self.assertEqual(sys.path, [f.data['source_root'], str(f.root / 'site-packages'), 'retained-tail'])
            self.assertEqual(os.environ['HERMES_HOME'], f.data['home'])
            self.assertEqual(set(modules), {'hermes_cli.profiles', 'hermes_constants',
                'hermes_cli.version_info', 'hermes_cli.config', 'hermes_cli.plugins'})
            self.assertEqual(f.calls, [])

    def test_synthetic_entry_origin_spec_root_profile_home_interpreter_mismatch_refuse(self):
        # Removing any original-entry binding would accept its otherwise valid fixture.
        modes = ('spec', 'root', 'root-slot', 'profile', 'home', 'constants-home',
                 'interpreter', 'trace', 'orig-argv', 'initializing', 'mixed-loaded', 'mixed-import')
        for mode in modes:
            with self.subTest(mode=mode), synthetic_native_fixture() as f:
                boot = f.modules['hermes_bootstrap']; profiles = f.modules['hermes_cli.profiles']
                if mode == 'spec': boot.__spec__.origin = f.modules['hermes_constants'].__file__
                elif mode == 'root': boot._root = f.root
                elif mode == 'root-slot': sys.path[0] = str(f.root)
                elif mode == 'profile': profiles.normalize_profile_name = lambda name: 'wrong-profile'
                elif mode == 'home': profiles.resolve_profile_env = lambda name: str(f.root)
                elif mode == 'constants-home': f.modules['hermes_constants'].get_hermes_home = lambda: f.root
                elif mode == 'interpreter': f.data['argv'][0] = f.data['binary']
                elif mode == 'trace': sys.modules['__main__'].__spec__.name = 'not-trace'
                elif mode == 'orig-argv': sys.orig_argv = list(f.data['argv']) + ['extra']
                elif mode == 'initializing': boot.__spec__._initializing = True
                else:
                    module = f.modules['hermes_cli.plugins']
                    module.__file__ = f.modules['hermes_constants'].__file__
                    module.__spec__.origin = module.__file__
                    if mode == 'mixed-import': sys.modules.pop('hermes_cli.plugins')
                with self.assertRaises(ValueError): f.driver.native_context(f.data)
                self.assertEqual(f.calls, [])


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
