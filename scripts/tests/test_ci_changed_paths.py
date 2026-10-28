import importlib.util
from pathlib import Path
import unittest


class ChangedPaths(unittest.TestCase):
    def test_explicit_docs_allowlist(self):
        path = Path(__file__).resolve().parents[1] / 'ci-changed-paths.py'
        spec = importlib.util.spec_from_file_location('paths', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        allowed = ['RELEASE.md', 'AGENTS.md', 'CLAUDE.md', 'docs/release.md',
                   'docs/superpowers/plans/design.md', 'docs/superpowers/runs/evidence.txt']
        # Docs may be compiled assets or tested contracts: suffix alone is no exemption.
        heavy = ['README.md', 'CHANGELOG.md', 'CHANGELOG', 'TRUST-POLICY.md',
                 'integrations/skill/SKILL.md', 'integrations/claude/README.md',
                 'docs/install.md', 'docs/compatibility/harness-versions.json',
                 'docs/compatibility/policy.md', 'docs/evidence/report.md', 'docs/other.md',
                 'notes.md', 'src/main.rs', 'nested/RELEASE.md', 'docs/superpowers-other/file.md']
        for event in ['push', 'pull_request']:
            self.assertFalse(module.heavy(event, 'branch', allowed))
            for path in heavy:
                with self.subTest(event=event, path=path):
                    self.assertTrue(module.heavy(event, 'branch', [path]))
                    self.assertTrue(module.heavy(event, 'branch', allowed + [path]))
            for paths in [None, [], ['']]:
                self.assertTrue(module.heavy(event, 'branch', paths))
        for event, ref in [('workflow_dispatch', 'branch'), ('push', 'tag')]:
            self.assertTrue(module.heavy(event, ref, allowed))
