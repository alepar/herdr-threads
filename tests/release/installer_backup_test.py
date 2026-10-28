#!/usr/bin/env python3
"""Offline installer pre-upgrade backup: pruning end to end and helper selection.

usage: tests/release/installer_backup_test.py BINARY

Stand-in packages model each release's executable; their
`internal pre-upgrade-backup` runs BINARY (a real herdr-threads build) when the
release is marked as having the helper, and exits 2 otherwise, as a build that
predates it does. The store and every backup are real SQLite files. No daemon,
no Herdr and no real configuration is used.
"""
import hashlib
import io
import os
import pathlib
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
BINARY = pathlib.Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else None
KEY = 'SECRET-NODE-KEY-MARKER-0123456789'

STUB = r'''#!/bin/bash
case "$1" in
    --version) [ __RUNS__ = 1 ] || exit 126; echo "herdr-threads __VERSION__" ;;
    internal)
        [ "${2:-}" = pre-upgrade-backup ] || exit 2
        case __HELPER__ in
            1) exec "$REAL_BIN" "$@" ;;
            # A helper that exists but fails (an older SQLite on a newer store).
            fail) [ "${3:-}" = --help ] && exit 0; echo "herdr-threads: error: pre-upgrade backup failed: stand-in" >&2; exit 1 ;;
            *) exit 2 ;;
        esac ;;
    daemon) exit 0 ;;
    *) exit 2 ;;
esac
'''

# (version, helper: True / False / 'fail', runs)
RELEASES = [('0.1.0', True, True), ('0.1.1', True, True), ('0.1.2', False, True),
            ('0.1.3', False, True), ('0.1.4', True, False), ('0.1.5', 'fail', True)]


def user_version(path, version):
    db = sqlite3.connect(path)
    db.execute(f'PRAGMA user_version={version}')
    db.commit()
    db.close()


@unittest.skipIf(BINARY is None, 'needs the herdr-threads binary as the first argument')
class InstallerBackup(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ht-installer-backup-')
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name).resolve()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.prefix = self.root / 'package'
        self.plugins = self.root / 'state/herdr/plugins'
        self.state = self.plugins / 'herdr-threads'
        self.backups = self.plugins / 'herdr-threads-backups'
        self.release = self.root / 'release'
        system = os.uname()
        platform = 'macos' if system.sysname == 'Darwin' else 'linux'
        arch = 'aarch64' if system.machine in ('arm64', 'aarch64') else 'x86_64'
        self.asset = f'herdr-threads-{platform}-{arch}.tar.gz'
        for release in RELEASES:
            self.package(*release)

    def package(self, version, helper, runs):
        folder = self.release / 'download' / f'v{version}'
        folder.mkdir(parents=True)
        archive = folder / self.asset
        mode = 'fail' if helper == 'fail' else ('1' if helper else '0')
        stub = (STUB.replace('__VERSION__', version).replace('__HELPER__', mode)
                .replace('__RUNS__', '1' if runs else '0'))
        files = {'herdr-plugin.toml': b'id="herdr-threads"\n', 'bin/herdr-threads': stub.encode(),
                 'scripts/view.sh': b'#!/bin/sh\n', 'scripts/build.sh': b'#!/bin/sh\n',
                 'VERSION': version.encode(), 'PREBUILT': b'1'}
        with tarfile.open(archive, 'w:gz') as tar:
            for name, data in files.items():
                entry = tarfile.TarInfo(f'herdr-threads/{name}')
                entry.size = len(data)
                entry.mode = 0o755 if name.endswith('.sh') or name == 'bin/herdr-threads' else 0o644
                tar.addfile(entry, io.BytesIO(data))
        (folder / 'SHA256SUMS').write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {self.asset}\n')

    def install(self, version, *extra, expect=3):
        env = {'HOME': str(self.root / 'home'), 'CLAUDE_CONFIG_DIR': str(self.root / 'claude'),
               'CODEX_HOME': str(self.root / 'codex'), 'HERDR_BIN': str(self.root / 'no-host'),
               'PATH': f'{self.bin}:/usr/bin:/bin:/usr/sbin:/sbin', 'TMPDIR': str(self.root),
               'XDG_STATE_HOME': str(self.root / 'state'), 'REAL_BIN': str(BINARY)}
        if 'HT_LEAK_RUN_ID' in os.environ:
            env['HT_LEAK_RUN_ID'] = os.environ['HT_LEAK_RUN_ID']
        out = subprocess.run(['/bin/bash', str(REPO / 'scripts/install.sh'),
                              '--prefix', str(self.prefix), '--bin-dir', str(self.bin),
                              '--release-url', self.release.as_uri(), '--version', f'v{version}',
                              '--no-herdr', '--no-setup', *extra], cwd=self.root, env=env,
                             stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
        output = out.stdout + out.stderr
        # --no-herdr deliberately leaves registration incomplete (exit 3).
        self.assertEqual(out.returncode, expect, output)
        self.assertNotIn(KEY, output)
        return output

    def store(self, schema):
        node = self.state / 'node'
        node.mkdir(parents=True, exist_ok=True)
        db = sqlite3.connect(node / 'herdr-threads.db')
        db.executescript('PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS t(x); INSERT INTO t VALUES (1);')
        db.execute(f'PRAGMA user_version={schema}')
        db.commit()
        db.close()
        (node / 'node.key').write_text(KEY)
        (node / 'node.json').write_text('{}')

    def entry(self, name, schema):
        path = self.backups / name
        path.mkdir(parents=True)
        if schema is not None:
            user_version(path / 'herdr-threads.db', schema)
        return path

    def names(self):
        return sorted(p.name for p in self.backups.iterdir())

    def test_upgrade_prunes_only_its_own_backups(self):
        self.install('0.1.0')
        self.assertFalse(self.backups.exists(), 'a fresh install has no store to back up')
        self.store(9)
        self.backups.mkdir(mode=0o700)
        self.entry('pre-v2.0.1-20261001T000000Z', 1)        # newest schema 1: kept
        self.entry('pre-v2.1.0-20261002T000000Z', 2)        # pruned
        self.entry('pre-v2.1.1-20261003T000000Z', 2)        # newest schema 2: kept
        self.entry('pre-v2.2.0-20261004T000000Z', None)     # unreadable: kept, warned
        self.entry('pre-v2.4.1-20261005T000000Z', 5)        # pruned
        self.entry('pre-v2.4.2-20261006T000000Z', 5)        # third newest: kept
        self.entry('pre-v2.4.3-20261007T000000Z', 9)        # second newest: kept
        self.entry('pre-v2.0.0_20261001T000000Z', 1)        # off the grammar
        target = self.entry('keep-target', 3)
        (self.backups / 'pre-v1.0.0-20250101T000000Z').symlink_to(target)
        # The manual directories of earlier releases' procedure, in all three
        # name formats: never read, counted or removed.
        manual = []
        for name in ['herdr-threads-pre-v2.0.1-backup-20261017T160720Z',
                     'herdr-threads-pre-v2.1.0-backup-20261018T012914',
                     'herdr-threads-pre-v2.2.1-backup-20261018-180456']:
            path = self.plugins / name
            path.mkdir()
            user_version(path / 'herdr-threads.db', 1)
            manual.append((path, (path / 'herdr-threads.db').read_bytes()))

        out = self.install('0.1.1')
        taken = [n for n in self.names() if n.startswith('pre-v0.1.1-')]
        self.assertEqual(len(taken), 1, out)
        self.assertIn(f'(schema 9, integrity ok)', out)
        self.assertIn('pruned old backup', out)
        self.assertIn('pre-v2.2.0-20261004T000000Z: cannot read its store schema', out)
        self.assertEqual(self.names(), sorted([
            taken[0], 'keep-target', 'pre-v1.0.0-20250101T000000Z', 'pre-v2.0.0_20261001T000000Z',
            'pre-v2.0.1-20261001T000000Z', 'pre-v2.1.1-20261003T000000Z',
            'pre-v2.2.0-20261004T000000Z', 'pre-v2.4.2-20261006T000000Z',
            'pre-v2.4.3-20261007T000000Z']))
        self.assertTrue((target / 'herdr-threads.db').is_file())
        for path, data in manual:
            self.assertEqual((path / 'herdr-threads.db').read_bytes(), data)
        copy = sqlite3.connect(f'file:{self.backups / taken[0] / "herdr-threads.db"}?mode=ro', uri=True)
        self.assertEqual(copy.execute('PRAGMA integrity_check').fetchone()[0], 'ok')
        copy.close()

    def test_repeated_upgrades_keep_the_newest_three(self):
        self.install('0.1.0')
        self.store(9)
        for version in ['0.1.1', '0.1.0', '0.1.1', '0.1.0', '0.1.1']:
            out = self.install(version)
        last = out.split(f' to {self.backups}/', 1)[1].split(' ', 1)[0]
        names = self.names()
        self.assertEqual(len(names), 3, names)
        self.assertIn(last, names, 'the backup just taken survives')

    def test_a_release_without_the_helper_uses_the_installed_one(self):
        self.install('0.1.1')
        self.store(9)
        out = self.install('0.1.2')
        self.assertIn('integrity ok', out)
        self.assertEqual([n[:11] for n in self.names()], ['pre-v0.1.2-'])

    def test_no_helper_anywhere_refuses_until_no_backup(self):
        self.install('0.1.2')
        self.store(9)
        out = self.install('0.1.3', expect=1)
        self.assertIn('neither 0.1.3 nor the installed executable can take it', out)
        self.assertEqual((self.prefix / 'VERSION').read_text(), '0.1.2')
        out = self.install('0.1.3', '--no-backup')
        self.assertIn('--no-backup: no pre-upgrade backup of', out)
        self.assertEqual((self.prefix / 'VERSION').read_text(), '0.1.3')
        self.assertFalse(self.backups.exists())

    def test_a_failing_package_helper_falls_back_to_the_installed_one(self):
        self.install('0.1.1')
        self.store(9)
        out = self.install('0.1.5')
        self.assertIn('pre-upgrade backup failed: stand-in', out)
        self.assertIn('integrity ok', out)
        self.assertEqual([n[:11] for n in self.names()], ['pre-v0.1.5-'])

    def test_a_symlinked_store_fails_closed(self):
        self.install('0.1.0')
        self.store(9)
        node = self.state / 'node'
        real = self.root / 'elsewhere.db'
        (node / 'herdr-threads.db').rename(real)
        for sidecar in ('-wal', '-shm'):
            (node / f'herdr-threads.db{sidecar}').unlink(missing_ok=True)
        (node / 'herdr-threads.db').symlink_to(real)
        out = self.install('0.1.1', expect=1)
        self.assertIn('is not a regular file', out)
        self.assertIn('nothing was stopped or replaced', out)
        self.assertEqual((self.prefix / 'VERSION').read_text(), '0.1.0')
        out = self.install('0.1.1', '--no-backup')
        self.assertIn('--no-backup: no pre-upgrade backup of', out)
        self.assertEqual((self.prefix / 'VERSION').read_text(), '0.1.1')

    def test_a_download_that_cannot_run_is_named(self):
        self.install('0.1.1')
        self.store(9)
        out = self.install('0.1.4', expect=1)
        self.assertIn('the downloaded 0.1.4 executable does not run here', out)
        self.assertEqual((self.prefix / 'VERSION').read_text(), '0.1.1')


if __name__ == '__main__':
    unittest.main()
