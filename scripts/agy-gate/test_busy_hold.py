"""Offline regression for busy response lifetimes through send publication."""
import hashlib
import sys
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE / 'flows'), str(HERE.parent / 'agy-sandbox')]
import delivery  # noqa: E402
import _l6  # noqa: E402


class BusyResponsePublication(unittest.TestCase):
    def test_all_sends_commit_before_release(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / ('hold-' + hashlib.sha256(b'busy').hexdigest())
            world = SimpleNamespace(ctl=root, check=lambda name, ok, *rest: self.assertTrue(ok, name))
            pane = SimpleNamespace(prompt=lambda text: Path(str(path) + '.entered').touch(), wait=lambda *a: True)
            def send(*args):
                time.sleep(.02)
                self.assertFalse(Path(str(path) + '.release').exists())
                return 'mTEST'
            with mock.patch.object(_l6, 'dismiss_survey', return_value=False), \
                 mock.patch.object(delivery._l5, 'await_step', return_value={}), \
                 mock.patch.object(_l6, 'send', side_effect=send):
                ids = delivery.busy_turn_send(world, pane, None, None, 'go', 'busy', [('one', ()), ('two', ())])
            self.assertEqual(ids, ['mTEST', 'mTEST'])
            self.assertTrue(Path(str(path) + '.release').exists())

    def test_expiry_during_publication_fails_setup_and_releases(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / ('hold-' + hashlib.sha256(b'busy').hexdigest())
            world = SimpleNamespace(ctl=root, check=lambda *a: None)
            pane = SimpleNamespace(prompt=lambda text: Path(str(path) + '.entered').touch(), wait=lambda *a: True)
            def send(*args):
                Path(str(path) + '.expired').touch()
                return 'mTEST'
            with mock.patch.object(_l6, 'dismiss_survey', return_value=False), \
                 mock.patch.object(delivery._l5, 'await_step', return_value={}), \
                 mock.patch.object(_l6, 'send', side_effect=send):
                with self.assertRaisesRegex(RuntimeError, 'expired before publication completed'):
                    delivery.busy_turn_send(world, pane, None, None, 'go', 'busy', [('one', ())])
            self.assertTrue(Path(str(path) + '.release').exists())
