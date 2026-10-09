"""Spike driver: runs Claude Code with the spike mod in a private tmux server.

usage: python3 drive.py <run_dir> <command> [args]
  start <name> <permission-mode>   start a session (default|bypassPermissions|auto|acceptEdits)
  feed <name> <json>               append one message line to the session's feed
  keys <name> <tmux keys...>       send keys (literal text with -l prefix arg 'lit:')
  screen <name>                    print the visible screen
  events <name>                    print the mod's event ledger
  stop <name>                      kill the session
"""
import json, os, subprocess, sys, time

CB = os.path.expanduser('~/.local/share/claude/versions/2.1.294')
MOD = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'mod')
run = sys.argv[1]
cmd = sys.argv[2]
args = sys.argv[3:]


def tmux(*a, check=True):
    return subprocess.run(['tmux', '-L', 'htspike', *a], check=check, capture_output=True, text=True).stdout


def sdir(name):
    return os.path.join(run, 'sessions', name)


if cmd == 'start':
    name, mode = args[0], args[1]
    d = sdir(name)
    os.makedirs(d, exist_ok=True)
    open(os.path.join(d, 'feed.jsonl'), 'a').close()
    env = ['env', '-u', 'HERDR_SOCKET_PATH', '-u', 'CLAUDE_CODE_MESSAGING_SOCKET', '-u', 'CLAUDE_CODE_MESSAGING_TOKEN',
           '-u', 'CLAUDECODE', '-u', 'CLAUDE_CODE_ENTRYPOINT', 'DISABLE_AUTOUPDATER=1',
           f'CLAUDE_CONFIG_DIR={run}/profile', f'HT_SPIKE_DIR={d}', f'HERDR_PANE_ID=spike-{name}']
    extra = args[2:]
    line = ' '.join(env + [CB, '--plugin-dir', MOD, '--debug-file', f'{d}/debug.log', '--permission-mode', mode] + extra)
    tmux('new-session', '-d', '-s', name, '-x', '160', '-y', '50', '-c', f'{run}/project', line)
    print('started', name)
elif cmd == 'feed':
    with open(os.path.join(sdir(args[0]), 'feed.jsonl'), 'a') as f:
        f.write(args[1].strip() + '\n')
    print('fed', time.time())
elif cmd == 'keys':
    name = args[0]
    for k in args[1:]:
        if k.startswith('lit:'):
            tmux('send-keys', '-t', name, '-l', k[4:])
        else:
            tmux('send-keys', '-t', name, k)
        time.sleep(0.15)
elif cmd == 'screen':
    out = tmux('capture-pane', '-p', '-t', args[0], '-J')
    lines = out.rstrip('\n').split('\n')
    print('\n'.join(l for l in lines if l.strip())[-6000:])
elif cmd == 'events':
    import glob
    rows = []
    for p in glob.glob(os.path.join(sdir(args[0]), 'events-*.jsonl')):
        rows += [json.loads(l) for l in open(p) if l.strip()]
    rows.sort(key=lambda r: r['at'])
    t0 = None
    for r in rows:
        t0 = t0 or r['at']
        at = r.pop('at')
        print(f"{(at - t0) / 1000:8.2f}s", json.dumps(r)[:300])
elif cmd == 'stop':
    tmux('kill-session', '-t', args[0], check=False)
