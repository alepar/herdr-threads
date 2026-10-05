#!/usr/bin/env python3
"""Focused installer shell dispatch without downloads, Herdr, or real config."""
import os
import pathlib
import subprocess
import tempfile
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
SOURCE = (REPO / 'scripts/install.sh').read_text()
# Execute the actual setup lane against a controlled older/newer binary boundary.
LANE = SOURCE.split('# --- harness setup ')[1].split('# --- next steps ')[0]
LANE = LANE[LANE.index('\nharnesses='):]
CAPABILITY = SOURCE[SOURCE.index('installer_integrations() {'):SOURCE.index('\nbare_setup() {')]


class InstallerDispatch(unittest.TestCase):
    def run_lane(self, mode='ask', modern=True, fail=False, registered=1):
        with tempfile.TemporaryDirectory(prefix='ht-installer-shell-') as folder:
            root = pathlib.Path(folder)
            binary = root / 'binary'
            log = root / 'calls'
            binary.write_text('''#!/bin/bash
printf '%s\\n' "$*" >> "$CALL_LOG"
if [ "$1 $2 $3" = 'internal installer-integrations --help' ]; then
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
detected_harnesses() { printf 'claude\\ncodex\\n'; }
user_level_setup() { return 0; }
bare_setup() { return 0; }
can_prompt() { return 1; }
confirm() { return 1; }
OUT_SETUP=none
OUT_SETUP_FAILED=''
OUT_CODEX_SET_UP=0
''' + CAPABILITY + LANE + '\nprintf "outcome=%s\\n" "$OUT_SETUP"\n'
            env = dict(os.environ, installed_binary=str(binary), setup=mode,
                       registered=str(registered), CALL_LOG=str(log),
                       CAPABILITY_STATUS='0' if modern else '2',
                       INTEGRATION_STATUS='1' if fail else '0')
            out = subprocess.run(['bash', '-c', script], env=env, capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stderr)
            return out.stdout, log.read_text().splitlines() if log.exists() else []

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
