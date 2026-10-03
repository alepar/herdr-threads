#!/usr/bin/env python3
"""Offline --dry-run cases for file_issues.py (nested spec §D10). Stdlib only; no gh, no network."""
import sys
# scripts/canary/bisect.py would shadow the stdlib `bisect` (needed by random/tempfile) when this
# directory is sys.path[0]; drop it and load file_issues by path instead.
_saved = list(sys.path)
sys.path[:] = [p for p in sys.path if p not in ("", __import__("os").path.dirname(__import__("os").path.abspath(__file__)))]
sys.modules.pop("bisect", None)
import importlib.util, json, os, pathlib, re, subprocess, unittest
sys.path[:] = _saved  # later test modules of the same discover run import their siblings

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "file_issues.py"
REPORTS = HERE / "testdata" / "reports"
EXISTING = HERE / "testdata" / "existing-issues"
REPO = "owner/herdr-threads"
RUN_URL = "https://github.com/owner/herdr-threads/actions/runs/42"


def load_module():
    spec = importlib.util.spec_from_file_location("file_issues", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(report, existing=None, extra=()):
    cmd = [sys.executable, str(SCRIPT), "--report", str(REPORTS / report), "--repo", REPO,
           "--run-url", RUN_URL, "--dry-run", *extra]
    if existing:
        cmd += ["--existing-issues", str(EXISTING / existing)]
    env = {k: v for k, v in os.environ.items() if k != "GH_TOKEN"}
    p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=60)
    return p


def gh_lines(out):
    """The dry-run `gh ...` command lines (each printed as `+ gh ...`)."""
    return [l[2:] for l in out.splitlines() if l.startswith("+ gh ")]


class FileIssues(unittest.TestCase):
    def ok(self, report, existing=None, extra=()):
        p = run(report, existing, extra)
        self.assertEqual(p.returncode, 0, p.stderr)
        return p.stdout

    def test_new_break_creates_label_then_issue(self):
        out = self.ok("break.json", "none.json")
        cmds = gh_lines(out)
        self.assertEqual(len([c for c in cmds if c.startswith("gh label create harness-canary --force")]), 1)
        creates = [c for c in cmds if c.startswith("gh issue create")]
        self.assertEqual(len(creates), 1)
        self.assertIn("--title 'harness-canary: claude 2.1.285 breaks herdr-threads'", creates[0])
        self.assertIn("--label harness-canary", creates[0])
        self.assertLess(cmds.index(next(c for c in cmds if "label create" in c)), cmds.index(creates[0]))
        self.assertFalse([c for c in cmds if "issue comment" in c])

    def test_new_break_body_content(self):
        out = self.ok("break.json", "none.json")
        for needle in ("verdict: break", ">= 2.1.285", "Last good version: 2.1.284", "t0.hook-fires",
                       RUN_URL, "harness-canary-report", "known_broken: &[VersionSet::Interval",
                       "| version | role | result |", "<!-- canary-digest: "):
            self.assertIn(needle, out)
        # the 60-line failing detail is cut to 40 lines
        self.assertIn("line 40:", out)
        self.assertNotIn("line 41:", out)

    def test_lookup_command_is_exact_title_search(self):
        p = run("break.json")  # no --existing-issues: the lookup is printed, not run
        self.assertEqual(p.returncode, 0, p.stderr)
        looks = [c for c in gh_lines(p.stdout) if c.startswith("gh issue list")]
        self.assertEqual(len(looks), 1)
        for needle in ("--repo owner/herdr-threads", "--state open", "--label harness-canary",
                       "in:title", "--json number,title,body"):
            self.assertIn(needle, looks[0])

    def test_same_digest_on_open_issue_is_noop(self):
        out = self.ok("break.json", "break-same-digest.json")
        self.assertEqual([c for c in gh_lines(out) if c.startswith(("gh issue create", "gh issue comment", "gh label"))], [])
        self.assertIn("unchanged", out)

    def test_changed_digest_comments_once(self):
        out = self.ok("break.json", "break-changed-digest.json")
        cmds = gh_lines(out)
        comments = [c for c in cmds if c.startswith("gh issue comment")]
        self.assertEqual(len(comments), 1)
        self.assertIn("gh issue comment 7 ", comments[0])
        self.assertFalse([c for c in cmds if c.startswith(("gh issue create", "gh label"))])

    def test_latest_comment_digest_wins_over_body(self):
        # body holds an old digest, the latest comment already carries the current one
        out = self.ok("break.json", "break-comment-current.json")
        self.assertEqual([c for c in gh_lines(out) if c.startswith(("gh issue create", "gh issue comment"))], [])

    def test_other_title_does_not_match(self):
        # an open issue for a different first_bad must not suppress the new one
        out = self.ok("break.json", "break-other-version.json")
        self.assertEqual(len([c for c in gh_lines(out) if c.startswith("gh issue create")]), 1)

    def test_inconclusive_uses_last_good_title_and_files_only_that_harness(self):
        out = self.ok("inconclusive.json", "none.json")
        creates = [c for c in gh_lines(out) if c.startswith("gh issue create")]
        self.assertEqual(len(creates), 1)
        self.assertIn("--title 'harness-canary: codex inconclusive above 0.160.0'", creates[0])
        self.assertIn("verdict: inconclusive", out)

    def test_inconclusive_falls_back_to_verified_max(self):
        out = self.ok("inconclusive-no-last-good.json", "none.json")
        creates = [c for c in gh_lines(out) if c.startswith("gh issue create")]
        self.assertEqual(len(creates), 1)
        self.assertIn("--title 'harness-canary: claude inconclusive above 2.1.283'", creates[0])

    def test_nothing_filed_for_non_break_statuses(self):
        for name in ("all_pass", "no_candidates", "infra_error", "known_broken_persists"):
            with self.subTest(report=name):
                out = self.ok(f"{name}.json", "none.json")
                self.assertEqual([c for c in gh_lines(out) if re.match(r"gh (issue (create|comment)|label)", c)], [], out)

    def test_never_closes_or_opens_prs(self):
        for name in ("break.json", "inconclusive.json"):
            for ex in ("none.json", "break-changed-digest.json"):
                out = self.ok(name, ex)
                self.assertNotRegex(out, r"gh (issue (close|edit|delete)|pr )")

    def test_non_dry_run_without_token_is_refused(self):
        cmd = [sys.executable, str(SCRIPT), "--report", str(REPORTS / "break.json"), "--repo", REPO,
               "--run-url", RUN_URL, "--existing-issues", str(EXISTING / "none.json")]
        env = {k: v for k, v in os.environ.items() if k not in ("GH_TOKEN", "GITHUB_TOKEN")}
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=60)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("GH_TOKEN", p.stderr)

    def test_write_urls_records_each_break_issue(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / "issues.json"
            self.ok("break.json", "none.json", ("--write-urls", str(path)))
            self.assertEqual(json.loads(path.read_text()), {"claude 2.1.285": "dry-run"})
            # an existing open issue is recorded by its number (fixture rows carry no url)
            self.ok("break.json", "break-same-digest.json", ("--write-urls", str(path)))
            urls = json.loads(path.read_text())
            self.assertRegex(urls["claude 2.1.285"], r"^https://github\.com/owner/herdr-threads/issues/\d+$")
            # no break: an empty document is still written (the writer reads it unconditionally)
            self.ok("all_pass.json", "none.json", ("--write-urls", str(path)))
            self.assertEqual(json.loads(path.read_text()), {})
            # inconclusive blocks have no first_bad and so no key
            self.ok("inconclusive.json", "none.json", ("--write-urls", str(path)))
            self.assertEqual(json.loads(path.read_text()), {})

    def test_digest_ignores_timings_but_not_results(self):
        file_issues = load_module()
        rep = json.loads((REPORTS / "break.json").read_text())
        block = rep["harnesses"][0]
        d1 = file_issues.digest(block)
        for p in block["probes"]:
            for a in p["attempts"]:
                a["duration_ms"] += 999
        self.assertEqual(file_issues.digest(block), d1)
        block["probes"][0]["result"] = "pass"
        self.assertNotEqual(file_issues.digest(block), d1)

    def test_fixture_digests_match_the_report(self):
        file_issues = load_module()
        block = json.loads((REPORTS / "break.json").read_text())["harnesses"][0]
        want = file_issues.digest(block)
        same = json.loads((EXISTING / "break-same-digest.json").read_text())
        self.assertIn(f"<!-- canary-digest: {want} -->", same[0]["body"])


if __name__ == "__main__":
    unittest.main()
