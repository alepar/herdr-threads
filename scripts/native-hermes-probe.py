#!/usr/bin/env python3
"""Bounded acceptance measurement contract; a dry result is never native proof.

Native commands are explicit reviewed argv, not shell templates. The selective
canary is the built-in native callback producer. Recognition/launch and model
evidence retain their own scopes; callback domain completion cannot supply them.
No enable, permission bypass, credential provisioning or receipt is performed.
"""
import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import selectors
import signal
import shlex
import sqlite3
import stat
import subprocess
import sys
import time
import uuid

STAGES = ('source_capture', 'recognition', 'plugin_discovery', 'enablement',
          'guarded_launch', 'callback_context', 'child', 'model_accept',
          'model_inbox', 'model_read', 'model_ack', 'reset')
ROOT = Path(__file__).resolve().parents[1]
OUTPUT_CAP = 65536
ERROR_CAP = 8192
# The platform scratch root an isolation root must live under: /private/tmp on macOS, /tmp on Linux.
SCRATCH_ROOT = '/private/tmp' if sys.platform == 'darwin' else '/tmp'


def token(value, cap=4096):
    return (isinstance(value, str) and 0 < len(value.encode()) <= cap
            and not any(ord(c) < 32 or ord(c) == 127 for c in value))


def digest(path):
    hasher = hashlib.sha256()
    with open(path, 'rb') as stream:
        while chunk := stream.read(1048576):
            hasher.update(chunk)
    return hasher.hexdigest()


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate_field')
        result[key] = value
    return result


def document(raw):
    if len(raw) > OUTPUT_CAP:
        raise ValueError('output_bound')
    return json.loads(raw, object_pairs_hook=unique_pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite')))


def read_private(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError('private_input_required')
        return document(os.read(fd, OUTPUT_CAP + 1))
    finally:
        os.close(fd)


def argv(value):
    if (not isinstance(value, list) or not 1 <= len(value) <= 64
            or not all(token(v) for v in value) or not os.path.isabs(value[0])
            or sum(len(v.encode()) for v in value) > 32768):
        raise ValueError('argv')
    return value


def validate(data):
    fields = {'schema_version', 'producer', 'isolation_root', 'profile', 'home',
              'physical_home', 'state_root', 'host_endpoint', 'installation_token',
              'interpreter', 'launcher', 'source_root', 'runtime_identity', 'threads',
              'host', 'api_mode', 'timeout_seconds', 'owned_services', 'cleanup_argv', 'stages'}
    optional = {'runtime_command_file', 'native_scope', 'model_evidence', 'launch', 'measurement'}
    if (not isinstance(data, dict) or not fields <= set(data) or set(data) - fields - optional
            or data['schema_version'] != 1 or data['producer'] not in ('synthetic_fixture', 'prepared_native')
            or not token(data['profile'], 256) or not token(data['api_mode'], 64)):
        raise ValueError('input_schema')
    uuid.UUID(data['installation_token'])
    for key in ('isolation_root', 'home', 'physical_home', 'state_root', 'host_endpoint',
                'interpreter', 'launcher', 'source_root'):
        if not token(data[key]) or not os.path.isabs(data[key]):
            raise ValueError('input_path')
    root = Path(data['isolation_root']).resolve(strict=True)
    if root == Path('/') or root == Path.home() or not str(root).startswith(SCRATCH_ROOT + '/'):
        raise ValueError('isolated_private_tmp_required')
    for key in ('home', 'physical_home', 'state_root', 'host_endpoint'):
        if not Path(data[key]).resolve().is_relative_to(root):
            raise ValueError('scope_escape')
    if Path(data['home']).resolve() != Path(data['physical_home']):
        raise ValueError('physical_scope')
    if (type(data['timeout_seconds']) not in (int, float)
            or not math.isfinite(data['timeout_seconds']) or not 0 < data['timeout_seconds'] <= 120):
        raise ValueError('deadline')
    for label in ('threads', 'host'):
        record = data[label]
        expected = {'path', 'sha256', 'source_commit'} | ({'modified', 'required_capability'} if label == 'host' else set())
        if (not isinstance(record, dict) or set(record) != expected or not token(record['path'])
                or not os.path.isabs(record['path']) or len(record['sha256']) != 64
                or any(c not in '0123456789abcdef' for c in record['sha256'])
                or len(record['source_commit']) != 40
                or any(c not in '0123456789abcdef' for c in record['source_commit'])):
            raise ValueError('binary_identity')
        if digest(record['path']) != record['sha256']:
            raise ValueError('binary_digest_mismatch')
    if data['host']['modified'] is not True or data['host']['required_capability'] != 'agent_start_process_hint_v1':
        raise ValueError('private_host_capability_scope')
    if (not isinstance(data['owned_services'], list) or len(data['owned_services']) > 2
            or not isinstance(data['stages'], dict) or set(data['stages']) - set(STAGES)):
        raise ValueError('command_plan')
    names = set()
    for service in data['owned_services']:
        if set(service) != {'name', 'argv'} or service['name'] not in ('private_host', 'private_daemon') or service['name'] in names:
            raise ValueError('owned_service')
        names.add(service['name'])
        argv(service['argv'])
    argv(data['cleanup_argv'])
    for command in data['stages'].values():
        argv(command)
    if 'measurement' in data:
        companion_module().measurement_input(data['measurement'])
        if data['measurement']['target'] != launch_scope(data)['pane']:
            raise ValueError('measurement_target_scope')
    if not isinstance(data['runtime_identity'], dict):
        raise ValueError('runtime_identity')
    return data


class OwnedProcesses:
    """Signal only sessions created here; no environment/path scans or foreign PIDs."""
    def __init__(self, env, cwd):
        self.env, self.cwd, self.children = env, cwd, []

    def spawn(self, command, pipes=True):
        child = subprocess.Popen(command, cwd=self.cwd, env=self.env, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE if pipes else subprocess.DEVNULL,
                                 stderr=subprocess.PIPE if pipes else subprocess.DEVNULL,
                                 start_new_session=True)
        self.children.append(child)
        return child

    @staticmethod
    def stop(child):
        # Group identity is established by start_new_session, never supplied by input.
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(child.pid, sig)
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=.2)
            except subprocess.TimeoutExpired:
                continue
        child.wait(timeout=.2)
        for stream in (child.stdout, child.stderr):
            if stream is not None:
                stream.close()
        return child.poll() is not None

    def close(self):
        clean = True
        for child in reversed(self.children):
            try:
                clean = self.stop(child) and clean
            except (OSError, subprocess.TimeoutExpired):
                clean = False
        return clean

    def capture(self, command, deadline):
        if time.monotonic() >= deadline:
            return None, b'', 'deadline'
        try:
            child = self.spawn(command)
        except OSError:
            return None, b'', 'dependency_unavailable'
        out = bytearray()
        sizes = [0, 0]
        reason = None
        try:
            with selectors.DefaultSelector() as selector:
                for index, stream in enumerate((child.stdout, child.stderr)):
                    os.set_blocking(stream.fileno(), False)
                    selector.register(stream, selectors.EVENT_READ, index)
                while selector.get_map():
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        reason = 'deadline'
                        break
                    for key, _ in selector.select(min(.05, remaining)):
                        chunk = os.read(key.fileobj.fileno(), 8192)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            continue
                        index = key.data
                        sizes[index] += len(chunk)
                        if sizes[index] > (OUTPUT_CAP if index == 0 else ERROR_CAP):
                            reason = 'output_bound'
                            break
                        if index == 0:
                            out.extend(chunk)
                    if reason:
                        break
                if reason is None:
                    try:
                        child.wait(timeout=max(.001, deadline - time.monotonic()))
                    except subprocess.TimeoutExpired:
                        reason = 'deadline'
        finally:
            self.stop(child)
        return child.returncode, bytes(out) if reason is None else b'', reason


def row(stage, verdict='INCONCLUSIVE', reason='unobserved', samples=0, exit_code=None):
    return dict(stage=stage, verdict=verdict, reason=reason, samples=samples, exit_code=exit_code)


def classify(stage, raw, provenance):
    observation = document(raw)
    if (not isinstance(observation, dict) or set(observation) != {
            'schema_version', 'stage', 'provenance', 'samples', 'observed', 'supported'}
            or observation['schema_version'] != 1 or observation['stage'] != stage
            or observation['provenance'] != provenance or type(observation['samples']) is not int
            or not 0 <= observation['samples'] <= 1024
            or type(observation['observed']) is not bool or type(observation['supported']) is not bool):
        raise ValueError('observation_schema')
    count = observation['samples']
    if not observation['supported']:
        return row(stage, 'SKIPPED', 'unsupported_form', count)
    if count == 0:
        return row(stage, samples=0)
    return row(stage, 'PASS' if observation['observed'] else 'FAIL', 'measured_predicate', count)


def isolated_environment(data, mode):
    # Inherit tool/provider environment only in explicitly authorized native mode.
    # Values and argv are private inputs and never copied into public results.
    if mode == 'dry-run':
        allowed = {'PATH', 'LANG', 'LC_ALL', 'LC_CTYPE', 'CARGO_HOME', 'RUSTUP_HOME',
                   'CARGO_TARGET_DIR', 'HT_LEAK_RUN_ID'}
        env = {k: v for k, v in os.environ.items() if k in allowed}
    else:
        env = {k: v for k, v in os.environ.items() if not k.startswith(('HERDR_', 'HERMES_'))}
    root = Path(data['isolation_root'])
    env.update(HOME=data['home'], HERMES_HOME=data['home'],
               CODEX_HOME=str(root/'codex'), CLAUDE_CONFIG_DIR=str(root/'claude'),
               HERDR_SOCKET_PATH=data['host_endpoint'],
               HERDR_PLUGIN_STATE_DIR=data['state_root'], HERDR_CONFIG_PATH=str(root/'herdr.toml'),
               XDG_CONFIG_HOME=str(root/'config'), XDG_STATE_HOME=str(root/'xdg-state'),
               XDG_RUNTIME_DIR=str(root/'runtime'), TMPDIR=SCRATCH_ROOT, PYTHONDONTWRITEBYTECODE='1')
    return env


def native_preflight(data):
    if data['producer'] != 'prepared_native':
        raise ValueError('synthetic_input_cannot_run_native')
    scope = data.get('native_scope')
    if scope not in ('recognition_only', 'native_callbacks', 'live'):
        raise ValueError('native_scope_required')
    # Recognition-only excludes selective loader/callback invocation and model use.
    allowed = {'source_capture', 'recognition', 'guarded_launch'}
    if scope != 'recognition_only':
        allowed |= {'plugin_discovery', 'enablement', 'callback_context', 'child', 'reset'}
    if scope == 'live':
        allowed |= {'model_accept', 'model_inbox', 'model_read', 'model_ack'}
    if set(data['stages']) - allowed:
        raise ValueError('native_scope_exceeded')
    # Generic stage-projection commands are the SYNTHETIC boundary only. Actual
    # native execution below uses captured CLI/canary output, never fixture booleans.
    return scope


def launch_scope(data):
    value = data.get('launch')
    if (not isinstance(value, dict) or set(value) != {'pane','terminal','agent_name'}
            or not all(token(value[k],256) for k in value)):
        raise ValueError('explicit_launch_target_required')
    return value


def native_commands(data):
    """Construct real scoped producers; caller stage argv cannot substitute echo."""
    scope = launch_scope(data)
    launch = [data['threads']['path'], '--state-dir', data['state_root'],
        '--host-endpoint', data['host_endpoint'], '--json', 'launch',
        '--pane', scope['pane'], '--kind', 'hermes', '--name', scope['agent_name'],
        '--', '--profile', data['profile'], '--cli']
    recognition = [data['host']['path'], 'agent', 'get', scope['pane']]
    commands = dict(data['stages'])
    for stage, command in (('guarded_launch',launch),('recognition',recognition)):
        if stage in commands and commands[stage] != command:
            raise ValueError('unbound_native_command')
        commands[stage] = command
    commands['source_capture'] = [data['threads']['path'], '--version']
    # Recognition is performed after the guarded start, not from an unrelated prior pane.
    if data.get('native_scope') in ('native_callbacks', 'live'):
        path = data.get('runtime_command_file')
        if not token(path) or not os.path.isabs(path):
            raise ValueError('captured_runtime_command_required')
        commands['native_domains'] = [sys.executable, '-I', '-B',
            str(ROOT/'scripts/canary/adapters/hermes.py'), '--harness','hermes',
            '--attempt',data['installation_token'], '--stage','no_model',
            '--work-dir',data['isolation_root'], '--binary',data['threads']['path'],
            '--runtime-command-file',path]
    return commands


def companion_module():
    spec = importlib.util.spec_from_file_location('hermes_probe_companion',
                                                  ROOT/'scripts/canary/adapters/hermes.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def capture_domains(data, owned, deadline):
    """Real companion input/output joins; never run in the synthetic boundary."""
    companion = companion_module()  # imports stdlib/repository modules only
    captured = companion.input_document(data['runtime_command_file'])
    if (captured['producer'] != 'official_native_selective'
            or captured['identity'] != data['runtime_identity']
            or captured['binary'] != data['threads']['path']
            or captured['argv'][0] != data['interpreter']):
        raise ValueError('captured_source_binding')
    for key in ('profile','home','physical_home','source_root','isolation_root',
                'state_root','host_endpoint','installation_token'):
        if captured[key] != data[key]:
            raise ValueError('captured_scope_binding')
    code, raw, error = owned.capture([data['threads']['path'],'adapters','--json'],deadline)
    if error or code != 0:
        raise ValueError('same_binary_discovery_unavailable')
    discovery = document(raw)
    if discovery.get('schema_version') != 1:
        raise ValueError('discovery_schema')
    adapter = next(a for a in discovery['adapters'] if a['id'] == 'hermes')
    path = Path(data['isolation_root'])/'request.json'
    fd = os.open(path, os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'w') as stream:
        json.dump({'adapter':adapter},stream)
    opt_in=data.get('measurement')
    if opt_in is not None and captured.get('measurement') != opt_in:
        raise ValueError('measurement_input_binding')
    if opt_in is None and 'measurement' in captured:
        raise ValueError('unexpected_measurement_mode')
    command=captured['argv'] if opt_in is not None else native_commands(data)['native_domains']
    code, raw, error = owned.capture(command,deadline)
    if error or code != 0:
        raise ValueError('selective_callback_unavailable')
    engine = companion.runner()
    if opt_in is not None:
        observation=document(raw)
        if (set(observation)!={'schema_version','provenance','identity','native_loaded','domains','measurement'}
                or observation['schema_version']!=1 or observation['provenance']!='official_native_selective'
                or observation['native_loaded'] is not True or observation['identity']!=data['runtime_identity']):
            raise ValueError('selective_observation')
        diagnostic=decode_measurement(observation['measurement'],opt_in)
        candidate=dict(schema_version=1,harness='hermes',attempt=data['installation_token'],
            identity=observation['identity'],evidence_stage='no_model',outcome='complete',reason=None,domains=observation['domains'])
        raw=json.dumps(candidate).encode()
    measured = engine.validate_result(raw,adapter,data['installation_token'],'no_model')
    if measured['identity'] != data['runtime_identity'] or len(engine.verified_domains(measured,adapter)) != 2:
        raise ValueError('native_domains_incomplete')
    # The companion verifies source/profile/generation, the real native enabled
    # loader gates, callback shape and timely real client evidence replies.
    return dict(domains=measured['domains'],measurement=diagnostic if opt_in is not None else None)


def decode_measurement(value, expected):
    fields={'schema_version','scope','invocation_sha256','rows'}
    reasons={'unobserved','selective_return_and_canonical_completed_operation',
        'native_form_omits_callback','selective_declared_child_return_and_private_state_conservation',
        'declared_hint_consumed_clear_and_canonical_transition'}
    if (not isinstance(value,dict) or set(value)!=fields or value['schema_version']!=1
            or value['scope']!='selective_native_api_invocation_not_model_delivery'
            or value['invocation_sha256']!=hashlib.sha256(expected['invocation_id'].encode()).hexdigest()
            or not isinstance(value['rows'],list) or len(value['rows'])!=3):
        raise ValueError('measurement_schema')
    pass_reasons={'callback_context':'selective_return_and_canonical_completed_operation',
        'child':'selective_declared_child_return_and_private_state_conservation',
        'reset':'declared_hint_consumed_clear_and_canonical_transition'}
    for stage,item in zip(('callback_context','child','reset'),value['rows']):
        allowed={'stage','verdict','reason','samples','exit_code'}
        if item.get('verdict')=='PASS' and stage=='callback_context':allowed|={'context_sha256','context_bytes'}
        if (set(item)!=allowed or item.get('stage')!=stage or item.get('verdict') not in ('PASS','FAIL','INCONCLUSIVE','SKIPPED')
                or item.get('reason') not in reasons or item.get('exit_code') is not None
                or type(item.get('samples')) is not int or not 0<=item['samples']<=1
                or (item['verdict']=='PASS' and item['samples']!=1)):
            raise ValueError('measurement_row')
        if (item['verdict']=='PASS' and item['reason']!=pass_reasons[stage]) or (item['verdict']!='PASS' and
                (item['samples']!=0 or item['reason'] not in ('unobserved','native_form_omits_callback'))) or (
                item['reason']=='native_form_omits_callback' and (stage!='child' or item['verdict']!='SKIPPED')):
            raise ValueError('measurement_predicate')
        if 'context_bytes' in item:
            h=item['context_sha256']
            if (type(item['context_bytes']) is not int or not 1<=item['context_bytes']<=4096
                    or not isinstance(h,str) or len(h)!=64 or any(c not in '0123456789abcdef' for c in h)):
                raise ValueError('measurement_context')
    return value


def native_row(stage, code, raw, data):
    """Parse actual producer output. No generic observed:true native promotion."""
    if stage == 'source_capture':
        return row(stage, 'PASS', 'explicit_source_binary_digest', 1)
    if stage == 'guarded_launch':
        value = document(raw)
        scope = launch_scope(data)
        h = value.get('hermes', {})
        v = value.get('harness_version', {})
        if (value.get('outcome') == 'started' and value.get('harness') == 'hermes'
                and value.get('pane') == scope['pane']
                and value.get('agent_name') == scope['agent_name'] and token(value.get('seat'),256)
                and value.get('argv') == ['--profile',data['profile'],'--cli','chat']
                and value.get('config_dir',{}).get('path') == data['home']
                and v.get('binary') == data['launcher'] and v.get('admission') == 'prelaunch_observed'
                and h.get('profile') == data['profile'] and h.get('home') == data['home']
                and h.get('identity') == data['runtime_identity']
                and h.get('identity_provenance') == 'startup_captured_prelaunch_observation'
                and h.get('environment_scope') == 'declared_child_input_plus_native_bootstrap_profile_effects'
                and h.get('api') == 'presence_only' and h.get('callback_qualified') is False
                and h.get('native_acceptance') == 'unmet'):
            return row(stage, 'PASS', 'managed_launch_only', 1, code)
        return row(stage, reason='startup_not_correlated', exit_code=code)
    if stage == 'recognition':
        scope = launch_scope(data)
        value = document(raw).get('result',{}).get('agent',{})
        if (value.get('pane_id') == scope['pane'] and value.get('terminal_id') == scope['terminal']
                and value.get('name') == scope['agent_name'] and value.get('agent') == 'hermes'
                and value.get('launch_pending',False) is False
                and value.get('agent_status') in ('idle','working','blocked','done')
                and type(value.get('revision')) is int and value['revision'] > 0):
            return row(stage,'PASS','advisory_private_agent_match_no_epoch_attestation',1,code)
        return row(stage,reason='private_agent_not_correlated',exit_code=code)
    # The companion's strict real selective result supplies two DOMAIN samples,
    # not context/API/model consumption. Retain those separately in the result.
    return row(stage, reason='independent_native_observation_required', exit_code=code)



def instance_directory(data):
    # Same raw endpoint bytes as RuntimeContext / InstancePaths, not a root DB guess.
    return Path(data['state_root'])/'instances'/hashlib.sha256(os.fsencode(data['host_endpoint'])).hexdigest()


def private_bytes(path, cap):
    path = Path(path)
    for parent in (path.parent, path.parent.parent):
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError('private_directory_required')
    fd = os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError('private_file_required')
        raw = os.read(fd,cap+1)
        if len(raw)>cap: raise ValueError('private_bound')
        return raw
    finally: os.close(fd)


def captured_instance(data):
    directory = instance_directory(data)
    observed=uuid.UUID(private_bytes(directory/'namespace',128).decode().strip())
    if observed.int==0: raise ValueError('namespace_nil')
    namespace=str(observed)
    return directory, namespace


def callback_event(session, turn, api_request):
    raw = json.dumps(['pre_llm_call',session,turn,None,api_request],
                     ensure_ascii=False,separators=(',',':')).encode()
    return 'hermes:'+hashlib.sha256(raw).hexdigest()


def journals(data, deadline):
    directory, namespace = captured_instance(data)
    root = directory/'contexts'
    info = root.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError('contexts_private')
    records = []
    with os.scandir(root) as entries:
        for index, entry in enumerate(entries):
            if index >= 256 or time.monotonic() >= deadline: raise ValueError('contexts_bound')
            raw = private_bytes(Path(entry.path)/'context.json',1048576)
            value = json.loads(raw,object_pairs_hook=unique_pairs,parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite')))
            if (value.get('version') != 1 or value.get('instance') != namespace
                    or not token(value.get('seat'),256) or not isinstance(value.get('completed'),list)
                    or len(value['completed'])>128 or len(value.get('declared_resets',[]))>32
                    or len(value.get('prepared_kinds',[]))>129):
                raise ValueError('context_schema')
            records.append((Path(entry.path),value))
    return directory, namespace, records


def matched_current(data, measurement, session, deadline):
    directory, namespace, records = journals(data, deadline)
    selected = [(path,value) for path,value in records if value['seat']==measurement['seat']
                and isinstance(value.get('current'),dict)
                and value['current'].get('target')==measurement['target']]
    if len(selected)!=1: raise ValueError('context_ambiguous_or_absent')
    path, value = selected[0]; current = value['current']
    if (current.get('format_version')!=1 or current.get('instance')!=namespace
            or current.get('seat')!=measurement['seat'] or current.get('harness')!='Hermes'
            or current.get('role')!='TopLevel' or current.get('session')!={'Native':session}
            or type(current.get('binding_generation')) is not int or current['binding_generation']<=0):
        raise ValueError('current_context_mismatch')
    if uuid.UUID(current['execution']).int==0: raise ValueError('execution_nil')
    connection = readonly_database(directory/'threads.sqlite3',deadline)
    try:
        rows = connection.execute("SELECT generation,execution_id,native_session FROM occupant_bindings WHERE seat_id=? AND target_id=? AND harness='hermes' AND observation_provenance='cooperative_top_level' AND registered_at IS NOT NULL AND ended_at IS NULL LIMIT 2",
            (measurement['seat'],measurement['target'])).fetchall()
        if len(rows)!=1 or tuple(rows[0])!=(current['binding_generation'],current['execution'],session):
            raise ValueError('canonical_binding_mismatch')
    finally: connection.close()
    if time.monotonic()>=deadline: raise ValueError('deadline')
    return path,value,current


def request_kind_digest(request, kind):
    fields=('operation_id','mode','context','expected_generation','event_id','payload_version','payload')
    context_fields=('format_version','instance','seat','target','harness','binding_generation','execution','session','role')
    ordered={name:request[name] for name in fields}
    ordered['context']={name:request['context'][name] for name in context_fields}
    return 'sha256:'+hashlib.sha256(json.dumps([ordered,kind],ensure_ascii=False,
        separators=(',',':')).encode()).hexdigest()


def qualified_return(data, measurement, session, turn, api_request, returned, started, deadline):
    # Official dispatcher list contains only the one loaded owned plugin result.
    if (not isinstance(returned,list) or len(returned)!=1 or not isinstance(returned[0],dict)
            or set(returned[0])!={'context'} or not isinstance(returned[0]['context'],str)
            or not 0<len(returned[0]['context'].encode())<=4096
            or measurement['context_marker'] not in returned[0]['context']):
        raise ValueError('qualified_return_absent')
    path,value,current = matched_current(data,measurement,session,deadline)
    event=callback_event(session,turn,api_request)
    done=[d for d in value['completed'] if d.get('request',{}).get('event_id')==event]
    if len(done)!=1 or value.get('pending') is not None: raise ValueError('completed_event_absent')
    done=done[0];request,response=done['request'],done['response']
    if uuid.UUID(request['operation_id']).int==0: raise ValueError('operation_nil')
    if (response.get('historical') is not False or response.get('context')!=current
            or any(request.get('context',{}).get(k)!=current.get(k) for k in ('format_version','instance','seat','target','harness','execution','session','role'))
            or (request.get('mode')=='Current' and request['context'].get('binding_generation')!=current['binding_generation'])
            or (request.get('mode')=='Lifecycle' and current['binding_generation']<=(request.get('expected_generation') or 0))
            or type(done.get('completed_at_millis')) is not int
            or not started<=done['completed_at_millis']<=int(time.time()*1000)):
        raise ValueError('completed_event_mismatch')
    kinds=[k for k in value.get('prepared_kinds',[]) if k.get('event_id')==event
           and k.get('operation_id')==request['operation_id']]
    if (len(kinds)!=1 or kinds[0].get('kind') not in ('Startup','Tool','Clear')
            or kinds[0].get('request_digest')!=request_kind_digest(request,kinds[0]['kind'])
            or request.get('mode')!=('Current' if kinds[0]['kind']=='Tool' else 'Lifecycle')):
        raise ValueError('prepared_kind_mismatch')
    context=returned[0]['context'].encode()
    result=row('callback_context','PASS','selective_return_and_canonical_completed_operation',1)
    result.update(context_sha256=hashlib.sha256(context).hexdigest(),context_bytes=len(context))
    return result,value,current,kinds[0]['kind']


def conservation_snapshot(data, deadline):
    directory,namespace,records=journals(data,deadline)
    private=[]
    for path,value in records:
        private.append((str(path.relative_to(directory)),hashlib.sha256(private_bytes(path/'context.json',1048576)).hexdigest()))
        attention=path/'attention.json'
        if os.path.lexists(attention):
            private.append((str(attention.relative_to(directory)),hashlib.sha256(private_bytes(attention,65536)).hexdigest()))
    connection=readonly_database(directory/'threads.sqlite3',deadline)
    try:
        connection.execute('BEGIN')
        tables=connection.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name LIMIT 257").fetchall()
        if len(tables)>256: raise ValueError('table_bound')
        canonical=[];size=0
        for table in tables:
            name=table[0]
            if not name.isidentifier() or time.monotonic()>=deadline: raise ValueError('table_or_deadline')
            columns=[r[1] for r in connection.execute('PRAGMA table_info("'+name+'")').fetchall()]
            if not columns or len(columns)>256 or any(not c.isidentifier() for c in columns):
                raise ValueError('column_bound')
            sizes='+'.join('COALESCE(length(CAST("'+c+'" AS BLOB)),0)' for c in columns)
            count,total,largest=connection.execute('SELECT COUNT(*),COALESCE(SUM('+sizes+'),0),COALESCE(MAX('+sizes+'),0) FROM "'+name+'"').fetchone()
            if count>4096 or total>4194304-size or largest>65536:
                raise ValueError('row_or_byte_bound')
            rows=connection.execute('SELECT * FROM "'+name+'" LIMIT 4097').fetchall()
            if len(rows)!=count: raise ValueError('snapshot_changed')
            normalized=[[v.hex() if isinstance(v,bytes) else v for v in r] for r in rows]
            encoded=sorted(json.dumps(r,ensure_ascii=False,separators=(',',':')) for r in normalized)
            size+=sum(len(v.encode()) for v in encoded)
            if size>4194304: raise ValueError('snapshot_bound')
            canonical.append((name,encoded))
    finally: connection.close()
    raw=json.dumps([namespace,sorted(private),canonical],ensure_ascii=False,separators=(',',':')).encode()
    return hashlib.sha256(raw).hexdigest()


def readonly_database(path, deadline):
    """Source-shaped SQLite projection, not an installed SessionDB import.

    Opening mode=ro never creates a missing database. The explicitly captured
    physical home/state root select the paths; no import-time default is used.
    Each query has a decreasing deadline and bounded row/body projections.
    """
    info=Path(path).lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=os.getuid(): raise ValueError('database_ownership')
    connection = sqlite3.connect(Path(path).resolve(strict=True).as_uri()+'?mode=ro',
                                 uri=True, timeout=0)
    connection.row_factory = sqlite3.Row
    connection.execute('PRAGMA query_only=ON')
    connection.set_progress_handler(lambda: int(time.monotonic() >= deadline), 100)
    return connection


def collect_model_evidence(data, deadline):
    """Observe persistence/actions with source-defined user-row boundaries.

    Native schema has no persisted callback turn_id. User-row ID to the next
    active user row scopes this projection, not an exact callback-turn witness.
    Assistant call IDs join actual native terminal results; canonical observation
    joins bind accept/ACK to the live seat/generation/session/execution. Prepared
    api_content alone never proves network delivery or model consumption.
    """
    result = dict(persistence=row('persisted_user_context'), delivery='UNMEASURED',
        callback_turn='UNMEASURED', attribution='UNQUALIFIED',
        correlation='native_user_row_boundary_and_exact_tool_call_id',
        actions=[row(stage) for stage in STAGES if stage.startswith('model_')])
    expected = data.get('model_evidence')
    fields = {'session_id','after_id','user_row_id','started_at','finished_at',
              'context_marker','seat_id','generation','execution_id','target_id',
              'thread_id','message_id'}
    if not isinstance(expected,dict) or set(expected) != fields:
        return result
    integers = ('after_id','user_row_id','generation')
    if (any(type(expected[k]) is not int or expected[k] < 0 for k in integers)
            or expected['user_row_id'] <= expected['after_id']
            or any(not token(expected[k],256) for k in fields-set(integers)-{'started_at','finished_at'})
            or any(type(expected[k]) not in (int,float) or not math.isfinite(expected[k])
                   for k in ('started_at','finished_at'))
            or not 0 < expected['finished_at']-expected['started_at'] <= 120):
        return result
    native = canonical = None
    try:
        native = readonly_database(Path(data['physical_home'])/'state.db',deadline)
        directory, namespace = captured_instance(data)
        canonical = readonly_database(directory/'threads.sqlite3',deadline)
        messages = native.execute("""SELECT id,role,content,api_content,tool_calls,
            tool_call_id,tool_name,effect_disposition,timestamp FROM messages
            WHERE session_id=? AND active=1 AND id>? AND timestamp>=? AND timestamp<=?
            AND length(COALESCE(content,''))<=65536 AND length(COALESCE(api_content,''))<=65536
            AND length(COALESCE(tool_calls,''))<=65536 ORDER BY id LIMIT 129""",
            (expected['session_id'],expected['after_id'],expected['started_at'],expected['finished_at'])).fetchall()
        if len(messages)>128 or time.monotonic()>=deadline:
            return result
        users = [message for message in messages if message['role']=='user']
        user = next((message for message in users if message['id']==expected['user_row_id']),None)
        if user is None or not isinstance(user['content'],str):
            return result
        # A supplied command/target in a user prompt cannot be presented as
        # context-derived cooperative action evidence. Raw text stays private.
        prompt=user['content']
        hints=(expected['context_marker'],expected['thread_id'],expected['message_id'],
               data['threads']['path'],'herdr-threads')
        if any(hint in prompt for hint in hints):
            result['attribution']='UNQUALIFIED_PROMPT'
            return result
        if data['api_mode']!='ordinary_string':
            result['persistence']=row('persisted_user_context',reason='selected_api_mode_not_source_qualified')
        elif isinstance(user['api_content'],str) and expected['context_marker'] in user['api_content']:
            result['persistence']=row('persisted_user_context','PASS','prepared_user_api_content_only',1)
        # The marker is a private expected predicate, not a callback-return
        # capture. No callback/API-send/consumption conclusion follows from it.
        result['attribution']='MODEL_CALL_AND_CANONICAL_OBSERVATION_ONLY'
        next_user=native.execute("SELECT id FROM messages WHERE session_id=? AND active=1 AND role='user' AND id>? ORDER BY id LIMIT 1", (expected['session_id'],user['id'])).fetchone()
        stop=next_user['id'] if next_user is not None else 2**63
        window=[message for message in messages if user['id']<message['id']<stop]
        binding=canonical.execute("""SELECT * FROM occupant_bindings WHERE seat_id=?
            AND generation=? AND target_id=? AND harness='hermes' AND native_session=?
            AND execution_id=? AND observation_provenance='cooperative_top_level'
            AND registered_at IS NOT NULL AND ended_at IS NULL LIMIT 2""",
            (expected['seat_id'],expected['generation'],expected['target_id'],
             expected['session_id'],expected['execution_id'])).fetchall()
        if len(binding)!=1:
            return result
        completed={}
        for message in window:
            if message['role']=='tool' and message['tool_name']=='terminal' and message['effect_disposition'] is None:
                completed.setdefault(message['tool_call_id'],[]).append(message)
        observations={}
        for message in window:
            if message['role']!='assistant' or not message['tool_calls']:
                continue
            calls=document(message['tool_calls'].encode())
            if not isinstance(calls,list) or len(calls)>32:
                continue
            for call in calls:
                function=call.get('function',{})
                if function.get('name')!='terminal' or call.get('type')!='function':
                    continue
                arguments=document(function['arguments'].encode())
                command=arguments.get('command')
                if not isinstance(command,str) or arguments.get('background'):
                    continue
                # Support only plain foreground CLI argv. Wrappers, chains,
                # substitution, polling and background actions stay unqualified.
                if any(character in command for character in (';','&','|','`','$','\n','>','<')):
                    continue
                parts=shlex.split(command)
                if len(parts)<2 or parts[0]!=data['threads']['path']:
                    continue
                # Source-defined global CLI options may precede the verb.
                tail=parts[1:]
                supplied={}
                scoped={'--state-dir':data['state_root'],'--host-endpoint':data['host_endpoint'],
                        '--cooperative-seat':expected['seat_id'],'--cooperative-target':expected['target_id'],
                        '--cooperative-harness':'hermes','--cooperative-role':'top-level'}
                while tail and tail[0].startswith('--'):
                    option=tail.pop(0)
                    if option in ('--json','--machine'):
                        continue
                    if option not in scoped or not tail or tail[0]!=scoped[option] or option in supplied:
                        tail=[]
                        break
                    supplied[option]=tail.pop(0)
                claims={key for key in supplied if key.startswith('--cooperative-')}
                if claims and len(claims)!=4:
                    continue
                if not tail:
                    continue
                verb=tail[0]
                targets={'accept':[expected['thread_id']], 'inbox':[],
                         'read':[expected['thread_id']], 'ack':[expected['message_id']]}
                if verb not in targets or tail[1:]!=targets[verb]:
                    continue
                replies=completed.get(call.get('id'),[])
                if len(replies)!=1 or replies[0]['id']<=message['id']:
                    continue
                response=document(replies[0]['content'].encode())
                if response.get('exit_code')!=0 or response.get('error') is not None:
                    continue
                observations[verb]=(message['timestamp'],replies[0]['timestamp'])
        def matches(observation, actor, generation, at, verb):
            value=document(observation.encode())
            return (actor==expected['seat_id'] and generation==expected['generation']
                and expected['started_at']*1000<=at<=expected['finished_at']*1000
                and observations[verb][0]*1000-100<=at<=observations[verb][1]*1000+100
                and value.get('harness')=='hermes' and value.get('session')==expected['session_id']
                and value.get('execution')==expected['execution_id']
                and value.get('binding_generation')==expected['generation']
                and value.get('provenance')=='cooperative_top_level'
                and value.get('action_provenance') != 'cooperative_inbox_display')
        for index,stage in enumerate(('model_accept','model_inbox','model_read','model_ack')):
            verb=stage.removeprefix('model_')
            if verb not in observations:
                continue
            observed=True
            if verb=='accept':
                rows=canonical.execute("""SELECT accepted_observation,accepted_actor_seat_id,
                    accepted_generation,accepted_at FROM invitations WHERE thread_id=?
                    AND seat_id=? AND state='accepted' ORDER BY accepted_at DESC LIMIT 2""",
                    (expected['thread_id'],expected['seat_id'])).fetchall()
                observed=bool(rows) and matches(*rows[0],verb)
            if verb=='ack':
                # Match the actual current effective receipt projection. A
                # manifested send owns receipt_state; never fall back to a
                # stale legacy row when its manifest exists.
                manifest=canonical.execute("SELECT thread_id,preparation_id FROM send_manifests WHERE message_id=? LIMIT 2",
                    (expected['message_id'],)).fetchall()
                if manifest:
                    rows=canonical.execute("""SELECT rs.ack_observation,rs.ack_actor_seat_id,
                        rs.ack_generation,rs.acked_at FROM send_manifests sm
                        JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id
                        JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id
                        WHERE sm.message_id=? AND sm.thread_id=? AND pr.seat_id=?
                        AND rs.state='acked' LIMIT 2""",
                        (expected['message_id'],expected['thread_id'],expected['seat_id'])).fetchall()
                else:
                    rows=canonical.execute("""SELECT ack_observation,ack_actor_seat_id,
                        ack_generation,acked_at FROM receipts WHERE message_id=? AND thread_id=?
                        AND seat_id=? AND state='acked' LIMIT 2""",
                        (expected['message_id'],expected['thread_id'],expected['seat_id'])).fetchall()
                observed=len(rows)==1 and matches(*rows[0],verb)
            if observed:
                reason=('native_model_call_result_and_canonical_actor' if verb in ('accept','ack')
                        else 'native_model_call_and_completed_terminal_result')
                result['actions'][index]=row(stage,'PASS',reason,1)
        return result
    except (OSError,sqlite3.Error,ValueError,TypeError,KeyError,AttributeError):
        return result
    finally:
        for connection in (native,canonical):
            if connection is not None:
                connection.close()


def run(data, mode):
    result = dict(schema_version=1, evidence_stage={'dry-run':'synthetic_dry_run',
        'preview':'action_preview', 'native':'native_measurement'}[mode], native_acceptance='UNMET',
        preflight=row('preflight', 'PASS', 'bounded_explicit_input', 1),
        matrix=[row(s) for s in STAGES], unsupported=[row(s,'SKIPPED','uncaptured_form')
            for s in ('resume','compression','native_abandonment')],
        cleanup=row('cleanup','SKIPPED','unperformed'),
        provenance={'threads_source_commit':data['threads']['source_commit'],
            'threads_binary_sha256':data['threads']['sha256'], 'host_source_commit':data['host']['source_commit'],
            'host_binary_sha256':data['host']['sha256'], 'host_modified':data['host']['modified'],
            'profile_sha256':hashlib.sha256(data['profile'].encode()).hexdigest(),
            'installation_token':data['installation_token'], 'api_mode':data['api_mode'],
            'startup_identity_sha256':hashlib.sha256(json.dumps(data['runtime_identity'],sort_keys=True).encode()).hexdigest(),
            'input_sha256':hashlib.sha256(json.dumps(data,sort_keys=True).encode()).hexdigest()},
        domains=[], permissions={'enablement':'manual','recognition':'separate_scope',
            'native_callbacks':'separate_scope','model':'separate_scope'})
    if mode == 'preview':
        return result
    try:
        if mode == 'native':
            native_preflight(data)
            native_commands(data)
        elif data['producer'] != 'synthetic_fixture':
            raise ValueError('dry_requires_labeled_standins')
    except ValueError:
        result['preflight'] = row('preflight', reason='scope_not_authorized_by_input')
        return result
    env = isolated_environment(data, mode)
    if mode == 'native' and data.get('measurement') is not None:
        env.update(HERDR_ENV='1',HERDR_PANE_ID=data['measurement']['target'],
                   HERDR_HERMES_CANARY_WORK=data['isolation_root'])
    owned = OwnedProcesses(env, data['isolation_root'])
    deadline = time.monotonic() + data['timeout_seconds']
    try:
        for name in ('home','state_root'):
            Path(data[name]).mkdir(mode=0o700, parents=True, exist_ok=True)
        for service in data['owned_services']:
            owned.spawn(service['argv'], pipes=False)
        commands = data['stages'] if mode == 'dry-run' else native_commands(data)
        order = list(STAGES)
        if mode == 'native':
            order.remove('recognition'); order.insert(order.index('guarded_launch')+1,'recognition')
        for stage in order:
            index = STAGES.index(stage)
            command = commands.get(stage)
            if command is None:
                continue
            code, raw, error = owned.capture(command, deadline)
            if error:
                measured = row(stage, reason=error, exit_code=code)
            elif code != 0:
                measured = row(stage, reason='command_failed', exit_code=code)
            else:
                try:
                    measured = (classify(stage, raw, 'synthetic_fixture') if mode == 'dry-run'
                                else native_row(stage, code, raw, data))
                except (ValueError, TypeError, AttributeError):
                    measured = row(stage, reason='observation_unavailable', exit_code=code)
            result['matrix'][index] = measured
        if mode == 'native' and 'native_domains' in commands:
            try:
                captured = capture_domains(data,owned,deadline)
                result['domains'] = captured['domains']
                for stage in ('plugin_discovery','enablement'):
                    result['matrix'][STAGES.index(stage)] = row(stage,'PASS','native_selective_loader_gate',1)
                # Actual callback context/API/model delivery remains independent
                # of the measured two-domain evidence reply milestones.
                result['matrix'][STAGES.index('callback_context')] = row(
                    'callback_context',reason='domain_reply_is_not_context_consumption')
                if captured['measurement'] is not None:
                    result['selective_api_measurement']=captured['measurement']
                    for observed in captured['measurement']['rows']:
                        result['matrix'][STAGES.index(observed['stage'])]=observed
            except (ValueError, OSError, TypeError, KeyError, StopIteration):
                result['domains'] = []
        if mode == 'native' and data.get('native_scope') == 'live':
            result['model_evidence'] = collect_model_evidence(data,deadline)
            for observed in result['model_evidence']['actions']:
                result['matrix'][STAGES.index(observed['stage'])] = observed
    except (OSError, ValueError):
        result['preflight'] = row('preflight', reason='owned_lifecycle_unavailable')
    finally:
        # Cleanup has its own strictly bounded reserve even after measurement
        # exhausted the decreasing deadline. It must not depend on stage success.
        code, _, error = owned.capture(data['cleanup_argv'], time.monotonic() + 1)
        clean = owned.close()
        result['cleanup'] = row('cleanup', 'PASS' if code == 0 and not error and clean else 'FAIL',
                                'owned_only_reaped' if clean else 'owned_process_survived',
                                len(owned.children), code)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    for mode in ('dry-run', 'preview', 'native'):
        modes.add_argument('--'+mode, action='store_true')
    parser.add_argument('--input', required=True, help='private strict schema1 captured command input')
    parser.add_argument('--result', required=True, help='new sanitized result file; never overwritten')
    args = parser.parse_args()
    mode = 'dry-run' if args.dry_run else 'preview' if args.preview else 'native'
    try:
        data = validate(read_private(args.input))
        result = run(data, mode)
    except (ValueError, OSError, TypeError, KeyError):
        result = dict(schema_version=1,evidence_stage='unavailable',native_acceptance='UNMET',
                      preflight=row('preflight',reason='bounded_input_unavailable'),matrix=[],
                      cleanup=row('cleanup','SKIPPED','no_owned_process_started'))
    fd = os.open(args.result, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(result, stream, sort_keys=True, allow_nan=False)
        stream.write('\n')
    print(json.dumps({'schema_version':1,'native_acceptance':result['native_acceptance'],
                      'evidence_stage':result['evidence_stage']}))


if __name__ == '__main__':
    main()
