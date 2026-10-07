#!/usr/bin/env python3
"""Bounded no-account native TUI !shell probe against an owned app-server.

Uses scratch HOME/CODEX_HOME, an explicit private socket, no shared Herdr calls,
and only sanitized environment marker/PID captures. No wrapper is installed.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import time


def group_exists(pid):
    try:
        os.killpg(pid, 0)
        return True
    except ProcessLookupError:
        return False


def stop_owned(process):
    # Leaders can exit before descendants. Always stop the owned process group.
    if group_exists(process.pid):
        os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        pass
    deadline = time.monotonic() + 3
    while group_exists(process.pid) and time.monotonic() < deadline:
        time.sleep(.05)
    if group_exists(process.pid):
        os.killpg(process.pid, signal.SIGKILL)
    process.wait(timeout=3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    executable = str(Path(args.codex).resolve(strict=True))
    output = Path(args.output).resolve()
    root = Path(tempfile.mkdtemp(prefix='ht-tui-env-', dir='/private/tmp'))
    owned = []
    handles = []
    results = {'binary': executable, 'scenarios': [], 'cleanup': False}
    try:
        home = root / 'home'
        project = root / 'project'
        home.mkdir()
        project.mkdir()
        for name in ('claude', 'xdg'):
            (root / name).mkdir()
        home.joinpath('config.toml').write_text(
            'model = "canary"\nmodel_provider = "mock"\napproval_policy = "never"\n'
            'sandbox_mode = "danger-full-access"\nallow_login_shell = false\n'
            '[model_providers.mock]\nname = "mock"\n'
            'base_url = "http://127.0.0.1:1/v1"\nwire_api = "responses"\n'
            'requires_openai_auth = false\n'
            '[projects.' + json.dumps(str(project)) + ']\ntrust_level = "trusted"\n'
        )
        env = {'PATH': '/usr/bin:/bin:/opt/homebrew/bin', 'HOME': str(home),
               'CODEX_HOME': str(home), 'CLAUDE_CONFIG_DIR': str(root / 'claude'),
               'XDG_CONFIG_HOME': str(root / 'xdg'), 'SHELL': '/bin/bash',
               'TERM': 'xterm-256color', 'LANG': 'en_US.UTF-8',
               'HERDR_PANE_ID': 'probe-server:p1', 'HT_PROBE_MARKER': 'server'}
        results['version'] = subprocess.run([executable, '--version'], env=env,
                                            capture_output=True, text=True,
                                            timeout=10).stdout.strip()
        helper = root / 'capture.py'
        helper.write_text('import json,os,pathlib,sys\n'
                          'd={"pid":os.getpid(),"ppid":os.getppid(),'
                          '"HERDR_PANE_ID":os.environ.get("HERDR_PANE_ID"),'
                          '"HT_PROBE_MARKER":os.environ.get("HT_PROBE_MARKER"),'
                          '"CODEX_THREAD_ID":os.environ.get("CODEX_THREAD_ID")}\n'
                          'pathlib.Path(sys.argv[1]).write_text(json.dumps(d))\n'
                          'print("HT_ENV_CAPTURE",json.dumps(d))\n')
        socket_path = root / 'server.sock'
        server_argv = [executable, 'app-server', '--listen', 'unix://' + str(socket_path)]
        server_log = open(root / 'server.log', 'wb')
        handles.append(server_log)
        server = subprocess.Popen(server_argv, env=env, cwd=project,
                                  stdin=subprocess.DEVNULL, stdout=server_log,
                                  stderr=server_log, start_new_session=True)
        owned.append(server)
        results['server'] = {'pid': server.pid, 'argv': server_argv,
                             'HERDR_PANE_ID': env['HERDR_PANE_ID']}
        deadline = time.monotonic() + 10
        while not socket_path.exists():
            if server.poll() is not None:
                raise RuntimeError('private app-server exited: ' + (root / 'server.log').read_text()[-1500:])
            if time.monotonic() > deadline:
                raise RuntimeError('private app-server did not create its socket')
            time.sleep(.05)
        for name, flags in (
            ('private-server', ['--remote', 'unix://' + str(socket_path)]),
            ('private-server-tool-policy', ['--remote', 'unix://' + str(socket_path),
                                           '-c', 'shell_environment_policy.set.HERDR_PANE_ID="probe-client:p2"']),
            ('embedded-control', ['--no-daemon']),
        ):
            client_env = dict(env, HERDR_PANE_ID='probe-client:p2', HT_PROBE_MARKER='client')
            capture = root / (name + '.json')
            command = '/usr/bin/python3 ' + shlex.quote(str(helper)) + ' ' + shlex.quote(str(capture))
            argv = [executable, *flags, '--no-alt-screen', '-C', str(project)]
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
            process = subprocess.Popen(argv, env=client_env, cwd=project,
                                       stdin=slave, stdout=slave, stderr=slave,
                                       start_new_session=True)
            owned.append(process)
            os.close(slave)
            buffer = bytearray()
            started = time.monotonic()
            submitted = False
            try:
                while time.monotonic() - started < 20:
                    ready, _, _ = select.select([master], [], [], .1)
                    if ready:
                        try:
                            chunk = os.read(master, 65536)
                        except OSError:
                            break
                        buffer.extend(chunk)
                        if b'\x1b[6n' in chunk:
                            os.write(master, b'\x1b[1;1R')
                        if b'\x1b[c' in chunk:
                            os.write(master, b'\x1b[?1;2c')
                    if not submitted and time.monotonic() - started > 3:
                        os.write(master, ('!' + command).encode())
                        time.sleep(.1)
                        os.write(master, b'\r')
                        submitted = True
                    if capture.exists() or process.poll() is not None:
                        break
                record = {'name': name, 'client_pid': process.pid, 'argv': argv,
                          'client_HERDR_PANE_ID': client_env['HERDR_PANE_ID'],
                          'path': 'TUI ! shortcut', 'submitted': submitted}
                if capture.exists():
                    record['capture'] = json.loads(capture.read_text())
                else:
                    record['failure'] = 'no capture within bounded TUI probe'
                    # Scratch-only UI text; contains no user configuration or auth.
                    record['scratch_ui_tail'] = buffer.decode('utf-8', 'replace')[-3500:]
                results['scenarios'].append(record)
            finally:
                stop_owned(process)
                os.close(master)
        results['success'] = all('capture' in result for result in results['scenarios'])
    finally:
        for process in reversed(owned):
            stop_owned(process)
        for handle in handles:
            handle.close()
        results['cleanup_groups'] = [{'leader': p.pid, 'returncode': p.poll(),
                                     'group_exists': group_exists(p.pid)} for p in owned]
        results['cleanup'] = all(p.poll() is not None and not group_exists(p.pid) for p in owned)
        shutil.rmtree(root)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(results, indent=2) + '\n')
        print(json.dumps(results, indent=2))
    return 0 if results.get('success') and results['cleanup'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
