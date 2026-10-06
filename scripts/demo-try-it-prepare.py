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
import tomllib
import uuid

ACTIVE_RUN = None


def write_toml(path, data):
    def scalar(value):
        if isinstance(value, bool):
            return "true" if value else "false"
        if isinstance(value, str):
            return json.dumps(value, ensure_ascii=False)
        if isinstance(value, list):
            return "[" + ", ".join(scalar(v) for v in value) + "]"
        if isinstance(value, dict):
            return "{ " + ", ".join(json.dumps(k) + " = " + scalar(v) for k, v in value.items()) + " }"
        if isinstance(value, (int, float)):
            return str(value)
        return value.isoformat()
    lines = []
    def table(keys, values):
        if keys:
            lines.append("[" + ".".join(json.dumps(k) for k in keys) + "]")
        for key, value in values.items():
            if not isinstance(value, dict):
                lines.append(json.dumps(key) + " = " + scalar(value))
        lines.append("")
        for key, value in values.items():
            if isinstance(value, dict):
                table(keys + [key], value)
    table([], data)
    text = "\n".join(lines)
    if tomllib.loads(text) != data:
        raise ValueError("profile configuration did not round-trip")
    path.write_text(text)
    path.chmod(0o600)


def copy_settings(root, codex_source, claude_source):
    """Copy preferences/resources; retain already reviewed private hook routing."""
    def private_ht(groups):
        kept = []
        for group in groups:
            group = dict(group)
            group["hooks"] = [h for h in group.get("hooks", [])
                              if "herdr-threads" in h.get("command", "")]
            if group["hooks"]:
                kept.append(group)
        return kept
    def remap_codex(text):
        return text.replace(str(codex_source), str(root / "codex-home")).replace(
            str(Path.home() / ".codex"), str(root / "codex-home"))
    def copy_resources(source, target):
        def copy_private(src, dst):
            destination = Path(dst)
            if destination.is_file():
                destination.chmod(destination.stat().st_mode | 0o200)
            return shutil.copy2(src, dst)
        for name in ("hooks", "skills", "rules", "plugins", "scripts", "agents", "prompts", "AGENTS.md", "CLAUDE.md"):
            path = source / name
            if path.is_dir():
                shutil.copytree(path, target / name, dirs_exist_ok=True, symlinks=False,
                                copy_function=copy_private,
                                ignore=shutil.ignore_patterns(".git", "temp_subdir_*"))
            elif path.is_file():
                shutil.copy2(path, target / name)
        for name in ("hooks", "skills", "rules", "plugins", "scripts", "agents", "prompts"):
            directory = target / name
            if not directory.exists():
                continue
            for resource in directory.rglob("*"):
                if not resource.is_file() or resource.suffix not in (".json", ".toml", ".sh", ".py", ".md"):
                    continue
                try:
                    text = resource.read_text()
                except UnicodeDecodeError:
                    continue
                remapped = text.replace(str(source), str(target))
                if remapped != text:
                    resource.chmod(resource.stat().st_mode | 0o200)
                    resource.write_text(remapped)
    copy_resources(codex_source, root / "codex-home")
    copy_resources(claude_source, root / "claude-config")
    original_hooks = codex_source / "hooks.json"
    private_hooks = root / "codex-home/hooks.json"
    if original_hooks.exists():
        source_hooks = json.loads(original_hooks.read_text()).get("hooks", {})
        existing_hooks = json.loads(private_hooks.read_text()).get("hooks", {}) if private_hooks.exists() else {}
        for event, groups in source_hooks.items():
            retained = []
            for group in groups:
                group = dict(group)
                group["hooks"] = [h for h in group.get("hooks", [])
                                  if "herdr-threads" not in h.get("command", "")]
                if group["hooks"]:
                    retained.append(group)
            source_hooks[event] = retained + private_ht(existing_hooks.get(event, []))
        for event, groups in existing_hooks.items():
            source_hooks.setdefault(event, private_ht(groups))
        text = json.dumps({"hooks": source_hooks})
        text = remap_codex(text)
        save(private_hooks, json.loads(text))
        script = Path.home() / ".codex/herdr-agent-state.sh"
        if script.exists():
            shutil.copy2(script, root / "codex-home/herdr-agent-state.sh")
    private_config = root / "codex-home/config.toml"
    current = tomllib.loads(private_config.read_text()) if private_config.exists() else {}
    source = codex_source / "config.toml"
    def rebase_values(value):
        if isinstance(value, str):
            return remap_codex(value)
        if isinstance(value, list):
            return [rebase_values(item) for item in value]
        if isinstance(value, dict):
            # Trust keys are historical native identities, not resource paths.
            # Rebasing two aliases into one key would either corrupt TOML or
            # conflate distinct approvals. New private hooks are reviewed natively.
            return {key: rebase_values(item) for key, item in value.items()}
        return value
    config = rebase_values(tomllib.loads(source.read_text())) if source.exists() else {}
    # Copied historical trust stays intact; preserve approvals actually acquired
    # for the generated demo hooks/project. Never manufacture trust hashes.
    state = config.setdefault("hooks", {}).setdefault("state", {})
    state.update(current.get("hooks", {}).get("state", {}))
    config.setdefault("projects", {}).update(current.get("projects", {}))
    config.update(check_for_update_on_startup=False, approval_policy="on-request",
                  sandbox_mode="workspace-write")
    config.setdefault("features", {})["daemon_auto_start"] = False
    write_toml(private_config, config)
    for name in ("settings.json", "settings.local.json", ".claude.json"):
        source = claude_source / name
        target = root / "claude-config" / name
        if not source.exists():
            continue
        original = json.loads(source.read_text())
        current = json.loads(target.read_text()) if target.exists() else {}
        if name == "settings.json":
            # The source's ht commands target real profile state. Retain the
            # generated private ht hooks and copy all other hooks/preferences.
            hooks = {}
            for event, groups in original.get("hooks", {}).items():
                kept = []
                for group in groups:
                    group = dict(group)
                    group["hooks"] = [h for h in group.get("hooks", [])
                                      if "herdr-threads" not in h.get("command", "")]
                    if group["hooks"]:
                        kept.append(group)
                hooks[event] = kept + private_ht(current.get("hooks", {}).get(event, []))
            for event, groups in current.get("hooks", {}).items():
                hooks.setdefault(event, private_ht(groups))
            original["hooks"] = hooks
        else:
            original.update(current)
        text = json.dumps(original)
        text = text.replace(str(claude_source), str(root / "claude-config"))
        save(target, json.loads(text))
    # Claude's default-home startup/trust records also belong to the copy.
    state = Path.home() / ".claude.json"
    if state.exists():
        copied = json.loads(state.read_text())
        current_path = root / "claude-config/.claude.json"
        current = json.loads(current_path.read_text()) if current_path.exists() else {}
        for key, value in current.items():
            copied.setdefault(key, value)
        projects = dict(copied.get("projects", {}))
        projects.update(current.get("projects", {}))
        copied["projects"] = projects
        save(current_path, copied)
        save(root / "home/.claude.json", copied)
    tmux = root / "home/.tmux"
    tmux.mkdir(exist_ok=True)
    for name in ("claude-statusline.sh", "claude-usage.sh"):
        source = Path.home() / ".tmux" / name
        if source.exists():
            shutil.copy2(source, tmux / name)
    usage = tmux / "claude-usage.sh"
    if usage.exists():
        text = usage.read_text()
        begin = text.find('# Read OAuth token')
        end = text.find('# Call the usage API', begin)
        if begin >= 0 and end >= 0:
            # Same status display, credential supplied from private wrapper copy.
            text = text[:begin] + 'TOKEN="${CLAUDE_CODE_OAUTH_TOKEN:-}"\n' + text[end:]
            usage.write_text(text)


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


def install_candidate_skill(root, env):
    """Use the captured binary's guide in private profiles, never a stale copy."""
    guide = run([str(root / "bin/herdr-threads"), "skill"], env)
    if not guide.startswith("---\nname: herdr-threads\n"):
        raise ValueError("candidate did not return the embedded Herdr skill")
    for name in ("codex-home", "claude-config"):
        folder = root / name / "skills/herdr-threads"
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / "SKILL.md"
        if path.exists():
            path.chmod(path.stat().st_mode | 0o200)
        path.write_text(guide)
        path.chmod(0o600)
        if path.read_text() != guide:
            raise ValueError("private skill differs from candidate guide")


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
    for name in ("bin", "home", "project", "claude-config", "codex-home", "tmp"):
        (root / name).mkdir(mode=0o700)
    manifest = {"root": str(root), "run_id": str(uuid.uuid4())}
    env_values = {
        "HOME": str(root / "home"), "CLAUDE_CONFIG_DIR": str(root / "claude-config"),
        "CODEX_HOME": str(root / "codex-home"), "HERDR_PLUGIN_STATE_DIR": str(root / "state"),
        "HERDR_SOCKET_PATH": os.environ["HERDR_SOCKET_PATH"],
        "XDG_CONFIG_HOME": str(root / "home/config"), "XDG_STATE_HOME": str(root / "home/state"),
        "XDG_CACHE_HOME": str(root / "home/cache"),
        "PATH": str(root / "bin") + ":" + os.environ["PATH"],
        "SHELL": "/bin/zsh", "HT_LEAK_RUN_ID": manifest["run_id"],
        "ZDOTDIR": str(root / "home"),
        "TMPDIR": str(root / "tmp"),
        "DISABLE_AUTOUPDATER": "1", "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION": "false",
    }
    manifest["env"] = env_values
    save(root / "private.json", manifest)
    print(f"Private run (preserve this path for cleanup): {root}", flush=True)
    env = private_env(env_values)
    shutil.copy2(args.bin.resolve(), root / "bin/herdr-threads")
    shutil.copy2(Path(__file__).with_name("demo-try-it.py"), root / "record.py")
    shutil.copy2(Path(__file__).with_name("demo-try-it-camera.py"), root / "camera.py")
    (root / "camera-config.toml").write_text(
        'onboarding = false\n[ui]\nsidebar_start_collapsed = true\n'
        'sidebar_collapsed_mode = "hidden"\n[experimental]\nallow_nested = true\n'
        '[terminal]\nkitty_graphics = false\n[update]\nversion_check = false\nmanifest_check = false\n'
    )
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
    copy_settings(root, args.codex_home, args.claude_config)
    install_candidate_skill(root, env)
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
    manifest.update(tab=created["tab"]["tab_id"], alice=created["root_pane"]["pane_id"])
    save(root / "private.json", manifest)
    # Split the full-width bottom first, then divide only the upper half.
    for name, direction in (("human", "down"), ("bob", "right")):
        split = ["herdr", "pane", "split", manifest["alice"], "--direction", direction,
                 "--ratio", "0.67" if direction == "down" else "0.5",
                 "--cwd", str(root / "project"), "--no-focus"]
        for key, value in env_values.items():
            split += ["--env", f"{key}={value}"]
        pane = json.loads(run(split, env))["result"]["pane"]["pane_id"]
        manifest[name] = pane
        save(root / "private.json", manifest)
        run(["herdr", "pane", "rename", pane, name], env)
    run(["herdr", "pane", "rename", manifest["alice"], "alice"], env)
    run(["herdr", "pane", "rename", manifest["human"], "you"], env)
    alice = (
        "You are Alice. Make the case for spaces and discuss it with Bob in this thread. Wait for his ready message before Alice 1. "
        "Post at most 45 words per turn, labeled Alice 1, Alice 2, Alice 3. Each later turn must answer "
        "a specific Bob objection before raising your next concern. Wait for Bob between turns; no polling. "
        "After Bob joins, use --require-ack-pane bob for each post. "
        "Only after Bob 3, post Shared recommendation with your conclusion."
    )
    bob = (
        "You are Bob. Make the case for tabs and discuss it with Alice in this thread. After joining, tell Alice you are ready, then wait for Alice 1. "
        "Post at most 45 words per turn, labeled Bob 1, Bob 2, Bob 3. Each turn must answer a specific "
        "Alice objection before raising your next concern. Wait for Alice between turns; no polling. "
        "Use --require-ack-pane alice for each post. "
        "Only after Alice posts Shared recommendation, confirm it or state a remaining tradeoff."
    )
    alice_command = "herdr-threads handoff --new-thread --thread-name review \\\n  --topic \"Tabs or spaces?\" --pane alice --kind claude -- \\\n  "
    bob_command = "herdr-threads handoff --thread review --pane bob --kind codex -- \\\n  "
    commands = [
        "# We will create a new thread and invite Alice and Bob.\n"
        "# They will debate spaces versus tabs through the thread.\n"
        "# We will observe their work above and follow the conversation below.\n"
        "herdr-threads me init",
        {"command": alice_command + shlex.quote(alice), "fast_from": len(alice_command)},
        {"command": bob_command + shlex.quote(bob), "fast_from": len(bob_command)},
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
    parser.add_argument("--claude-config", type=Path, default=Path(os.environ.get("CLAUDE_CONFIG_DIR", Path.home() / ".claude")))
    parser.add_argument("--copy-settings", type=Path, help="refresh settings copies in an existing owned run")
    parser.add_argument("--cleanup", type=Path)
    parser.add_argument("--camera", type=Path, help="attach private camera client for an existing owned run")
    args = parser.parse_args()
    if args.copy_settings:
        root = args.copy_settings.resolve()
        if root.parent != Path("/private/tmp") or not root.name.startswith("ht-try-it."):
            parser.error("settings require an owned private run")
        manifest = json.loads((root / "private.json").read_text())
        if manifest["root"] != str(root) or manifest.get("tab_closed"):
            parser.error("settings require an active owned run")
        copy_settings(root, args.codex_home, args.claude_config)
        install_candidate_skill(root, private_env(manifest["env"]))
    elif args.camera:
        root = args.camera.resolve()
        if root.parent != Path("/private/tmp") or not root.name.startswith("ht-try-it."):
            parser.error("camera requires an owned private run")
        manifest = json.loads((root / "private.json").read_text())
        if manifest["root"] != str(root) or manifest.get("tab_closed"):
            parser.error("camera requires an active owned run")
        env = private_env(manifest["env"])
        env.update(HERDR_CONFIG_PATH=str(root / "camera-config.toml"), TERM="xterm-256color")
        os.execve(sys.executable, [sys.executable, str(root / "camera.py"), "--root", str(root)], env)
    elif args.cleanup:
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
