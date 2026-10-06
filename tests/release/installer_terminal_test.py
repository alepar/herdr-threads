#!/usr/bin/env python3
"""Real controlling-terminal installer consent, with private config roots."""
import fcntl
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import termios
import time

binary = pathlib.Path(sys.argv[1]).resolve()


def run(root, answers, redirected=False, no_color=False):
    master, slave = pty.openpty()
    env = {"HOME": str(root / 'home'), "CLAUDE_CONFIG_DIR": str(root / 'claude'),
           "CODEX_HOME": str(root / 'codex'), "PATH": str(root / 'bin'), "TERM": 'xterm'}
    if no_color:
        env['NO_COLOR'] = ''
    if 'HT_LEAK_RUN_ID' in os.environ:
        env['HT_LEAK_RUN_ID'] = os.environ['HT_LEAK_RUN_ID']

    def terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    child = subprocess.Popen([str(binary), '--state-dir', str(root / 'state'),
                              '--host-endpoint', str(root / 'herdr.sock'),
                              'internal', 'installer-integrations'], cwd=root, env=env,
                             stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE if redirected else slave,
                             stderr=slave, preexec_fn=terminal)
    os.close(slave)
    data = b''
    supplied = 0
    deadline = time.monotonic() + 10
    try:
        while child.poll() is None:
            if time.monotonic() > deadline:
                raise AssertionError(f'installer did not settle: {data!r}')
            if select.select([master], [], [], 0.05)[0]:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                data += chunk
                while data.count(b'[y/N] ') > supplied:
                    if supplied >= len(answers):
                        raise AssertionError(f'unexpected confirmation: {data!r}')
                    os.write(master, answers[supplied].encode() + b'\n')
                    supplied += 1
        child.wait(timeout=2)
        if redirected:
            data += child.stdout.read()
            child.stdout.close()
        else:
            while select.select([master], [], [], 0)[0]:
                try:
                    chunk = os.read(master, 65536)
                    if not chunk:
                        break
                    data += chunk
                except OSError:
                    break
        assert child.returncode == 0, data
        assert supplied == len(answers), data
        return data
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)


with tempfile.TemporaryDirectory(prefix='ht-installer-tty-') as folder:
    root = pathlib.Path(folder)
    (root / 'bin').mkdir()
    for name, version in [('claude', '2.1.284 (Claude Code)'), ('codex', 'codex-cli 0.158.0')]:
        harness = root / 'bin' / name
        harness.write_text(f"#!/bin/sh\nprintf '%s\\n' '{version}'\n")
        harness.chmod(0o755)
    # stdin is /dev/null; consent must use /dev/tty. Decisions are per component.
    data = run(root, ['n', 'y', 'n', 'y'], no_color=True)
    assert data.count(b'[y/N] ') == 4, data
    assert b'\x1b[' not in data, data
    assert not (root / 'claude/settings.json').exists()
    assert not (root / 'codex/hooks.json').exists()
    for name in ['claude', 'codex']:
        assert (root / name / 'skills/herdr-threads/SKILL.md').is_file()
    # Existing owned skills do not ask, while missing hooks still ask.
    data = run(root, ['n', 'n'])
    assert data.count(b'[y/N] ') == 2, data
    assert b'\x1b[1;32m' in data, data
    assert b'claude skill: updated' in data, data
    # A controlling tty with redirected stdout never asks.
    data = run(root, [], redirected=True)
    assert b'[y/N]' not in data and b'\x1b[' not in data, data
    assert b'claude hooks: skipped' in data, data
print('INSTALLER_TERMINAL_PASS')
