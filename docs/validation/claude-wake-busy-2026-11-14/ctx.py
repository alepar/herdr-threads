#!/usr/bin/env python3
"""Print main-chain transcript entries around a timestamp. Usage: ctx.py PROJECTS_DIR SESSION ISO_TS [BEFORE] [AFTER]"""
import glob, json, os, sys
root, sid, at = sys.argv[1:4]
before = int(sys.argv[4]) if len(sys.argv) > 4 else 8
after = int(sys.argv[5]) if len(sys.argv) > 5 else 5
f = glob.glob(os.path.join(root, '*', sid + '.jsonl'))[0]
rows = []
for line in open(f, errors='replace'):
    if line.startswith('{"type":"api-request-blob"') or '"api-request-blob"' in line[-200:]:
        continue
    try:
        d = json.loads(line)
    except Exception:
        continue
    if d.get('isSidechain') or not d.get('timestamp'):
        continue
    rows.append(d)
idx = next(i for i, d in enumerate(rows) if d['type'] == 'queue-operation' and d['timestamp'] == at)
for d in rows[max(0, idx - before): idx + after + 1]:
    m = d.get('message') if isinstance(d.get('message'), dict) else {}
    c = m.get('content')
    if isinstance(c, list):
        c = ' | '.join(x.get('type', '') + ':' + str(x.get('text') or x.get('name') or x.get('content') or '')[:70] for x in c if isinstance(x, dict))
    ex = {k: d[k] for k in ('subtype', 'operation', 'content', 'isMeta', 'durationMs', 'queuePriority') if k in d}
    if 'origin' in d: ex['origin'] = d['origin'].get('kind')
    print(('>> ' if d is rows[idx] else '   ') + d['timestamp'][11:], d['type'], m.get('stop_reason') or '', str(c)[:150].replace('\n', '\\n') if c else '', str(ex)[:200].replace('\n', '\\n'))
