#!/usr/bin/env python3
"""Offline installer daemon replacement against a racing old-executable ensure (ht-xe3v).

Stand-in executables model the daemon: `daemon ensure` starts a process whose
argv[0] is the ensuring executable and publishes its endpoint.json; `daemon
stop` stops it. A racing client models one that started before the swap: after
each of the next N stops it ensures the daemon from its own old executable,
the old package's binary (before the swap) or <prefix>.old's (after it: on
Linux a client's executable follows the rename). No Herdr, no host and
no real configuration is used.
"""
import hashlib
import io
import json
import os
import pathlib
import signal
import subprocess
import tarfile
import tempfile
import time
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]

STUB = r'''#!/bin/bash
version=__VERSION__
self=$0
state=${XDG_STATE_HOME}/herdr/plugins/herdr-threads
while :; do
    case "${1:-}" in
        --state-dir) state=$2; shift 2 ;;
        --host-endpoint) shift 2 ;;
        *) break ;;
    esac
done
node=$state/node
endpoint=$node/endpoint.json
live() {
    pid=$(sed -n 's/.*"pid":\([0-9]*\).*/\1/p' "$endpoint" 2>/dev/null)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}
# The racing client's executable: the old package, renamed or not.
old_client() {
    if [ -x "$FAKE_PREFIX.old/bin/herdr-threads" ]; then
        "$FAKE_PREFIX.old/bin/herdr-threads" "$@"
    else
        "$FAKE_PREFIX/bin/herdr-threads" "$@"
    fi
}
start() {
    mkdir -p "$node"
    (exec -a "$self daemon run --state-dir $state" sleep 300) < /dev/null > /dev/null 2>&1 &
    echo $! >> "$FAKE_DIR/pids"
    printf '{"software_version":"%s","pid":%s}\n' "$version" "$!" > "$endpoint"
}
case "$1" in
    --version) echo "herdr-threads $version" ;;
    internal)
        [ "${2:-}" = json-field ] || exit 2
        sed -n "s/.*\"$3\":\"\{0,1\}\([^\",}]*\).*/\1/p" | grep . ;;
    daemon)
        case "${2:-}" in
            stop)
                if live; then kill "$pid"; wait "$pid" 2>/dev/null; rm -f "$endpoint"; fi
                # A client of the old executable restarts the daemon after a stop.
                n=$(cat "$FAKE_DIR/race" 2>/dev/null || echo 0)
                if [ "$n" -gt 0 ]; then
                    echo $((n - 1)) > "$FAKE_DIR/race"
                    old_client --state-dir "$state" daemon ensure
                fi ;;
            ensure)
                # The old client wins the race to the next N ensures of this executable.
                n=$(cat "$FAKE_DIR/race-ensure-$version" 2>/dev/null || echo 0)
                if [ "$n" -gt 0 ]; then
                    echo $((n - 1)) > "$FAKE_DIR/race-ensure-$version"
                    old_client --state-dir "$state" daemon ensure
                fi
                if [ -e "$FAKE_DIR/ensure-fail-$version" ]; then echo "ensure refused" >&2; exit 1; fi
                if live; then
                    observed=$(sed -n 's/.*"software_version":"\([^"]*\)".*/\1/p' "$endpoint")
                    [ "$observed" = "$version" ] || { echo "daemon version mismatch: $observed" >&2; exit 1; }
                else
                    start
                fi ;;
            *) exit 2 ;;
        esac ;;
    *) exit 2 ;;
esac
'''


class InstallerDaemonRace(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ht-installer-daemon-')
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name).resolve()
        self.addCleanup(self.stop_daemons)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.prefix = self.root / 'package'
        self.state = self.root / 'state'
        self.endpoint = self.state / 'herdr/plugins/herdr-threads/node/endpoint.json'
        self.release = self.root / 'release'
        system = os.uname()
        platform = 'macos' if system.sysname == 'Darwin' else 'linux'
        arch = 'aarch64' if system.machine in ('arm64', 'aarch64') else 'x86_64'
        self.asset = f'herdr-threads-{platform}-{arch}.tar.gz'
        for version in ['0.1.0', '0.1.1']:
            self.package(version)

    def stop_daemons(self):
        pids = self.root / 'pids'
        if pids.exists():
            for pid in pids.read_text().split():
                try:
                    os.kill(int(pid), signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def package(self, version):
        folder = self.release / 'download' / f'v{version}'
        folder.mkdir(parents=True)
        archive = folder / self.asset
        files = {'herdr-plugin.toml': b'id="herdr-threads"\n',
                 'bin/herdr-threads': STUB.replace('__VERSION__', version).encode(),
                 'scripts/view.sh': b'#!/bin/sh\n', 'scripts/build.sh': b'#!/bin/sh\n',
                 'VERSION': version.encode(), 'PREBUILT': b'1'}
        with tarfile.open(archive, 'w:gz') as tar:
            for name, data in files.items():
                entry = tarfile.TarInfo(f'herdr-threads/{name}')
                entry.size = len(data)
                entry.mode = 0o755 if name.endswith('.sh') or name == 'bin/herdr-threads' else 0o644
                tar.addfile(entry, io.BytesIO(data))
        (folder / 'SHA256SUMS').write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {self.asset}\n')

    def run_installer(self, version):
        env = {'HOME': str(self.root / 'home'), 'CLAUDE_CONFIG_DIR': str(self.root / 'claude'),
               'CODEX_HOME': str(self.root / 'codex'), 'HERDR_BIN': str(self.root / 'no-host'),
               'PATH': f'{self.bin}:/usr/bin:/bin:/usr/sbin:/sbin', 'TMPDIR': str(self.root),
               'XDG_STATE_HOME': str(self.state), 'XDG_CONFIG_HOME': str(self.root / 'config'),
               'FAKE_DIR': str(self.root), 'FAKE_PREFIX': str(self.prefix)}
        if 'HT_LEAK_RUN_ID' in os.environ:
            env['HT_LEAK_RUN_ID'] = os.environ['HT_LEAK_RUN_ID']
        out = subprocess.run(['/bin/bash', str(REPO / 'scripts/install.sh'),
                              '--prefix', str(self.prefix), '--bin-dir', str(self.bin),
                              '--release-url', self.release.as_uri(), '--version', f'v{version}',
                              '--no-herdr', '--no-setup'], cwd=self.root, env=env,
                             stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
        # --no-herdr deliberately leaves registration incomplete.
        self.assertEqual(out.returncode, 3, out.stdout + out.stderr)
        return out.stdout + out.stderr

    def running(self):
        endpoint = json.loads(self.endpoint.read_text())
        command = subprocess.check_output(['ps', '-ww', '-p', str(endpoint['pid']), '-o', 'command='], text=True)
        return endpoint['software_version'], command.split(' daemon run ')[0]

    def upgrade_with_racing_client(self, restarts, ensure_races=0):
        self.run_installer('0.1.0')
        subprocess.run([str(self.prefix / 'bin/herdr-threads'), 'daemon', 'ensure'], check=True,
                       env={**os.environ, 'XDG_STATE_HOME': str(self.state), 'FAKE_DIR': str(self.root)})
        self.assertEqual(self.running(), ('0.1.0', str(self.prefix / 'bin/herdr-threads')))
        (self.root / 'race').write_text(str(restarts))
        (self.root / 'race-ensure-0.1.1').write_text(str(ensure_races))
        return self.run_installer('0.1.1')

    def test_old_daemon_restarted_after_the_swap_is_replaced(self):
        # Restarted after both stops before and both after the swap, and once
        # more: the installer still ends on the installed executable.
        out = self.upgrade_with_racing_client(5)
        self.assertEqual(self.running(), ('0.1.1', str(self.prefix / 'bin/herdr-threads')), out)
        self.assertIn('runs 0.1.0; installed executable is 0.1.1', out)
        self.assertIn('replaced the stale daemon; the installed executable runs', out)
        self.assertNotIn('running daemon does not match', out)
        self.assertNotIn('old daemon still running', out)
        self.assertEqual((self.root / 'race').read_text().strip(), '0')
        self.assertFalse((self.root / 'package.old').exists())

    def test_old_daemon_started_between_stop_and_ensure_is_replaced(self):
        # The old client starts the daemon right before each of the installer's
        # first two ensures: the ensure meets a version mismatch, and the next
        # check replaces it again.
        out = self.upgrade_with_racing_client(4, ensure_races=2)
        self.assertEqual(self.running(), ('0.1.1', str(self.prefix / 'bin/herdr-threads')), out)
        self.assertIn('daemon version mismatch', out)
        self.assertIn('replaced the stale daemon; the installed executable runs', out)
        self.assertNotIn('running daemon does not match', out)
        self.assertEqual((self.root / 'race-ensure-0.1.1').read_text().strip(), '0')

    def test_failed_reensure_after_a_replacement_is_not_reported_as_success(self):
        (self.root / 'ensure-fail-0.1.1').write_text('')
        out = self.upgrade_with_racing_client(5)
        self.assertFalse(self.endpoint.exists(), out)
        self.assertIn('installed daemon verification failed', out)
        self.assertNotIn('replaced the stale daemon', out)
        self.assertNotIn('Check it: herdr-threads doctor', out)

    def test_a_daemon_from_another_installation_is_left_alone(self):
        # A dev checkout's newer daemon: reported, never stopped or downgraded.
        self.run_installer('0.1.1')
        dev = self.root / 'dev/bin'
        dev.mkdir(parents=True)
        (dev / 'herdr-threads').write_text(STUB.replace('__VERSION__', '0.2.0-dev'))
        (dev / 'herdr-threads').chmod(0o755)
        subprocess.run([str(dev / 'herdr-threads'), 'daemon', 'ensure'], check=True,
                       env={**os.environ, 'XDG_STATE_HOME': str(self.state), 'FAKE_DIR': str(self.root)})
        before = self.running()
        out = self.run_installer('0.1.1')
        self.assertEqual(self.running(), before, out)
        self.assertEqual(before, ('0.2.0-dev', str(dev / 'herdr-threads')))
        self.assertIn('runs 0.2.0-dev; installed executable is 0.1.1', out)
        self.assertIn(f'runs from {dev}/herdr-threads, not from this installation; left alone', out)
        self.assertNotIn('replaced the stale daemon', out)

    def test_replacement_is_bounded(self):
        start = time.monotonic()
        out = self.upgrade_with_racing_client(100)
        self.assertLess(time.monotonic() - start, 30)
        self.assertIn('runs 0.1.0; installed executable is 0.1.1', out)
        self.assertNotIn('replaced the stale daemon', out)
        self.assertIn('could not stop the stale daemon', out)
        self.assertIn('could not replace the stale daemon (pid ', out)
        self.assertGreater(int((self.root / 'race').read_text()), 50)
        self.assertFalse((self.root / 'package.old').exists())


if __name__ == '__main__':
    unittest.main()
