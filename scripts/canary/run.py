#!/usr/bin/env python3
"""Process-group runner (nested spec §D2): `run.py --timeout S [--until-file F] -- argv...`.

Starts argv in its own session/process group and kills the whole group when the deadline passes, when F
appears, or when the leader exits (no orphan survives). Exit code: the leader's own code when it exits first;
0 when F appeared; 124 on timeout; 127 when argv cannot be started. Pure stdlib."""
import argparse, os, signal, subprocess, sys, time


def _kill_group(proc):
    """SIGTERM then SIGKILL the leader's group. macOS answers EPERM for a group whose only member is an
    unreaped zombie, so the leader is reaped between the two signals and EPERM counts as 'gone'."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(proc.pid, sig)
        except (ProcessLookupError, PermissionError):
            break
        try:
            proc.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            pass
    proc.wait()


def run(argv, timeout, until_file=None, poll=0.05):
    try:
        proc = subprocess.Popen(argv, start_new_session=True)
    except OSError as e:
        print(f"run.py: cannot start {argv[0]}: {e}", file=sys.stderr)
        return 127
    deadline = time.monotonic() + timeout
    code = None
    try:
        while True:
            rc = proc.poll()
            if rc is not None:
                code = rc if rc >= 0 else 128 - rc
                break
            if until_file and os.path.exists(until_file):
                code = 0
                break
            if time.monotonic() >= deadline:
                code = 124
                break
            time.sleep(poll)
    finally:
        _kill_group(proc)
    return code


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--timeout", type=float, required=True)
    ap.add_argument("--until-file")
    ap.add_argument("argv", nargs=argparse.REMAINDER)
    args = ap.parse_args(argv)
    cmd = args.argv[1:] if args.argv[:1] == ["--"] else args.argv
    if not cmd:
        ap.error("no command given")
    return run(cmd, args.timeout, args.until_file)


if __name__ == "__main__":
    sys.exit(main())
