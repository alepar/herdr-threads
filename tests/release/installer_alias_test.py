#!/usr/bin/env python3
"""Offline actual-installer alias lifecycle; no host or real configuration access."""
import io
import os
import pathlib
import subprocess
import tarfile
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]


class InstallerAlias(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ht-installer-alias-')
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.prefix = self.root / 'package'
        self.binary = self.prefix / 'bin/herdr-threads'
        self.alias = self.bin / 'ht'
        self.release = self.root / 'release'
        system = os.uname()
        platform = 'macos' if system.sysname == 'Darwin' else 'linux'
        arch = 'aarch64' if system.machine in ('arm64', 'aarch64') else 'x86_64'
        self.asset = f'herdr-threads-{platform}-{arch}.tar.gz'
        for version in ['0.1.0', '0.1.1']:
            self.package(version)

    def package(self, version):
        import hashlib
        folder = self.release / 'download' / f'v{version}'
        folder.mkdir(parents=True)
        archive = folder / self.asset
        files = {'herdr-plugin.toml': b'id="herdr-threads"\n',
                 'bin/herdr-threads': f'#!/bin/sh\ncase "$1" in --version) echo "herdr-threads {version}";; daemon) exit 0;; *) exit 2;; esac\n'.encode(),
                 'scripts/view.sh': b'#!/bin/sh\n', 'scripts/build.sh': b'#!/bin/sh\n',
                 'VERSION': version.encode(), 'PREBUILT': b'1'}
        with tarfile.open(archive, 'w:gz') as tar:
            for name, data in files.items():
                entry = tarfile.TarInfo(f'herdr-threads/{name}')
                entry.size = len(data)
                entry.mode = 0o755 if name.endswith('.sh') or name == 'bin/herdr-threads' else 0o644
                tar.addfile(entry, io.BytesIO(data))
        (folder / 'SHA256SUMS').write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {self.asset}\n')

    def run_installer(self, *args, path=None):
        env = {'HOME': str(self.root / 'home'), 'CLAUDE_CONFIG_DIR': str(self.root / 'claude'),
               'CODEX_HOME': str(self.root / 'codex'), 'HERDR_BIN': str(self.root / 'no-host'),
               'PATH': path or f'{self.bin}:/usr/bin:/bin:/usr/sbin:/sbin', 'TMPDIR': str(self.root),
               'XDG_STATE_HOME': str(self.root / 'state'), 'XDG_CONFIG_HOME': str(self.root / 'config')}
        if 'HT_LEAK_RUN_ID' in os.environ:
            env['HT_LEAK_RUN_ID'] = os.environ['HT_LEAK_RUN_ID']
        out = subprocess.run(['/bin/bash', str(REPO / 'scripts/install.sh'),
                              '--prefix', str(self.prefix), '--bin-dir', str(self.bin),
                              '--release-url', self.release.as_uri(), '--version', 'v0.1.0',
                              '--no-herdr', '--no-setup', *args], cwd=self.root, env=env,
                             stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10)
        # --no-herdr deliberately leaves registration incomplete.
        self.assertEqual(out.returncode, 3, out.stdout + out.stderr)
        return out.stdout + out.stderr

    def test_default_alias_reinstall_upgrade_and_owned_uninstall(self):
        out = self.run_installer()
        self.assertTrue(self.alias.is_symlink(), out)
        self.assertEqual(self.alias.readlink(), self.binary)
        self.assertIn('ht is available on PATH', out)
        self.run_installer()
        self.run_installer('--version', 'v0.1.1')
        self.assertEqual(subprocess.check_output([str(self.alias), '--version'], text=True).strip(),
                         'herdr-threads 0.1.1')
        self.run_installer('--uninstall', '--yes')
        self.assertFalse(self.alias.is_symlink())

    def test_foreign_entries_are_preserved_even_with_force(self):
        for kind in ['file', 'directory', 'symlink', 'broken-symlink']:
            with self.subTest(kind=kind):
                foreign = self.root / 'foreign'
                foreign.write_text('unrelated executable')
                if kind == 'file':
                    self.alias.write_text('unrelated executable')
                elif kind == 'directory':
                    self.alias.mkdir()
                else:
                    self.alias.symlink_to(foreign if kind == 'symlink' else self.root / 'missing')
                out = self.run_installer('--force')
                if kind == 'directory':
                    self.assertTrue(self.alias.is_dir())
                    self.alias.rmdir()
                elif kind.endswith('symlink'):
                    self.assertNotEqual(self.alias.readlink(), self.binary)
                    self.alias.unlink()
                else:
                    self.assertEqual(self.alias.read_text(), 'unrelated executable')
                    self.alias.unlink()
                self.assertIn('preserved', out)
                self.assertIn(str(self.alias), out)
                self.assertIn(str(self.binary), out)

    def test_foreign_replacement_survives_upgrade_and_uninstall(self):
        self.run_installer()
        self.assertTrue(self.alias.is_symlink())
        self.alias.unlink()
        self.alias.write_text('foreign replacement')
        self.run_installer('--version', 'v0.1.1', '--force')
        self.run_installer('--uninstall', '--yes', '--force')
        self.assertEqual(self.alias.read_text(), 'foreign replacement')

    def test_path_shadowing_names_selected_executable_and_absolute_alias(self):
        tex = self.root / 'texbin'
        tex.mkdir()
        other = tex / 'ht'
        other.write_text('#!/bin/sh\necho TeX\n')
        other.chmod(0o755)
        out = self.run_installer(path=f'{tex}:{self.bin}:/usr/bin:/bin:/usr/sbin:/sbin')
        self.assertTrue(self.alias.is_symlink(), out)
        self.assertIn(f'PATH selects {other}', out)
        self.assertIn(f'{self.alias} skill', out)
        self.assertNotIn('ht is available on PATH', out)

    def test_missing_bin_directory_on_path_gives_absolute_invocation(self):
        out = self.run_installer(path='/usr/bin:/bin:/usr/sbin:/sbin')
        self.assertTrue(self.alias.is_symlink(), out)
        self.assertIn(f'{self.alias} skill', out)
        self.assertNotIn('ht is available on PATH', out)


if __name__ == '__main__':
    unittest.main()
