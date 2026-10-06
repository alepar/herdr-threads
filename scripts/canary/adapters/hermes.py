#!/usr/bin/env python3
"""Exact Hermes selective-loader canary; fixtures never verify a native domain.

The official machine argv is opaque. Native mode requires an already prepared,
owned isolated profile and real configured Rust child/private endpoint. It uses
captured-source private APIs, not full CLI discovery or model interaction.
"""
import argparse
import hashlib
import importlib
import importlib.util
import json
import math
import os
from pathlib import Path
import stat
import sys
import sysconfig
import time
import unicodedata
import uuid

HERE = Path(__file__).resolve()
NAMES = ('__init__.py', 'bridge_config.json', 'plugin.yaml')


def runner():
    spec = importlib.util.spec_from_file_location('hermes_canary_runner', HERE.parents[1] / 'run.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def text(value, limit=4096):
    return isinstance(value, str) and 0 < len(value.encode()) <= limit and not any(unicodedata.category(c) == 'Cc' for c in value)


def read(path, cap=65536, private=False):
    path = Path(path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > cap or (private and (info.st_uid != os.geteuid() or info.st_mode & 0o077)):
            raise ValueError('file')
        raw = os.read(fd, cap + 1)
        if len(raw) > cap:
            raise ValueError('bound')
        return raw
    finally:
        os.close(fd)


def document(path, cap=65536, private=False):
    return runner()._json(read(path, cap, private), cap)


def identity(value):
    engine = runner()
    schema = json.loads((HERE.parents[1] / 'companion_schema.json').read_text())
    engine._schema(value, schema['properties']['identity'])
    if value is None:
        raise ValueError('identity')
    engine.validate_identity(value)
    return value


def input_document(path):
    data = document(path, 32768)
    common = {'schema_version', 'producer', 'argv', 'timeout_seconds'}
    native = {'profile', 'home', 'physical_home', 'source_root', 'isolation_root', 'state_root',
              'host_endpoint', 'identity', 'installation_token', 'binary'}
    if (not isinstance(data, dict) or data.get('schema_version') != 1
            or data.get('producer') not in ('synthetic_fixture', 'official_native_selective')
            or set(data) != common | (native if data['producer'] == 'official_native_selective' else set()) | ({'measurement'} if 'measurement' in data else set())
            or not isinstance(data['argv'], list) or not 1 <= len(data['argv']) <= 16
            or not all(text(v) for v in data['argv']) or sum(len(v.encode()) for v in data['argv']) > 16384
            or not os.path.isabs(data['argv'][0])
            or type(data['timeout_seconds']) not in (int, float)
            or not math.isfinite(data['timeout_seconds']) or not 0 < data['timeout_seconds'] <= 30):
        raise ValueError('input')
    if 'measurement' in data:
        measurement_input(data['measurement'])
        if data['producer'] != 'official_native_selective': raise ValueError('measurement_producer')
    if data['producer'] == 'official_native_selective':
        identity(data['identity'])
        if not text(data['profile'], 256) or not text(data['installation_token'], 256):
            raise ValueError('scope')
        uuid.UUID(data['installation_token'])
        for name in native - {'identity', 'profile', 'installation_token'}:
            if not text(data[name]) or not os.path.isabs(data[name]):
                raise ValueError('scope')
        root = Path(data['isolation_root']).resolve(strict=True)
        for name in ('home', 'physical_home', 'state_root', 'host_endpoint'):
            if not Path(data[name]).resolve().is_relative_to(root):
                raise ValueError('isolation')
        if str(Path(data['home']).resolve(strict=True)) != data['physical_home']:
            raise ValueError('scope')
        # Exact official trace argument shape; never inspect/edit the opaque bootstrap.
        argv = data['argv']
        if (len(argv) != 9 or argv[1:3] != ['-I', '-c']
                or argv[4:] != ['--count', '--no-report', str(HERE), '--native-driver', str(Path(path).resolve())]):
            raise ValueError('entry')
    return data


def origin(module):
    file = getattr(module, '__file__', None)
    spec = getattr(getattr(module, '__spec__', None), 'origin', None)
    if not text(file) or not text(spec) or not os.path.isabs(file) or not os.path.isabs(spec):
        raise ValueError('origin')
    path = Path(file).resolve(strict=True)
    if path != Path(spec).resolve(strict=True):
        raise ValueError('origin')
    return path


def native_context(data):
    """Qualify the reviewed completed bootstrap/trace entry before native imports."""
    bootstrap = sys.modules.get('hermes_bootstrap')
    root = origin(bootstrap).parent
    if (str(root) != data['source_root'] or origin(bootstrap) != root / 'hermes_bootstrap.py'
            or getattr(bootstrap, '_root', None) != root
            or getattr(bootstrap, '_pm_repair', None) is not False
            or '_launch_python' not in vars(bootstrap) or bootstrap._launch_python is not None
            or getattr(getattr(bootstrap, '__spec__', None), '_initializing', True)
            or Path(sys.executable).resolve(strict=True) != Path(data['argv'][0]).resolve(strict=True)):
        raise ValueError('entry')
    main = sys.modules.get('__main__')
    if (getattr(getattr(main, '__spec__', None), 'name', None) != 'trace'
            or origin(main) != Path(sysconfig.get_path('stdlib')).resolve(strict=True) / 'trace.py'
            or sys.orig_argv != data['argv'] or sys.argv != data['argv'][-3:]
            or len(sys.path) < 2 or sys.path[0] != str(HERE.parent)):
        raise ValueError('entry')
    selected = sys.path[1]
    if (not text(selected) or not os.path.isabs(selected) or Path(selected).name != 'site-packages'
            or not Path(selected).is_dir() or os.environ.get('PYTHONPATH') != os.pathsep.join((str(root), selected))):
        raise ValueError('activation')
    for name, module in tuple(sys.modules.items()):
        if name in ('hermes_bootstrap', 'hermes_constants', 'hermes_cli', 'pm') or name.startswith(('hermes_cli.', 'pm.')):
            stem = root / name.replace('.', '/')
            if origin(module) not in (stem.with_suffix('.py'), stem / '__init__.py'):
                raise ValueError('mixed_origin')
    sys.path[0] = str(root)  # Restore only trace's replaced slot; retain remaining ordering.
    modules = {}
    for name in ('hermes_cli.profiles', 'hermes_constants', 'hermes_cli.version_info', 'hermes_cli.config', 'hermes_cli.plugins'):
        module = importlib.import_module(name)
        if origin(module) != root / (name.replace('.', '/') + '.py'):
            raise ValueError('origin')
        modules[name] = module
        if name == 'hermes_cli.profiles':
            if module.normalize_profile_name(data['profile']) != data['profile']:
                raise ValueError('profile')
            module.validate_profile_name(data['profile'])
            home = module.resolve_profile_env(data['profile'])
            if home != data['home'] or str(Path(home).resolve(strict=True)) != data['physical_home']:
                raise ValueError('profile')
            os.environ['HERMES_HOME'] = home  # Official selected-profile result, not a guessed path.
    # Imports may add dispatch/loader/ledger modules; bind those origins too.
    for name, module in tuple(sys.modules.items()):
        if name.startswith('hermes_cli.') or name in ('hermes_cli', 'hermes_constants', 'hermes_bootstrap', 'utils', 'registration_lifecycle', 'hermes_yaml'):
            stem = root / name.replace('.', '/')
            if origin(module) not in (stem.with_suffix('.py'), stem / '__init__.py'):
                raise ValueError('mixed_origin')
    if str(modules['hermes_constants'].get_hermes_home()) != data['home']:
        raise ValueError('profile')
    info = modules['hermes_cli.version_info'].get_version_info()
    descriptor = dict(release_version=None, source=info.source, base_version=info.base_version,
                      derived_version=info.derived_version, commit=info.commit, dirty=info.dirty, distance=info.distance)
    key = 'build:' + hashlib.sha256(json.dumps(dict(descriptor, schema_version=1), sort_keys=True,
        separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
    observed = identity(dict(descriptor, key=key))
    if observed != data['identity']:
        raise ValueError('identity')
    return modules, observed


def owned_generation(data):
    """Check the existing private transaction and exact three assets; never publish."""
    import fcntl
    home, state = Path(data['physical_home']), Path(data['state_root'])
    assets = home / 'plugins' / 'herdr-threads'
    if assets.is_symlink() or assets.parent.is_symlink() or not assets.is_dir():
        raise ValueError('assets')
    lock = os.open(home / 'plugins' / '.herdr-threads-operation.lock', os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(lock)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
            raise ValueError('lock')
        fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
        path = state / 'setup' / 'hermes' / ('sha256:' + hashlib.sha256(str(home).encode()).hexdigest() + '.json')
        m = document(path, 1048576, True)
        if (set(m) != {'schema_version', 'home', 'installation_token', 'phase', 'lock_inode', 'directory_inode', 'stage_name', 'assets'}
                or m['schema_version'] != 1 or m['home'] != str(home) or m['phase'] != 'complete'
                or m['installation_token'] != data['installation_token'] or m['stage_name'] is not None
                or m['lock_inode'] != [info.st_dev, info.st_ino]
                or m['directory_inode'] != [assets.stat().st_dev, assets.stat().st_ino]
                or set(m['assets']) != set(NAMES)):
            raise ValueError('generation')
        for name in NAMES:
            a = m['assets'][name]
            raw = read(assets / name, 1048576, True)
            if set(a) != {'digest', 'prior', 'bytes'} or a['bytes'] != list(raw) or 'sha256:' + hashlib.sha256(raw).hexdigest() != a['digest']:
                raise ValueError('generation')
        settings = document(assets / 'bridge_config.json', private=True)
        if (set(settings) != {'schema_version', 'bridge_schema_version', 'installation_token', 'rust_executable', 'state_root', 'host_endpoint'}
                or settings['schema_version'] != 1 or settings['bridge_schema_version'] != 1
                or settings['installation_token'] != data['installation_token']
                or settings['rust_executable'] != data['binary'] or settings['state_root'] != data['state_root']
                or settings['host_endpoint'] != data['host_endpoint']):
            raise ValueError('settings')
        return assets, hashlib.sha256(json.dumps(m, sort_keys=True).encode()).hexdigest()
    finally:
        os.close(lock)  # No profile lock is held across native I/O.


def gate_paths(data, adapter, session):
    paths = []
    for c in adapter['contracts']:
        runtime = {name: data['identity'][name] for name in ('key', 'release_version', 'source', 'base_version', 'derived_version', 'commit', 'dirty', 'distance')}
        key = dict(harness='hermes', runtime=runtime, unavailable_reason=None,
                   domain=c['domain'], origin=c['origin'], contract_id=c['id'], session_id=session)
        digest = hashlib.sha256(json.dumps(key, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()
        paths.append((Path(data['state_root']) / 'harness' / 'evidence-v2' / (digest + '.json'), key, c))
    return paths


def completed_gates(paths, started):
    domains = []
    for path, key, contract in paths:
        # Reject foreign/private-path replacements and output without a timely real ACK.
        parent = path.parent
        for directory in (parent, parent.parent, parent.parent.parent):
            info = directory.lstat()
            if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
                raise ValueError('gate_directory')
        raw = document(path, 8192, True)
        if (set(raw) != {'version', 'key', 'verified', 'milestones', 'heartbeat_at_ms', 'sent'}
                or type(raw['version']) is not int or raw['version'] != 2 or raw['key'] != key
                or raw['verified'] is not True or not isinstance(raw['milestones'], list)
                or len(raw['milestones']) != len(set(raw['milestones']))
                or set(raw['milestones']) != set(contract['required_milestones'])
                or type(raw['heartbeat_at_ms']) is not int or not started <= raw['heartbeat_at_ms'] <= int(time.time() * 1000)
                or raw['sent'] != []):
            raise ValueError('gate')
        domains.append(dict(domain=contract['domain'], origin=contract['origin'], contract_id=contract['id'],
            successful_milestones=raw['milestones'], violations=[], outcome='compatible'))
    return domains


def require_settled_dispatcher(manager, deadline):
    """Pinned dispatcher tokens release only after callback cleanup, before unload.

    Gate ACKs are independent of callback settlement. Never wait for its lock or
    trust missing bookkeeping; unload clears these maps without joining workers.
    """
    lock = getattr(manager, '_hook_timeout_lock', None)
    if time.monotonic() >= deadline or lock is None or not lock.acquire(blocking=False):
        raise ValueError('dispatcher_unknown_or_contended')
    try:
        running = getattr(manager, '_hook_running_callbacks', None)
        abandoned = getattr(manager, '_hook_abandoned', None)
        if (type(running) is not dict or type(abandoned) is not dict
                or running or abandoned or time.monotonic() >= deadline):
            raise ValueError('dispatcher_pending_or_unknown')
    finally:
        lock.release()


def measurement_input(value):
    fields={'schema_version','invocation_id','target','seat','context_marker','child_form'}
    if (not isinstance(value,dict) or set(value)!=fields or type(value['schema_version']) is not int or value['schema_version']!=1
            or not all(text(value[k],256) for k in fields-{'schema_version'})
            or value['child_form'] not in ('explicit_parent_callback','persist_disabled_no_callback')):
        raise ValueError('measurement_input')
    if uuid.UUID(value['invocation_id']).int==0: raise ValueError('measurement_invocation')
    return value


def measurement_module():
    spec=importlib.util.spec_from_file_location('hermes_native_measurement',HERE.parents[2]/'native-hermes-probe.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


def measure_callbacks(data, manager, plugin, returned, session, turn, api_request, started, deadline):
    """Opt-in selective API diagnostics. Neither model delivery nor domain credit.

    All state reads are bounded private snapshots; only derived scalars leave here.
    A reset observer's None is never success, and child absence never means read-only.
    """
    engine=measurement_module();measurement=data['measurement']
    output=dict(schema_version=1,scope='selective_native_api_invocation_not_model_delivery',
        invocation_sha256=hashlib.sha256(measurement['invocation_id'].encode()).hexdigest(),
        rows=[engine.row(s) for s in ('callback_context','child','reset')])
    try:
        observed,initial_doc,initial,kind=engine.qualified_return(data,measurement,session,turn,
            api_request,returned,started,deadline)
        output['rows'][0]=observed
    except (ValueError,OSError,KeyError,TypeError,AttributeError,engine.sqlite3.Error):
        return output
    try:
        if measurement['child_form']=='persist_disabled_no_callback':
            output['rows'][1]=engine.row('child','SKIPPED','native_form_omits_callback',0)
        else:
            before=engine.conservation_snapshot(data,deadline)
            child_results=manager.invoke_hook('pre_llm_call',platform='cli',
                session_id=session+'-child',parent_session_id=session,
                turn_id=turn+'-child',api_request_id=api_request+'-child')
            require_settled_dispatcher(manager,deadline)
            restriction=getattr(plugin,'CHILD_RESTRICTION',None)
            if (isinstance(restriction,str) and restriction
                    and child_results==[{'context':restriction}]
                    and engine.conservation_snapshot(data,deadline)==before):
                output['rows'][1]=engine.row('child','PASS','selective_declared_child_return_and_private_state_conservation',1)
    except (ValueError,OSError,KeyError,TypeError,AttributeError,engine.sqlite3.Error):
        pass
    try:
        new_session=session+'-reset';new_turn=turn+'-reset';new_api=api_request+'-reset'
        if time.monotonic()>=deadline: raise ValueError('deadline')
        reset_started=int(time.time()*1000)
        manager.invoke_hook('on_session_reset',platform='cli',session_id=new_session,reason='new_session')
        require_settled_dispatcher(manager,deadline)
        _,hint_doc,current=engine.matched_current(data,measurement,session,deadline)
        hints=[h for h in hint_doc.get('declared_resets',[]) if h.get('target')==measurement['target']
               and h.get('harness')=='Hermes' and h.get('session')==new_session
               and h.get('generation')==initial['binding_generation'] and h.get('consumed') is False
               and type(h.get('observed_at_millis')) is int
               and reset_started<=h['observed_at_millis']<=int(time.time()*1000)]
        if len(hints)!=1 or current!=initial: raise ValueError('reset_hint_absent')
        hint=hints[0]
        returned_reset=manager.invoke_hook('pre_llm_call',platform='cli',session_id=new_session,
            parent_session_id='',turn_id=new_turn,api_request_id=new_api)
        require_settled_dispatcher(manager,deadline)
        _,finished,new_current,kind=engine.qualified_return(data,measurement,new_session,new_turn,
            new_api,returned_reset,reset_started,deadline)
        consumed=[h for h in finished.get('declared_resets',[]) if h.get('event_key')==hint.get('event_key')
            and h.get('session')==new_session and h.get('target')==measurement['target']
            and h.get('generation')==initial['binding_generation'] and h.get('consumed') is True]
        if (kind=='Clear' and len(consumed)==1 and new_current['seat']==initial['seat']
                and new_current['target']==initial['target']
                and new_current['binding_generation']>initial['binding_generation']
                and new_current['execution']!=initial['execution']):
            output['rows'][2]=engine.row('reset','PASS','declared_hint_consumed_clear_and_canonical_transition',1)
    except (ValueError,OSError,KeyError,TypeError,AttributeError,engine.sqlite3.Error):
        pass
    return output


def native_driver(path):
    """Native entry; offline tests inject labeled fake APIs, never installed Hermes."""
    data = input_document(path)
    if data['producer'] != 'official_native_selective':
        raise ValueError('producer')
    adapter = document(Path(os.environ['HERDR_HERMES_CANARY_WORK']) / 'request.json')['adapter']
    if len(adapter['contracts']) != 2 or {(c['domain'], c['origin']) for c in adapter['contracts']} != {
            ('native_callback', 'native_shape_observation'), ('bridge_envelope', 'bridge_envelope')}:
        raise ValueError('contracts')
    deadline = time.monotonic() + data['timeout_seconds'] - .1
    # Parent-prepared private endpoint only; this driver never starts a daemon.
    endpoint = Path(data['host_endpoint']).lstat()
    if not stat.S_ISSOCK(endpoint.st_mode) or endpoint.st_uid != os.geteuid():
        raise ValueError('endpoint')
    executable = Path(data['binary']).lstat()
    if not stat.S_ISREG(executable.st_mode) or not os.access(data['binary'], os.X_OK):
        raise ValueError('binary')
    modules, observed = native_context(data)
    plugins, config = modules['hermes_cli.plugins'], modules['hermes_cli.config']
    assets, generation = owned_generation(data)
    measurement=data.get('measurement')
    session = ('measurement-'+measurement['invocation_id']) if measurement else 'canary-'+str(uuid.uuid4())
    paths = gate_paths(data, adapter, session)
    if any(os.path.lexists(p) for p, _, _ in paths):
        raise ValueError('baseline')
    manager = plugins.PluginManager(scope_key=data['physical_home'])
    loaded = False
    started = int(time.time() * 1000)
    try:
        with plugins._plugin_home_scope(manager.home_path):
            effective = config.load_config_readonly()
            if isinstance(effective, config.FailedConfigRead) or type(effective) is not dict:
                raise ValueError('config')
            selection = effective.get('plugins')
            if type(selection) is not dict:
                raise ValueError('config')
            enabled, disabled = selection.get('enabled'), selection.get('disabled')
            if not all(type(v) is list and len(v) <= 128 and all(text(n, 256) for n in v) for v in (enabled, disabled)):
                raise ValueError('config')
            manifests = manager._scan_directory(assets.parent, 'user')
            if len(manifests) != 1:
                raise ValueError('foreign_candidates')
            m = manifests[0]
            if (m.name != 'herdr-threads' or Path(m.path) != assets or m.kind != 'standalone'
                    or m.source != 'user' or m.manifest_version != 2 or m.portable or m.requires_env
                    or m.requires_plugins or m.python_dependencies or m.provides_tools or m.capabilities):
                raise ValueError('manifest')
            if not manager._gate_manifest(m, set(disabled), set(enabled)):
                raise ValueError('disabled')  # Includes actual native offline removal gate.
            manager._warn_python_dependencies(m)
            manager._validate_plugin_config_schema(m)
            if owned_generation(data)[1] != generation or time.monotonic() >= deadline:
                raise ValueError('generation')
            manager._load_plugin(m)
            loaded = any(p.enabled and p.manifest.name == 'herdr-threads' and not p.error for p in manager._plugins.values())
            if not loaded or not all(manager._hooks.get(n) for n in ('pre_llm_call', 'post_tool_call')):
                raise ValueError('load')
            # Wait for the existing single native observation reader, never replace it.
            module = next(p.module for p in manager._plugins.values() if p.manifest.name == 'herdr-threads')
            reservation = sys.modules.get(module.SLOT)
            while time.monotonic() < deadline:
                reader = getattr(reservation, 'reader', None)
                if reader is not None and reader.snapshot() is not None:
                    break
                time.sleep(min(.02, max(0, deadline - time.monotonic())))
            if time.monotonic() >= deadline:
                raise ValueError('timeout')
            returned = manager.invoke_hook('pre_llm_call', platform='cli', session_id=session,
                                parent_session_id='', turn_id='canary-turn', api_request_id='canary-request')
            if time.monotonic() >= deadline:
                raise ValueError('timeout')
            manager.invoke_hook('post_tool_call', platform='cli', session_id=session, turn_id='canary-turn',
                                tool_call_id='canary-tool', api_request_id='canary-request')
            if time.monotonic() >= deadline or owned_generation(data)[1] != generation:
                raise ValueError('timeout_or_drift')
            domains = completed_gates(paths, started)
            require_settled_dispatcher(manager, deadline)
            if measurement is not None:
                measured=measure_callbacks(data,manager,module,returned,session,'canary-turn','canary-request',started,deadline)
                if owned_generation(data)[1]!=generation: raise ValueError('generation')
                require_settled_dispatcher(manager,deadline)
    finally:
        manager.unload()
        # Uncancellable native read may outlive bounded unload: never claim clean PASS.
        slot = sys.modules.get('_herdr_threads_hermes_reader_schema1')
        reader = getattr(slot, 'reader', None)
        if reader is not None:
            child = getattr(getattr(reader, 'bridge', None), 'unreaped_child', None)
            if reader.thread.is_alive() or (child is not None and child.poll() is None):
                raise ValueError('owned_worker_survived')
    result=dict(schema_version=1, provenance='official_native_selective', identity=observed,
                native_loaded=loaded, domains=domains)
    if measurement is not None: result['measurement']=measured
    return result


def main(argv=None):
    parser = argparse.ArgumentParser()
    for name in ('harness', 'attempt', 'stage', 'work-dir', 'binary', 'runtime-command-file', 'native-driver'):
        parser.add_argument('--' + name)
    args = parser.parse_args(argv)
    if args.native_driver:
        try:
            print(json.dumps(native_driver(args.native_driver), separators=(',', ':'), allow_nan=False))
        except (Exception, SystemExit):
            print(json.dumps(dict(schema_version=1, provenance='unavailable', identity=None, native_loaded=False, domains=[])))
        return
    result = dict(schema_version=1, harness=args.harness, attempt=args.attempt, identity=None,
                  evidence_stage=args.stage, outcome='inconclusive', reason='explicit runtime input required', domains=[])
    if args.harness != 'hermes' or args.stage not in ('source_captured', 'no_model'):
        result.update(outcome='unsupported', reason='unsupported harness or stage')
    elif args.runtime_command_file:
        try:
            data = input_document(args.runtime_command_file)
            if data['producer'] == 'official_native_selective' and data['binary'] != args.binary:
                raise ValueError('binary')
            if args.stage == 'source_captured':
                result['reason'] = 'source-only request; native load and dispatch unperformed'
                print(json.dumps(result, separators=(',', ':')))
                return
            env = dict(os.environ, HERDR_HERMES_CANARY_WORK=args.work_dir)
            if data['producer'] == 'official_native_selective':
                env['HERMES_HOME'] = data['home']
            code, raw, _ = runner().bounded_capture(data['argv'], timeout=data['timeout_seconds'], env=env, cwd=args.work_dir)
            if code != 0:
                raise ValueError('runtime')
            observation = runner()._json(raw, 65536)
            if (not isinstance(observation, dict) or set(observation) != {'schema_version', 'provenance', 'identity', 'native_loaded', 'domains'}
                    or observation['schema_version'] != 1 or type(observation['native_loaded']) is not bool
                    or observation['provenance'] != data['producer']):
                raise ValueError('producer')
            observed = identity(observation['identity'])
            result['identity'] = observed
            if data['producer'] == 'synthetic_fixture':
                result['reason'] = 'synthetic fixture; native measurement unperformed'
            elif args.stage == 'source_captured':
                result['reason'] = 'source stage cannot verify native milestones'
            elif observation['native_loaded'] and observed == data['identity']:
                candidate = dict(result, domains=observation['domains'], outcome='complete', reason=None)
                adapter = document(Path(args.work_dir) / 'request.json')['adapter']
                runner().validate_result(json.dumps(candidate).encode(), adapter, args.attempt, args.stage)
                if len(runner().verified_domains(candidate, adapter)) != 2:
                    raise ValueError('milestones')
                result = candidate
            else:
                raise ValueError('load')
        except (Exception, SystemExit) as error:
            result.update(identity=None, domains=[], outcome='inconclusive',
                          reason='runtime timeout or bounded input/output unavailable' if 'deadline' in str(error) else 'runtime input or observation unavailable')
    if result['domains']:
        work = Path(args.work_dir)
        captures = []
        for domain in result['domains']:
            stem = domain['domain']
            # Sanitized client-output projection; no raw callback/config/tool paths or bodies.
            (work / (stem + '.json')).write_text(json.dumps(dict(provenance='completed_private_v2_client_output',
                evidence_stage=args.stage, identity_key=result['identity']['key'], **domain), separators=(',', ':')))
            metadata = dict(domain=stem, origin=domain['origin'], evidence_stage=args.stage, path=stem + '.json')
            name = stem + '-capture.json'
            (work / name).write_text(json.dumps(metadata, separators=(',', ':')))
            captures.append(name)
        (work / 'captures.json').write_text(json.dumps(captures))
    print(json.dumps(result, separators=(',', ':'), allow_nan=False))


if __name__ == '__main__':
    main()
