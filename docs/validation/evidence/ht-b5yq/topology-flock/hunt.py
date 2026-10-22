#!/usr/bin/env python3
"""Run the historical failing shape (<lib bin> topology_handoff, default threads)
repeatedly under CPU oversubscription; keep every failing run's full output."""
import os, subprocess, sys, time, json, signal, re
from pathlib import Path
S = Path(sys.argv[0]).parent
BIN = open(S/'libbin.txt').read().strip()
RUNS = int(sys.argv[1]); LANES = int(sys.argv[2]); BURNERS = int(sys.argv[3])
FILTER = sys.argv[4] if len(sys.argv) > 4 else 'topology_handoff'
TAG = sys.argv[5] if len(sys.argv) > 5 else 'hunt'
out = S/f'{TAG}-results'; out.mkdir(exist_ok=True)
env = dict(os.environ, HT_LEAK_RUN_ID=open(S/'runid.txt').read().strip())
burn = [subprocess.Popen([sys.executable, '-c', 'while True: pass'],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) for _ in range(BURNERS)]
results = []
try:
    def one(i):
        t = time.time()
        p = subprocess.run([BIN, FILTER, *sys.argv[6:]], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           text=True, timeout=600)
        el = time.time() - t
        fails = re.findall(r'^test (\S+) \.\.\. FAILED$', p.stdout, re.M)
        m = re.search(r'finished in ([\d.]+)s', p.stdout)
        r = {'run': i, 'exit': p.returncode, 'wall': round(el, 2), 'inner': m.group(1) if m else None, 'failed': fails}
        if p.returncode != 0:
            (out/f'run-{i}.log').write_text(p.stdout)
        print(json.dumps(r), flush=True)
        return r
    import concurrent.futures as cf
    with cf.ThreadPoolExecutor(LANES) as ex:
        for r in ex.map(one, range(RUNS)):
            results.append(r)
finally:
    for b in burn:
        b.kill()
    for b in burn:
        b.wait()
(out/'summary.json').write_text(json.dumps(results, indent=1))
bad = [r for r in results if r['exit'] != 0]
print(f'{len(results)} runs, {len(bad)} failed; failing tests:', sorted({t for r in bad for t in r["failed"]}))
