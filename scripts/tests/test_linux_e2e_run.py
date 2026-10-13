"""Tests of scripts/linux-e2e/run without Docker: a fake `docker` on PATH records every call.

Covers argument parsing (driver option ownership and abbreviations), the credential preflight, container
lifecycle (watchdog command, lease heartbeat, stop -> copy -> remove on success, failure, SIGINT, SIGTERM,
SIGHUP and a second signal during cleanup), credential staging (env-file mode and lifetime, nothing in the
evidence), `clean`'s scoping, and the in-container launcher's preflight (scripts/linux-e2e/in-container.sh
wake-e2e-pane, run with bash against a scratch home and a fake python3).
Run: python3 -m unittest discover -s scripts/tests -p test_linux_e2e_run.py  (or scripts/linux-e2e/run --self-test)
"""
from __future__ import annotations

import datetime
import fcntl
import hashlib
import importlib.machinery
import importlib.util
import io
import json
import os
import signal
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

RUN = Path(__file__).resolve().parent.parent / "linux-e2e" / "run"
IN_CONTAINER = RUN.parent / "in-container.sh"
IMAGE_FETCH = RUN.parent / "image-fetch.sh"
DOCKERFILE = RUN.parent / "Dockerfile"
STEP_SCRIPT = "/home/tester/src/scripts/linux-e2e/in-container.sh"
SECRET = "sk-ant-oat01-fake-token-for-the-runner-self-test"

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
    def mark(name):
        path = os.environ.get("FAKE_DOCKER_MARK_" + name)
        if path:
            open(path, "w").write("1")
    if cmd == "info":
        print("aarch64")
    elif cmd == "build":
        # Every build is a new immutable image; the shared tag moves to the newest one.
        state = os.environ["FAKE_DOCKER_STATE"]
        image = "sha256:" + os.urandom(8).hex()
        open(argv[argv.index("--iidfile") + 1], "w").write(image + "\n")
        open(os.path.join(state, "tag-" + argv[argv.index("-t") + 1].replace(":", "_")), "w").write(image)
    elif cmd == "inspect":
        fmt = argv[argv.index("-f") + 1]
        if fmt == "{{.Image}}":
            print(open(os.path.join(os.environ["FAKE_DOCKER_STATE"], "image-of-" + argv[-1])).read())
        elif fmt == "{{.State.ExitCode}}":
            print(os.environ.get("FAKE_DOCKER_EXIT_CODE", "0"))
        else:
            finished = json.loads(os.environ.get("FAKE_DOCKER_FINISHED", "{}"))
            print(finished.get(argv[-1], "2020-01-01T00:00:00.000000000Z"))
    elif cmd == "volume" and argv[1] == "create":
        print(argv[-1])
    elif cmd == "volume" and argv[1] == "ls":
        print(os.environ.get("FAKE_DOCKER_VOLUMES", ""))
    elif cmd == "images":
        print(os.environ.get("FAKE_DOCKER_IMAGES", ""))
    elif cmd == "ps":
        if "status=exited" in argv:
            print(os.environ.get("FAKE_DOCKER_EXITED", ""))
        elif "name=" not in " ".join(argv):
            print(os.environ.get("FAKE_DOCKER_CONTAINERS", ""))
    elif argv[:1] == ["exec"] and argv[-2:] == ["touch", "/home/tester/run/lease"]:
        print("lease renewal refused", file=sys.stderr)
        sys.exit(int(os.environ.get("FAKE_DOCKER_TOUCH_RC", "0")))
    elif cmd == "run":
        # The image is the word before the container command (`bash -c WATCHDOG`); a tag resolves to its image now.
        image = argv[argv.index("bash") - 1]
        tagged = os.path.join(os.environ["FAKE_DOCKER_STATE"], "tag-" + image.replace(":", "_"))
        if os.path.exists(tagged):
            image = open(tagged).read()
        name = argv[argv.index("--name") + 1]
        open(os.path.join(os.environ["FAKE_DOCKER_STATE"], "image-of-" + name), "w").write(image)
        print("fakecontainerid")
    elif cmd == "stop":
        mark("STOP")
        time.sleep(float(os.environ.get("FAKE_DOCKER_STOP_S", "0")))
    elif cmd == "cp":
        if os.environ.get("FAKE_DOCKER_CP_RC"):
            print("simulated copy failure: no space left on device", file=sys.stderr)
            sys.exit(int(os.environ["FAKE_DOCKER_CP_RC"]))
        dest = argv[-1]
        os.makedirs(dest, exist_ok=True)
        open(os.path.join(dest, "copied.txt"), "w").write("from the container\n")
        for name in ("doctor-outside", "doctor-pane"):
            open(os.path.join(dest, name + ".txt"), "w").write(os.environ.get("FAKE_DOCKER_DOCTOR", ""))
    elif step is not None:
        if step == os.environ.get("FAKE_DOCKER_HANG"):
            mark("HANG")
            time.sleep(60)
        sys.exit(rcs.get(step, 0))
    sys.exit(0)
''').lstrip()


def git(repo, *argv):
    return subprocess.run(["git", "-C", str(repo), *argv], check=True, capture_output=True).stdout


def age(path, seconds=3600):
    os.utime(path, (time.time() - seconds, time.time() - seconds))


def wait_for(path, what, seconds=30):
    deadline = time.monotonic() + seconds
    while not Path(path).exists():
        if time.monotonic() > deadline:
            raise AssertionError(f"{what} never happened")
        time.sleep(0.02)


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
        (self.tmp / "state").mkdir()
        self.evidence = self.tmp / "evidence"
        self.env = dict(os.environ, PATH=f"{bindir}{os.pathsep}{os.environ['PATH']}",
                        FAKE_DOCKER_LOG=str(self.log), HT_LINUX_E2E_REPO=str(self.repo), TMPDIR=str(self.tmp),
                        FAKE_DOCKER_STATE=str(self.tmp / "state"))
        self.env.pop("HT_LINUX_E2E_DOCKER", None)

    def cli(self, *argv, evidence=True):
        args = [sys.executable, str(RUN), *argv]
        if evidence and argv and argv[0] != "clean":  # right after the subcommand: never past a `--`
            args[3:3] = ["--evidence-dir", str(self.evidence)]
        return args

    def run_cli(self, *argv, env=None, evidence=True):
        return subprocess.run(self.cli(*argv, evidence=evidence), env=dict(self.env, **(env or {})),
                              capture_output=True, text=True, timeout=60)

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def argvs(self):
        return [c["argv"] for c in self.calls()]

    def steps(self):
        return [c.get("step") for c in self.calls() if c.get("step")]

    def removed(self):
        return [a for a in self.argvs() if a[:3] == ["rm", "-f", "-v"]]

    def facts(self):
        return json.loads((self.evidence / "runner.json").read_text())


class Parsing(Fake):
    def test_a_subcommand_is_required(self):
        r = self.run_cli(evidence=False)
        self.assertEqual(r.returncode, 2)
        self.assertIn("a subcommand is required", r.stderr)

    def test_unknown_subcommand(self):
        self.assertEqual(self.run_cli("nope", evidence=False).returncode, 2)

    def test_runner_options_are_not_abbreviated(self):
        r = self.run_cli("wake-e2e", "--liv")
        self.assertEqual(r.returncode, 2)

    def test_runner_owned_driver_options_are_refused(self):
        for owned in ("--bin", "--codex-auth-home=/x", "--self-test", "--evidence-dir=/x"):
            r = self.run_cli("wake-e2e", "--", owned, *([] if "=" in owned or owned == "--self-test" else ["v"]))
            self.assertEqual(r.returncode, 2, owned)
            self.assertIn("sets this driver option itself", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_driver_option_abbreviations_are_refused(self):
        for abbrev in (["--codex-auth-h", "/x"], ["--harn", "codex"], ["--evid=/x"], ["--ca", "x"]):
            r = self.run_cli("wake-e2e", "--", *abbrev)
            self.assertEqual(r.returncode, 2, abbrev)
            self.assertIn("abbreviations are refused", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_unlisted_herdr_version_is_refused_before_docker(self):
        r = self.run_cli("release-smoke", "--versions", "0.9.1", "9.9.9")
        self.assertEqual(r.returncode, 2)
        self.assertIn("9.9.9", r.stderr)
        self.assertFalse(any(a[:1] in (["run"], ["build"]) for a in self.argvs()))

    def test_unknown_harness_is_refused(self):
        r = self.run_cli("wake-e2e", "--", "--harness", "claude,hermes")
        self.assertEqual(r.returncode, 2)
        self.assertIn("unknown harness: hermes", r.stderr)
        self.assertEqual(self.calls(), [])


class Lifecycle(Fake):
    def test_happy_path(self):
        r = self.run_cli("release-smoke", "--versions", "0.9.2")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        run = next(a for a in self.argvs() if a[:1] == ["run"])
        for flag in ("--init", "dev.herdr-threads.linux-e2e=1", "--cap-drop", "no-new-privileges"):
            self.assertIn(flag, run)
        self.assertTrue(any(a.startswith("dev.herdr-threads.linux-e2e.worktree=") for a in run))
        self.assertTrue(any(a.startswith("dev.herdr-threads.linux-e2e.run=") for a in run))
        self.assertNotIn("--privileged", run)
        self.assertEqual(run[-3:-1], ["bash", "-c"])  # the watchdog, not `sleep infinity`
        self.assertIn("HT_LE2E_LEASE_S", run[-1])
        mounts = [run[i + 1] for i, a in enumerate(run) if a == "-v"]
        self.assertTrue(all(m.split(":", 1)[0].startswith("ht-linux-e2e-runner-") for m in mounts), mounts)
        export = next(c for c in self.calls() if "tar -x" in " ".join(c["argv"]))
        self.assertGreater(export["stdin_bytes"], 0)
        sha = git(self.repo, "rev-parse", "HEAD").decode().strip()
        prep = next(c["argv"] for c in self.calls() if c.get("step") == "prepare-src")
        self.assertEqual(prep[-2], sha)
        commit = git(self.repo, "cat-file", "commit", "HEAD")
        staged = next(c for c in self.calls() if "export-commit" in " ".join(c["argv"]))
        self.assertEqual(staged["stdin_bytes"], len(commit))  # the raw commit object prepare-src rehashes
        self.assertEqual(self.steps(), ["prepare-src", "release-smoke"])
        self.assertEqual(next(c["argv"] for c in self.calls() if c.get("step") == "release-smoke")[-1], "0.9.2")
        order = [a[0] for a in self.argvs() if a[:1] in (["stop"], ["cp"], ["rm"])]
        self.assertEqual(order, ["stop", "cp", "rm"])  # agents stopped before the evidence is copied
        self.assertEqual(len(self.removed()), 1)
        self.assertTrue((self.evidence / "container" / "copied.txt").exists())
        self.assertTrue(self.facts()["container_removed"])

    def test_archives_the_resolved_sha(self):
        self.assertEqual(self.run_cli("self-test").returncode, 0)
        log = (self.evidence / "host").glob("*export-source.log")
        self.assertTrue(list(log))
        sha = git(self.repo, "rev-parse", "HEAD").decode().strip()
        self.assertEqual(self.facts()["sha"], sha)

    def test_lease_heartbeat(self):
        p = subprocess.Popen(self.cli("self-test"), env=dict(self.env, FAKE_DOCKER_HANG="self-test",
                                                               HT_LINUX_E2E_LEASE_S="4",
                                                               FAKE_DOCKER_MARK_HANG=str(self.tmp / "hang")),
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        wait_for(self.tmp / "hang", "the hanging step")
        deadline = time.monotonic() + 20
        while not any(a[-2:] == ["touch", "/home/tester/run/lease"] for a in self.argvs()):
            self.assertLess(time.monotonic(), deadline, "no lease heartbeat")
            time.sleep(0.1)
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=30)

    def test_lease_failures_are_logged(self):
        p = subprocess.Popen(self.cli("self-test"), env=dict(self.env, FAKE_DOCKER_HANG="self-test",
                                                               HT_LINUX_E2E_LEASE_S="4", FAKE_DOCKER_TOUCH_RC="1",
                                                               FAKE_DOCKER_MARK_HANG=str(self.tmp / "hang")),
                             stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        wait_for(self.tmp / "hang", "the hanging step")
        wait_for(self.evidence / "host" / "lease.log", "a logged lease failure")
        p.send_signal(signal.SIGTERM)
        _, err = p.communicate(timeout=30)
        self.assertIn("lease renewal failed", err)
        lease = self.facts()["lease"]
        self.assertGreaterEqual(lease["failures"], 1)
        self.assertIn("lease renewal refused", lease["last_error"])

    def test_exited_containers_of_this_worktree_are_removed_with_their_evidence(self):
        live_lock = self.tmp / "ht-linux-e2e-run.liverun.lock"  # TMPDIR is self.tmp
        fd = os.open(live_lock, os.O_RDWR | os.O_CREAT)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX)
        recent = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.0Z")
        exited = "\n".join(["ht-linux-e2e-old\toldrun", "ht-linux-e2e-live\tliverun", "ht-linux-e2e-recent\tx"])
        r = self.run_cli("self-test", env={"FAKE_DOCKER_EXITED": exited,
                                           "FAKE_DOCKER_FINISHED": json.dumps({"ht-linux-e2e-recent": recent})})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(["rm", "-v", "ht-linux-e2e-old"], self.argvs())
        self.assertNotIn(["rm", "-v", "ht-linux-e2e-live"], self.argvs())  # its runner is alive
        self.assertNotIn(["rm", "-v", "ht-linux-e2e-recent"], self.argvs())  # stopped just now
        ps = next(a for a in self.argvs() if "status=exited" in a)
        self.assertTrue(any(x.startswith("label=dev.herdr-threads.linux-e2e.worktree=") for x in ps))
        self.assertTrue((self.evidence / "leftovers" / "ht-linux-e2e-old" / "copied.txt").exists())
        self.assertEqual(self.facts()["leftovers_removed"], ["ht-linux-e2e-old"])

    def test_cache_volumes_are_labelled_per_arch_and_worktree(self):
        self.assertEqual(self.run_cli("self-test").returncode, 0)
        creates = [a for a in self.argvs() if a[:2] == ["volume", "create"]]
        self.assertEqual(len(creates), 3)
        for argv in creates:
            self.assertIn("dev.herdr-threads.linux-e2e=1", argv)
            self.assertTrue(any(a.startswith("dev.herdr-threads.linux-e2e.worktree=") for a in argv))
            self.assertIn("-arm64-", argv[-1])

    def test_no_cache_creates_no_volume(self):
        self.assertEqual(self.run_cli("self-test", "--no-cache").returncode, 0)
        self.assertFalse(any(a[:1] == ["volume"] for a in self.argvs()))
        self.assertNotIn("-v", next(a for a in self.argvs() if a[:1] == ["run"]))

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

    def interrupted(self, *signals, stop_s="0"):
        marks = {"FAKE_DOCKER_MARK_HANG": str(self.tmp / "hang"), "FAKE_DOCKER_MARK_STOP": str(self.tmp / "stop")}
        p = subprocess.Popen(self.cli("self-test"), env=dict(self.env, FAKE_DOCKER_HANG="self-test",
                                                               FAKE_DOCKER_STOP_S=stop_s, **marks),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        wait_for(self.tmp / "hang", "the hanging step")
        p.send_signal(signals[0])
        for sig in signals[1:]:
            wait_for(self.tmp / "stop", "docker stop")
            p.send_signal(sig)
        out, err = p.communicate(timeout=30)
        return p.returncode, out, err

    def test_signals_remove_the_container(self):
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            with self.subTest(sig=sig):
                self.log.unlink(missing_ok=True)
                for m in ("hang", "stop"):
                    (self.tmp / m).unlink(missing_ok=True)
                rc, out, err = self.interrupted(sig)
                self.assertEqual(rc, 128 + sig, out + err)
                self.assertIn("interrupted", err)
                self.assertNotIn("Traceback", err)
                self.assertEqual(len(self.removed()), 1)
                self.assertTrue(self.facts()["container_removed"])

    def test_a_second_signal_during_cleanup_is_ignored(self):
        rc, out, err = self.interrupted(signal.SIGTERM, signal.SIGINT, stop_s="1.5")
        self.assertEqual(rc, 128 + signal.SIGTERM, out + err)
        self.assertNotIn("Traceback", err)
        self.assertIn("already stopping", err)
        self.assertEqual(len(self.removed()), 1)

    def test_leftovers_are_reported(self):
        stale = self.tmp / "ht-linux-e2e-cred.999999.abc"
        stale.mkdir()
        (stale / "lock").write_text("")  # nobody holds it
        age(stale)
        r = self.run_cli("self-test", env={"FAKE_DOCKER_CONTAINERS": "ht-linux-e2e-old (Exited (3))"})
        self.assertEqual(r.returncode, 0)
        self.assertIn("labelled containers exist", r.stderr)
        self.assertIn(str(stale), r.stderr)

    def test_doctor_verdict_reads_both_reports(self):
        r = self.run_cli("doctor", env={"FAKE_DOCKER_DOCTOR": "doctor: degraded\ndaemon: healthy\n"})
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
        self.assertFalse(any("--env-file" in a for a in self.argvs()))
        staged = [" ".join(a) for a in self.argvs() if a[:2] == ["exec", "-i"]]
        self.assertFalse(any(".ht-creds" in a or "/run/live" in a for a in staged), staged)
        self.assertEqual(self.steps(), ["prepare-src", "build-bin", "herdr-up", "harness-snapshot", "pane-run",
                                        "harness-snapshot"])
        self.assertEqual(len(self.removed()), 1)

    def test_credentials_without_live_are_refused_before_docker(self):
        codex = self.secret_file("auth.json", json.dumps({"tokens": SECRET}))
        r = self.run_cli("wake-e2e", "--codex-auth-file", str(codex), "--", "--harness", "codex")
        self.assertEqual(r.returncode, 2)
        self.assertIn("without --live", r.stderr)
        self.assertFalse(any(a[:1] in (["run"], ["build"]) for a in self.argvs()))

    def test_live_refuses_a_dirty_tree_unless_allowed(self):
        (self.repo / "untracked.txt").write_text("x")
        codex = self.secret_file("auth.json", json.dumps({"tokens": SECRET}))
        r = self.run_cli("wake-e2e", "--live", "--codex-auth-file", str(codex), "--", "--harness", "codex")
        self.assertEqual(r.returncode, 2)
        self.assertIn("uncommitted changes", r.stderr)
        self.assertFalse(any(a[:1] in (["run"], ["build"]) for a in self.argvs()))
        r = self.run_cli("wake-e2e", "--live", "--allow-dirty", "--codex-auth-file", str(codex), "--",
                         "--harness", "codex")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_bad_credential_files_are_refused(self):
        cases = [
            (["--claude-oauth-token-file", str(self.tmp / "absent")], "cannot read"),
            (["--claude-oauth-token-file", str(self.secret_file("t1", "a\nb\n"))], "exactly one token"),
            (["--claude-oauth-token-file", str(self.secret_file("t2", '{"token": "x"}'))], "looks like JSON"),
            (["--claude-oauth-token-file", str(self.secret_file("t3", "CLAUDE_CODE_OAUTH_TOKEN=x"))], "one token"),
            (["--claude-oauth-token-file", str(self.secret_file("t4", "sk-ant-api03-x"))], "setup-token"),
            (["--anthropic-api-key-file", str(self.secret_file("t5", "sk-ant-api03-a sk-ant-api03-b"))], "one token"),
            (["--codex-auth-file", str(self.secret_file("a.json", "not json"))], "not JSON"),
        ]
        for argv, why in cases:
            r = self.run_cli("wake-e2e", "--live", *argv)
            self.assertEqual(r.returncode, 2, argv)
            self.assertIn(why, r.stderr, argv)
        self.assertFalse(any(a[:1] in (["run"], ["build"]) for a in self.argvs()))

    def test_live_credentials_are_staged_privately(self):
        token = self.secret_file("token")
        codex = self.secret_file("auth.json", json.dumps({"tokens": SECRET}))
        r = self.run_cli("wake-e2e", "--live", "--claude-oauth-token-file", str(token),
                         "--codex-auth-file", str(codex), "--", "--cases", "invite/group", "--harness=codex,claude")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        with_env = [c for c in self.calls() if "env_file" in c]
        self.assertEqual([c["step"] for c in with_env], ["herdr-up"])  # only the Herdr server gets it
        info = with_env[0]["env_file"]
        self.assertEqual(info["mode"], "0o600")
        self.assertEqual(info["keys"], ["CLAUDE_CODE_OAUTH_TOKEN"])
        self.assertFalse(Path(info["path"]).exists(), "the env-file outlived the run")
        self.assertIn("ht-linux-e2e-cred.", info["path"])
        self.assertNotIn("--env-file", next(a for a in self.argvs() if a[:1] == ["run"]))
        stage = next(c for c in self.calls() if "cat > /home/tester/.ht-creds/codex/auth.json" in " ".join(c["argv"]))
        self.assertIn("umask 077", " ".join(stage["argv"]))
        self.assertEqual(stage["stdin_bytes"], codex.stat().st_size)
        args = next(c for c in self.calls() if "driver-args" in " ".join(c["argv"]))
        self.assertEqual(args["stdin_bytes"], len(b"--cases\0invite/group\0--harness\0codex,claude\0"))
        self.assertTrue(any("/run/live" in " ".join(a) for a in self.argvs()))
        self.assertNotIn(SECRET, " ".join(" ".join(a) for a in self.argvs()))
        self.assert_no_secret_in_evidence()
        facts = self.facts()["credentials"]
        self.assertEqual(facts["claude"], "CLAUDE_CODE_OAUTH_TOKEN")
        self.assertTrue(facts["codex_auth_json"])


class Clean(Fake):
    def filters(self, argv):
        return [argv[i + 1] for i, a in enumerate(argv) if a == "--filter"]

    def test_default_clean_is_this_worktree_and_dangling_images(self):
        # Stale: an unheld lock, even though the name's pid (ours) is alive; or no lock file and old.
        stale = self.tmp / f"ht-linux-e2e-cred.{os.getpid()}.stale"
        stale.mkdir()
        (stale / "lock").write_text("")
        age(stale)
        old = self.tmp / "ht-linux-e2e-cred.1.old"
        old.mkdir()
        age(old)
        young = self.tmp / "ht-linux-e2e-cred.1.young"  # unlocked but within the grace: its runner may be starting
        young.mkdir()
        (young / "lock").write_text("")
        stale_run = self.tmp / "ht-linux-e2e-run.deadrun.lock"
        stale_run.write_text("")
        age(stale_run)
        # Live: a held lock (a runner mid-run), or no lock file yet but just created.
        live = self.tmp / "ht-linux-e2e-cred.999999.live"
        live.mkdir()
        fd = os.open(live / "lock", os.O_RDWR | os.O_CREAT)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX)
        age(live)  # old enough that only the held lock keeps it
        fresh = self.tmp / "ht-linux-e2e-cred.999998.fresh"
        fresh.mkdir()
        env = {"FAKE_DOCKER_CONTAINERS": "c1", "FAKE_DOCKER_VOLUMES": "v1\nv2", "FAKE_DOCKER_IMAGES": "i1"}
        r = self.run_cli("clean", env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        argvs = self.argvs()
        for argv in argvs:
            if argv[:1] == ["ps"] or argv[:2] == ["volume", "ls"]:
                self.assertTrue(any(f.startswith("label=dev.herdr-threads.linux-e2e.worktree=")
                                    for f in self.filters(argv)), argv)
        images = next(a for a in argvs if a[:1] == ["images"])
        self.assertIn("dangling=true", self.filters(images))
        self.assertIn(["rm", "-f", "-v", "c1"], argvs)
        self.assertIn(["volume", "rm", "-f", "v1", "v2"], argvs)
        self.assertIn(["rmi", "i1"], argvs)  # unforced: an image in use is kept
        self.assertFalse(stale.exists())
        self.assertFalse(old.exists())
        self.assertTrue(live.exists())
        self.assertTrue(fresh.exists())
        self.assertTrue(young.exists())
        self.assertFalse(stale_run.exists())

    def test_clean_all(self):
        r = self.run_cli("clean", "--all", env={"FAKE_DOCKER_IMAGES": "i1"})
        self.assertEqual(r.returncode, 0, r.stderr)
        for argv in self.argvs():
            if argv[:1] in (["ps"], ["images"]) or argv[:2] == ["volume", "ls"]:
                self.assertIn("label=dev.herdr-threads.linux-e2e", self.filters(argv))
        images = next(a for a in self.argvs() if a[:1] == ["images"])
        self.assertIn("reference=ht-linux-e2e-runner", self.filters(images))
        self.assertIn(["rmi", "-f", "i1"], self.argvs())

    def test_keep_image(self):
        r = self.run_cli("clean", "--all", "--keep-image", env={"FAKE_DOCKER_IMAGES": "i1"})
        self.assertEqual(r.returncode, 0)
        self.assertFalse(any(a[:1] in (["images"], ["rmi"]) for a in self.argvs()))


class InPanePreflight(unittest.TestCase):
    """in-container.sh wake-e2e-pane on the host: scratch HT_LE2E_HOME, fake python3 recording the driver call."""

    def setUp(self):
        self.home = Path(tempfile.mkdtemp(prefix="ht-le2e-home."))
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.home)])
        (self.home / "src").mkdir()
        (self.home / "run").mkdir()
        bindir = self.home / "fakebin"
        bindir.mkdir()
        self.driver_log = self.home / "driver-call"
        fake = bindir / "python3"
        fake.write_text(f"#!/bin/sh\nprintf '%s\\n' \"$@\" > {self.driver_log}\n")
        fake.chmod(0o755)
        self.env = {"PATH": f"{bindir}{os.pathsep}{os.environ['PATH']}", "HT_LE2E_HOME": str(self.home),
                    "HOME": str(self.home)}

    def launch(self, driver_args, env=None, creds=False, live=False):
        (self.home / "run" / "driver-args").write_bytes(b"".join(a.encode() + b"\0" for a in driver_args))
        if creds:
            (self.home / ".ht-creds" / "codex").mkdir(parents=True)
            (self.home / ".ht-creds" / "codex" / "auth.json").write_text("{}")
        if live:
            (self.home / "run" / "live").write_text("live\n")
        e = dict(self.env, **(env or {}))
        return subprocess.run(["bash", str(IN_CONTAINER), "wake-e2e-pane"], env=e, capture_output=True, text=True,
                              timeout=30)

    PANE = {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HT_LINUX_E2E_ENV_PROBE": "1"}

    def test_no_credentials_refuse_without_starting_the_driver(self):
        r = self.launch(["--harness", "claude,codex"], env=self.PANE)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        for why in ("claude: no credential", "codex: no credential", "live run not requested"):
            self.assertIn(why, r.stdout)
        self.assertIn("server environment reaches panes: yes", r.stdout)
        self.assertFalse(self.driver_log.exists())

    def test_credentials_without_live_refuse(self):
        r = self.launch(["--harness", "codex"], env=self.PANE, creds=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("live run not requested", r.stdout)
        self.assertNotIn("codex: no credential", r.stdout)
        self.assertFalse(self.driver_log.exists())

    def test_outside_a_pane_refuses(self):
        r = self.launch(["--harness", "codex"], creds=True, live=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("not inside a Herdr pane", r.stdout)
        self.assertFalse(self.driver_log.exists())

    def test_ready_runs_the_driver_with_runner_owned_options(self):
        r = self.launch(["--cases", "invite/group", "--harness", "codex,claude"], creds=True, live=True,
                        env=dict(self.PANE, CLAUDE_CODE_OAUTH_TOKEN=SECRET))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn(SECRET, r.stdout + r.stderr)
        call = self.driver_log.read_text().splitlines()
        self.assertEqual(call[0], "scripts/wake-e2e.py")
        self.assertEqual(call[call.index("--bin") + 1], f"{self.home}/src/bin/herdr-threads")
        self.assertEqual(call[call.index("--evidence-dir") + 1], f"{self.home}/evidence/wake-e2e")
        self.assertEqual(call[call.index("--codex-auth-home") + 1], f"{self.home}/.ht-creds/codex")
        self.assertEqual(call[-4:], ["--cases", "invite/group", "--harness", "codex,claude"])


if __name__ == "__main__":
    unittest.main()


def load_runner():
    loader = importlib.machinery.SourceFileLoader("linux_e2e_run", str(RUN))
    spec = importlib.util.spec_from_loader("linux_e2e_run", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class ImageSelection(Fake):
    def test_interleaved_builds_never_swap_the_started_image(self):
        """A builds, B builds (retagging the shared tag), A starts: A's container runs A's own image."""
        mod = load_runner()
        with mock.patch.dict(os.environ, self.env, clear=True), redirect_stdout(io.StringIO()):
            runners = []
            for name in ("a", "b"):
                args = mod.parse(["self-test", "--no-cache", "--evidence-dir", str(self.tmp / f"ev-{name}")])
                runner = mod.Runner(args)
                runner.source()
                runners.append(runner)
            a, b = runners
            image_a = a.build_image("arm64")
            image_b = b.build_image("arm64")
            self.assertNotEqual(image_a, image_b)
            tag = (self.tmp / "state" / "tag-ht-linux-e2e-runner_arm64").read_text()
            self.assertEqual(tag, image_b)  # the shared tag now names B's image
            try:
                a.start(image_a, "arm64")
            finally:
                a.beat_stop.set()
        run = next(x for x in self.argvs() if x[:1] == ["run"])
        self.assertIn(image_a, run)
        self.assertNotIn("ht-linux-e2e-runner:arm64", run)
        self.assertEqual(a.facts["image"], {"tag": "ht-linux-e2e-runner:arm64", "id": image_a,
                                            "container_image": image_a})

    def test_the_cli_starts_the_built_id(self):
        self.assertEqual(self.run_cli("self-test").returncode, 0)
        image = self.facts()["image"]
        self.assertTrue(image["id"].startswith("sha256:"))
        self.assertEqual(image["container_image"], image["id"])
        self.assertIn(image["id"], next(x for x in self.argvs() if x[:1] == ["run"]))


class EvidenceCopyFailure(Fake):
    def test_copy_failure_keeps_the_scrubbed_container_and_fails_the_run(self):
        r = self.run_cli("self-test", env={"FAKE_DOCKER_CP_RC": "1"})
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertFalse(any(a[:1] == ["rm"] for a in self.argvs()))  # the stopped container is the only copy
        facts = self.facts()
        self.assertEqual(facts["exit"], 3)
        self.assertFalse(facts["container_removed"])
        self.assertEqual(facts["kept_container"], facts["container"])
        self.assertTrue(facts["evidence_copy"].startswith("failed:"))
        self.assertEqual(facts["watchdog_exit"], "0")
        self.assertIn(f"docker cp {facts['container']}:", r.stderr)  # recovery command printed
        order = [a[0] for a in self.argvs() if a[:1] in (["stop"], ["cp"])]
        self.assertEqual(order, ["stop", "cp"])

    def test_copy_failure_with_an_unconfirmed_scrub_removes_the_container(self):
        r = self.run_cli("self-test", env={"FAKE_DOCKER_CP_RC": "1", "FAKE_DOCKER_EXIT_CODE": "137"})
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(len(self.removed()), 1)
        self.assertIn("its evidence is lost", r.stderr)
        facts = self.facts()
        self.assertTrue(facts["evidence_copy"].startswith("failed:"))
        self.assertNotIn("kept_container", facts)

    def test_a_failing_run_with_a_copy_failure_still_reports_3(self):
        r = self.run_cli("self-test", env={"FAKE_DOCKER_CP_RC": "1",
                                           "FAKE_DOCKER_RC": json.dumps({"self-test": 1})})
        self.assertEqual(r.returncode, 3)


FAKE_CURL = """#!/bin/sh
[ -z "$FAKE_CURL_RC" ] || exit "$FAKE_CURL_RC"
out=''
while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift 2 ;; *) shift ;; esac; done
cp "$FAKE_CURL_FILE" "$out"
"""
FAKE_SHA256SUM = """#!PYTHON
import hashlib, sys
ok = True
for line in sys.stdin:
    digest, path = line.rstrip("\\n").split("  ", 1)
    actual = hashlib.sha256(open(path, "rb").read()).hexdigest()
    print(path + (": OK" if actual == digest else ": FAILED"))
    ok = ok and actual == digest
sys.exit(0 if ok else 1)
"""


class ImageFetch(unittest.TestCase):
    """Negative controls for scripts/linux-e2e/image-fetch.sh (the Dockerfile's downloads) with fake curl and
    sha256sum: a failed download, a checksum mismatch or a wrong version exits non-zero and promotes nothing."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="ht-image-fetch."))
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.tmp)])
        bindir = self.tmp / "bin"
        bindir.mkdir()
        (bindir / "curl").write_text(FAKE_CURL)
        (bindir / "sha256sum").write_text(FAKE_SHA256SUM.replace("PYTHON", sys.executable))
        for f in bindir.iterdir():
            f.chmod(0o755)
        self.dest = self.tmp / "dest"
        self.dest.mkdir()
        self.env = dict(os.environ, PATH=f"{bindir}{os.pathsep}{os.environ['PATH']}", HOME=str(self.tmp / "home"))

    def artifact(self, name, content, mode=0o755):
        path = self.tmp / name
        path.write_bytes(content)
        path.chmod(mode)
        return path, hashlib.sha256(content).hexdigest()

    def fetch(self, *argv, served, curl_rc=None):
        env = dict(self.env, FAKE_CURL_FILE=str(served))
        if curl_rc:
            env["FAKE_CURL_RC"] = str(curl_rc)
        return subprocess.run(["bash", str(IMAGE_FETCH), *argv], env=env, capture_output=True, text=True, timeout=60)

    def leftovers(self):
        return sorted(p.name for p in self.dest.iterdir())

    def herdr(self, prints, digest_of=None, curl_rc=None):
        binary, digest = self.artifact("herdr-bin", f"#!/bin/sh\necho '{prints}'\n".encode())
        if digest_of is not None:
            digest = hashlib.sha256(digest_of).hexdigest()
        tsv = self.tmp / "releases.tsv"
        tsv.write_text(f"# header\n0.9.2\therdr-linux-aarch64\t{'0' * 64}\n0.9.3\therdr-linux-aarch64\t{digest}\n")
        return self.fetch("herdr", str(tsv), "aarch64", str(self.dest / "herdr"), str(self.dest / "herdr-version"),
                          served=binary, curl_rc=curl_rc)

    def test_herdr_good(self):
        r = self.herdr("herdr 0.9.3")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.leftovers(), ["herdr", "herdr-version"])
        self.assertEqual((self.dest / "herdr-version").read_text(), "0.9.3\n")

    def test_herdr_checksum_mismatch(self):
        r = self.herdr("herdr 0.9.3", digest_of=b"something else")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("checksum mismatch", r.stderr)
        self.assertEqual(self.leftovers(), [])  # no binary, no stamp, no scratch dir

    def test_herdr_download_failure(self):
        r = self.herdr("herdr 0.9.3", curl_rc=22)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("download failed", r.stderr)
        self.assertEqual(self.leftovers(), [])

    def test_herdr_wrong_version(self):
        r = self.herdr("herdr 0.9.2")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("expected 'herdr 0.9.3'", r.stderr)
        self.assertEqual(self.leftovers(), [])

    def node(self, prints, bad_digest=False):
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:xz") as tar:
            script = f"#!/bin/sh\necho {prints}\n".encode()
            info = tarfile.TarInfo("node-v22.23.3-linux-arm64/bin/node")
            info.size, info.mode = len(script), 0o755
            tar.addfile(info, io.BytesIO(script))
        tarball, digest = self.artifact("node.tar.xz", buf.getvalue(), mode=0o644)
        if bad_digest:
            digest = "0" * 64
        return self.fetch("node", "22.23.3", "arm64", digest, str(self.dest / "node"), served=tarball)

    def test_node_good(self):
        r = self.node("v22.23.3")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue((self.dest / "node" / "bin" / "node").exists())

    def test_node_checksum_mismatch(self):
        r = self.node("v22.23.3", bad_digest=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.leftovers(), [])

    def test_node_wrong_version(self):
        r = self.node("v22.0.0")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(self.leftovers(), [])

    def rustup(self, toolchain_prints, bad_digest=False):
        marker = self.tmp / "rustup-ran"
        rustc = f"#!/bin/sh\necho '{toolchain_prints}'\n"
        init = (f"#!/bin/sh\ntouch '{marker}'\nmkdir -p \"$HOME/.cargo/bin\"\n"
                f"cat > \"$HOME/.cargo/bin/rustc\" <<'EOF'\n{rustc}EOF\nchmod 755 \"$HOME/.cargo/bin/rustc\"\n").encode()
        served, digest = self.artifact("rustup-init", init)
        if bad_digest:
            digest = "0" * 64
        r = self.fetch("rustup", "1.29.1", "aarch64", digest, "1.94.0", "aarch64-unknown-linux-musl", served=served)
        return r, marker

    def test_rustup_good(self):
        r, marker = self.rustup("rustc 1.94.0 (4a4ef493e 2026-03-02)")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(marker.exists())

    def test_rustup_checksum_mismatch_never_executes(self):
        r, marker = self.rustup("rustc 1.94.0 (x)", bad_digest=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(marker.exists())

    def test_rustup_wrong_toolchain(self):
        r, _ = self.rustup("rustc 1.93.0 (x)")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("expected 1.94.0", r.stderr)

    def test_dockerfile_uses_errexit_and_the_checked_fetcher(self):
        text = DOCKERFILE.read_text()
        self.assertIn('SHELL ["/bin/bash", "-euo", "pipefail", "-c"]', text)
        for what in ("herdr", "node", "rustup"):
            self.assertIn(f"/opt/ht/image-fetch.sh {what} ", text)
        self.assertNotIn("curl -", text)  # every download goes through image-fetch.sh
