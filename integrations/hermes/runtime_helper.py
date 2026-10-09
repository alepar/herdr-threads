"""Source-bound startup/profile observation, never runtime or callback admission.

Only the official completed-bootstrap trace entry is accepted. Native inspection
may initialize/recover Hermes-owned state. CLI dotenv and post-profile scratch
re-home are deliberately absent. No registrars, auth/model or plugin discovery.
"""
import importlib
import json
import os
from pathlib import Path
import sys
import sysconfig
import unicodedata

SCOPE = 'declared_child_input_plus_native_bootstrap_profile_effects'
FIELDS = ('runtime_descriptor', 'profile', 'home', 'physical_home', 'interpreter',
          'source_root', 'module_origins', 'dependency_paths', 'enabled', 'disabled')


def opaque(value, limit=256):
    return (isinstance(value, str) and 0 < len(value.encode()) <= limit
            and not any(unicodedata.category(c) == 'Cc' for c in value))


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate')
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite')))


def origin(module):
    file = getattr(module, '__file__', None)
    spec = getattr(getattr(module, '__spec__', None), 'origin', None)
    if not all(opaque(p, 4096) and os.path.isabs(p) for p in (file, spec)):
        raise ValueError('origin')
    resolved = Path(file).resolve(strict=True)
    if resolved != Path(spec).resolve(strict=True):
        raise ValueError('origin')
    return resolved


def empty(reason, prelaunch=False):
    result = {'schema_version': 3 if prelaunch else 2, 'status': 'unavailable', 'reason': reason,
            **dict.fromkeys(FIELDS), 'config_quality': 'unknown', 'fallback_kind': None,
            'environment_scope': SCOPE, 'cli_dotenv_loaded': False,
            'cli_scratch_rehomed': False, 'identity_provenance': 'startup_captured',
            'evidence_stage': 'unavailable'}
    if prelaunch:
        result['api'] = None
    return result


def inspect(profile):
    raw = os.environ.get('HERDR_HERMES_INSPECTION_SCOPE', '')
    if len(raw.encode()) > 16384 or not opaque(profile):
        raise ValueError('scope')
    scope = strict_json(raw)
    discovery = isinstance(scope, dict) and set(scope) == {'mode', 'interpreter', 'profile'} and scope['mode'] in ('discover_selected_profile', 'inspect_prelaunch')
    prelaunch = discovery and scope['mode'] == 'inspect_prelaunch'
    if not discovery and (not isinstance(scope, dict) or set(scope) != {'interpreter', 'source_root', 'profile', 'home'}):
        raise ValueError('scope')
    if not opaque(scope.get('interpreter'), 4096) or not os.path.isabs(scope['interpreter']):
        raise ValueError('scope')
    if scope['profile'] != profile:
        raise ValueError('scope')
    helper = Path(__file__).resolve(strict=True)
    interpreter = Path(sys.executable).resolve(strict=True)
    bootstrap = sys.modules.get('hermes_bootstrap')
    root = origin(bootstrap).parent
    if (root / 'hermes_bootstrap.py' != origin(bootstrap)
            or (not discovery and str(root) != scope['source_root']) or str(interpreter) != scope['interpreter']
            or getattr(bootstrap, '_root', None) != root
            or getattr(bootstrap, '_pm_repair', None) is not False
            or '_launch_python' not in vars(bootstrap) or bootstrap._launch_python is not None
            or getattr(getattr(bootstrap, '__spec__', None), '_initializing', True)):
        raise ValueError('entry')
    main = sys.modules.get('__main__')
    if (getattr(getattr(main, '__spec__', None), 'name', None) != 'trace'
            or origin(main) != Path(sysconfig.get_path('stdlib')).resolve(strict=True) / 'trace.py'
            or sys.argv != [str(helper), '--profile', profile]
            or len(sys.orig_argv) != 9
            or sys.orig_argv[1:3] != ['-I', '-c']
            or sys.orig_argv[-5:] != ['--count', '--no-report', str(helper), '--profile', profile]
            or len(sys.path) < 2 or sys.path[0] != str(helper.parent)):
        raise ValueError('entry')
    selected = sys.path[1]
    if (not opaque(selected, 4096) or not os.path.isabs(selected)
            or Path(selected).name != 'site-packages' or not Path(selected).is_dir()
            or os.environ.get('PYTHONPATH') != os.pathsep.join((str(root), selected))):
        raise ValueError('activation_shape')
    # Check every already loaded native root/package origin before new imports.
    for name, module in tuple(sys.modules.items()):
        if name in ('hermes_bootstrap', 'hermes_constants') or name == 'hermes_cli' or name.startswith(('hermes_cli.', 'pm.')) or name == 'pm':
            path = origin(module)
            stem = root / name.replace('.', '/')
            if path not in (stem.with_suffix('.py'), stem / '__init__.py'):
                raise ValueError('mixed_origin')
    sys.path[0] = str(root)  # Exactly the one slot trace replaced; retain all other order.
    profiles = importlib.import_module('hermes_cli.profiles')
    if origin(profiles) != root / 'hermes_cli' / 'profiles.py':
        raise ValueError('origin')
    canon = profiles.normalize_profile_name(profile)
    profiles.validate_profile_name(canon)
    if canon != profile:
        raise ValueError('profile')
    home = profiles.resolve_profile_env(profile)
    if not opaque(home, 4096) or not os.path.isabs(home) or (not discovery and home != scope['home']):
        raise ValueError('profile')
    os.environ['HERMES_HOME'] = home
    constants = importlib.import_module('hermes_constants')
    version = importlib.import_module('hermes_cli.version_info')
    config = importlib.import_module('hermes_cli.config')
    modules = {'hermes_bootstrap': bootstrap, 'hermes_constants': constants,
               'hermes_cli.profiles': profiles, 'hermes_cli.version_info': version,
               'hermes_cli.config': config}
    origins = {name: str(origin(module)) for name, module in modules.items()}
    for name, path in origins.items():
        wanted = root / (name.replace('.', '/') + '.py')
        if Path(path) != wanted:
            raise ValueError('origin')
    if str(constants.get_hermes_home()) != home:
        raise ValueError('profile')
    physical = str(Path(home).resolve(strict=True))
    if not Path(physical).is_dir():
        raise ValueError('profile')
    info = version.get_version_info()
    descriptor = {'release_version': None, 'source': info.source, 'base_version': info.base_version,
                  'derived_version': info.derived_version, 'commit': info.commit,
                  'dirty': info.dirty, 'distance': info.distance}
    if (descriptor['source'] not in ('build', 'commit-build', 'ci', 'docker', 'fallback', 'git', 'local', 'nix')
            or not all(opaque(descriptor[k], 128) for k in ('base_version', 'derived_version'))
            or type(descriptor['dirty']) is not bool
            or (descriptor['distance'] is not None and (type(descriptor['distance']) is not int or not 0 <= descriptor['distance'] <= 4294967295))
            or (descriptor['commit'] is not None and (not isinstance(descriptor['commit'], str) or len(descriptor['commit']) != 40 or any(c not in '0123456789abcdef' for c in descriptor['commit'])))
            or (info.source == 'git' and (info.commit is None or info.distance is None))):
        raise ValueError('identity')
    quality, enabled, disabled = 'unknown', None, None
    try:
        data = config.load_config_readonly()
        failed_type = config.FailedConfigRead
        if isinstance(data, failed_type):
            quality = 'failed_config_read'
        elif type(data) is dict:
            plugins = data.get('plugins')
            if type(plugins) is dict:
                lists = [plugins.get(k) for k in ('enabled', 'disabled')]
                if all(type(v) is list and len(v) <= 128 and all(opaque(n) for n in v) for v in lists):
                    quality = 'successful'
                    enabled, disabled = lists
    except Exception:
        pass  # Only allowlisted quality, never config values/errors or inferred fallback.
    api = None
    if prelaunch:
        plugins = importlib.import_module('hermes_cli.plugins')
        dispatch = importlib.import_module('hermes_cli.plugins_dispatch')
        for name, module in (('hermes_cli.plugins', plugins), ('hermes_cli.plugins_dispatch', dispatch)):
            path = origin(module)
            if path != root / (name.replace('.', '/') + '.py'):
                raise ValueError('origin')
            origins[name] = str(path)
        # Membership/callability only: no instance, registrar, manager getter,
        # catalog scan, callback, authentication or model operation.
        for name, module in tuple(sys.modules.items()):
            if name.startswith('hermes_cli.') or name in ('hermes_cli', 'utils', 'registration_lifecycle', 'hermes_yaml'):
                path = origin(module)
                stem = root / name.replace('.', '/')
                if path not in (stem.with_suffix('.py'), stem / '__init__.py'):
                    raise ValueError('mixed_origin')
        context, manager = plugins.PluginContext, plugins.PluginManager
        hooks = ['pre_llm_call', 'post_tool_call', 'on_session_start', 'on_session_reset']
        if (not isinstance(context, type) or not isinstance(manager, type)
                or context.__module__ != plugins.__name__ or manager.__module__ != plugins.__name__
                or getattr(context.register_hook, '__module__', None) != plugins.__name__
                or getattr(context.on_unload, '__module__', None) != plugins.__name__
                or not callable(getattr(context, 'register_hook', None))
                or not callable(getattr(context, 'on_unload', None))
                or not callable(getattr(manager, 'invoke_hook', None))
                or getattr(manager, 'invoke_hook') is not dispatch.PluginDispatchMixin.invoke_hook
                or type(plugins.VALID_HOOKS) not in (set, frozenset)
                or not all(hook in plugins.VALID_HOOKS for hook in hooks)):
            raise ValueError('api')
        api = {'register_hook': True, 'on_unload': True, 'invoke_hook': True, 'callbacks': hooks}
    result = empty(None, prelaunch)
    result.update(status='observed', runtime_descriptor=descriptor, profile=profile,
                  home=home, physical_home=physical, interpreter=str(interpreter), source_root=str(root),
                  module_origins=origins, dependency_paths=[selected], enabled=enabled, disabled=disabled,
                  config_quality=quality, evidence_stage='startup_profile_observation')
    if prelaunch:
        result.update(api=api, evidence_stage='prelaunch_api_profile_observation')
    return result


def main():
    sys.dont_write_bytecode = True
    try:
        if len(sys.argv) != 3 or sys.argv[1] != '--profile':
            raise ValueError('args')
        result = inspect(sys.argv[2])
        raw = json.dumps(result, separators=(',', ':'), allow_nan=False)
        if len(raw.encode()) > 16384:
            raise ValueError('output')
    except (Exception, SystemExit):
        prelaunch = False
        try:
            prelaunch = strict_json(os.environ.get('HERDR_HERMES_INSPECTION_SCOPE', ''))['mode'] == 'inspect_prelaunch'
        except (Exception, SystemExit):
            pass
        raw = json.dumps(empty('inspection_unavailable', prelaunch), separators=(',', ':'))
    print(raw)


if __name__ == '__main__':
    main()
