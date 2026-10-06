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


class NativeHermesProbeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='ht-hermes-driver-', dir='/private/tmp')
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
                                  env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',TMPDIR='/private/tmp'))
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

    def test_real_native_command_contract_and_cli_start_consumer(self):
        """Source-only argv/decoder test; does not run native commands."""
        driver=self.driver_module()
        self.assertTrue(callable(getattr(driver,'native_commands',None)),
                        'real captured native companion command route is required')
        self.input.update(producer='prepared_native',native_scope='native_callbacks',
                          runtime_command_file=str(self.root/'captured-runtime.json'))
        planned=driver.native_commands(self.input)
        callback=planned['native_domains']
        self.assertIn(str(DRIVER.parents[1]/'scripts/canary/adapters/hermes.py'),callback)
        self.assertEqual(callback[callback.index('--stage')+1],'no_model')
        self.assertEqual(callback[callback.index('--runtime-command-file')+1],self.input['runtime_command_file'])
        measured=driver.native_row('guarded_launch',0,json.dumps({'outcome':'started','harness':'hermes'}).encode(),self.input)
        self.assertEqual(measured['verdict'],'PASS')
        self.assertEqual(measured['reason'],'managed_launch_only')
        measured=driver.native_row('guarded_launch',0,b'{"outcome":"unknown","harness":"hermes"}',self.input)
        self.assertEqual(measured['verdict'],'INCONCLUSIVE')
        for stage in ('callback_context','child','model_ack'):
            measured=driver.native_row(stage,0,b'{"observed":true,"samples":1}',self.input)
            self.assertEqual(measured['verdict'],'INCONCLUSIVE','caller JSON cannot become native/model proof')

    def test_native_persisted_context_and_model_actions_require_actual_correlated_rows(self):
        driver=self.driver_module()
        self.assertTrue(callable(getattr(driver,'collect_model_evidence',None)),
                        'source-shaped native persistence/tool/receipt consumer is required')
        home=Path(self.input['home']); home.mkdir()
        state=Path(self.input['state_root']); state.mkdir()
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
