"""Prepare one private fixture and finite owned Herdr server. No shared host actions.

Use prepare RUN_DIR, then server RUN_DIR. Native commands are deliberately issued
separately after checking the owned pane's inherited context and approval state.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "tests/native/support"))
from fixture import NativeFixture


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None


def prepare(root, minimal_config=None):
    fixture = NativeFixture.create(root)
    private = root / "config/claude"
    private.mkdir(mode=0o700)
    settings_source = Path.home() / ".claude/settings.json"
    settings = json.loads(settings_source.read_text())
    # Preserve permission rules without executing unrelated user/plugin callbacks.
    # The original files remain untouched. A private harmless existing hook tests
    # composition with the --settings overlay; this is not a global-hook claim.
    capture = str(Path(__file__).with_name("capture.py"))
    command = shlex.join([sys.executable, capture])
    baseline = {"permissions": settings.get("permissions", {}),
                "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command + " sentinel", "timeout": 10}]}]}}
    (private / "settings.json").write_text(json.dumps(baseline))
    for source in (Path.home()/".claude.json", Path.home()/".claude/.claude.json", Path.home()/".claude/.credentials.json"):
        if source.exists() and not (minimal_config is not None and source.name == ".claude.json"):
            destination = private / source.name
            if not destination.exists():
                shutil.copyfile(source, destination)
                destination.chmod(0o600)
    if minimal_config is not None:
        state = private/".claude.json"
        state.write_text(json.dumps(minimal_config))
        state.chmod(0o600)
    overlay = {"hooks": {event: [{"hooks": [{"type": "command", "command": command, "timeout": 10}]}]
                          for event in ("SessionStart", "SessionEnd", "PreCompact")}}
    (root/"config/overlay.json").write_text(json.dumps(overlay))
    environment = fixture.session_env() | {
        "CLAUDE_CONFIG_DIR": str(private), "CLAUDE_LIFECYCLE_LOG": str(root/"events.jsonl"),
        "CLAUDE_LIFECYCLE_VERSION": subprocess.check_output(["claude", "--version"], text=True).strip(),
        "XDG_CONFIG_HOME": str(root/"config"), "XDG_STATE_HOME": str(root/"state"),
        "XDG_RUNTIME_DIR": str(root/"state/runtime"), "HERDR_CONFIG_PATH": str(root/"config/herdr.toml"),
        "HERDR_SOCKET_PATH": str(root/"state/herdr.sock"),
    }
    (root/"state/runtime").mkdir(mode=0o700)
    # No externally installed plugins or native recipient tools are needed.
    (root/"config/herdr.toml").write_text("")
    (root/"env.json").write_text(json.dumps(environment))
    fingerprint = {str(path): digest(path) for path in (settings_source, Path.home()/".claude/settings.local.json", Path.home()/".claude.json")}
    (root/"global-before.json").write_text(json.dumps(fingerprint, sort_keys=True))
    print(json.dumps({"root": str(root), "permissions_preserved": baseline["permissions"] == settings.get("permissions", {}), "global_config_hashes": fingerprint}))


def prepare_trusted(root):
    # The checkout whose Claude trust decision is mirrored (default: this repository root).
    trusted_cwd = os.environ.get("HT_CLAUDE_TRUSTED_CWD") or str(Path(__file__).resolve().parents[4])
    source = json.loads((Path.home()/".claude.json").read_text())
    if source.get("projects", {}).get(trusted_cwd, {}).get("hasTrustDialogAccepted") is not True:
        raise ValueError("exact existing assignment trust decision absent")
    minimal = {key: source[key] for key in ("oauthAccount", "userID", "hasCompletedOnboarding", "lastOnboardingVersion", "installMethod") if key in source}
    minimal["projects"] = {trusted_cwd: {"hasTrustDialogAccepted": True}}
    prepare(root, minimal_config=minimal)
    (root/"trust-provenance.json").write_text(json.dumps({"cwd": trusted_cwd, "existing_exact_trust": True, "private_project_entries": 1}))
    print(json.dumps({"trusted_cwd": trusted_cwd, "existing_decision_mirrored": True}))


def environment(root):
    env = dict(os.environ)
    for key in tuple(env):
        if key.startswith("HERDR_") or key in ("CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"):
            env.pop(key, None)
    env.update(json.loads((root/"env.json").read_text()))
    return env


def bounded_number(value, maximum):
    if not value.isascii() or not value.isdecimal() or not 1 <= int(value) <= maximum:
        raise ValueError("number outside private capture bounds")


def nonoption(value):
    # Herdr 0.9.1 session/remote scanners inspect every pre-delimiter argv.
    # pane run/agent prompt do not consume a native -- delimiter, so option-shaped
    # values cannot safely be represented there. Keep all other bytes literal.
    if not value or value.startswith("-") or "\0" in value:
        raise ValueError("empty or option-shaped value outside native delimiter")


def private_command(root, arguments, fixture):
    args = list(arguments)
    owned = {row["id"] for row in fixture.owned_resources() if row["kind"] == "pane"}

    def pane(value):
        if not re.fullmatch(r"w[0-9]+:p[0-9]+", value) or value not in owned:
            raise ValueError("target must be an exact currently owned pane ID")

    if args == ["workspace", "list"]:
        return ["herdr", *args], False
    if (
        len(args) == 7
        and args[:3] == ["workspace", "create", "--cwd"]
        and args[4] == "--label"
        and args[6] == "--no-focus"
    ):
        nonoption(args[3])
        nonoption(args[5])
        if not Path(args[3]).is_absolute():
            raise ValueError("creation cwd must be absolute")
        return ["herdr", *args], True
    if len(args) == 3 and args[:2] == ["pane", "get"]:
        pane(args[2])
        return ["herdr", *args], False
    if len(args) == 5 and args[:2] == ["pane", "run"] and args[3] == "--":
        pane(args[2])
        nonoption(args[4])
        return ["herdr", *args[:3], args[4]], False
    if args[:2] == ["agent", "prompt"] and len(args) in (5, 8):
        pane(args[2])
        if len(args) == 5 and args[3] == "--":
            options = []
        elif (
            len(args) == 8 and args[3:5] == ["--wait", "--timeout"] and args[6] == "--"
        ):
            bounded_number(args[5], 60000)
            options = args[3:6]
        else:
            raise ValueError("unsupported prompt grammar")
        nonoption(args[-1])
        return ["herdr", *args[:3], args[-1], *options], False
    if (
        len(args) == 7
        and args[:2] == ["agent", "read"]
        and args[3] == "--source"
        and args[5] == "--lines"
    ):
        pane(args[2])
        if args[4] not in ("visible", "recent", "recent-unwrapped", "detection"):
            raise ValueError("unsupported read source")
        bounded_number(args[6], 120)
        return ["herdr", *args], False
    if (
        len(args) >= 10
        and args[:2] == ["agent", "start"]
        and args[3:6] == ["--kind", "claude", "--pane"]
        and args[7] == "--timeout"
        and args[9] == "--"
    ):
        if not re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", args[2]):
            raise ValueError("invalid private agent name")
        pane(args[6])
        bounded_number(args[8], 60000)
        recipe = [
            "--model",
            "sonnet",
            "--effort",
            "low",
            "--settings",
            str(root / "config/overlay.json"),
            "--setting-sources",
            "user",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            '{"mcpServers":{}}',
        ]
        native = args[10:]
        if native[: len(recipe)] != recipe:
            raise ValueError("unsupported native capture recipe")
        tail = native[len(recipe) :]
        if tail and not (
            len(tail) == 2
            and tail[0] == "--resume"
            and re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", tail[1])
        ):
            raise ValueError("unsupported native resume grammar")
        return ["herdr", *args], False
    raise ValueError("command outside finite private capture grammar")


def private_cli(root, arguments):
    fixture = NativeFixture.open(root)
    argv, creates_pane = private_command(root, arguments, fixture)
    result = subprocess.run(
        argv, env=environment(root), timeout=60, capture_output=True, text=True
    )
    if creates_pane and result.returncode == 0:
        try:
            response = json.loads(result.stdout)
            created = response["result"]["root_pane"]["pane_id"]
            if (
                response["result"]["type"] != "workspace_created"
                or not isinstance(created, str)
                or not re.fullmatch(r"w[0-9]+:p[0-9]+", created)
            ):
                raise ValueError("invalid creation identity")
            fixture.record_owned("pane", created)
        except (KeyError, TypeError, json.JSONDecodeError, ValueError) as error:
            # Preserve the response rather than retry an ambiguous creation.
            print(result.stdout, end="")
            raise ValueError(
                "creation response cannot establish ownership; inspect private server"
            ) from error
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    return result.returncode


def cleanup(root):
    fixture = NativeFixture.open(root)
    latest = json.loads((root / "server-pid.json").read_text())["pid"]

    class Host:
        def close_exact_owned_id(self, kind, resource_id):
            if kind == "pane":
                subprocess.run(["herdr", "pane", "close", resource_id], env=environment(root), timeout=10, check=True)
            elif kind == "server":
                pid = int(resource_id)
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    return
                if pid != latest:
                    raise RuntimeError("old server PID remains alive; preserve ambiguous ownership")
                subprocess.run(["herdr", "server", "stop"], env=environment(root), timeout=10, check=True)
            else:
                raise RuntimeError("unexpected resource kind")

    fixture.cleanup(Host())
    print(json.dumps({"remaining_owned_resources": fixture.owned_resources()}))


def server(root):
    fixture = NativeFixture.open(root)
    env = environment(root)
    with (root/"server.log").open("a") as log:
        process = subprocess.Popen(["herdr", "server"], env=env, stdout=log, stderr=log)
    fixture.record_owned("server", str(process.pid))
    (root/"server-pid.json").write_text(json.dumps({"pid": process.pid}))
    print(json.dumps({"owned_server_pid": process.pid, "socket": env["HERDR_SOCKET_PATH"]}), flush=True)
    try:
        code = process.wait(timeout=1200)
    except subprocess.TimeoutExpired:
        process.terminate()
        process.wait(timeout=10)
        code = 124
    sys.exit(code)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "prepare-trusted", "server", "cli", "verify-global", "cleanup"))
    parser.add_argument("root", type=Path)
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.action != "cli" and args.arguments:
        parser.error("dedicated actions accept no surplus arguments")
    if args.action == "prepare":
        prepare(args.root)
    elif args.action == "prepare-trusted":
        prepare_trusted(args.root)
    elif args.action == "server":
        server(args.root)
    elif args.action == "cleanup":
        cleanup(args.root)
    elif args.action == "cli":
        try:
            code = private_cli(args.root, args.arguments)
        except ValueError as error:
            parser.error(str(error))
        sys.exit(code)
    else:
        before = json.loads((args.root / "global-before.json").read_text())
        after = {path: digest(Path(path)) for path in before}
        print(
            json.dumps(
                {"global_unchanged": before == after, "after": after}, sort_keys=True
            )
        )
        sys.exit(0 if before == after else 1)
