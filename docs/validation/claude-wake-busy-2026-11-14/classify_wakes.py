#!/usr/bin/env python3
"""Classify plugin-submitted `wake up` prompts in Claude Code session transcripts.

Read-only. Usage: classify_wakes.py PROJECTS_DIR OUT.json

A wake is the mod's `$.prompt.submit('wake up')`: the transcript records it as a
queue-operation `enqueue` with content `wake up`, and later as a user row with
origin.kind == 'plugin' when the engine starts its turn. For each enqueue this
records what the main session was doing at that instant (rows sorted by
timestamp, sidechains excluded) and how long the prompt waited for its turn.
"""
import glob, json, os, re, sys
from datetime import datetime

WAKE_ROW = 'The herdr-threads plugin sent a message:\nwake up'

def ts(s):
    return datetime.fromisoformat(s.replace('Z', '+00:00')).timestamp()

def text_of(c):
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return ' '.join(str(x.get('text') or x.get('content') or '') for x in c if isinstance(x, dict))
    return ''

def load(path):
    rows = []
    for line in open(path, errors='replace'):
        if line.startswith('{"type":"api-request-blob"') or '"api-request-blob"' in line[-200:]:
            continue
        try:
            d = json.loads(line)
        except Exception:
            continue
        if d.get('isSidechain') or not d.get('timestamp'):
            continue
        rows.append((ts(d['timestamp']), d))
    rows.sort(key=lambda r: r[0])
    return rows

def agent_spans(path):
    """(first, last) timestamps of every subagent transcript of this session."""
    spans = []
    for f in glob.glob(os.path.join(path[:-6], 'subagents', '**', '*.jsonl'), recursive=True):
        first = last = None
        with open(f, errors='replace') as fh:
            for line in fh:
                m = re.search(r'"timestamp":"([^"]+)"', line[-600:]) or re.search(r'"timestamp":"([^"]+)"', line)
                if m:
                    t = ts(m.group(1))
                    first = first if first is not None else t
                    last = t
        if first is not None:
            spans.append((first, last))
    return spans

def scan(path):
    rows = load(path)
    spans = agent_spans(path)
    out, waiting = [], []
    open_turn, turn_start, turn_kind = False, None, None
    last, last_turn_end, last_append = None, None, None
    pending, bg = {}, set()
    prompts = []  # (ts, kind, source) of every non-wake prompt that started a turn
    version = None
    for when, d in rows:
        t = d.get('type')
        version = d.get('version') or version
        if t == 'queue-operation':
            if d.get('operation') == 'enqueue' and d.get('content') == 'wake up':
                rec = {
                    'session': os.path.basename(path)[:-6], 'at': d['timestamp'], 'version': version,
                    'turn_open': open_turn, 'turn_kind': turn_kind if open_turn else None,
                    'turn_age_s': round(when - turn_start, 3) if open_turn else None,
                    'last_kind': last[0] if last else None,
                    'last_age_s': round(when - last[1], 3) if last else None,
                    'pending_tools': sorted(set(pending.values())),
                    'since_turn_end_s': round(when - last_turn_end, 3) if last_turn_end and not open_turn else None,
                    'append_age_s': round(when - last_append, 3) if last_append else None,
                    'bg_shells': len(bg),
                    'subagent_active': any(a <= when <= b for a, b in spans),
                    '_t': when,
                }
                out.append(rec)
                waiting.append(rec)
            continue
        if t == 'system':
            if d.get('subtype') == 'turn_duration':
                open_turn, last_turn_end, pending = False, when, {}
                last = ('turn_end', when)
            continue
        if t == 'assistant':
            m = d.get('message', {})
            if not open_turn:
                open_turn, turn_start, turn_kind = True, when, 'unknown'
            for x in m.get('content') or []:
                if isinstance(x, dict) and x.get('type') == 'tool_use':
                    pending[x.get('id')] = x.get('name')
            last = ('assistant:' + str(m.get('stop_reason')), when)
            continue
        if t != 'user':
            continue
        c = d.get('message', {}).get('content')
        txt = text_of(c)
        kind = (d.get('origin') or {}).get('kind')
        if kind == 'plugin' and txt.startswith(WAKE_ROW):
            if waiting:
                w = waiting.pop(0)
                w['wait_s'] = round(when - w['_t'], 3)
                w['_w'] = when
            open_turn, turn_start, turn_kind = True, when, 'wake'
            last = ('wake_prompt', when)
            continue
        if isinstance(c, list) and any(isinstance(x, dict) and x.get('type') == 'tool_result' for x in c):
            for x in c:
                if isinstance(x, dict) and x.get('type') == 'tool_result':
                    pending.pop(x.get('tool_use_id'), None)
            m = re.search(r'running in background with ID: (\w+)', txt)
            if m:
                bg.add(m.group(1))
            last = ('tool_result', when)
            continue
        if d.get('isMeta'):
            if kind == 'plugin' and txt.startswith('[herdr-threads]'):
                last_append = when
            continue
        if d.get('isCompactSummary') or d.get('isVisibleInTranscriptOnly'):
            continue
        if '[Request interrupted by user' in txt:
            open_turn, last_turn_end, pending = False, when, {}
            last = ('interrupted', when)
            continue
        kind = kind or d.get('promptSource')
        if not kind:
            continue  # local command echo and similar rows start no turn
        if kind == 'task-notification':
            m = re.search(r'<task-id>(\w+)</task-id>', txt)
            if m:
                bg.discard(m.group(1))
        prompts.append((when, kind, d.get('promptSource')))
        if not open_turn:
            open_turn, turn_start, turn_kind = True, when, kind
        last = ('prompt:' + str(kind), when)
    for r in out:
        t, w = r.pop('_t'), r.pop('_w', None)
        # the nearest other prompt that began a turn around this wake's submission
        near = [(p[0] - t, p[1], p[2]) for p in prompts if t - 2 <= p[0] <= (w if w is not None else t + 2)]
        if near:
            dt, k, src = min(near, key=lambda n: abs(n[0]))
            r['rival'] = {'kind': k, 'source': src, 'dt_s': round(dt, 3)}
    return out

def main():
    rows = []
    for f in sorted(glob.glob(os.path.join(sys.argv[1], '*', '*.jsonl'))):
        rows += scan(f)
    json.dump(rows, open(sys.argv[2], 'w'), indent=0)
    print(len(rows), 'wake submissions in', len({r['session'] for r in rows}), 'sessions')

if __name__ == '__main__':
    main()
