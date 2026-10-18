#!/usr/bin/env python3
"""Tests for scripts/check-wire-compat (the peer wire version-tolerance check).

Each test builds a throwaway repository root under a temp directory holding
every scanned wire file (empty unless a case fills it), writes a baseline (and
optionally a base baseline for the R4 delta) and runs the script against it.
"""

import json
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "check-wire-compat")
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))

FRAME = "src/sync/frame.rs"
ENVELOPE = "src/sync/envelope.rs"
SCANNED = [
    FRAME,
    ENVELOPE,
    "src/sync/facts/payload.rs",
    "src/sync/facts/node_genesis.rs",
    "src/sync/facts/node_retag.rs",
    "src/sync/facts/seat.rs",
    "src/sync/facts/thread.rs",
    "src/sync/facts/invitation.rs",
    "src/sync/facts/membership.rs",
    "src/sync/facts/message.rs",
    "src/sync/facts/receipt.rs",
    "src/protocol/sync_admin.rs",
]
FRAME_BASE = 'pub const FORK_CAP: &str = "fork.v1";\n'
FACT = "src/sync/facts/seat.rs"

DERIVE = "#[derive(Debug, Serialize, Deserialize)]\n"


def ok_line(v=0, s=0, f=0, n=0, e=0):
    return "check-wire-compat: ok — %d violations, %d stale, %d fields, %d variants, %d baseline entries" % (
        v,
        s,
        f,
        n,
        e,
    )


class Case(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = self._tmp.name
        for rel in SCANNED:
            self.write(rel, "")
        self.write(FRAME, FRAME_BASE)
        self.baseline([])

    def write(self, rel, text):
        path = os.path.join(self.root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def baseline(self, lines, name="baseline.txt", header=True):
        text = ("# header\n\n" if header else "") + "".join(l + "\n" for l in lines)
        self.write(name, text)

    def run_check(self, *extra):
        cmd = [SCRIPT, "--root", self.root, "--baseline", os.path.join(self.root, "baseline.txt")]
        cmd += list(extra)
        return subprocess.run(cmd, capture_output=True, text=True)

    def assertOk(self, proc):
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("check-wire-compat: ok", proc.stdout.splitlines()[-1])

    def assertFails(self, proc, *needles):
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertIn("check-wire-compat: FAILED", proc.stdout.splitlines()[-1])
        for needle in needles:
            self.assertIn(needle, proc.stdout)


class PassingTests(Case):
    def test_baselined_struct_and_frame_pass(self):
        self.write(
            FACT,
            DERIVE
            + "pub struct Foo {\n    pub a: u32,\n    pub b: Vec<String>,\n}\n"
            + "pub struct NotWire {\n    pub z: u32,\n}\n",
        )
        self.write(
            FRAME,
            FRAME_BASE
            + DERIVE
            + '#[serde(tag = "t")]\npub enum PeerFrame {\n    Accept,\n    Ping { n: u64 },\n'
            + "    Hello(PeerHello),\n}\n",
        )
        self.baseline(
            [
                "field Foo.a",
                "field Foo.b",
                "field PeerFrame::Ping.n",
                "variant PeerFrame::Accept",
                "variant PeerFrame::Hello",
                "variant PeerFrame::Ping",
            ]
        )
        p = self.run_check()
        self.assertOk(p)
        self.assertEqual(p.stdout.splitlines()[-1], ok_line(f=3, n=3, e=6))

    def test_new_field_with_serde_default(self):
        for attr in (
            "#[serde(default)]",
            '#[serde(default, skip_serializing_if = "Option::is_none")]',
            '#[serde(default = "f")]',
            '#[serde(rename = "x", default)]',
        ):
            with self.subTest(attr=attr):
                self.write(FACT, DERIVE + "pub struct Foo {\n    " + attr + "\n    pub a: Option<u32>,\n}\n")
                self.assertOk(self.run_check())

    def test_new_gated_variant_with_marker(self):
        self.write(
            FRAME,
            FRAME_BASE
            + DERIVE
            + "pub enum PeerFrame {\n    /// doc\n    // cap: FORK_CAP\n"
            + '    #[serde(skip_deserializing)]\n    BranchNone { },\n}\n',
        )
        self.baseline([])
        self.assertOk(self.run_check())

    def test_deny_unknown_fields_in_comment_or_string_is_fine(self):
        self.write(
            FACT,
            "// never #[serde(deny_unknown_fields)] here\n"
            '/* deny_unknown_fields */\nconst X: &str = "deny_unknown_fields";\n'
            + DERIVE
            + "pub struct Foo {\n    #[serde(default)]\n    pub a: u32,\n}\n",
        )
        self.assertOk(self.run_check())

    def test_test_only_struct_is_skipped(self):
        self.write(
            FACT,
            "#[cfg(test)]\nmod tests {\n    "
            + DERIVE
            + "    #[serde(deny_unknown_fields)]\n    struct Shadow {\n        a: u32,\n    }\n}\n"
            + "#[cfg(feature = \"test-support\")]\n"
            + DERIVE
            + "pub struct Helper {\n    pub q: u32,\n}\n",
        )
        self.assertOk(self.run_check())

    def test_struct_nested_in_fn_and_tuple_and_unit_are_handled(self):
        self.write(
            FACT,
            "fn decode() {\n    "
            + DERIVE
            + "    struct Tag {\n        t: String,\n    }\n}\n"
            + DERIVE
            + "pub struct Tup(pub u32, pub String);\n"
            + DERIVE
            + "pub struct Unit;\n",
        )
        self.baseline(["field Tag.t"])
        p = self.run_check()
        self.assertOk(p)
        self.assertEqual(p.stdout.splitlines()[-1], ok_line(f=1, e=1))

    def test_type_restricted_file(self):
        self.write(
            ENVELOPE,
            DERIVE
            + "pub struct WireEnvelope {\n    pub node: String,\n}\n"
            + DERIVE
            + "pub struct Other {\n    pub free: u32,\n}\n",
        )
        self.baseline(["field WireEnvelope.node"])
        self.assertOk(self.run_check())

    def test_generic_and_multiline_field_types(self):
        self.write(
            FACT,
            DERIVE
            + "pub struct Foo {\n    pub m: std::collections::HashMap<String, Vec<u32>>,\n"
            + "    pub t: (u32, u32),\n    pub arr: [u8; 4],\n    pub last: u32\n}\n",
        )
        self.baseline(["field Foo.arr", "field Foo.last", "field Foo.m", "field Foo.t"])
        p = self.run_check()
        self.assertOk(p)
        self.assertEqual(p.stdout.splitlines()[-1], ok_line(f=4, e=4))


class RuleTests(Case):
    def test_r1_deny_unknown_fields(self):
        self.write(FACT, "#[derive(Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct Foo {\n    pub a: u32,\n}\n")
        self.baseline(["field Foo.a"])
        p = self.run_check()
        self.assertFails(p, "src/sync/facts/seat.rs:2: R1:")

    def test_r1_applies_outside_the_type_restriction(self):
        self.write(ENVELOPE, "#[serde(deny_unknown_fields)]\nstruct Other {}\n")
        self.assertFails(self.run_check(), "src/sync/envelope.rs:1: R1:")

    def test_r2_new_field_without_default(self):
        for decl in (
            "    pub a: u32,\n",
            '    #[serde(rename = "x")]\n    pub a: u32,\n',
            "    #[serde(skip_serializing_if = \"Option::is_none\")]\n    pub a: Option<u32>,\n",
        ):
            with self.subTest(decl=decl):
                self.write(FACT, DERIVE + "pub struct Foo {\n" + decl + "}\n")
                self.assertFails(self.run_check(), "R2:", "Foo.a")

    def test_r2_new_field_of_struct_like_variant(self):
        self.write(
            FRAME,
            FRAME_BASE + DERIVE + "pub enum PeerFrame {\n    Have {\n        full: bool,\n        extra: u32,\n    },\n}\n",
        )
        self.baseline(["field PeerFrame::Have.full", "variant PeerFrame::Have"])
        p = self.run_check()
        self.assertFails(p, "src/sync/frame.rs:6: R2:", "PeerFrame::Have.extra")
        self.assertNotIn("PeerFrame::Have.full", p.stdout.replace("field PeerFrame::Have.full", ""))

    def test_r2_default_on_other_field_does_not_leak(self):
        self.write(
            FACT,
            DERIVE
            + "pub struct Foo {\n    #[serde(default)]\n    pub a: u32,\n    pub b: u32,\n}\n",
        )
        self.assertFails(self.run_check(), "R2:", "Foo.b")

    def test_r3_new_variant_without_marker(self):
        self.write(FRAME, FRAME_BASE + DERIVE + "pub enum PeerFrame {\n    Fresh,\n}\n")
        self.assertFails(self.run_check(), "src/sync/frame.rs:4: R3:", "PeerFrame::Fresh")

    def test_r3_marker_must_name_a_const(self):
        self.write(FRAME, FRAME_BASE + DERIVE + "pub enum PeerFrame {\n    // cap: NOT_A_CONST\n    Fresh,\n}\n")
        self.assertFails(self.run_check(), "R3:", "NOT_A_CONST")

    def test_r3_marker_separated_by_code_line(self):
        self.write(
            FRAME,
            FRAME_BASE + DERIVE + "pub enum PeerFrame {\n    // cap: FORK_CAP\n    Known,\n    Fresh,\n}\n",
        )
        self.baseline(["variant PeerFrame::Known"])
        self.assertFails(self.run_check(), "src/sync/frame.rs:6: R3:", "PeerFrame::Fresh")

    def test_r3_const_must_be_a_pub_str_const_in_frame_rs(self):
        self.write(FRAME, 'const PRIVATE_CAP: &str = "x";\n' + DERIVE + "pub enum PeerFrame {\n    // cap: PRIVATE_CAP\n    Fresh,\n}\n")
        self.assertFails(self.run_check(), "R3:", "PRIVATE_CAP")

    def test_r3_covers_wire_item_when_present(self):
        self.write(
            FRAME,
            FRAME_BASE + "pub enum WireItem {\n    // cap: FORK_CAP\n    Opaque(OpaqueWire),\n    Known(WireEnvelope),\n}\n",
        )
        p = self.run_check()
        self.assertFails(p, "src/sync/frame.rs:5: R3:", "WireItem::Known")
        self.assertNotIn("WireItem::Opaque", p.stdout.split("R3:", 1)[1].split("\n")[0])

    def test_r4_new_field_of_baselined_type_is_a_violation(self):
        self.write(FACT, DERIVE + "pub struct Foo {\n    pub a: u32,\n    pub b: u32,\n}\n")
        self.baseline(["field Foo.a", "field Foo.b"])
        self.baseline(["field Foo.a"], name="old.txt")
        p = self.run_check("--base-file", os.path.join(self.root, "old.txt"))
        self.assertFails(p, "baseline + field Foo.b", "R4:", "baseline may only grow by new types")

    def test_r4_new_type_is_allowed_and_delta_printed(self):
        self.write(
            FACT,
            DERIVE + "pub struct Foo {\n    pub a: u32,\n}\n" + DERIVE + "pub struct Bar {\n    pub a: u32,\n}\n",
        )
        self.baseline(["field Bar.a", "field Foo.a"])
        self.baseline(["field Foo.a", "field Gone.x"], name="old.txt")
        p = self.run_check("--base-file", os.path.join(self.root, "old.txt"))
        # Gone.x is stale in the baseline? No: it is only in the OLD file.
        self.assertOk(p)
        self.assertIn("baseline + field Bar.a", p.stdout)
        self.assertIn("baseline - field Gone.x", p.stdout)

    def test_r4_without_base_is_skipped(self):
        p = self.run_check()
        self.assertOk(p)
        self.assertIn("R4: no base, delta skipped", p.stdout)

    def test_r4_base_ref_missing_file_means_all_new(self):
        subprocess.run(["git", "init", "-q", self.root], check=True)
        subprocess.run(
            ["git", "-C", self.root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"],
            check=True,
        )
        subprocess.run(["git", "-C", self.root, "branch", "-M", "main"], check=True)
        self.write(FACT, DERIVE + "pub struct Foo {\n    pub a: u32,\n}\n")
        self.baseline(["field Foo.a"])
        p = self.run_check("--base", "main")
        self.assertOk(p)
        self.assertIn("baseline + field Foo.a", p.stdout)

    def test_r4_unresolvable_base_ref_is_a_usage_error(self):
        p = self.run_check("--base", "no-such-ref")
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)

    def test_stale_baseline_line_fails(self):
        self.baseline(["field Foo.a"])
        self.assertFails(self.run_check(), "stale baseline entry: field Foo.a")


class ConfigTests(Case):
    def test_unsorted_baseline(self):
        self.baseline(["field Foo.b", "field Foo.a"])
        p = self.run_check()
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("field Foo.a", p.stdout + p.stderr)

    def test_duplicate_baseline_line(self):
        self.baseline(["field Foo.a", "field Foo.a"])
        self.assertEqual(self.run_check().returncode, 2)

    def test_malformed_baseline_line(self):
        for line in ("Foo.a", "field Foo", "variant Foo", "bogus Foo.a"):
            with self.subTest(line=line):
                self.baseline([line])
                self.assertEqual(self.run_check().returncode, 2)

    def test_comment_after_first_entry(self):
        self.baseline(["field Foo.a", "# late comment"])
        self.assertEqual(self.run_check().returncode, 2)

    def test_unknown_flag(self):
        self.assertEqual(self.run_check("--bogus").returncode, 2)

    def test_missing_listed_file(self):
        os.remove(os.path.join(self.root, "src/protocol/sync_admin.rs"))
        p = self.run_check()
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("src/protocol/sync_admin.rs", p.stdout + p.stderr)


class OutputTests(Case):
    def test_json_shape(self):
        self.write(FACT, DERIVE + "pub struct Foo {\n    pub a: u32,\n}\n")
        self.baseline(["field Foo.a"])
        p = self.run_check("--json")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(
            json.loads(p.stdout),
            {
                "status": "ok",
                "violations": 0,
                "stale": 0,
                "fields": 1,
                "variants": 0,
                "entries": 1,
                "delta_added": 0,
                "delta_removed": 0,
            },
        )

    def test_json_failed_reports_violations_on_stderr(self):
        self.write(FACT, DERIVE + "pub struct Foo {\n    pub a: u32,\n}\n")
        p = self.run_check("--json")
        self.assertEqual(p.returncode, 1)
        out = json.loads(p.stdout)
        self.assertEqual((out["status"], out["violations"], out["fields"]), ("failed", 1, 1))
        self.assertIn("R2:", p.stderr)

    def test_json_delta_counts(self):
        self.write(FACT, DERIVE + "pub struct Bar {\n    pub a: u32,\n}\n")
        self.baseline(["field Bar.a"])
        self.baseline(["field Gone.x", "field Gone.y"], name="old.txt")
        p = self.run_check("--json", "--base-file", os.path.join(self.root, "old.txt"))
        out = json.loads(p.stdout)
        self.assertEqual((out["delta_added"], out["delta_removed"]), (1, 2))

    def test_suggest_is_sorted_unique_and_honours_the_type_restriction(self):
        self.write(
            FACT,
            DERIVE + "pub struct Zed {\n    pub z: u32,\n}\n" + DERIVE + "pub struct Foo {\n    pub b: u32,\n    pub a: u32,\n}\n",
        )
        self.write(
            FRAME,
            FRAME_BASE
            + DERIVE
            + "pub enum PeerFrame {\n    Ping { n: u64 },\n    Accept,\n}\n"
            + DERIVE
            + "pub struct Foo {\n    pub a: u32,\n}\n",
        )
        self.write(
            ENVELOPE,
            DERIVE
            + "pub struct WireEnvelope {\n    pub node: String,\n}\n"
            + DERIVE
            + "pub struct Other {\n    pub free: u32,\n}\n",
        )
        p = self.run_check("--suggest")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(
            p.stdout.splitlines(),
            [
                "field Foo.a",
                "field Foo.b",
                "field PeerFrame::Ping.n",
                "field WireEnvelope.node",
                "field Zed.z",
                "variant PeerFrame::Accept",
                "variant PeerFrame::Ping",
            ],
        )

    def test_suggest_output_bootstraps_a_passing_baseline(self):
        self.write(FACT, DERIVE + "pub struct Foo {\n    pub a: u32,\n}\n")
        sug = self.run_check("--suggest").stdout
        self.write("baseline.txt", "# header\n" + sug)
        self.assertOk(self.run_check())


class RealTreeTests(unittest.TestCase):
    def test_current_tree_passes(self):
        if not os.path.isdir(os.path.join(REPO, "src", "sync")):
            self.skipTest("not running inside the repository")
        p = subprocess.run([SCRIPT, "--root", REPO], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("check-wire-compat: ok", p.stdout.splitlines()[-1])


if __name__ == "__main__":
    unittest.main()
