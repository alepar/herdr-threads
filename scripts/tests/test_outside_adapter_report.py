#!/usr/bin/env python3
"""Tests for scripts/outside-adapter-report (the R13 outside-adapter accounting tool).

Each test class builds a throwaway git repository (local user config only) with a stub
scripts/check-harness-neutral that prints the JSON stored in the tree, and runs the script
against commits in it.
"""

import json
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "outside-adapter-report")

AGY_ROW = "Registration::new(&crate::harness::agy::AgyAdapter),"

STUB = """#!/bin/sh
# stub: print the JSON committed in the tree next to the script
root=""
while [ $# -gt 0 ]; do
  case "$1" in --root) root="$2"; shift ;; esac
  shift
done
cat "$root/neutral-state.json"
"""


def neutral(violations, to_migrate):
    return json.dumps(
        {
            "status": "ok",
            "violations": violations,
            "stale": 0,
            "allowlisted": to_migrate + 1,
            "allowlisted_by_category": {"to-migrate": to_migrate, "history": 1},
            "entries": to_migrate + 1,
            "entries_by_category": {"to-migrate": to_migrate, "history": 1},
        }
    )


ALLOWLIST_BASE = (
    "# header\n"
    "src/app.rs:a — to-migrate: ht-poh.2.5 one\n"
    "src/app.rs:b — to-migrate: ht-poh.2.5 two\n"
    "src/app.rs:c — to-migrate: ht-poh.2.5 three\n"
    "src/db.rs:h — history: old shape\n"
)
ALLOWLIST_HEAD = "# header\nsrc/app.rs:c — to-migrate: ht-poh.2.5 three\nsrc/db.rs:h — history: old shape\n"


class Fixture:
    def __init__(self, tmp):
        self.dir = tmp
        self.git("init", "-q")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "commit.gpgsign", "false")

    def git(self, *args):
        return subprocess.run(
            ["git"] + list(args), cwd=self.dir, check=True, capture_output=True, text=True
        ).stdout.strip()

    def write(self, rel, text, mode=None):
        p = os.path.join(self.dir, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write(text)
        if mode:
            os.chmod(p, mode)

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")


def build_repo(tmp):
    fx = Fixture(tmp)
    fx.write("scripts/check-harness-neutral", STUB, 0o755)
    fx.write("scripts/harness-neutral-allowlist.txt", ALLOWLIST_BASE)
    fx.write("neutral-state.json", neutral(0, 3))
    fx.write(
        "src/harness/registry.rs",
        "// registry\nRegistration::new(&crate::harness::claude::ClaudeAdapter),\nfn keep() {}\n",
    )
    fx.write("src/harness/adapter.rs", "trait A {}\n")
    fx.write("src/cli/setup.rs", "fn setup() {\n    old();\n}\n")
    fx.write("src/app.rs", "fn app() {}\n")
    fx.sha_base = fx.commit("base")

    # (a) agy module + builtins row + one other registry line
    fx.write("src/harness/agy/mod.rs", "pub struct AgyAdapter;\npub fn x() {}\n")
    fx.write(
        "src/harness/registry.rs",
        "// registry\nRegistration::new(&crate::harness::claude::ClaudeAdapter),\n"
        + AGY_ROW
        + "\nfn keep() {}\npub const NEW: u32 = 1;\n",
    )
    fx.sha_a = fx.commit(
        "ht-poh.4.2: add agy module\n\nbody\n\nOutside-adapter: src/harness/registry.rs"
    )

    # (b) trait extension + caller edit, core-sweep bead, removes a line
    fx.write("src/harness/adapter.rs", "trait A {}\ntrait B {}\n")
    fx.write("src/cli/setup.rs", "fn setup() {\n    new1();\n    new2();\n}\n")
    fx.sha_b = fx.commit(
        "ht-poh.2.1: widen adapter\n\nOutside-adapter: src/harness/adapter.rs, src/cli/setup.rs"
    )

    # (c) no trailer
    fx.write("src/app.rs", "fn app() {}\nfn more() {}\nfn more2() {}\n")
    fx.sha_c = fx.commit("tweak app without a trailer")

    # (d) drop two to-migrate allowlist lines
    fx.write("scripts/harness-neutral-allowlist.txt", ALLOWLIST_HEAD)
    fx.write("neutral-state.json", neutral(0, 1))
    fx.sha_d = fx.commit("ht-poh.2.2: migrate sites\n\nOutside-adapter: none")
    fx.sha_head = fx.sha_d
    return fx


def run(fx, *extra, base=None, head=None):
    return subprocess.run(
        [
            SCRIPT,
            "--base",
            base or fx.sha_base,
            "--head",
            head or fx.sha_head,
            "--repo",
            fx.dir,
        ]
        + list(extra),
        capture_output=True,
        text=True,
    )


class OutsideAdapterReportTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory()
        cls.fx = build_repo(cls._tmp.name)
        res = run(cls.fx, "--json")
        assert res.returncode == 0, res.stderr
        cls.data = json.loads(res.stdout)
        cls.files = {(f["path"], f["bucket"]): f for f in cls.data["files"]}

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def test_agy_files_and_builtins_row_are_adapter(self):
        self.assertEqual(self.files[("src/harness/agy/mod.rs", "adapter")]["added"], 2)
        row = [f for (p, b), f in self.files.items() if b == "adapter" and p.startswith("src/harness/registry.rs")]
        self.assertEqual(len(row), 1)
        self.assertEqual(row[0]["added"], 1)

    def test_rest_of_registry_is_outside_trait_extension(self):
        reg = self.files[("src/harness/registry.rs", "outside")]
        self.assertEqual(reg["added"], 1)  # NEW const only, the agy row moved to adapter
        self.assertEqual(reg["category"], "trait-extension")

    def test_adapter_rs_is_trait_extension(self):
        self.assertEqual(
            self.files[("src/harness/adapter.rs", "outside")]["category"], "trait-extension"
        )

    def test_core_sweep_commit_marks_reworked_and_unattributed_marks_callers(self):
        self.assertEqual(self.files[("src/cli/setup.rs", "outside")]["category"], "reworked")
        self.assertEqual(self.files[("src/app.rs", "outside")]["category"], "callers")

    def test_summary_totals(self):
        s = self.data["summary"]
        self.assertEqual(s["adapter"]["added"], 3)
        self.assertEqual(s["adapter"]["files"], 2)
        # registry 1 + adapter.rs 1 + setup.rs 2 + app.rs 2 + neutral-state.json 1; allowlist adds 0
        self.assertEqual(s["outside"]["added"], 7)
        self.assertEqual(s["outside"]["files"], 6)

    def test_missing_trailer_listed_but_still_counted(self):
        missing = self.data["missing_trailers"]
        self.assertEqual([m["sha"] for m in missing], [self.fx.sha_c])
        self.assertEqual(self.files[("src/app.rs", "outside")]["added"], 2)

    def test_subepic_grouping(self):
        sub = self.data["by_subepic"]
        self.assertEqual(sub["ht-poh.4"]["commits"], 1)
        self.assertEqual(sub["ht-poh.2"]["commits"], 2)
        self.assertEqual(sub["unattributed"]["commits"], 1)
        self.assertIn("src/harness/registry.rs", sub["ht-poh.4"]["trailers"][0])
        self.assertIn("src/harness/registry.rs", sub["ht-poh.4"]["outside_paths"])
        self.assertNotIn("src/harness/agy/mod.rs", sub["ht-poh.4"]["outside_paths"])

    def test_allowlist_delta(self):
        d = self.data["allowlist_delta"]
        self.assertEqual(d["removed"], {"to-migrate": 2})
        self.assertEqual(d["added"], {})

    def test_neutral_counts_and_delta(self):
        n = self.data["neutral"]
        self.assertEqual(n["base"]["allowlisted_by_category"]["to-migrate"], 3)
        self.assertEqual(n["head"]["allowlisted_by_category"]["to-migrate"], 1)
        self.assertEqual(n["delta"]["allowlisted_by_category"]["to-migrate"], -2)

    def test_markdown_sections(self):
        res = run(self.fx)
        self.assertEqual(res.returncode, 0, res.stderr)
        last = -1
        for h in (
            "## Summary",
            "## By category",
            "## By subepic",
            "## Neutral check counts",
            "## Allowlist delta",
            "## Missing trailers",
        ):
            idx = res.stdout.find(h)
            self.assertGreater(idx, last, h)
            last = idx
        self.assertIn("-2 to-migrate", res.stdout)
        self.assertIn(self.fx.sha_c[:8], res.stdout)

    def test_bad_sha_exits_2(self):
        res = run(self.fx, base="deadbeef" * 5)
        self.assertEqual(res.returncode, 2)
        self.assertIn("unknown", res.stderr.lower())

    def test_bad_args_exit_2(self):
        res = subprocess.run([SCRIPT, "--bogus"], capture_output=True, text=True)
        self.assertEqual(res.returncode, 2)

    def test_worktrees_removed(self):
        self.assertEqual(len(self.fx.git("worktree", "list").splitlines()), 1)


class NoNeutralScriptTest(unittest.TestCase):
    def test_not_present_at_sha(self):
        with tempfile.TemporaryDirectory() as tmp:
            fx = Fixture(tmp)
            fx.write("src/app.rs", "fn a() {}\n")
            base = fx.commit("base")
            fx.write("src/app.rs", "fn a() {}\nfn b() {}\n")
            head = fx.commit("ht-poh.9: edit\n\nOutside-adapter: src/app.rs")
            res = run(fx, "--json", base=base, head=head)
            self.assertEqual(res.returncode, 0, res.stderr)
            data = json.loads(res.stdout)
            self.assertIsNone(data["neutral"]["base"])
            self.assertIsNone(data["neutral"]["head"])
            md = run(fx, base=base, head=head).stdout
            self.assertIn("not present at " + base[:8], md)

    def test_rename_counts_new_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            fx = Fixture(tmp)
            fx.write("src/old.rs", "".join("line %d\n" % i for i in range(20)))
            base = fx.commit("base")
            os.makedirs(os.path.join(tmp, "src/harness/agy"))
            fx.git("mv", "src/old.rs", "src/harness/agy/new.rs")
            head = fx.commit("ht-poh.4.1: move\n\nOutside-adapter: none")
            data = json.loads(run(fx, "--json", base=base, head=head).stdout)
            paths = {(f["path"], f["bucket"]) for f in data["files"]}
            self.assertEqual(paths, {("src/harness/agy/new.rs", "adapter")})


if __name__ == "__main__":
    unittest.main()
