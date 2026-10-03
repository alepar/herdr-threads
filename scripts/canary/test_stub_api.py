#!/usr/bin/env python3
"""stub_api.py must start when scripts/canary/bisect.py sits next to it (Linux/py3.12 regression, ht-p03.14.7)."""
import os, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
# scripts/canary/bisect.py shadows the stdlib `bisect` (needed by `random`, hence `tempfile`) when
# `discover -s scripts/canary` puts this directory first on sys.path: import the stdlib ones without it.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if os.path.realpath(p or ".") != str(HERE)]
sys.modules.pop("bisect", None)
import shutil, subprocess, tempfile, unittest  # noqa: E402
sys.path[:] = _saved


class StubStartsBesideBisect(unittest.TestCase):
    def test_prints_port_with_a_poisoned_bisect_next_to_it(self):
        with tempfile.TemporaryDirectory() as d:
            shutil.copy(HERE / "stub_api.py", d)
            # A sibling named like a stdlib module that `random` imports; importing it from here would fail.
            pathlib.Path(d, "bisect.py").write_text("raise ImportError('shadowed stdlib bisect')\n")
            # Put the poisoned dir first on sys.path, as running the script by path does.
            code = ("import sys, runpy; sys.path.insert(0, %r); sys.argv = ['stub_api.py', '--lifetime', '2']; "
                    "runpy.run_path(%r, run_name='__main__')") % (d, str(pathlib.Path(d, "stub_api.py")))
            p = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=30)
            first = p.stdout.splitlines()[0] if p.stdout else ""
            self.assertTrue(first.isdigit(), f"no port on stdout; stderr={p.stderr[-400:]!r}")


if __name__ == "__main__":
    unittest.main()
