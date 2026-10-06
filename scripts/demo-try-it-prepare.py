#!/usr/bin/env python3
"""Prepare an owned, private walkthrough tab; clean it with --cleanup RUN_DIR.

Requires an existing Herdr pane, a built herdr-threads binary, authenticated
Claude/Codex, and explicit authorization to run a live demo. Never changes the
shared server or real profiles. See docs/media/try-it.md before using it.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

ACTIVE_RUN = None


def private_env(values):
    # Retain only host routing and ordinary terminal/runtime context. Do not
    # inherit profile overrides, authentication envs or agent caller markers.
    keep = {"PATH", "TERM", "LANG", "LC_ALL", "TMPDIR"}
    env = {key: value for key, value in os.environ.items()
           if key in keep or key.startswith("HERDR_")}
    env.pop("HERDR_AGENT", None)
    return dict(env, **values)


def run(argv, env=None):
    return subprocess.run(argv, env=env, check=True, capture_output=True, text=True, timeout=60).stdout


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")
    path.chmod(0o600)


def stop_tagged(run_id):
    """Reap detached native helpers carrying this exact run's inherited tag."""
    def pids():
        rows = run(["ps", "axeww", "-o", "pid=", "-o", "command="]).splitlines()
        tag = "HT_LEAK_RUN_ID=" + run_id
        return [int(row.split()[0]) for row in rows
                if tag in row.split() and int(row.split()[0]) != os.getpid()]
    stopped = []
    for _ in range(3):
        owned = pids()
        if not owned:
            return stopped
        stopped.extend(owned)
        for pid in owned:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        time.sleep(1)
        for pid in pids():
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        time.sleep(0.1)
    if pids():
        raise RuntimeError("owned native helpers still present")
    return stopped


def cleanup(root):
    root = root.resolve()
    if root.parent != Path("/private/tmp") or not root.name.startswith("ht-try-it."):
        raise ValueError("cleanup requires the exact owned /private/tmp/ht-try-it.* run")
    manifest = json.loads((root / "private.json").read_text())
    if manifest["root"] != str(root):
        raise ValueError("run ownership record does not match")
    env = private_env(manifest["env"])
    errors = []
    if manifest.get("tab_pending") and not manifest.get("tab"):
        try:
            tabs = json.loads(run(["herdr", "tab", "list", "--workspace", manifest["workspace"]], env))
            matches = [tab for tab in tabs["result"]["tabs"] if tab.get("label") == manifest["label"]]
            if len(matches) > 1:
                raise ValueError("ambiguous owned tab recovery; preserve run and inspect")
            if matches:
                manifest["tab"] = matches[0]["tab_id"]
                save(root / "private.json", manifest)
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            errors.append(f"owned tab recovery failed: {error}")
    # Close only the tab returned by this run's successful creation call.
    if manifest.get("tab") and not manifest.get("tab_closed"):
        try:
            run(["herdr", "tab", "close", manifest["tab"]], env)
            manifest["tab_closed"] = True
            save(root / "private.json", manifest)
        except (OSError, subprocess.SubprocessError) as error:
            errors.append(f"owned tab close failed: {error}")
    if manifest.get("daemon_started") and not manifest.get("daemon_stopped"):
        try:
            run([str(root / "bin/herdr-threads"), "daemon", "stop"], env)
            manifest["daemon_stopped"] = True
            save(root / "private.json", manifest)
        except (OSError, subprocess.SubprocessError) as error:
            errors.append(f"private daemon stop failed: {error}")
    try:
        manifest["reaped_native_helpers"] = stop_tagged(manifest["run_id"])
        save(root / "private.json", manifest)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        errors.append(f"owned native helper cleanup failed: {error}")
    # Evidence/cast/state survive; credential-bearing copies do not.
    for name in ("home", "claude-config", "codex-home"):
        try:
            if (root / name).exists():
                shutil.rmtree(root / name)
        except OSError as error:
            errors.append(f"private {name} removal failed: {error}")
    manifest["credentials_removed"] = all(not (root / name).exists()
                                          for name in ("home", "claude-config", "codex-home"))
    if not manifest["credentials_removed"]:
        errors.append("credential directories remain")
    save(root / "private.json", manifest)
    if errors:
        raise RuntimeError("; ".join(errors))
    print(f"Owned tab closed, private daemon stopped, credential copies removed: {root}")


def prepare(args):
    global ACTIVE_RUN
    if os.environ.get("HERDR_ENV") != "1" or not os.environ.get("HERDR_SOCKET_PATH"):
        raise ValueError("run inside the authorized Herdr task pane")
    root = Path(tempfile.mkdtemp(prefix="ht-try-it.", dir="/private/tmp"))
    root.chmod(0o700)
    ACTIVE_RUN = root
    for name in ("bin", "home", "project", "claude-config", "codex-home"):
        (root / name).mkdir(mode=0o700)
    manifest = {"root": str(root), "run_id": str(uuid.uuid4())}
    env_values = {
        "HOME": str(root / "home"), "CLAUDE_CONFIG_DIR": str(root / "claude-config"),
        "CODEX_HOME": str(root / "codex-home"), "HERDR_PLUGIN_STATE_DIR": str(root / "state"),
        "HERDR_SOCKET_PATH": os.environ["HERDR_SOCKET_PATH"],
        "XDG_CONFIG_HOME": str(root / "home/config"), "XDG_STATE_HOME": str(root / "home/state"),
        "PATH": str(root / "bin") + ":" + os.environ["PATH"],
        "SHELL": "/bin/zsh", "HT_LEAK_RUN_ID": manifest["run_id"],
        "ZDOTDIR": str(root / "home"),
        "DISABLE_AUTOUPDATER": "1", "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION": "false",
    }
    manifest["env"] = env_values
    save(root / "private.json", manifest)
    print(f"Private run (preserve this path for cleanup): {root}", flush=True)
    env = private_env(env_values)
    shutil.copy2(args.bin.resolve(), root / "bin/herdr-threads")
    shutil.copy2(Path(__file__).with_name("demo-try-it.py"), root / "record.py")
    # A real copy, never a writable symlink back to the source profile.
    shutil.copyfile(args.codex_home / "auth.json", root / "codex-home/auth.json")
    (root / "codex-home/auth.json").chmod(0o600)
    # Claude credentials are read into memory, never printed or passed in argv.
    if args.claude_credentials:
        credentials = json.loads(args.claude_credentials.read_text())
    else:
        credentials = json.loads(run(["/usr/bin/security", "find-generic-password", "-s",
                                      "Claude Code-credentials", "-w"]))
    save(root / "claude-config/credentials-copy.json", credentials)
    native_claude = shutil.which("claude")
    native_codex = shutil.which("codex")
    if not native_claude or not native_codex:
        raise ValueError("claude and codex must be on PATH")
    # The token environment is read from the private COPY by the wrapper.
    # No source refresh token/config directory is available to the agent.
    wrapper = (
        "#!/usr/bin/env python3\nimport json, os, sys\nfrom pathlib import Path\n"
        "keep={'PATH','HOME','SHELL','TERM','TMPDIR','LANG','CLAUDE_CONFIG_DIR','CODEX_HOME','ZDOTDIR',"
        "'DISABLE_AUTOUPDATER','CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION'}\n"
        "clean={k:v for k,v in os.environ.items() if k in keep or k.startswith(('HERDR_','HT_','XDG_'))}\n"
        "os.environ.clear(); os.environ.update(clean)\n"
        "data=json.loads((Path(os.environ['CLAUDE_CONFIG_DIR'])/'credentials-copy.json').read_text())\n"
        "os.environ['CLAUDE_CODE_OAUTH_TOKEN']=data['claudeAiOauth']['accessToken']\n"
        f"os.execv({native_claude!r}, [{native_claude!r}, *sys.argv[1:]])\n"
    )
    (root / "bin/claude").write_text(wrapper)
    (root / "bin/claude").chmod(0o700)
    (root / "bin/codex").write_text(
        "#!/usr/bin/env python3\nimport os, sys\n"
        "keep={'PATH','HOME','SHELL','TERM','TMPDIR','LANG','CLAUDE_CONFIG_DIR','CODEX_HOME','ZDOTDIR',"
        "'DISABLE_AUTOUPDATER','CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION'}\n"
        "clean={k:v for k,v in os.environ.items() if k in keep or k.startswith(('HERDR_','HT_','XDG_'))}\n"
        "os.environ.clear(); os.environ.update(clean)\n"
        f"os.execv({native_codex!r}, [{native_codex!r}, *sys.argv[1:]])\n"
    )
    (root / "bin/codex").chmod(0o700)
    (root / "codex-home/config.toml").write_text(
        'check_for_update_on_startup = false\napproval_policy = "on-request"\n'
        'sandbox_mode = "workspace-write"\n'
    )
    # Shell startup is private and deterministic; no real dotfiles are loaded.
    (root / "home/.zshrc").write_text("export PATH=" + shlex.quote(env_values["PATH"]) + "\nPS1='demo % '\n")
    run(["git", "init", "-q", str(root / "project")], env)
    (root / "project/AGENTS.md").write_text(
        "This is an isolated recording of the Herdr Threads README walkthrough. "
        "Use only herdr-threads commands to read, join, acknowledge and discuss your assigned thread. "
        "Do not change files, spawn subagents, inspect credentials, or use network tools. "
        "Native permissions apply. Codex: execute herdr-threads outside the sandbox with a CLI-only "
        'approval request; request prefix_rule ["herdr-threads"] when appropriate.\n'
    )
    for harness in ("claude", "codex"):
        command = [str(root / "bin/herdr-threads"), "setup", harness]
        if harness == "claude":
            command.append("--disable-prompt-suggestions")
        (root / f"setup-{harness}.txt").write_text(run(command, env))
    manifest["daemon_started"] = True
    save(root / "private.json", manifest)
    run([str(root / "bin/herdr-threads"), "daemon", "ensure"], env)
    manifest.update(workspace=os.environ["HERDR_WORKSPACE_ID"],
                    label="try-it-" + manifest["run_id"][:8], tab_pending=True)
    save(root / "private.json", manifest)
    command = ["herdr", "tab", "create", "--workspace", os.environ["HERDR_WORKSPACE_ID"],
               "--cwd", str(root / "project"), "--label", manifest["label"], "--no-focus"]
    for key, value in env_values.items():
        command += ["--env", f"{key}={value}"]
    created = json.loads(run(command, env))["result"]
    manifest.update(tab=created["tab"]["tab_id"], human=created["root_pane"]["pane_id"])
    save(root / "private.json", manifest)
    for name in ("alice", "bob"):
        split = ["herdr", "pane", "split", manifest["human"], "--direction", "right",
                 "--cwd", str(root / "project"), "--no-focus"]
        for key, value in env_values.items():
            split += ["--env", f"{key}={value}"]
        pane = json.loads(run(split, env))["result"]["pane"]["pane_id"]
        manifest[name] = pane
        save(root / "private.json", manifest)
        run(["herdr", "pane", "rename", pane, name], env)
    run(["herdr", "pane", "rename", manifest["human"], "you"], env)
    alice = (
        "You are Alice. Read this thread, make the case for spaces, and discuss it with Bob when he joins. "
        "Post at most 45 words per turn, labeled Alice 1, Alice 2, Alice 3. Each later turn must answer "
        "a specific Bob objection before raising your next concern. Wait for Bob between turns; no polling. "
        "After Bob joins, use --require-ack-pane bob for each post. "
        "Only after Bob 3, post Shared recommendation with your conclusion."
    )
    bob = (
        "You are Bob. Read this thread, make the case for tabs, and discuss it with Alice. "
        "Post at most 45 words per turn, labeled Bob 1, Bob 2, Bob 3. Each turn must answer a specific "
        "Alice objection before raising your next concern. Wait for Alice between turns; no polling. "
        "Use --require-ack-pane alice for each post. "
        "Only after Alice posts Shared recommendation, confirm it or state a remaining tradeoff."
    )
    commands = [
        "# Adjacent empty panes named alice (Claude) and bob (Codex) are prepared.\n"
        "# Native trust and CLI approvals happen in those panes, outside this camera.\n"
        "herdr-threads me init",
        "herdr-threads handoff --new-thread --thread-name review \\\n  --topic \"Tabs or spaces?\" --pane alice --kind claude -- \\\n  " + shlex.quote(alice),
        "herdr-threads handoff --thread review --pane bob --kind codex -- \\\n  " + shlex.quote(bob),
        "herdr-threads send review --require-ack-pane alice --require-ack-pane bob \\\n  --body \"Please settle on one recommendation and explain the tradeoff, after three replies each.\"",
        "herdr-threads pending-receipts --thread review",
        "herdr-threads follow review",
    ]
    save(root / "commands.json", commands)
    print(json.dumps({key: manifest[key] for key in ("root", "run_id", "tab", "human", "alice", "bob")}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=Path)
    parser.add_argument("--codex-home", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")))
    parser.add_argument("--claude-credentials", type=Path, help="private Claude credentials JSON (else macOS keychain)")
    parser.add_argument("--cleanup", type=Path)
    args = parser.parse_args()
    if args.cleanup:
        cleanup(args.cleanup)
    elif args.bin:
        prepare(args)
    else:
        parser.error("provide --bin or --cleanup")


if __name__ == "__main__":
    def interrupted(signum, _frame):
        raise RuntimeError(f"preparation interrupted by signal {signum}")
    for signum in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        signal.signal(signum, interrupted)
    try:
        main()
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"demo-try-it-prepare: {error}", file=sys.stderr)
        if ACTIVE_RUN is not None and (ACTIVE_RUN / "private.json").exists():
            try:
                cleanup(ACTIVE_RUN)
            except Exception as cleanup_error:
                print(f"Cleanup requires attention: {cleanup_error}", file=sys.stderr)
        sys.exit(1)
