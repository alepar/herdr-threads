"""Tests of scripts/linux-e2e/run without Docker: a fake `docker` on PATH records every call.

Covers argument parsing, the credential preflight, container removal on success, failure and SIGTERM,
credential staging (env-file mode and lifetime, nothing in the evidence) and `clean`'s label scoping.
Run: python3 -m unittest discover -s scripts/tests -p test_linux_e2e_run.py  (or scripts/linux-e2e/run --self-test)
"""
from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

RUN = Path(__file__).resolve().parent.parent / "linux-e2e" / "run"
STEP_SCRIPT = "/home/tester/src/scripts/linux-e2e/in-container.sh"
SECRET = "sk-fake-token-for-the-runner-self-test"

FAKE_DOCKER = textwrap.dedent(r'''
    #!/usr/bin/env python3
    import json, os, stat, sys, time
    argv = sys.argv[1:]
    rec = {"argv": argv}
    step = None
    if argv[:1] == ["exec"]:
        if "-i" in argv:
            rec["stdin_bytes"] = len(sys.stdin.buffer.read())
        if "--env-file" in argv:
            path = argv[argv.index("--env-file") + 1]
            st = os.stat(path)
            rec["env_file"] = {"path": path, "mode": oct(stat.S_IMODE(st.st_mode)),
                               "keys": [l.split("=", 1)[0] for l in open(path).read().splitlines()]}
        script = "''' + STEP_SCRIPT + r'''"
        if script in argv:
            step = argv[argv.index(script) + 1]
            rec["step"] = step
    with open(os.environ["FAKE_DOCKER_LOG"], "a") as log:
        log.write(json.dumps(rec) + "\n")
    cmd = argv[0] if argv else ""
    rcs = json.loads(os.environ.get("FAKE_DOCKER_RC", "{}"))
    if cmd == "info":
        print("aarch64")
    elif cmd == "image":
        print("sha256:fake")
    elif cmd == "volume" and argv[1] == "create":
        print(argv[-1])
    elif cmd == "volume" and argv[1] == "ls":
        print(os.environ.get("FAKE_DOCKER_VOLUMES", ""))
    elif cmd == "images":
        print(os.environ.get("FAKE_DOCKER_IMAGES", ""))
    elif cmd == "ps":
        if "name=" not in " ".join(argv):
            print(os.environ.get("FAKE_DOCKER_CONTAINERS", ""))
    elif cmd == "run":
        print("fakecontainerid")
    elif cmd == "cp":
        dest = argv[-1]
        os.makedirs(dest, exist_ok=True)
        open(os.path.join(dest, "copied.txt"), "w").write("from the container\n")
        for name in ("doctor-outside", "doctor-pane"):
            open(os.path.join(dest, name + ".txt"), "w").write(os.environ.get("FAKE_DOCKER_DOCTOR", ""))
    elif step is not None:
        if step == os.environ.get("FAKE_DOCKER_HANG"):
            print("hanging", flush=True)
            time.sleep(60)
        sys.exit(rcs.get(step, 0))
    sys.exit(0)
''').lstrip()


def git(repo, *argv):
    subprocess.run(["git", "-C", str(repo), *argv], check=True, capture_output=True)


class Fake(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="ht-linux-e2e-test."))
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.tmp)])
        bindir = self.tmp / "bin"
        bindir.mkdir()
        docker = bindir / "docker"
        docker.write_text(FAKE_DOCKER.replace("#!/usr/bin/env python3", "#!" + sys.executable, 1))
        docker.chmod(0o755)
        self.repo = self.tmp / "repo"
        (self.repo / "scripts/linux-e2e").mkdir(parents=True)
        (self.repo / "tests").mkdir()
        (self.repo / "scripts/linux-e2e/Dockerfile").write_text("FROM scratch\n")
        (self.repo / "tests/herdr-releases.tsv").write_text(
            "# header\n0.9.1\therdr-linux-aarch64\tabc\n0.9.2\therdr-linux-aarch64\tdef\n")
        git(self.repo, "init", "-q")
        git(self.repo, "add", "-A")
        git(self.repo, "-c", "user.name=t", "-c", "user.email=t@invalid", "commit", "-q", "-m", "fixture")
        self.log = self.tmp / "docker.log"
        self.evidence = self.tmp / "evidence"
        self.env = dict(os.environ, PATH=f"{bindir}{os.pathsep}{os.environ['PATH']}",
                        FAKE_DOCKER_LOG=str(self.log), HT_LINUX_E2E_REPO=str(self.repo), TMPDIR=str(self.tmp))
        self.env.pop("HT_LINUX_E2E_DOCKER", None)

    def run_cli(self, *argv, env=None, evidence=True):
        args = [sys.executable, str(RUN), *argv]
        if evidence and argv and argv[0] not in ("clean",):  # right after the subcommand: never past a `--`
            args[3:3] = ["--evidence-dir", str(self.evidence)]
        return subprocess.run(args, env=dict(self.env, **(env or {})), capture_output=True, text=True, timeout=60)

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def steps(self):
        return [c.get("step") for c in self.calls() if c.get("step")]

    def removed(self):
        return [c["argv"] for c in self.calls() if c["argv"][:3] == ["rm", "-f", "-v"]]

    def facts(self):
        return json.loads((self.evidence / "runner.json").read_text())


class Parsing(Fake):
    def test_a_subcommand_is_required(self):
        r = self.run_cli(evidence=False)
        self.assertEqual(r.returncode, 2)
        self.assertIn("a subcommand is required", r.stderr)

    def test_unknown_subcommand(self):
        self.assertEqual(self.run_cli("nope", evidence=False).returncode, 2)

    def test_runner_owned_driver_options_are_refused(self):
        for owned in ("--bin", "--codex-auth-home=/x", "--self-test"):
            r = self.run_cli("wake-e2e", "--", owned)
            self.assertEqual(r.returncode, 2, owned)
            self.assertIn("sets these driver options itself", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_unlisted_herdr_version_is_refused_before_docker(self):
        r = self.run_cli("release-smoke", "--versions", "0.9.1", "9.9.9")
        self.assertEqual(r.returncode, 2)
        self.assertIn("9.9.9", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_unknown_harness_is_refused(self):
        r = self.run_cli("wake-e2e", "--", "--harness", "claude,hermes")
        self.assertEqual(r.returncode, 2)
        self.assertIn("unknown harness: hermes", r.stderr)
        self.assertEqual(self.calls(), [])


class Lifecycle(Fake):
    def test_happy_path_exports_source_and_removes_the_container(self):
        r = self.run_cli("release-smoke", "--versions", "0.9.2")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        calls = [c["argv"] for c in self.calls()]
        run = next(a for a in calls if a[:1] == ["run"])
        self.assertIn("--init", run)
        self.assertIn("dev.herdr-threads.linux-e2e=1", run)
        self.assertNotIn("--privileged", run)
        self.assertFalse(any(m.startswith(("/Users", "/home", "/root", str(self.tmp))) for m in run
                             if ":" in m and not m.startswith("ht-linux-e2e-runner")), run)
        export = next(c for c in self.calls() if "tar -x" in " ".join(c["argv"]))
        self.assertGreater(export["stdin_bytes"], 0)
        sha = subprocess.run(["git", "-C", str(self.repo), "rev-parse", "HEAD"], capture_output=True,
                             text=True).stdout.strip()
        prep = next(c["argv"] for c in self.calls() if c.get("step") == "prepare-src")
        self.assertEqual(prep[-2], sha)
        commit = subprocess.run(["git", "-C", str(self.repo), "cat-file", "commit", "HEAD"], capture_output=True).stdout
        staged = next(c for c in self.calls() if "export-commit" in " ".join(c["argv"]))
        self.assertEqual(staged["stdin_bytes"], len(commit))  # the raw commit object prepare-src rehashes
        self.assertEqual(self.steps(), ["prepare-src", "release-smoke"])
        self.assertEqual(next(c["argv"] for c in self.calls() if c.get("step") == "release-smoke")[-1], "0.9.2")
        self.assertEqual(len(self.removed()), 1)
        self.assertTrue((self.evidence / "container" / "copied.txt").exists())
        self.assertTrue(self.facts()["container_removed"])

    def test_cache_volumes_are_labelled_and_per_arch(self):
        self.assertEqual(self.run_cli("self-test").returncode, 0)
        creates = [c["argv"] for c in self.calls() if c["argv"][:2] == ["volume", "create"]]
        self.assertEqual(len(creates), 3)
        for argv in creates:
            self.assertIn("dev.herdr-threads.linux-e2e=1", argv)
            self.assertTrue(argv[-1].endswith("-arm64"), argv)

    def test_no_cache_creates_no_volume(self):
        self.assertEqual(self.run_cli("self-test", "--no-cache").returncode, 0)
        self.assertFalse(any(c["argv"][:1] == ["volume"] for c in self.calls()))
        run = next(c["argv"] for c in self.calls() if c["argv"][:1] == ["run"])
        self.assertNotIn("-v", run)

    def test_a_failing_step_still_removes_the_container(self):
        r = self.run_cli("self-test", env={"FAKE_DOCKER_RC": json.dumps({"self-test": 1})})
        self.assertEqual(r.returncode, 1)
        self.assertEqual(len(self.removed()), 1)
        self.assertTrue((self.evidence / "container" / "copied.txt").exists())

    def test_a_tree_mismatch_fails(self):
        r = self.run_cli("self-test", env={"FAKE_DOCKER_RC": json.dumps({"prepare-src": 1})})
        self.assertEqual(r.returncode, 1)
        self.assertNotIn("self-test", self.steps())
        self.assertEqual(len(self.removed()), 1)

    def test_sigterm_removes_the_container(self):
        p = subprocess.Popen([sys.executable, str(RUN), "self-test", "--evidence-dir", str(self.evidence)],
                             env=dict(self.env, FAKE_DOCKER_HANG="self-test"), stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True)
        deadline = time.monotonic() + 30
        while "self-test" not in self.steps():
            self.assertLess(time.monotonic(), deadline, "the hanging step never started")
            time.sleep(0.05)
        time.sleep(0.3)
        p.send_signal(signal.SIGTERM)
        out, err = p.communicate(timeout=30)
        self.assertEqual(p.returncode, 128 + signal.SIGTERM, out + err)
        self.assertIn("interrupted", err)
        self.assertEqual(len(self.removed()), 1)
        self.assertTrue(self.facts()["container_removed"])

    def test_doctor_verdict_reads_both_reports(self):
        ok = "doctor: degraded\ndaemon: healthy\n"
        r = self.run_cli("doctor", env={"FAKE_DOCKER_DOCTOR": ok})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.steps(), ["prepare-src", "herdr-up", "build-release", "install-release",
                                        "daemon-wait", "doctor-outside", "pane-run"])
        r = self.run_cli("doctor", env={"FAKE_DOCKER_DOCTOR": "doctor: unavailable\ndaemon: unreachable\n"})
        self.assertEqual(r.returncode, 1)

    def test_hide_harnesses_reaches_every_step(self):
        self.assertEqual(self.run_cli("doctor", "--hide-harnesses",
                                      env={"FAKE_DOCKER_DOCTOR": "doctor: ok\ndaemon: healthy\n"}).returncode, 0)
        for c in self.calls():
            if c.get("step"):
                self.assertIn("HT_HIDE_HARNESSES=1", c["argv"], c["step"])


class Credentials(Fake):
    def secret_file(self, name, text=SECRET + "\n"):
        path = self.tmp / name
        path.write_text(text)
        path.chmod(0o600)
        return path

    def assert_no_secret_in_evidence(self):
        for f in self.evidence.rglob("*"):
            if f.is_file():
                self.assertNotIn(SECRET, f.read_text(errors="replace"), f)

    def test_no_credentials_prepares_then_refuses_in_the_pane(self):
        r = self.run_cli("wake-e2e", env={"FAKE_DOCKER_RC": json.dumps({"pane-run": 2})})
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("no credentials for claude", r.stdout)
        self.assertIn("refused by the in-pane preflight", r.stdout)
        names = [c["argv"] for c in self.calls()]
        self.assertFalse(any("--env-file" in a for a in names))
        self.assertFalse(any("auth.json" in " ".join(a) or "/run/live" in " ".join(a) for a in names))
        self.assertEqual(self.steps(), ["prepare-src", "herdr-up", "build-bin", "harness-snapshot", "pane-run",
                                        "harness-snapshot"])
        self.assertEqual(len(self.removed()), 1)

    def test_credentials_without_live_are_refused_before_docker(self):
        codex = self.secret_file("auth.json", json.dumps({"tokens": SECRET}))
        r = self.run_cli("wake-e2e", "--codex-auth-file", str(codex), "--", "--harness", "codex")
        self.assertEqual(r.returncode, 2)
        self.assertIn("without --live", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_missing_or_bad_credential_files_are_refused(self):
        r = self.run_cli("wake-e2e", "--live", "--claude-oauth-token-file", str(self.tmp / "absent"))
        self.assertEqual(r.returncode, 2)
        self.assertIn("cannot read", r.stderr)
        two_lines = self.secret_file("token", "a\nb\n")
        r = self.run_cli("wake-e2e", "--live", "--claude-oauth-token-file", str(two_lines))
        self.assertEqual(r.returncode, 2)
        self.assertIn("exactly one line", r.stderr)
        not_json = self.secret_file("auth.json", "not json")
        r = self.run_cli("wake-e2e", "--live", "--codex-auth-file", str(not_json))
        self.assertEqual(r.returncode, 2)
        self.assertEqual(self.calls(), [])

    def test_live_credentials_are_staged_privately(self):
        token = self.secret_file("token")
        codex = self.secret_file("auth.json", json.dumps({"tokens": SECRET}))
        r = self.run_cli("wake-e2e", "--live", "--claude-oauth-token-file", str(token),
                         "--codex-auth-file", str(codex), "--", "--cases", "invite/group")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        with_env = [c for c in self.calls() if "env_file" in c]
        self.assertEqual([c["step"] for c in with_env], ["herdr-up"])  # only the Herdr server gets it
        info = with_env[0]["env_file"]
        self.assertEqual(info["mode"], "0o600")
        self.assertEqual(info["keys"], ["CLAUDE_CODE_OAUTH_TOKEN"])
        self.assertFalse(Path(info["path"]).exists(), "the env-file outlived the run")
        run = next(c["argv"] for c in self.calls() if c["argv"][:1] == ["run"])
        self.assertNotIn("--env-file", run)
        stage = next(c for c in self.calls() if "auth.json" in " ".join(c["argv"]))
        self.assertIn("umask 077", " ".join(stage["argv"]))
        self.assertEqual(stage["stdin_bytes"], codex.stat().st_size)
        args = next(c for c in self.calls() if "driver-args" in " ".join(c["argv"]))
        self.assertEqual(args["stdin_bytes"], len(b"--cases\0invite/group\0"))
        self.assertTrue(any("/run/live" in " ".join(c["argv"]) for c in self.calls()))
        self.assertNotIn(SECRET, " ".join(" ".join(c["argv"]) for c in self.calls()))
        self.assert_no_secret_in_evidence()
        facts = self.facts()["credentials"]
        self.assertEqual(facts["claude"], "CLAUDE_CODE_OAUTH_TOKEN")
        self.assertTrue(facts["codex_auth_json"])


class Clean(Fake):
    def test_clean_removes_only_labelled_objects(self):
        env = {"FAKE_DOCKER_CONTAINERS": "c1", "FAKE_DOCKER_VOLUMES": "v1\nv2", "FAKE_DOCKER_IMAGES": "i1"}
        r = self.run_cli("clean", env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        calls = [c["argv"] for c in self.calls()]
        for argv in calls:
            if argv[:1] in (["ps"], ["images"]) or argv[:2] == ["volume", "ls"]:
                self.assertIn("label=dev.herdr-threads.linux-e2e", argv)
        self.assertIn(["rm", "-f", "-v", "c1"], calls)
        self.assertIn(["volume", "rm", "-f", "v1", "v2"], calls)
        self.assertIn(["rmi", "-f", "i1"], calls)
        images_ls = next(a for a in calls if a[:1] == ["images"])
        self.assertIn("reference=ht-linux-e2e-runner", images_ls)

    def test_keep_image(self):
        r = self.run_cli("clean", "--keep-image", env={"FAKE_DOCKER_IMAGES": "i1"})
        self.assertEqual(r.returncode, 0)
        self.assertFalse(any(c["argv"][:1] in (["images"], ["rmi"]) for c in self.calls()))


if __name__ == "__main__":
    unittest.main()
