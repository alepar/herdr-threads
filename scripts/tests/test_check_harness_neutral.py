#!/usr/bin/env python3
"""Tests for scripts/check-harness-neutral (the neutral-core seam check).

Each test builds a throwaway repository root under a temp directory with a
registry carrying the builtins() rows, writes fixture sources and an allowlist,
and runs the script against it.
"""

import json
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "check-harness-neutral")

REG = "src/harness/registry.rs"
ROWS = [
    "Registration::new(&crate::harness::claude::ClaudeAdapter),",
    "Registration::new(&crate::harness::codex::CodexAdapter),",
    "Registration::new(&crate::harness::hermes::HermesAdapter),",
]
AGY_ROW = "Registration::new(&crate::harness::agy::AgyAdapter),"

CLEAN = (
    "check-harness-neutral: ok — 0 violations, 0 allowlisted "
    "(to-migrate 0, frozen-wire 0, history 0, registration 0), 0 entries"
)


class Case(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = self._tmp.name
        self.write_registry(ROWS)

    def write_registry(self, rows):
        # The rows sit in a test-only block so the fixture registry itself is not a
        # violation; the real registry's rows are `registration` allowlist entries.
        body = "\n        ".join(rows)
        self.write(REG, "#[cfg(test)]\nfn builtins() {\n    [\n        " + body + "\n    ]\n}\n")

    def write(self, rel, text):
        path = os.path.join(self.root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def allow(self, *lines, header=True):
        text = ("# header\n\n" if header else "") + "".join(l + "\n" for l in lines)
        self.write("allow.txt", text)

    def run_check(self, *extra, allowlist=True):
        cmd = [SCRIPT, "--root", self.root]
        if allowlist:
            cmd += ["--allowlist", os.path.join(self.root, "allow.txt")]
        else:
            cmd += ["--allowlist", os.path.join(self.root, "missing.txt")]
        cmd += list(extra)
        return subprocess.run(cmd, capture_output=True, text=True)

    def flagged(self, source, rel="src/cli/x.rs"):
        self.write(rel, source)
        return self.run_check(allowlist=False)

    def assertViolations(self, proc, n):
        self.assertEqual(proc.returncode, 1 if n else 0, proc.stdout + proc.stderr)
        self.assertIn("%d violations" % n, proc.stdout.splitlines()[-1])


class BasicTests(Case):
    def test_clean_tree(self):
        self.write("src/cli/x.rs", "fn main() { let x = 1; }\n")
        p = self.run_check()
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout.splitlines()[-1], CLEAN)

    def test_literal_reported_with_location(self):
        p = self.flagged('let h = "claude";\n')
        self.assertEqual(p.returncode, 1)
        self.assertIn('src/cli/x.rs:1: claude: let h = "claude";', p.stdout)
        self.assertIn("FAILED — 1 violations", p.stdout.splitlines()[-1])

    def test_sql_literal_two_violations(self):
        p = self.flagged("let q = \"harness IN ('claude','codex')\";\n")
        self.assertViolations(p, 2)

    def test_name_shapes_flagged(self):
        for src in (
            "let h = Harness::Codex;",
            "harness_claude: u8,",
            "struct ClaudeContract;",
            "let d = CLAUDE_CONFIG_DIR;",
            "enum E { NotClaude }",
            'let p = "~/.gemini";',
            'let a = "agy";',
            "let e = ANTIGRAVITY_AGENT;",
            "let h = HermesHarness;",
        ):
            with self.subTest(src=src):
                self.assertViolations(self.flagged(src + "\n"), 1)

    def test_lookalikes_not_flagged(self):
        p = self.flagged("let strategy = legacy(claudette); let a = Agyle;\n")
        self.assertViolations(p, 0)

    def test_repeated_token_on_one_line_counts_once(self):
        p = self.flagged('f("claude", "claude");\n')
        self.assertViolations(p, 1)

    def test_registry_without_builtins_rows(self):
        self.write(REG, "fn builtins() {}\n")
        p = self.run_check(allowlist=False)
        self.assertEqual(p.returncode, 2)
        self.assertIn("no builtins() rows found in src/harness/registry.rs", p.stdout + p.stderr)

    def test_bad_flag_is_usage_error(self):
        p = self.run_check("--bogus")
        self.assertEqual(p.returncode, 2)


class ExemptionTests(Case):
    def test_adapter_owned_paths_exempt(self):
        for rel in (
            "src/harness/claude.rs",
            "src/harness/claude/setup.rs",
            "src/harness/claude_mod.rs",
            "src/harness/codex_schema.rs",
            "src/harness/hermes/launch.rs",
        ):
            with self.subTest(rel=rel):
                self.write(rel, 'let h = "claude"; let c = "codex";\n')
        p = self.run_check(allowlist=False)
        self.assertViolations(p, 0)

    def test_non_owned_harness_file_is_scanned(self):
        p = self.flagged('let h = "claude";\n', rel="src/harness/launch.rs")
        self.assertIn("src/harness/launch.rs:1: claude:", p.stdout)

    def test_agy_exempt_only_once_registered(self):
        self.write("src/harness/agy/mod.rs", 'let a = "agy";\n')
        p = self.run_check(allowlist=False)
        self.assertEqual(p.returncode, 1)
        self.assertIn("src/harness/agy/mod.rs:1: agy:", p.stdout)
        self.write_registry(ROWS + [AGY_ROW])
        p = self.run_check(allowlist=False)
        self.assertViolations(p, 0)

    def test_test_support_and_tests_not_scanned(self):
        self.write("src/test_support/x.rs", 'let h = "claude";\n')
        self.write("tests/x.rs", 'let h = "claude";\n')
        self.assertViolations(self.run_check(allowlist=False), 0)


class LexerTests(Case):
    def test_cfg_test_module_skipped_and_braces_resume(self):
        src = "\n".join(
            [
                "#[cfg(test)]",
                "mod tests {",
                '    let a = "claude";',
                '    let b = r#"{"agent":"claude"}"#;',
                "    let c = '{';",
                "    fn f<'a>(x: &'a str) {}",
                "}",
                'let c = "codex";',
                "",
            ]
        )
        p = self.flagged(src)
        self.assertViolations(p, 1)
        self.assertIn("src/cli/x.rs:8: codex:", p.stdout)

    def test_item_level_skips(self):
        cases = [
            '#[cfg(test)]\n#[allow(dead_code)]\nfn f() { "claude" }\n',
            "#[cfg(test)]\nuse crate::harness::claude::X;\n",
            '#[cfg(any(test, feature = "test-support"))]\nfn f() { "claude" }\n',
            '#[cfg(feature = "test-support")]\nconst A: &str = "claude";\n',
            '#[cfg(all(test, unix))]\nfn f() { "claude" }\n',
            '#[cfg(test)]\nconst A: [&str; 2] = ["claude", "codex"];\n',
        ]
        for src in cases:
            with self.subTest(src=src):
                self.assertViolations(self.flagged(src), 0)

    def test_cfg_not_test_is_scanned(self):
        p = self.flagged('#[cfg(not(test))]\nconst A: &str = "codex";\n')
        self.assertViolations(p, 1)

    def test_skip_ends_at_enclosing_close_brace(self):
        src = "struct S {\n    #[cfg(test)]\n    x: u8,\n}\n" 'let a = "claude";\n'
        p = self.flagged(src)
        self.assertViolations(p, 1)
        self.assertIn("src/cli/x.rs:5: claude:", p.stdout)

    def test_comments_ignored(self):
        src = "\n".join(
            [
                "// Claude",
                "/// Codex",
                "//! hermes",
                "let x = 1; // claude",
                "/* codex */",
                "/* start",
                "   claude",
                "   /* nested codex */ hermes",
                "end */ let y = 2;",
                "",
            ]
        )
        self.assertViolations(self.flagged(src), 0)

    def test_double_slash_inside_string_is_not_a_comment(self):
        self.assertViolations(self.flagged('let u = "http://claude";\n'), 1)

    def test_multiline_string_is_scanned(self):
        p = self.flagged('let s = "first\nclaude second";\nlet t = 1; // codex\n')
        self.assertViolations(p, 1)
        self.assertIn("src/cli/x.rs:2: claude:", p.stdout)

    def test_escaped_quote_does_not_end_string(self):
        p = self.flagged('let s = "a\\" claude";\n')
        self.assertViolations(p, 1)

    def test_char_quote_literal(self):
        # '"' must not open a string that swallows the next line's comment.
        p = self.flagged("let q = '\"';\nlet x = 1; // claude\n")
        self.assertViolations(p, 0)


class AllowlistTests(Case):
    def setUp(self):
        super().setUp()
        self.write("src/cli/x.rs", 'let h = "claude";\n')

    def test_entry_allows_and_counts(self):
        self.allow("src/cli/x.rs:claude — frozen-wire: test")
        p = self.run_check()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(
            p.stdout.splitlines()[-1],
            "check-harness-neutral: ok — 0 violations, 1 allowlisted "
            "(to-migrate 0, frozen-wire 1, history 0, registration 0), 1 entries",
        )

    def test_to_migrate_entry_is_a_hard_failure(self):
        self.allow("src/cli/x.rs:claude — to-migrate: test")
        p = self.run_check()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("to-migrate entries are forbidden: 1", p.stdout)
        self.assertEqual(
            p.stdout.splitlines()[-1],
            "check-harness-neutral: FAILED — 0 violations, 1 allowlisted "
            "(to-migrate 1, frozen-wire 0, history 0, registration 0), 1 entries",
        )

    def test_to_migrate_failure_counts_every_entry(self):
        self.write("src/cli/x.rs", 'let h = "claude"; let i = "codex";\n')
        self.allow("src/cli/x.rs:claude — to-migrate: a", "src/cli/x.rs:codex — to-migrate: b")
        p = self.run_check()
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("to-migrate entries are forbidden: 2", p.stdout)

    def test_to_migrate_json_failure(self):
        self.allow("src/cli/x.rs:claude — to-migrate: test")
        p = self.run_check("--json")
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        data = json.loads(p.stdout)
        self.assertEqual(data["status"], "failed")
        self.assertEqual(data["entries_by_category"]["to-migrate"], 1)
        self.assertIn("to-migrate entries are forbidden: 1", p.stderr)
        self.assertNotIn("forbidden", p.stdout)

    def test_no_forbidden_message_without_to_migrate(self):
        self.allow("src/cli/x.rs:claude — frozen-wire: kept")
        self.assertNotIn("forbidden", self.run_check().stdout)

    def test_line_scoped_pattern(self):
        self.write("src/cli/x.rs", 'registry.agent("claude");\nlet h = "claude";\n')
        self.allow('src/cli/x.rs:registry.agent("claude") — registration: test')
        p = self.run_check()
        self.assertEqual(p.returncode, 1)
        self.assertIn("src/cli/x.rs:2: claude:", p.stdout)
        self.assertNotIn("src/cli/x.rs:1:", p.stdout)
        self.assertIn("1 violations, 1 allowlisted", p.stdout.splitlines()[-1])
        self.assertIn("registration 1", p.stdout.splitlines()[-1])

    def test_pattern_without_token_does_not_allow(self):
        self.allow("src/cli/x.rs:let h — history: test")
        p = self.run_check()
        self.assertEqual(p.returncode, 1)
        self.assertIn("src/cli/x.rs:1: claude:", p.stdout)
        self.assertIn("stale allowlist entry: src/cli/x.rs:let h", p.stdout)

    def test_stale_entry_fails(self):
        self.allow(
            "src/cli/gone.rs:claude — frozen-wire: gone",
            "src/cli/x.rs:claude — frozen-wire: kept",
        )
        p = self.run_check()
        self.assertEqual(p.returncode, 1)
        self.assertIn("stale allowlist entry: src/cli/gone.rs:claude", p.stdout)
        self.assertIn("FAILED", p.stdout.splitlines()[-1])

    def test_first_covering_entry_gets_the_count(self):
        self.allow(
            "src/cli/x.rs:claude — history: first",
            'src/cli/x.rs:let h = "claude" — registration: second',
        )
        p = self.run_check()
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("1 allowlisted (to-migrate 0, frozen-wire 0, history 1, registration 0), 2 entries", p.stdout)

    def test_malformed_entries_exit_2(self):
        bad = [
            "src/cli/x.rs:claude - to-migrate: hyphen not em dash",
            "src/cli/x.rs:claude — frozen: unknown category",
            "src/cli/x.rs:claude — to-migrate:",
            "src/cli/x.rs:claude — to-migrate:    ",
            "no-colon-here — to-migrate: x",
            "src/cli/x.rs: — to-migrate: empty pattern",
        ]
        for line in bad:
            with self.subTest(line=line):
                self.allow(line)
                self.assertEqual(self.run_check().returncode, 2)

    def test_comment_after_first_entry_exit_2(self):
        self.allow("src/cli/x.rs:claude — to-migrate: a", "# late comment")
        self.assertEqual(self.run_check().returncode, 2)

    def test_unsorted_exit_2(self):
        self.allow(
            "src/cli/y.rs:claude — to-migrate: a",
            "src/cli/x.rs:claude — to-migrate: b",
        )
        p = self.run_check()
        self.assertEqual(p.returncode, 2)
        self.assertIn("src/cli/x.rs:claude", p.stderr)

    def test_duplicate_exit_2(self):
        line = "src/cli/x.rs:claude — to-migrate: a"
        self.allow(line, line)
        self.assertEqual(self.run_check().returncode, 2)

    def test_missing_allowlist_is_empty(self):
        p = self.run_check(allowlist=False)
        self.assertEqual(p.returncode, 1)


class SuggestAndJsonTests(Case):
    def setUp(self):
        super().setUp()
        self.write("src/cli/x.rs", 'let h = "claude";\nlet i = "claude"; let j = "codex";\n')

    def test_suggest_sorted_unique_and_roundtrips(self):
        p = self.run_check("--suggest", allowlist=False)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(
            p.stdout.splitlines(),
            [
                "src/cli/x.rs:claude — to-migrate: TODO",
                "src/cli/x.rs:codex — to-migrate: TODO",
            ],
        )
        # Suggested lines are a bootstrap aid: committed as-is they fail the strict check.
        self.allow(*p.stdout.splitlines())
        q = self.run_check()
        self.assertEqual(q.returncode, 1, q.stdout)
        self.assertIn("to-migrate entries are forbidden: 2", q.stdout)
        self.allow(*[l.replace("— to-migrate: TODO", "— frozen-wire: reviewed") for l in p.stdout.splitlines()])
        r = self.run_check()
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("3 allowlisted (to-migrate 0, frozen-wire 3", r.stdout.splitlines()[-1])

    def test_json_shape(self):
        self.allow("src/cli/x.rs:codex — history: h", "src/cli/z.rs:claude — frozen-wire: stale")
        p = self.run_check("--json")
        self.assertEqual(p.returncode, 1)
        data = json.loads(p.stdout)
        self.assertEqual(
            data,
            {
                "status": "failed",
                "violations": 2,
                "stale": 1,
                "allowlisted": 1,
                "allowlisted_by_category": {
                    "to-migrate": 0,
                    "frozen-wire": 0,
                    "history": 1,
                    "registration": 0,
                },
                "entries": 2,
                "entries_by_category": {
                    "to-migrate": 0,
                    "frozen-wire": 1,
                    "history": 1,
                    "registration": 0,
                },
            },
        )
        self.assertIn("src/cli/x.rs:1: claude:", p.stderr)
        self.assertIn("stale allowlist entry:", p.stderr)

    def test_json_ok(self):
        self.write("src/cli/x.rs", "fn f() {}\n")
        p = self.run_check("--json", allowlist=False)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(json.loads(p.stdout)["status"], "ok")


if __name__ == "__main__":
    unittest.main()
