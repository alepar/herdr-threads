#!/usr/bin/env node
// One-shot generator for tests/fixtures/delivery/frame-<case>.{json,txt}: the input
// items and the exact output of the JS `frame()` in integrations/claude/mod/hooks/register.js
// (read-only), plus the one-file copy the mod's own test imports. Kept for regeneration: `node scripts/gen-frame-goldens.mjs`.
// The hook file has no package.json, so its source is loaded from a temporary .mjs copy;
// the hook file itself is never edited.
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const source = readFileSync(join(root, 'integrations/claude/mod/hooks/register.js'), 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'frame-goldens-'))
let frame
try {
  const copy = join(scratch, 'register.mjs')
  writeFileSync(copy, source)
  ;({ frame } = await import(pathToFileURL(copy).href))
} finally {
  rmSync(scratch, { recursive: true, force: true })
}

const msg = (id, extra = {}) => ({
  kind: 'message',
  id,
  thread: 'T1',
  threadName: 'plans',
  sender: 'S1',
  senderName: 'alice',
  body: `body of ${id}`,
  ...extra,
})

const cases = {
  empty: [],
  multiline: [
    msg('m1', { body: 'first\r\nsecond third\n\nfifth after an empty line\rsixth\u000bseventh\u000ceighth\u0085ninth tenth' }),
    msg('m2', { kind: 'lazy', body: '' }),
    msg('m3', { body: 'trailing break\n' }),
  ],
  controls: [
    msg('m1', {
      thread: 'T\u0001\u001f1\u007f\u009f',
      threadName: 'pl\tans\n"quoted" \\ back slash',
      sender: 'S\r\n1',
      senderName: 'al\u0000ice  "x"',
      body: 'ok',
    }),
    msg('m\u00072', { kind: 'mes\nsage', body: 'ok' }),
  ],
  'name-equals-id': [
    msg('m1', { threadName: 'T1', senderName: 'S1' }),
    msg('m2', { threadName: 'T1\n', senderName: 'S1' }),
    msg('m3', { thread: '', threadName: 'only a name', sender: '', senderName: '' }),
    msg('m4', { thread: '', threadName: '', sender: '', senderName: '' }),
  ],
  markers: [
    msg('m1', { author_role: 'human', body: 'human author' }),
    msg('m2', { relays_user: true, body: 'relays the user' }),
    msg('m3', { user_intent: 'query', body: 'query' }),
    msg('m4', { user_intent: 'request', body: 'request' }),
    msg('m5', { user_intent: 'rule', body: 'rule' }),
    msg('m6', { user_intent: 'bogus', body: 'unknown intent adds no marker' }),
    msg('m7', { author_role: 'human', relays_user: true, user_intent: 'rule', ack_required: true, body: 'all markers, ack required' }),
    msg('m8', { author_role: 'agent', relays_user: false, ack_required: true, body: 'no marker' }),
  ],
  // Tagged public ids (<prefix><tag><8>, 12 chars) and a direct-thread id (d<10>, 11 chars) are opaque to the frame.
  'tagged-ids': [
    msg('mQ7aAb12Cd34', { thread: 'tQ7aZz99Yy88', sender: 'sQ7aK3Fq9a2B' }),
    msg('mQ7aEf56Gh78', { kind: 'lazy', thread: 'dK3Fq9a2B7x', sender: 'sQ7aK3Fq9a2B', senderName: 'bob' }),
  ],
  attention: [
    { kind: 'attention', id: 'attention:4', body: 'You have pending herdr-threads items.' },
    msg('m1'),
    { kind: 'attention', id: 'attention:5\n', body: 'line one\nline two' },
  ],
}

const dir = join(root, 'tests/fixtures/delivery')
mkdirSync(dir, { recursive: true })
for (const [name, items] of Object.entries(cases)) {
  writeFileSync(join(dir, `frame-${name}.json`), `${JSON.stringify(items, null, 2)}\n`)
  writeFileSync(join(dir, `frame-${name}.txt`), frame(items))
}
const header = frame([]).replace(/\n\n$/, '')
writeFileSync(join(dir, 'frame-header.txt'), header)

// The Claude plugin sandbox only imports files inside the mod's folder, so its test reads
// this one-file module (a JSON value after `export default `); a Rust test keeps the copy equal to the fixtures above.
const copy = { header }
for (const [name, items] of Object.entries(cases)) copy[name] = { items, text: frame(items) }
const copyDir = join(root, 'integrations/claude/mod/tests/fixtures')
mkdirSync(copyDir, { recursive: true })
writeFileSync(join(copyDir, 'frame-goldens.ts'), `export default ${JSON.stringify(copy, null, 2)}\n`)
console.log(`wrote ${Object.keys(cases).length} cases and the header to ${dir}`)
