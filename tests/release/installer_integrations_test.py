#!/usr/bin/env python3
"""Focused installer shell dispatch without downloads, Herdr, or real config."""
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
SOURCE = (REPO / 'scripts/install.sh').read_text()
# Execute the actual setup lane against a controlled older/newer binary boundary.
LANE = SOURCE.split('# --- harness setup ')[1].split('# --- next steps ')[0]
LANE = LANE[LANE.index('\n'):]
CAPABILITY = SOURCE[SOURCE.index('installer_integrations() {'):SOURCE.index('\nbare_setup() {')]
DETECTION = SOURCE[SOURCE.index('detected_harnesses() {'):SOURCE.index('\n# Asks a yes/no')]
BASH = shutil.which('bash')
TR = shutil.which('tr')
FOOTER = SOURCE.split('# --- next steps and final status ')[1]
FOOTER = FOOTER[FOOTER.index('\n'):].rsplit('\n}', 1)[0]


class InstallerDispatch(unittest.TestCase):
    def run_actual_lane(self, mode='ask', modern=True, fail=False, registered=1,
                        harnesses=(), user_level=True, bare=True, terminal=False,
                        consent=False, final_status=False, without_permissions=False,
                        help_text=''):
        with tempfile.TemporaryDirectory(prefix='ht-installer-actual-shell-') as folder:
            root = pathlib.Path(folder)
            path = root / 'bin'
            path.mkdir()
            (path / 'tr').symlink_to(TR)
            marker = root / 'native-invoked'
            for name in (*harnesses, 'registered-only'):
                sentinel = path / name
                sentinel.write_text(f'#!/bin/sh\nprintf x > "{marker}"\nexit 99\n')
                sentinel.chmod(0o755)
            binary = root / 'binary'
            log = root / 'calls'
            binary.write_text('''#!/bin/bash
printf '%s\\n' "$*" >> "$CALL_LOG"
if [ "$1 $2 $3" = 'internal installer-integrations --help' ]; then
  printf '%s\\n' "$HELP_TEXT"
  exit "$CAPABILITY_STATUS"
fi
if [ "$1 $2" = 'internal installer-integrations' ]; then
  exit "$INTEGRATION_STATUS"
fi
exit 0
''')
            binary.chmod(0o755)
            script = '''set -euo pipefail
say() { echo "$*"; }
warn() { echo "$*" >&2; }
user_level_setup() { [ "$USER_LEVEL" = 1 ]; }
bare_setup() { [ "$BARE_SETUP" = 1 ]; }
can_prompt() { [ "$TERMINAL" = 1 ]; }
confirm() { [ "$CONSENT" = 1 ]; }
OUT_SETUP=none
OUT_SETUP_FAILED=''
OUT_CODEX_SET_UP=0
''' + DETECTION + CAPABILITY + LANE + '''
printf 'outcome=%s\\nfailed_harnesses=%s\\n' "$OUT_SETUP" "$OUT_SETUP_FAILED"
'''
            if final_status:
                script += '''
color_enabled() { return 1; }
old_daemon_alive() { return 0; }
status_line() { echo "$*"; }
OUT_UPGRADED=0
OUT_LINKED=1
OUT_DAEMON_UP=0
on_path=1
# This extracted fixture omits installation; its private bin has no ht alias.
alias_owned=0
alias_on_path=0
selected_alias=''
REPO=fixture/repo
os=macos
new_version=fixture
''' + FOOTER
            env = dict(os.environ, PATH=str(path), HOME=str(root / 'home'),
                       CLAUDE_CONFIG_DIR=str(root / 'claude'), CODEX_HOME=str(root / 'codex'),
                       XDG_CONFIG_HOME=str(root / 'config'), XDG_STATE_HOME=str(root / 'state'),
                       installed_binary=str(binary), bin_dir=str(path), alias_path=str(path / 'ht'),
                       setup=mode, registered=str(registered),
                       CALL_LOG=str(log), CAPABILITY_STATUS='0' if modern else '2',
                       INTEGRATION_STATUS='1' if fail else '0', USER_LEVEL=str(int(user_level)),
                       BARE_SETUP=str(int(bare)), TERMINAL=str(int(terminal)),
                       CONSENT=str(int(consent)), HELP_TEXT=help_text,
                       without_permissions=str(int(without_permissions)))
            out = subprocess.run([BASH, '-c', script], env=env, cwd=root,
                                 capture_output=True, text=True, timeout=5)
            self.assertEqual(out.returncode, 3 if final_status and fail else 0, out.stderr)
            self.assertFalse(marker.exists(), 'PATH discovery must not execute a provider')
            return out.stdout + out.stderr, log.read_text().splitlines() if log.exists() else []

    # Kills dropping --without-permissions, or passing it to a build that cannot parse it.
    def test_without_permissions_reaches_only_a_build_that_knows_it(self):
        for help_text, wanted in [('  --without-permissions  Skip the permissions component',
                                   'internal installer-integrations --without-permissions'),
                                  ('', 'internal installer-integrations')]:
            with self.subTest(help_text=help_text):
                _, calls = self.run_actual_lane(without_permissions=True, help_text=help_text)
                self.assertEqual(calls[-1], wanted)
        _, calls = self.run_actual_lane(mode='yes', without_permissions=True,
                                        help_text='--without-permissions')
        self.assertEqual(calls[-1],
                         'internal installer-integrations --confirm-missing --without-permissions')

    # Kills the legacy membership gate suppressing a registry-only modern installer.
    def test_modern_registry_route_without_legacy_path(self):
        for mode, operation in [('ask', 'internal installer-integrations'),
                                ('yes', 'internal installer-integrations --confirm-missing')]:
            with self.subTest(mode=mode):
                output, calls = self.run_actual_lane(mode=mode)
                self.assertEqual(calls, ['internal installer-integrations --help', operation])
                self.assertIn('outcome=complete', output)
                self.assertNotIn('none_found', output)
                self.assertNotIn('no supported harness (claude, codex)', output)

    def test_modern_empty_and_hermes_only_membership_are_delegated(self):
        for harnesses in [(), ('hermes',)]:
            with self.subTest(harnesses=harnesses):
                output, calls = self.run_actual_lane(harnesses=harnesses)
                self.assertEqual(calls, ['internal installer-integrations --help',
                                         'internal installer-integrations'])
                self.assertIn('outcome=complete', output)

    def test_modern_guards_precede_capability_and_mutation(self):
        for options, outcome in [({'mode': 'no'}, 'skipped'),
                                 ({'registered': 0}, 'not_registered'),
                                 ({'user_level': False}, 'per_project')]:
            with self.subTest(options=options):
                output, calls = self.run_actual_lane(**options)
                self.assertEqual(calls, [])
                self.assertIn(f'outcome={outcome}', output)

    def test_modern_failure_is_terminal_without_fabricated_membership(self):
        for harnesses in [(), ('claude', 'codex')]:
            with self.subTest(harnesses=harnesses):
                output, calls = self.run_actual_lane(fail=True, harnesses=harnesses)
                self.assertEqual(calls, ['internal installer-integrations --help',
                                         'internal installer-integrations'])
                self.assertIn('outcome=failed', output)
                self.assertIn('failed_harnesses=\n', output)
                self.assertIn('per-component verdicts', output)

    def test_unavailable_capability_preserves_actual_legacy_boundary(self):
        output, calls = self.run_actual_lane(modern=False)
        self.assertEqual(calls, ['internal installer-integrations --help'])
        self.assertIn('outcome=none_found', output)
        for bare, operation in [(True, ['setup']), (False, ['setup claude', 'setup codex'])]:
            for mode, terminal, consent, wanted in [
                    ('yes', False, False, operation),
                    ('ask', False, False, []),
                    ('ask', True, False, []),
                    ('ask', True, True, operation)]:
                with self.subTest(bare=bare, mode=mode, terminal=terminal, consent=consent):
                    _, calls = self.run_actual_lane(
                        modern=False, harnesses=('claude', 'codex'), bare=bare,
                        mode=mode, terminal=terminal, consent=consent)
                    self.assertEqual(calls, ['internal installer-integrations --help', *wanted])

    def test_modern_failure_final_status_and_next_steps_are_generic(self):
        output, calls = self.run_actual_lane(fail=True, harnesses=('claude', 'codex'),
                                             final_status=True)
        self.assertEqual(calls, ['internal installer-integrations --help',
                                 'internal installer-integrations'])
        self.assertIn('harness integration reconciliation incomplete', output)
        self.assertIn('Fix the failed integration components', output)
        self.assertIn('retry it with: herdr-threads internal installer-integrations', output)
        self.assertNotIn('setup incomplete: claude codex', output)
        self.assertNotIn('finish it with: herdr-threads setup', output)

    def run_lane(self, mode='ask', modern=True, fail=False, registered=1):
        return self.run_actual_lane(mode=mode, modern=modern, fail=fail,
                                    registered=registered, harnesses=('claude', 'codex'))

    def test_default_reconciles_owned_even_without_terminal(self):
        output, calls = self.run_lane()
        self.assertIn('internal installer-integrations', calls)
        self.assertNotIn('setup', calls)
        self.assertIn('outcome=complete', output)

    def test_setup_explicitly_confirms_missing(self):
        _, calls = self.run_lane(mode='yes')
        self.assertIn('internal installer-integrations --confirm-missing', calls)

    def test_no_setup_bypasses_all_integration_calls(self):
        _, calls = self.run_lane(mode='no')
        self.assertEqual(calls, [])

    def test_modern_failure_never_falls_back_to_legacy_setup(self):
        output, calls = self.run_lane(fail=True)
        self.assertIn('outcome=failed', output)
        self.assertNotIn('setup', calls)

    def test_older_binary_keeps_legacy_explicit_setup(self):
        _, calls = self.run_lane(mode='yes', modern=False)
        self.assertIn('setup', calls)
        self.assertNotIn('internal installer-integrations --confirm-missing', calls)

    def test_unregistered_plugin_skips_integration_changes(self):
        _, calls = self.run_lane(mode='yes', registered=0)
        self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
