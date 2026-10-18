import importlib.util
from pathlib import Path
import unittest

class ChangedPaths(unittest.TestCase):
    def test_conservative_docs_policy(self):
        path = Path(__file__).resolve().parents[1] / 'ci-changed-paths.py'
        self.assertTrue(path.exists(), 'changed-path detector missing')
        spec = importlib.util.spec_from_file_location('paths', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for event, ref in [('workflow_dispatch', 'branch'), ('push', 'tag')]:
            self.assertTrue(module.heavy(event, ref, ['README.md']))
        for event in ['push', 'pull_request']:
            self.assertFalse(module.heavy(event, 'branch', ['docs/nested/a.txt', 'README.md', 'CHANGELOG']))
            self.assertTrue(module.heavy(event, 'branch', ['docs/a.md', 'src/main.rs']))
            self.assertTrue(module.heavy(event, 'branch', None))
            self.assertTrue(module.heavy(event, 'branch', []))
