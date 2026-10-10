"""Driver orchestration with labeled executables; never import installed Hermes."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest

DRIVER = Path(__file__).with_name('native-hermes-probe.py')
STAGES = ('source_capture', 'recognition', 'plugin_discovery', 'enablement',
          'guarded_launch', 'callback_context', 'child', 'model_accept',
          'model_inbox', 'model_read', 'model_ack', 'reset')
# The platform scratch root the driver requires the isolation root under: /private/tmp on macOS, /tmp on Linux.
SCRATCH_ROOT = '/private/tmp' if sys.platform == 'darwin' else '/tmp'


def non_scratch_dir():
    """A writable existing directory outside SCRATCH_ROOT (the platform temp dir when it is not scratch, else
    /dev/shm, /var/tmp or the home directory), or None."""
    for candidate in (tempfile.gettempdir(), '/dev/shm', '/var/tmp', str(Path.home())):
        path = Path(candidate).resolve()
        if (path.is_dir() and os.access(path, os.W_OK | os.X_OK)
                and str(path) != SCRATCH_ROOT and not str(path).startswith(SCRATCH_ROOT + '/')):
            return str(path)
    return None


class NativeHermesProbeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='ht-hermes-driver-', dir=SCRATCH_ROOT)
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fake = self.root / 'explicit synthetic executable.py'
        self.fake.write_text('''import json,os,sys,time
from pathlib import Path
mode=sys.argv[1]
if mode=='daemon':
 Path(sys.argv[2]).write_text(str(os.getpid()))
 while True: time.sleep(.05)
elif mode=='hang':
 Path(sys.argv[2]).write_text(str(os.getpid()))
 while True: time.sleep(.05)
elif mode=='overflow':
 print('secret-native-user-tool-body'*5000)
elif mode=='error':
 print('private-secret-credential',file=sys.stderr);sys.exit(7)
elif mode=='stage':
 print(sys.argv[2])
elif mode=='envcheck':
 root=Path(os.environ['HOME']).parent
 safe=('HT_PROBE_PRIVATE_SECRET' not in os.environ and
       Path(os.environ.get('CODEX_HOME','/')).is_relative_to(root) and
       Path(os.environ.get('CLAUDE_CONFIG_DIR','/')).is_relative_to(root))
 print(json.dumps(dict(schema_version=1,stage='source_capture',provenance='synthetic_fixture',samples=1,observed=safe,supported=True)))
elif mode=='cleanup':
 Path(sys.argv[2]).write_text('closed owned fake namespace')
 sys.exit(int(sys.argv[3]))
''')
        self.pid = self.root / 'owned.pid'
        self.closed = self.root / 'closed'
        self.command = [str(Path(sys.executable).resolve()), '-I', '-B', str(self.fake)]
        digest = hashlib.sha256(self.fake.read_bytes()).hexdigest()
        self.input = dict(schema_version=1, producer='synthetic_fixture',
            isolation_root=str(self.root), profile='default', home=str(self.root / 'home'),
            physical_home=str(self.root / 'home'), state_root=str(self.root / 'state'),
            host_endpoint=str(self.root / 'h.sock'), installation_token='00000000-0000-4000-8000-000000000001',
            interpreter=self.command[0], launcher=str(self.fake), source_root=str(self.root),
            runtime_identity={'key': 'build:' + 'a'*64, 'source': 'synthetic_fixture'},
            threads={'path':str(self.fake),'sha256':digest,'source_commit':'1'*40},
            host={'path':str(self.fake),'sha256':digest,'source_commit':'2'*40,
                  'modified':True,'required_capability':'agent_start_process_hint_v1'},
            api_mode='ordinary_string', timeout_seconds=2,
            owned_services=[{'name':'private_host','argv':self.command+['daemon',str(self.pid)]}],
            cleanup_argv=self.command+['cleanup',str(self.closed),'0'],
            stages={})
        for stage in STAGES:
            observation = dict(schema_version=1, stage=stage, provenance='synthetic_fixture',
                               samples=1, observed=True, supported=True)
            self.input['stages'][stage] = self.command+['stage',json.dumps(observation)]

    def run_driver(self, data=None, mode='dry-run'):
        path = self.root / ('input-' + str(time.monotonic_ns()) + '.json')
        path.write_text(json.dumps(self.input if data is None else data))
        path.chmod(0o600)
        result = self.root / ('result-' + str(time.monotonic_ns()) + '.json')
        process = subprocess.Popen([sys.executable,'-I','-B',str(DRIVER),'--'+mode,
                                   '--input',str(path),'--result',str(result)],
                                  stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True,
                                  env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',TMPDIR=SCRATCH_ROOT))
        try:
            out, err = process.communicate(timeout=12)
        finally:
            try: os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError: pass
            process.wait()
        self.assertEqual(process.returncode,0,(out+err).decode(errors='replace'))
        self.assertTrue(result.is_file(),'driver must publish its measured typed result')
        return json.loads(result.read_text()),out+err+result.read_bytes()

    def assert_reaped(self,path):
        self.assertTrue(path.is_file(),'owned stand-in must actually have executed')
        with self.assertRaises(ProcessLookupError): os.kill(int(path.read_text()),0)

    def test_native_hermes_driver_dry_run_typed_matrix_privacy_and_cleanup(self):
        """Kills fake-native promotion, missing classifications, raw logging and leaked lifecycle."""
        self.input['stages']['recognition'][-1] = json.dumps(dict(schema_version=1,
            stage='recognition',provenance='synthetic_fixture',samples=1,observed=False,supported=True))
        self.input['stages']['enablement'][-1] = json.dumps(dict(schema_version=1,
            stage='enablement',provenance='synthetic_fixture',samples=0,observed=True,supported=True))
        self.input['stages']['child'][-1] = json.dumps(dict(schema_version=1,
            stage='child',provenance='synthetic_fixture',samples=1,observed=False,supported=False))
        self.input['stages']['callback_context'] = self.command+['overflow']
        del self.input['stages']['model_accept']
        result,raw = self.run_driver()
        matrix = {r['stage']:r for r in result['matrix']}
        self.assertEqual(matrix['source_capture']['verdict'],'PASS')
        self.assertEqual(matrix['recognition']['verdict'],'FAIL')
        self.assertEqual(matrix['enablement']['verdict'],'INCONCLUSIVE')
        self.assertEqual(matrix['child']['verdict'],'SKIPPED')
        self.assertEqual(matrix['callback_context']['verdict'],'INCONCLUSIVE')
        self.assertEqual(matrix['model_accept']['verdict'],'INCONCLUSIVE')
        self.assertEqual(result['native_acceptance'],'UNMET')
        self.assertEqual(result['evidence_stage'],'synthetic_dry_run')
        self.assertEqual(result['cleanup']['verdict'],'PASS')
        self.assert_reaped(self.pid)
        self.assertTrue(self.closed.is_file())
        for secret in (b'secret-native-user-tool-body',b'private-secret-credential',str(self.root).encode()):
            self.assertNotIn(secret,raw)

    def test_dry_child_environment_is_isolated_and_excludes_provider_credentials(self):
        old=os.environ.get('HT_PROBE_PRIVATE_SECRET')
        os.environ['HT_PROBE_PRIVATE_SECRET']='private-credential-do-not-inherit'
        try:
            self.input['stages']['source_capture']=self.command+['envcheck']
            result,raw=self.run_driver()
            self.assertEqual(result['matrix'][0]['verdict'],'PASS')
            self.assertNotIn(b'private-credential-do-not-inherit',raw)
        finally:
            if old is None: os.environ.pop('HT_PROBE_PRIVATE_SECRET',None)
            else: os.environ['HT_PROBE_PRIVATE_SECRET']=old

    def test_timeout_nonzero_and_unknown_fields_are_not_success(self):
        hung = self.root / 'hung.pid'
        self.input['timeout_seconds']=.35
        self.input['stages']['source_capture']=self.command+['hang',str(hung)]
        self.input['stages']['recognition']=self.command+['error']
        result,raw=self.run_driver()
        self.assertEqual(result['matrix'][0]['verdict'],'INCONCLUSIVE')
        self.assertNotIn(b'private-secret-credential',raw)
        self.assert_reaped(hung)
        self.assert_reaped(self.pid)

    def test_unknown_observation_fields_and_child_zero_samples(self):
        value=dict(schema_version=1,stage='source_capture',provenance='synthetic_fixture',
                   samples=1,observed=True,supported=True,user_message='do not expose')
        self.input['stages']['source_capture'][-1]=json.dumps(value)
        self.input['stages']['child'][-1]=json.dumps(dict(schema_version=1,stage='child',
            provenance='synthetic_fixture',samples=0,observed=True,supported=True))
        result,raw=self.run_driver()
        matrix={r['stage']:r for r in result['matrix']}
        self.assertEqual(matrix['source_capture']['verdict'],'INCONCLUSIVE')
        self.assertEqual(matrix['child']['verdict'],'INCONCLUSIVE')
        self.assertNotIn(b'do not expose',raw)

    def test_cleanup_failure_and_foreign_lifecycle_preservation(self):
        foreign=subprocess.Popen(self.command+['daemon',str(self.root/'foreign.pid')],start_new_session=True)
        try:
            self.input['cleanup_argv'][-1]='9'
            result,_=self.run_driver()
            self.assertEqual(result['cleanup']['verdict'],'FAIL')
            self.assertIsNone(foreign.poll(),'foreign lifecycle must remain alive')
            self.assert_reaped(self.pid)
        finally:
            os.killpg(foreign.pid,signal.SIGKILL)
            foreign.wait()

    def test_preview_does_not_execute_and_synthetic_input_cannot_run_native(self):
        result,_=self.run_driver(mode='preview')
        self.assertFalse(self.pid.exists())
        self.assertEqual(result['evidence_stage'],'action_preview')
        self.assertEqual(result['native_acceptance'],'UNMET')
        result,_=self.run_driver(mode='native')
        self.assertFalse(self.pid.exists())
        self.assertEqual(result['preflight']['verdict'],'INCONCLUSIVE')

    def driver_module(self):
        spec=importlib.util.spec_from_file_location('native_hermes_probe',DRIVER)
        module=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_isolation_root_outside_the_scratch_root_is_refused(self):
        driver=self.driver_module()
        self.assertEqual(driver.SCRATCH_ROOT,SCRATCH_ROOT)
        driver.validate(json.loads(json.dumps(self.input)))  # the scratch fixture itself is accepted
        outside=non_scratch_dir()
        if outside is None:
            self.skipTest(f'no writable directory outside {SCRATCH_ROOT} here')
        elsewhere=tempfile.TemporaryDirectory(prefix='ht-hermes-outside-',dir=outside)
        self.addCleanup(elsewhere.cleanup)
        for root in (elsewhere.name,str(self.root)+'/../..'):
            bad=json.loads(json.dumps(self.input));bad['isolation_root']=root
            with self.subTest(root=root),self.assertRaisesRegex(ValueError,'isolated_private_tmp_required'):
                driver.validate(bad)

    def test_real_native_command_contract_and_cli_start_consumer(self):
        """Source-only argv/decoder test; does not run native commands."""
        driver=self.driver_module()
        self.assertTrue(callable(getattr(driver,'native_commands',None)),
                        'real captured native companion command route is required')
        self.input.update(producer='prepared_native',native_scope='native_callbacks',
                          runtime_command_file=str(self.root/'captured-runtime.json'),
                          launch=dict(pane='w1:p1',terminal='term-fixture',agent_name='hermes-fixture'),stages={})
        planned=driver.native_commands(self.input)
        callback=planned['native_domains']
        self.assertIn(str(DRIVER.parents[1]/'scripts/canary/adapters/hermes.py'),callback)
        self.assertEqual(callback[callback.index('--stage')+1],'no_model')
        self.assertEqual(callback[callback.index('--runtime-command-file')+1],self.input['runtime_command_file'])
        measured=driver.native_row('guarded_launch',0,json.dumps(self.launch_report(self.input)).encode(),self.input)
        self.assertEqual(measured['verdict'],'PASS')
        self.assertEqual(measured['reason'],'managed_launch_only')
        measured=driver.native_row('guarded_launch',0,b'{"outcome":"unknown","harness":"hermes"}',self.input)
        self.assertEqual(measured['verdict'],'INCONCLUSIVE')
        for stage in ('callback_context','child','model_ack'):
            measured=driver.native_row(stage,0,b'{"observed":true,"samples":1}',self.input)
            self.assertEqual(measured['verdict'],'INCONCLUSIVE','caller JSON cannot become native/model proof')

    def native_launch_input(self):
        data=dict(self.input,producer='prepared_native',native_scope='recognition_only',
                  stages={},launch=dict(pane='w1:p1',terminal='term-fixture',agent_name='hermes-fixture'))
        return data

    def launch_report(self,data):
        return dict(outcome='started',harness='hermes',pane=data['launch']['pane'],
            seat='s.fixture',agent_name=data['launch']['agent_name'],
            argv=['--profile',data['profile'],'--cli','chat'],
            config_dir={'path':data['home']},
            harness_version={'admission':'prelaunch_observed','binary':data['launcher']},
            hermes=dict(profile=data['profile'],home=data['home'],identity=data['runtime_identity'],
                identity_provenance='startup_captured_prelaunch_observation',
                environment_scope='declared_child_input_plus_native_bootstrap_profile_effects',
                api='presence_only',callback_qualified=False,native_acceptance='unmet'))

    def test_unbound_native_launch_echo_cannot_be_planned(self):
        data=self.native_launch_input()
        data['stages']['guarded_launch']=self.command+['stage','{"outcome":"started","harness":"hermes"}']
        with self.assertRaises(ValueError):self.driver_module().native_commands(data)

    def test_native_launch_requires_bound_executable_verb_scope_target_profile(self):
        driver=self.driver_module();data=self.native_launch_input()
        expected=[data['threads']['path'],'--state-dir',data['state_root'],
            '--host-endpoint',data['host_endpoint'],'--json','launch','--pane','w1:p1',
            '--kind','hermes','--name','hermes-fixture','--','--profile','default','--cli']
        self.assertEqual(driver.native_commands(data)['guarded_launch'],expected)
        for index,value in ((0,'/wrong'),(2,'/other'),(4,'/other.sock'),(6,'inbox'),
                            (8,'w1:p2'),(10,'codex'),(15,'work')):
            bad=dict(data,stages={'guarded_launch':expected.copy()});bad['stages']['guarded_launch'][index]=value
            with self.subTest(index=index),self.assertRaises(ValueError):driver.native_commands(bad)
        bad=dict(data,stages={'guarded_launch':expected+['chat']})
        with self.assertRaises(ValueError):driver.native_commands(bad)
        good=self.launch_report(data)
        self.assertEqual(driver.native_row('guarded_launch',0,json.dumps(good).encode(),data)['verdict'],'PASS')
        for field,value in [('pane','w1:p2'),('agent_name','foreign'),('argv',[]),
                            ('config_dir',{'path':'/other'}),('harness_version',{'binary':'/other'}),
                            ('hermes',dict(good['hermes'],profile='work'))]:
            with self.subTest(field=field):
                changed=dict(good,**{field:value})
                self.assertNotEqual(driver.native_row('guarded_launch',0,json.dumps(changed).encode(),data)['verdict'],'PASS')
        self.assertNotEqual(driver.native_row('guarded_launch',0,b'{"outcome":"started","harness":"hermes"}',data)['verdict'],'PASS')

    def test_native_recognition_uses_actual_private_agent_payload(self):
        driver=self.driver_module();data=self.native_launch_input()
        self.assertEqual(driver.native_commands(data)['recognition'],[data['host']['path'],'agent','get','w1:p1'])
        actual={'result':{'agent':dict(pane_id='w1:p1',terminal_id='term-fixture',name='hermes-fixture',
            agent='hermes',agent_status='idle',launch_pending=False,revision=1)}}
        self.assertEqual(driver.native_row('recognition',0,json.dumps(actual).encode(),data)['verdict'],'PASS')
        for key,value in [('pane_id','w1:p2'),('terminal_id','foreign'),('name','foreign'),
                          ('agent','unknown'),('launch_pending',True)]:
            changed=json.loads(json.dumps(actual));changed['result']['agent'][key]=value
            self.assertNotEqual(driver.native_row('recognition',0,json.dumps(changed).encode(),data)['verdict'],'PASS')

    def test_real_instance_layout_is_required_without_root_database_fallback(self):
        driver=self.driver_module()
        expected=Path(self.input['state_root'])/'instances'/hashlib.sha256(os.fsencode(self.input['host_endpoint'])).hexdigest()
        self.assertEqual(driver.instance_directory(self.input),expected)
        # Correct DB layout is a prerequisite, not permission to create or fall back.
        Path(self.input['state_root']).mkdir()
        sqlite3.connect(Path(self.input['state_root'])/'threads.sqlite3').close()
        with self.assertRaises((ValueError,OSError)):driver.captured_instance(self.input)

    def test_separate_measurement_decoder_rejects_unbound_zero_and_body_outputs(self):
        driver=self.driver_module();expected={'invocation_id':'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee'}
        value=dict(schema_version=1,scope='selective_native_api_invocation_not_model_delivery',
            invocation_sha256=hashlib.sha256(expected['invocation_id'].encode()).hexdigest(),
            rows=[dict(stage=s,verdict='INCONCLUSIVE',reason='unobserved',samples=0,exit_code=None)
                  for s in ('callback_context','child','reset')])
        self.assertEqual(driver.decode_measurement(value,expected),value)
        for mutation in ('nonce','scope','body','zero-pass'):
            bad=json.loads(json.dumps(value))
            if mutation=='nonce':bad['invocation_sha256']='a'*64
            if mutation=='scope':bad['scope']='actual_model_delivery'
            if mutation=='body':bad['rows'][0]['raw_context']='private user secret'
            if mutation=='zero-pass':bad['rows'][2]['verdict']='PASS'
            with self.subTest(mutation=mutation),self.assertRaises(ValueError):driver.decode_measurement(bad,expected)

    def test_captured_selective_measurement_actual_command_and_result_decoder(self):
        """Real orchestration with explicitly labeled source-shaped captured stand-in output."""
        driver=self.driver_module();companion=driver.companion_module()
        # Use fixed repository source fixtures, never installed modules or native process.
        spec=importlib.util.spec_from_file_location('source_fixture_companion_tests',DRIVER.parent/'canary/test_hermes_companion.py')
        fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)
        desc=fixture.adapter();identity=fixture.IDENTITY
        data=self.native_launch_input();data.update(native_scope='native_callbacks',runtime_identity=identity,
            runtime_command_file=str(self.root/'runtime.json'))
        measure=dict(schema_version=1,invocation_id='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee',target='w1:p1',
            seat='s.fixture',context_marker='cooperative',child_form='explicit_parent_callback')
        data['measurement']=measure
        Path(data['home']).mkdir(mode=0o700)
        captured={k:data[k] for k in ('profile','home','physical_home','source_root','isolation_root','state_root','host_endpoint','installation_token')}
        captured.update(schema_version=1,producer='official_native_selective',identity=identity,
            binary=data['threads']['path'],timeout_seconds=2,measurement=measure,
            argv=[data['interpreter'],'-I','-c','opaque synthetic source fixture; never executed',
                  '--count','--no-report',str(companion.HERE),'--native-driver',data['runtime_command_file']])
        Path(data['runtime_command_file']).write_text(json.dumps(captured))
        diagnostic=dict(schema_version=1,scope='selective_native_api_invocation_not_model_delivery',
            invocation_sha256=hashlib.sha256(measure['invocation_id'].encode()).hexdigest(),
            rows=[dict(stage=s,verdict='INCONCLUSIVE',reason='unobserved',samples=0,exit_code=None)
                  for s in ('callback_context','child','reset')])
        observation=dict(schema_version=1,provenance='official_native_selective',identity=identity,
            native_loaded=True,domains=[dict(domain=c['domain'],origin=c['origin'],contract_id=c['id'],
                successful_milestones=c['required_milestones'],violations=[],outcome='compatible') for c in desc['contracts']],
            measurement=diagnostic)
        class SourceShapedOwnedStandIn:
            def __init__(self):self.calls=[]
            def capture(self,command,deadline):
                self.calls.append(command)
                value={'schema_version':1,'adapters':[desc]} if command[-2:]==['adapters','--json'] else observation
                return 0,json.dumps(value).encode(),None
        owned=SourceShapedOwnedStandIn()
        result=driver.capture_domains(data,owned,time.monotonic()+2)
        self.assertEqual(owned.calls[-1],captured['argv'])
        self.assertEqual(result['measurement'],diagnostic)
        self.assertEqual(len(result['domains']),2)
        # An otherwise valid produced diagnostic with another invocation cannot be reused.
        observation['measurement']['invocation_sha256']='f'*64
        (self.root/'request.json').unlink()
        with self.assertRaises(ValueError):driver.capture_domains(data,owned,time.monotonic()+2)

    def test_native_persisted_context_and_model_actions_require_actual_correlated_rows(self):
        driver=self.driver_module()
        self.assertTrue(callable(getattr(driver,'collect_model_evidence',None)),
                        'source-shaped native persistence/tool/receipt consumer is required')
        home=Path(self.input['home']); home.mkdir()
        state=driver.instance_directory(self.input); state.mkdir(parents=True,mode=0o700)
        state.parent.chmod(0o700)
        (state/'namespace').write_text('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');(state/'namespace').chmod(0o600)
        native=sqlite3.connect(home/'state.db')
        native.execute('CREATE TABLE messages (id INTEGER,session_id TEXT,role TEXT,content TEXT,tool_call_id TEXT,tool_calls TEXT,tool_name TEXT,effect_disposition TEXT,timestamp REAL,active INTEGER,api_content TEXT)')
        context='native-plugin-unique-marker thread-private message-private'
        now=time.time()
        native.execute('INSERT INTO messages VALUES (1,?,?,?,?,?,?,?,?,?,?)',
            ('native-session','user','Please handle my pending work.',None,None,None,None,now,1,context))
        calls=[]
        for index,(verb,target) in enumerate((('accept','thread-private'),('inbox',''),('read','thread-private'),('ack','message-private'))):
            call='call-'+str(index)
            command=shlex.quote(self.input['threads']['path'])+' '+verb+(' '+target if target else '')
            calls.append(dict(id=call,type='function',function=dict(name='terminal',arguments=json.dumps(dict(command=command)))))
            native.execute('INSERT INTO messages VALUES (?,?,?,?,?,?,?,?,?,?,?)',
                (3+index,'native-session','tool',json.dumps(dict(output='private transcript body',exit_code=0,error=None)),call,None,'terminal',None,now+.2,1,None))
        native.execute('INSERT INTO messages VALUES (2,?,?,?,?,?,?,?,?,?,?)',
            ('native-session','assistant',None,None,json.dumps(calls),None,None,now+.1,1,None))
        native.commit()
        canonical=sqlite3.connect(state/'threads.sqlite3')
        canonical.execute('CREATE TABLE occupant_bindings (seat_id TEXT,generation INTEGER,target_id TEXT,harness TEXT,native_session TEXT,execution_id TEXT,observation_provenance TEXT,registered_at INTEGER,ended_at INTEGER)')
        canonical.execute('INSERT INTO occupant_bindings VALUES (?,?,?,?,?,?,?,?,?)',('seat-private',4,'target-private','hermes','native-session','execution-private','cooperative_top_level',int(now*1000),None))
        canonical.execute('CREATE TABLE invitations (thread_id TEXT,seat_id TEXT,state TEXT,accepted_actor_seat_id TEXT,accepted_generation INTEGER,accepted_observation TEXT,accepted_at INTEGER)')
        canonical.execute('CREATE TABLE receipts (message_id TEXT,thread_id TEXT,seat_id TEXT,state TEXT,ack_actor_seat_id TEXT,ack_generation INTEGER,ack_observation TEXT,acked_at INTEGER)')
        canonical.execute('CREATE TABLE send_manifests (message_id TEXT,thread_id TEXT,preparation_id TEXT)')
        canonical.execute('CREATE TABLE prepared_recipients (preparation_id TEXT,seat_id TEXT)')
        canonical.execute('CREATE TABLE receipt_state (message_id TEXT,seat_id TEXT,state TEXT,ack_actor_seat_id TEXT,ack_generation INTEGER,ack_observation TEXT,acked_at INTEGER)')
        observation=json.dumps(dict(harness='hermes',session='native-session',execution='execution-private',binding_generation=4,provenance='cooperative_top_level'))
        canonical.execute('INSERT INTO invitations VALUES (?,?,?,?,?,?,?)',('thread-private','seat-private','accepted','seat-private',4,observation,int((now+.2)*1000)))
        canonical.execute('INSERT INTO receipts VALUES (?,?,?,?,?,?,?,?)',('message-private','thread-private','seat-private','acked','seat-private',4,observation,int((now+.2)*1000)))
        canonical.commit()
        self.input['model_evidence']=dict(session_id='native-session',after_id=0,user_row_id=1,
            started_at=now-.1,finished_at=now+1,context_marker='native-plugin-unique-marker',
            seat_id='seat-private',generation=4,execution_id='execution-private',target_id='target-private',
            thread_id='thread-private',message_id='message-private')
        measured=driver.collect_model_evidence(self.input,time.monotonic()+2)
        self.assertEqual(measured['persistence']['verdict'],'PASS')
        self.assertEqual(measured['delivery'],'UNMEASURED')
        self.assertEqual({r['verdict'] for r in measured['actions']},{'PASS'})
        raw=json.dumps(measured)
        for private in ('native-plugin-unique-marker','private transcript body','thread-private','native-session',str(home)):
            self.assertNotIn(private,raw)
        # New canonical sends persist acknowledgments in receipt_state, with
        # thread/recipient identity supplied by send_manifests/prepared_recipients.
        canonical.execute('INSERT INTO send_manifests VALUES (?,?,?)',('message-private','thread-private','prep-private'))
        canonical.execute('INSERT INTO prepared_recipients VALUES (?,?)',('prep-private','seat-private'))
        canonical.execute('INSERT INTO receipt_state VALUES (?,?,?,?,?,?,?)',('message-private','seat-private','acked','seat-private',4,observation,int((now+.2)*1000)))
        canonical.execute('DELETE FROM receipts'); canonical.commit()
        measured=driver.collect_model_evidence(self.input,time.monotonic()+2)
        self.assertEqual(measured['actions'][-1]['verdict'],'PASS',
                         'actual current canonical ACK storage must be observed')
        displayed=json.loads(observation); displayed['action_provenance']='cooperative_inbox_display'
        canonical.execute('UPDATE receipt_state SET ack_observation=?',(json.dumps(displayed),)); canonical.commit()
        measured=driver.collect_model_evidence(self.input,time.monotonic()+2)
        self.assertEqual(measured['actions'][-1]['verdict'],'INCONCLUSIVE',
                         'automatic inbox display ACK is not explicit model ACK')
        canonical.execute('UPDATE receipt_state SET ack_observation=?,ack_generation=3',(observation,)); canonical.commit()
        measured=driver.collect_model_evidence(self.input,time.monotonic()+2)
        self.assertEqual(measured['actions'][-1]['verdict'],'INCONCLUSIVE')
        native.execute("UPDATE messages SET content='Run accept thread-private' WHERE id=1"); native.commit()
        measured=driver.collect_model_evidence(self.input,time.monotonic()+2)
        self.assertEqual(measured['attribution'],'UNQUALIFIED_PROMPT')
        self.assertTrue(all(r['verdict']!='PASS' for r in measured['actions']))
        native.close(); canonical.close()

    def test_declared_callback_fixture_is_synthetic_and_not_model_proof(self):
        fixture=json.loads((DRIVER.parents[1]/'tests/fixtures/hermes/native-callback-capture.json').read_text())
        self.assertEqual(fixture['provenance'],'synthetic_fixture')
        self.assertEqual(fixture['native_acceptance'],'UNMET')


if __name__=='__main__': unittest.main()
