#!/usr/bin/env python3
"""Native TUI model-selected exec_command probe, using a LOCAL scripted SSE model.

No model account, external model request, real HOME/config, or Herdr call.
Scratch hooks bypass trust ONLY to inspect environment; this is not trust proof.
SSE source: openai/codex rust-v0.160.1 core/tests/common/responses.rs,
lines 699-727, 900-909, 981-989. Test fixtures emit the same function-call path.
"""
import argparse
import fcntl
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import pty
import select
import shlex
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import threading
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
    root = Path(tempfile.mkdtemp(prefix='ht-model-env-', dir='/private/tmp'))
    owned = []
    handles = []
    httpd = None
    thread = None
    results = {'binary': executable, 'scenarios': [], 'cleanup': False,
               'model': 'local deterministic Responses SSE fixture; no external model/account',
               'hook_trust': 'scratch --dangerously-bypass-hook-trust; environment evidence only',
               'source': 'https://github.com/openai/codex/blob/rust-v0.160.1/codex-rs/core/tests/common/responses.rs#L699-L727'}
    try:
        home, project = root / 'home', root / 'project'
        for path in (home, project, root / 'claude', root / 'xdg'):
            path.mkdir()
        helper = root / 'capture.py'
        helper.write_text('import json,os,pathlib,sys\n'
            'd={"pid":os.getpid(),"ppid":os.getppid(),'
            '"HERDR_PANE_ID":os.environ.get("HERDR_PANE_ID"),'
            '"HT_PROBE_MARKER":os.environ.get("HT_PROBE_MARKER"),'
            '"CODEX_THREAD_ID":os.environ.get("CODEX_THREAD_ID")}\n'
            'if sys.argv[1]=="hook":\n'
            ' x=json.load(sys.stdin);d.update({k:x.get(k) for k in '
            '["hook_event_name","session_id","tool_name"]});'
            'f=open(sys.argv[2],"a");f.write(json.dumps(d)+"\\n");f.close()\n'
            'else:\n pathlib.Path(sys.argv[2]).write_text(json.dumps(d));'
            'print("HT_ENV_CAPTURE",json.dumps(d))\n')
        hook_capture = root / 'hooks.jsonl'
        hook_cmd = '/usr/bin/python3 ' + shlex.quote(str(helper)) + ' hook ' + shlex.quote(str(hook_capture))
        home.joinpath('hooks.json').write_text(json.dumps({'hooks': {
            event: [{'hooks': [{'type': 'command', 'command': hook_cmd}]}]
            for event in ('SessionStart', 'PreToolUse')}}))
        state = {'capture': None, 'requests': [], 'count': 0, 'emitted_call': None}

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                raw = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                try:
                    body = json.loads(raw)
                except (ValueError, UnicodeDecodeError):
                    body = {}
                request = {'path': self.path, 'content_encoding': self.headers.get('Content-Encoding'),
                           'tool_names': [tool.get('name') for tool in body.get('tools', [])],
                           'tool_outputs': [item for item in body.get('input', [])
                                            if item.get('type') == 'function_call_output']}
                state['requests'].append(request)
                state['count'] += 1
                response_id = 'probe-response-' + str(state['count'])
                events = [{'type': 'response.created', 'response': {'id': response_id}}]
                if 'exec_command' in request['tool_names'] and state['emitted_call'] is None:
                    command = '/usr/bin/python3 ' + shlex.quote(str(helper)) + ' tool ' + shlex.quote(str(state['capture']))
                    arguments = {'cmd': command, 'login': False, 'max_output_tokens': 1000}
                    events.append({'type': 'response.output_item.done', 'item': {
                        'type': 'function_call', 'call_id': 'probe-exec-call',
                        'name': 'exec_command', 'arguments': json.dumps(arguments)}})
                    state['emitted_call'] = events[-1]['item']
                else:
                    events.append({'type': 'response.output_item.done', 'item': {
                        'type': 'message', 'role': 'assistant', 'id': 'probe-done',
                        'content': [{'type': 'output_text', 'text': 'Probe complete.'}]}})
                events.append({'type': 'response.completed', 'response': {'id': response_id,
                    'usage': {'input_tokens': 0, 'input_tokens_details': None, 'output_tokens': 0,
                              'output_tokens_details': None, 'total_tokens': 0}}})
                payload = ''.join('event: ' + ev['type'] + '\ndata: ' + json.dumps(ev) + '\n\n'
                                  for ev in events).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

        httpd = HTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=httpd.serve_forever)
        thread.start()
        home.joinpath('config.toml').write_text(
            'model = "canary"\nmodel_provider = "mock"\napproval_policy = "never"\n'
            'sandbox_mode = "danger-full-access"\nallow_login_shell = false\n'
            '[features]\nhooks = true\n'
            '[model_providers.mock]\nname = "mock"\nbase_url = "http://127.0.0.1:'
            + str(httpd.server_port) + '/v1"\nwire_api = "responses"\nrequires_openai_auth = false\n'
            '[projects.' + json.dumps(str(project)) + ']\ntrust_level = "trusted"\n')
        env = {'PATH': '/usr/bin:/bin:/opt/homebrew/bin', 'HOME': str(home),
               'CODEX_HOME': str(home), 'CLAUDE_CONFIG_DIR': str(root / 'claude'),
               'XDG_CONFIG_HOME': str(root / 'xdg'), 'SHELL': '/bin/bash',
               'TERM': 'xterm-256color', 'LANG': 'en_US.UTF-8',
               'HERDR_PANE_ID': 'probe-server:p1', 'HT_PROBE_MARKER': 'server'}
        results['version'] = subprocess.run([executable, '--version'], env=env,
            capture_output=True, text=True, timeout=10).stdout.strip()
        socket_path = root / 'server.sock'
        server_log = open(root / 'server.log', 'wb')
        handles.append(server_log)
        server_argv = [executable, 'app-server', '--listen', 'unix://' + str(socket_path)]
        server = subprocess.Popen(server_argv, env=env, cwd=project,
            stdin=subprocess.DEVNULL, stdout=server_log, stderr=server_log, start_new_session=True)
        owned.append(server)
        results['server'] = {'pid': server.pid, 'argv': server_argv,
                             'HERDR_PANE_ID': env['HERDR_PANE_ID']}
        deadline = time.monotonic() + 10
        while not socket_path.exists():
            if server.poll() is not None:
                raise RuntimeError('app-server exited: ' + (root / 'server.log').read_text()[-1500:])
            if time.monotonic() > deadline:
                raise RuntimeError('private socket did not appear')
            time.sleep(.05)
        for name, flags in (
            ('private-server', ['--remote', 'unix://' + str(socket_path)]),
            ('private-server-tool-policy', ['--remote', 'unix://' + str(socket_path),
                '-c', 'shell_environment_policy.set.HERDR_PANE_ID="probe-client:p2"']),
            ('embedded-control', ['--no-daemon']),
        ):
            client_env = dict(env, HERDR_PANE_ID='probe-client:p2', HT_PROBE_MARKER='client')
            capture = root / (name + '.json')
            state.update(capture=capture, requests=[], count=0, emitted_call=None)
            hook_offset = len(hook_capture.read_text().splitlines()) if hook_capture.exists() else 0
            argv = [executable, *flags, '--dangerously-bypass-hook-trust', '--no-alt-screen', '-C', str(project)]
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
            process = subprocess.Popen(argv, env=client_env, cwd=project,
                stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            owned.append(process)
            os.close(slave)
            buffer = bytearray()
            started = time.monotonic()
            submitted = False
            try:
                while time.monotonic() - started < 30:
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
                        os.write(master, b'Run the environment capture probe.')
                        time.sleep(.1)
                        os.write(master, b'\r')
                        submitted = True
                    if (capture.exists() and any(r['tool_outputs'] for r in state['requests'])) or process.poll() is not None:
                        break
                record = {'name': name, 'client_pid': process.pid, 'argv': argv,
                          'client_HERDR_PANE_ID': client_env['HERDR_PANE_ID'],
                          'path': 'TUI prompt -> local Responses SSE function_call -> native exec_command',
                          'submitted': submitted, 'requests': list(state['requests']),
                          'emitted_call': state.get('emitted_call'),
                          'hook_captures': [json.loads(line) for line in hook_capture.read_text().splitlines()[hook_offset:]]
                          if hook_capture.exists() else []}
                if capture.exists():
                    record['capture'] = json.loads(capture.read_text())
                else:
                    record['failure'] = 'no capture within bounded native TUI probe'
                    record['scratch_ui_tail'] = buffer.decode('utf-8', 'replace')[-4500:]
                results['scenarios'].append(record)
            finally:
                stop_owned(process)
                os.close(master)
        results['success'] = all('capture' in rec and any(r['tool_outputs'] for r in rec['requests'])
                                 for rec in results['scenarios'])
    except Exception as exc:
        results['error'] = str(exc)
    finally:
        for process in reversed(owned):
            stop_owned(process)
        if httpd:
            httpd.shutdown()
            httpd.server_close()
        if thread:
            thread.join(timeout=3)
        for handle in handles:
            handle.close()
        results['cleanup_groups'] = [{'leader': p.pid, 'returncode': p.poll(),
                                     'group_exists': group_exists(p.pid)} for p in owned]
        results['cleanup'] = all(p.poll() is not None and not group_exists(p.pid) for p in owned)
        results['cleanup'] = results['cleanup'] and (thread is None or not thread.is_alive())
        shutil.rmtree(root)
        results['scratch_removed'] = not root.exists()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(results, indent=2) + '\n')
        print(json.dumps(results, indent=2))
    return 0 if results.get('success') and results['cleanup'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
