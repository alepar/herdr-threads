import test from 'node:test'
import assert from 'node:assert/strict'
import { register, nodeIO, frame } from '../herdr-threads.js'
import fs from 'node:fs'
import path from 'node:path'

// Controlled transport and clock only: callbacks, native draft results and receipt decisions
// execute the real registration/state machine. Native persistence is explicitly simulated.
function harness(options = {}) {
  const callbacks = new Map(), calls = [], watches = [], timers = [], wakes = [], diagnostics = []
  let branch = options.branch ?? [], sid = 'native-session-1', idle = true, draft = '', counter = 0
  const env = { ...options.env }, registry = options.registry ?? {}
  const io = {
    env, pid: 420, cwd: '/isolated/project', registry,
    nonce: () => `nonce-${++counter}`,
    run: async (argv, request) => {
      calls.push({ argv, ...request })
      if (options.run) return options.run(argv, request)
      if (argv.includes('ack')) return { code: 0, stdout: argv.slice(argv.indexOf('context') + 1).map(id => JSON.stringify({ id, result: 'settled' })).join('\n') }
      return { code: 0, stdout: argv.at(-1) === 'SessionStart' ? 'fixed guidance' : '' }
    },
    spawn: (argv, request, handlers) => {
      if (options.spawn) options.spawn(argv, request, handlers)
      const watch = { argv, request, ...handlers, stops: 0, stop() { this.stops++; return options.stop ? options.stop(this) : Promise.resolve() } }
      watches.push(watch); return watch
    },
    schedule: (ms, fn) => { const timer = { ms, fn, canceled: false }; timers.push(timer); return () => { timer.canceled = true } },
    diagnostic: text => diagnostics.push(text),
  }
  const pi = { on: (name, fn) => callbacks.set(name, fn), sendMessage: (...args) => wakes.push(args) }
  const ctx = {
    mode: options.mode ?? 'tui', cwd: '/isolated/project',
    sessionManager: { getSessionId: () => sid, getBranch: () => branch },
    isIdle: () => idle, ui: { getEditorText: () => draft },
  }
  register(pi, io)
  const emit = async (name, event = {}) => callbacks.get(name)?.({ type: name, ...event }, ctx)
  const start = async (reason = 'startup') => { await emit('session_start', { reason }); watches.at(-1)?.line({ schema: 1, id: 'status:0', kind: 'status', state: 'connected' }) }
  const line = (item) => watches.at(-1)?.line({ schema: 1, ...item })
  const message = (id, priority = 1, extras = {}) => line({ id, kind: 'message', body: `body ${id}`, body_len: 8, thread: 'thread', sender: 'sender', truncated: false, ack_required: true, wakes: false, priority, ...extras })
  const store = (result) => { const entries = result?.message ? [{ type: 'custom_message', ...result.message }] : result?.entries ?? []; branch.push(...entries.map((entry, i) => ({ id: `row-${branch.length + i}`, ...entry }))) }
  const context = () => emit('context', { messages: branch.filter(x => x.type === 'custom_message').map(x => ({ role: 'custom', customType: x.customType, content: x.content })) })
  const ackCalls = () => calls.filter(x => x.argv.includes('ack'))
  const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve() }
  const tick = async (ms) => { const due = timers.filter(x => !x.canceled && x.ms <= ms); for (const t of due) { t.canceled = true; t.fn() } await flush() }
  return { callbacks, calls, watches, wakes, diagnostics, env, registry, ctx, emit, start, line, message, store, context, ackCalls, flush, tick,
    setBranch: b => { branch = b }, branch: () => branch, setSid: s => { sid = s }, setIdle: v => { idle = v }, setDraft: s => { draft = s } }
}

test('admission uses native identity and refuses inherited child/nonterminal claims', async () => {
  for (const options of [{ env: { HERDR_THREADS_PI_PID: '419' } }, { env: { HERDR_THREADS_PI_ROLE: 'child' } }, { env: { HERDR_THREADS_PI_ROLE: 'mystery' } }, { mode: 'rpc' }, { mode: undefined, env: { HERDR_THREADS_PI_PID: 'bad' } }]) {
    const h = harness(options); await h.start(); assert.equal(h.watches.length, 0); assert.equal(h.calls.length, 0)
  }
  const h = harness(); await h.start()
  assert.equal(h.watches.length, 1)
  assert.deepEqual(JSON.parse(h.calls[0].stdin), { event: 'SessionStart', source: 'startup', session_id: 'native-session-1', role: 'top_level', mode: 'tui', pid: 420, cwd: '/isolated/project' })
  assert.deepEqual(h.watches[0].argv, ['herdr-threads', 'watch', '--harness', 'pi', '--session', 'native-session-1'])
  assert.equal(h.env.HERDR_THREADS_PI_SESSION, 'native-session-1')
})

test('sorted native page requires both finalized branch and exact context before ACK', async () => {
  const h = harness(); await h.start()
  h.message('third', 3); h.message('first', 0); h.message('second', 1)
  const r = await h.emit('before_agent_start'); assert.ok(r?.message)
  assert.match(r.message.content, /fixed guidance/)
  assert.ok(r.message.content.indexOf('body first') < r.message.content.indexOf('body second'))
  assert.ok(r.message.content.indexOf('body second') < r.message.content.indexOf('body third'))
  assert.equal(h.ackCalls().length, 0)
  h.store(r)
  await h.emit('context', { messages: [{ role: 'custom', customType: r.message.customType, content: r.message.content + ' altered' }] })
  assert.equal(h.ackCalls().length, 0)
  await h.context(); await h.flush()
  assert.deepEqual(h.ackCalls()[0].argv, ['herdr-threads', 'watch', 'ack', '--session', 'native-session-1', '--via', 'context', 'first', 'second', 'third'])
  assert.equal(h.wakes.length, 0)
  assert.equal((await h.emit('before_agent_start'))?.message, undefined)
})

test('a context-only preview cannot ACK rejected drafts; altered native entries are reproposed', async () => {
  const h = harness(); await h.start(); h.message('m')
  const r = await h.emit('before_agent_start')
  await h.emit('context', { messages: [{ role: 'custom', ...r.message }] })
  assert.equal(h.ackCalls().length, 0)
  const retry = await h.emit('before_agent_start'); assert.match(retry.message.content, /body m/)
  h.store({ message: { ...retry.message, content: 'transformed by later extension' } })
  await h.context(); assert.equal(h.ackCalls().length, 0)
  assert.match((await h.emit('before_agent_start')).message.content, /body m/)
})

test('only completed active boundaries append mail and preserve foreign entries/continuation', async () => {
  const h = harness(); await h.start(); h.setIdle(false); await h.emit('agent_start'); h.message('late')
  for (const outcome of ['error', 'aborted']) assert.equal(await h.emit('agent_before_settle', { outcome, entries: [], continue: false, context: {} }), undefined)
  const foreign = { type: 'custom', customType: 'other', data: { a: 1 } }
  const r = await h.emit('turn_end', { outcome: 'completed', entries: [foreign], continue: false, context: {} })
  assert.deepEqual(r.entries[0], foreign); assert.equal(r.continue, true)
  h.store(r)
  assert.equal(await h.emit('agent_before_settle', { outcome: 'completed', entries: [], continue: true, context: {} }), undefined)
  assert.equal(h.ackCalls().length, 0)
  await h.context(); await h.flush(); assert.equal(h.ackCalls().length, 1)
})

test('passive idle is silent; admitted wake is content-free, draft/UI guarded and single flight', async () => {
  const h = harness(); await h.start(); h.message('passive'); assert.equal(h.wakes.length, 0)
  h.setDraft('human draft'); h.message('urgent', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  h.setDraft(''); await h.emit('ui_prompt_start'); await h.emit('agent_settled'); assert.equal(h.wakes.length, 0)
  await h.emit('ui_prompt_end'); assert.equal(h.wakes.length, 1)
  assert.deepEqual(h.wakes[0][0].content, 'herdr-threads: wake up')
  assert.deepEqual(h.wakes[0][1], { triggerTurn: true })
  h.message('urgent2', 0, { wakes: true }); assert.equal(h.wakes.length, 1)
  await h.tick(30000); assert.equal(h.wakes.length, 1)
  assert.match(h.diagnostics.join('\n'), /wake unreceived/)
  await h.emit('agent_settled'); assert.equal(h.wakes.length, 2)
})

test('overflow keeps urgent mail and repays every overflow via replacement after ACK release', async () => {
  const h = harness(); await h.start()
  for (let i = 0; i < 257; i++) h.message(`old-${i}`, 3)
  h.message('urgent', 0, { wakes: true })
  assert.equal(h.watches.length, 1)
  const r = await h.emit('before_agent_start')
  assert.match(r.message.content, /body urgent/)
  h.store(r); await h.context(); await h.flush()
  assert.equal(h.watches.length, 2)
  assert.equal(h.watches[0].stops, 1)
  h.line({ id: 'status:1', kind: 'status', state: 'connected' })
  // Canonical replay is independent of exhausted logical reoffers on the old watch.
  h.message('old-256', 3)
  assert.equal(h.watches.length, 2)
})

test('lower-ranked incoming overflow creates replay debt without displacement', async () => {
  const h = harness(); await h.start()
  for (let i = 0; i < 256; i++) h.message(`retained-${i}`, 0)
  h.message('discarded', 9)
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  assert.equal(h.watches.length, 2)
})

test('failed ACK retries evidenced IDs; changed branch fences receipt retries', async () => {
  let fail = true
  const h = harness({ run: async argv => argv.includes('ack') ? { code: fail ? 1 : 0, stdout: '{"id":"m","result":"settled"}' } : { code: 0, stdout: '' } })
  await h.start(); h.message('m'); const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  assert.equal(h.ackCalls().length, 1)
  fail = false; await h.tick(30000); assert.equal(h.ackCalls().length, 2)
  h.message('n'); const n = await h.emit('before_agent_start'); h.store(n); await h.context(); await h.flush()
  const before = h.ackCalls().length
  h.setBranch([]); await h.emit('session_tree'); await h.tick(30000)
  assert.equal(h.ackCalls().length, before)
  assert.match((await h.emit('before_agent_start')).message.content, /body n/)
})

test('shutdown fences delayed admission, old watch callbacks and successor environment', async () => {
  let resolve
  const h = harness({ run: argv => argv.at(-1) === 'SessionStart' ? new Promise(r => { resolve = r }) : Promise.resolve({ code: 0, stdout: '' }) })
  const starting = h.start(); await h.flush(); const closing = h.emit('session_shutdown')
  h.env.HERDR_THREADS_PI_SESSION = 'successor'
  resolve({ code: 0, stdout: 'stale guidance' }); await starting; await closing
  assert.equal(h.watches.length, 0); assert.equal(h.env.HERDR_THREADS_PI_SESSION, 'successor')
  const live = harness(); await live.start(); const old = live.watches[0]; await live.emit('session_shutdown')
  old.line({ schema: 1, id: 'stale', kind: 'attention', text: 'old', attention_version: 1 })
  assert.equal(live.wakes.length, 0); assert.equal(old.stops, 1)
})

test('duplicate process installation stays inert and lifecycle admission failures retry', async () => {
  const registry = {}; const a = harness({ registry }), b = harness({ registry })
  await a.start(); await b.start(); assert.equal(a.watches.length + b.watches.length, 1)
  let count = 0
  const h = harness({ run: async argv => ({ code: 0, stdout: argv.at(-1) === 'SessionStart' && ++count === 1 ? 'herdr-threads: check-in unavailable (offline)' : '' }) })
  await h.start(); assert.equal(h.watches.length, 0)
  await h.emit('before_agent_start'); assert.equal(h.watches.length, 1)
})

test('concurrent duplicate starts cannot acquire two process owners', async () => {
  const registry = {}, a = harness({ registry }), b = harness({ registry })
  await Promise.all([a.start(), b.start()])
  assert.equal(a.watches.length + b.watches.length, 1)
  assert.equal(a.calls.length + b.calls.length, 1)
})

test('context consumption clears wake intent even when ACK fails, and settled replay stays silent', async () => {
  const h = harness({ run: async argv => ({ code: argv.includes('ack') ? 1 : 0, stdout: '' }) })
  await h.start(); h.message('m', 0, { wakes: true }); assert.equal(h.wakes.length, 1)
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  await h.emit('agent_settled'); assert.equal(h.wakes.length, 1)
  const done = harness(); await done.start(); done.message('m', 0, { wakes: true })
  const first = await done.emit('before_agent_start'); done.store(first); await done.context(); await done.flush(); await done.emit('agent_settled')
  done.message('m', 0, { wakes: true }); assert.equal(done.wakes.length, 1)
})

test('branch changes fence an in-flight ACK result even after new context evidence', async () => {
  const resolvers = []
  const h = harness({ run: async argv => argv.includes('ack') ? new Promise(resolve => resolvers.push(resolve)) : { code: 0, stdout: '' } })
  await h.start(); h.message('m'); const r = await h.emit('before_agent_start'); h.store(r); await h.context()
  await h.emit('session_tree'); await h.context()
  assert.equal(resolvers.length, 2)
  resolvers[0]({ code: 0, stdout: '{"id":"m","result":"settled"}' }); await h.flush()
  resolvers[1]({ code: 1, stdout: '' }); await h.flush()
  await h.tick(30000)
  assert.equal(h.ackCalls().length, 3)
})

test('failed replacement preserves overflow debt and retries with bounded backoff', async () => {
  let spawns = 0
  const h = harness({ spawn: () => { if (++spawns === 2) throw new Error('transport unavailable') } })
  await h.start(); for (let i = 0; i < 257; i++) h.message(`m-${i}`)
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  assert.equal(h.watches.length, 1); assert.match(h.diagnostics.join('\n'), /watch unavailable/)
  await h.tick(1000); assert.equal(h.watches.length, 2)
  h.line({ id: 'status:1', kind: 'status', state: 'connected' })
  // A second overflow after successful recovery must acquire fresh replay debt.
  for (let i = 257; i < 300; i++) h.message(`m-${i}`)
  const next = await h.emit('before_agent_start'); h.store(next); await h.context(); await h.flush()
  assert.equal(h.watches.length, 3)
})

test('large-body pressure remains bounded and later urgent arrivals are delivered first', async () => {
  const h = harness(); await h.start()
  const big = 'z'.repeat(8192)
  for (let i = 0; i < 300; i++) h.message(`large-${i}`, 3, { body: big })
  h.message('urgent', 0, { wakes: true })
  let total = 0
  for (let page = 0; page < 40; page++) {
    const r = await h.emit('before_agent_start')
    if (!r?.message) break
    const text = r.message.content
    assert.ok(Buffer.byteLength(text) <= 65536)
    assert.ok((text.match(/\[herdr-threads\] message /g) ?? []).length <= 32)
    if (page === 0) assert.ok(text.indexOf('body urgent') < text.indexOf(big))
    total += Buffer.byteLength(text)
    h.store(r); await h.context(); await h.flush()
    h.line({ id: 'status:1', kind: 'status', state: 'connected' })
  }
  // Only the bounded retained prefix can enter drafts without canonical restreaming.
  assert.ok(total < 2 * 1024 * 1024 + 8192, `retained ${total} bytes`)
  assert.ok(total > 1024 * 1024)
})

test('attention clears are epoch scoped and invitations/truncation never ACK as mail', async () => {
  const h = harness(); await h.start(); h.setDraft('hold')
  h.line({ kind: 'attention', id: 'attention:1', text: 'attention one', attention_version: 1, presentation_epoch: '11' })
  h.line({ kind: 'attention', id: 'attention:2', text: 'attention two', attention_version: 2, presentation_epoch: '12' })
  h.line({ kind: 'attention_cleared', id: 'clear:1', attention_version: 1, presentation_epoch: '11' })
  h.line({ kind: 'invitation', id: 'invite', thread: 't', body: 'accept manually', truncated: false, wakes: false, topic_revision: '1' })
  h.message('cut', 1, { truncated: true, body: 'prefix\n…truncated; next: herdr-threads inbox' })
  const r = await h.emit('before_agent_start'); assert.doesNotMatch(r.message.content, /attention one/); assert.match(r.message.content, /attention two/)
  assert.match(r.message.content, /next: herdr-threads inbox/)
  h.store(r); await h.context(); await h.flush(); assert.equal(h.ackCalls().length, 0)
})

test('foreign-session callbacks cannot consume or shut down the successor owner', async () => {
  const h = harness(); await h.start(); const oldCtx = { ...h.ctx, sessionManager: { getSessionId: () => 'old-session', getBranch: () => [] } }
  await h.callbacks.get('session_shutdown')({ type: 'session_shutdown' }, oldCtx)
  h.message('m'); assert.ok((await h.emit('before_agent_start'))?.message)
})

test('missing editor API disables only wake; manual compaction prevents boundary continuation', async () => {
  const h = harness(); await h.start(); delete h.ctx.ui.getEditorText
  h.message('m', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  assert.match(h.diagnostics.join('\n'), /wake API unavailable/)
  h.setIdle(false); await h.emit('agent_start'); await h.emit('session_before_compact', { reason: 'manual' })
  assert.equal(await h.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} }), undefined)
  await h.emit('session_compact')
  assert.match((await h.emit('before_agent_start')).message.content, /body m/)
})

test('reload reconciles accepted unseen frames instead of duplicating their mail', async () => {
  const first = harness(); await first.start(); first.message('m'); const r = await first.emit('before_agent_start'); first.store(r)
  await first.emit('session_shutdown', { reason: 'reload' })
  const next = harness({ registry: first.registry, branch: first.branch(), env: first.env })
  await next.start('reload'); next.message('m')
  const startup = await next.emit('before_agent_start')
  assert.doesNotMatch(startup?.message?.content ?? '', /body m/)
  assert.equal(next.ackCalls().length, 0)
  next.store(startup); await next.context(); await next.flush()
  assert.equal(next.ackCalls().length, 1)
})

test('cross-page ordering uses deadlines then stable first observation across reoffers', async () => {
  const h = harness(); await h.start()
  h.message('stable-a', 1); h.message('stable-b', 1); h.message('stable-a', 1)
  h.message('deadline-later', 0, { deadline_at: 300 }); h.message('deadline-first', 0, { deadline_at: 100 })
  h.message('deadline-none', 0)
  const r = await h.emit('before_agent_start'); const text = r.message.content
  const names = ['deadline-first', 'deadline-later', 'deadline-none', 'stable-a', 'stable-b']
  for (let i = 1; i < names.length; i++) assert.ok(text.indexOf(`body ${names[i-1]}`) < text.indexOf(`body ${names[i]}`))
})

test('per-item framing matches the shared delivery goldens', () => {
  const fixtures = new URL('../../../../tests/fixtures/delivery/', import.meta.url)
  for (const filename of fs.readdirSync(fixtures).filter(name => name.endsWith('.json'))) {
    const items = JSON.parse(fs.readFileSync(new URL(filename, fixtures), 'utf8'))
    const normalized = items.map(x => ({ ...x, thread_name: x.threadName, sender_name: x.senderName, sender_label: x.senderLabel, text: x.body }))
    const actual = '[herdr-threads] new messages:\n\n' + normalized.map(frame).join('\n\n')
    assert.equal(actual, fs.readFileSync(new URL(filename.replace('.json', '.txt'), fixtures), 'utf8'), filename)
  }
})

test('native IO bounds stdout, honors deadlines and reaps watch children', async () => {
  // No service is started: owned node children exercise the real transport.
  const root = fs.mkdtempSync(path.join(path.dirname(new URL(import.meta.url).pathname), '.scratch-io-'))
  const io = nodeIO()
  const env = { HOME: root, PATH: process.env.PATH, HT_LEAK_RUN_ID: process.env.HT_LEAK_RUN_ID ?? 'pi-extension-io' }
  try {
    const r = await io.run([process.execPath, '-e', 'process.stdout.write("oversized output");setInterval(()=>{},1000)'], { env, stdin: '', timeoutMs: 500, maxBytes: 4 })
    assert.equal(r.code, 1)
    const timed = await io.run([process.execPath, '-e', 'setInterval(()=>{},1000)'], { env, stdin: '', timeoutMs: 30, maxBytes: 4096 })
    assert.equal(timed.code, 1)
    let resolveLine, exited = false
    const line = new Promise(resolve => { resolveLine = resolve })
    const watcher = io.spawn([process.execPath, '-e', 'console.log(JSON.stringify({id:"status:0",schema:1,kind:"status",state:"connected"}));setInterval(()=>{},1000)'], { env, maxBytes: 65536 }, { line: resolveLine, exit: () => { exited = true } })
    assert.equal((await line).state, 'connected'); await watcher.stop(); assert.equal(exited, true)
  } finally { fs.rmSync(root, { recursive: true, force: true }) }
})

test('a replaced draft is attempted once per outer run, then waits for a later authentic run', async () => {
  const h = harness(); await h.start(); await h.emit('before_agent_start'); h.message('m'); h.setIdle(false)
  const boundary = { outcome: 'completed', entries: [], continue: false, context: {} }
  const r = await h.emit('turn_end', boundary); assert.equal(r.continue, true)
  h.store({ entries: r.entries.map(e => ({ ...e, content: 'later extension replacement' })) })
  await h.context()
  assert.equal(await h.emit('turn_end', boundary), undefined)
  assert.equal(await h.emit('agent_before_settle', boundary), undefined)
  assert.match((await h.emit('before_agent_start')).message.content, /body m/)
})

test('rejected urgent drafts and aborted runs cannot restart themselves at settlement', async () => {
  const h = harness(); await h.start(); h.setIdle(false); await h.emit('before_agent_start', { messageType: 'user' }); h.message('m', 0, { wakes: true })
  const r = await h.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} })
  h.store({ entries: r.entries.map(e => ({ ...e, content: 'rejected urgent frame' })) }); await h.context()
  h.setIdle(true); await h.emit('agent_settled', { aborted: false }); assert.equal(h.wakes.length, 0)
  h.message('m', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  h.message('new-attention', 0, { wakes: true }); assert.equal(h.wakes.length, 1)
  const aborted = harness(); await aborted.start(); aborted.setIdle(false); await aborted.emit('agent_start'); aborted.message('m', 0, { wakes: true })
  await aborted.emit('agent_before_settle', { outcome: 'error', entries: [], continue: false, context: {} })
  aborted.setIdle(true); await aborted.emit('agent_settled', { aborted: false }); assert.equal(aborted.wakes.length, 0)
})

test('partial ACK release repays overflow debt while failed exact IDs keep retry evidence', async () => {
  const h = harness({ run: async argv => argv.includes('ack') ? { code: 0, stdout: argv.slice(argv.indexOf('context') + 1).filter(id => id !== 'm-0').map(id => JSON.stringify({ id, result: 'settled' })).join('\n') } : { code: 0, stdout: '' } })
  await h.start(); for (let i = 0; i < 257; i++) h.message(`m-${i}`)
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  assert.equal(h.watches.length, 2)
  h.line({ id: 'status:1', kind: 'status', state: 'connected' })
  await h.tick(30000)
  assert.deepEqual(h.ackCalls().at(-1).argv.slice(-1), ['m-0'])
})

test('shutdown aborts and awaits each owned command before cleanup completes', async () => {
  let release, signal, closed = false
  const h = harness({ run: async (_argv, request) => { signal = request.signal; return new Promise(resolve => { release = resolve }) } })
  const starting = h.start(); await h.flush()
  const closing = h.emit('session_shutdown').then(() => { closed = true }); await h.flush()
  assert.equal(signal.aborted, true); assert.equal(closed, false)
  release({ code: 1, stdout: '' }); await closing; await starting; assert.equal(closed, true)
})

test('missing native branch shape degrades delivery without inventing receipt evidence', async () => {
  const h = harness(); await h.start(); h.message('m')
  h.ctx.sessionManager.getBranch = () => null
  assert.equal(await h.emit('before_agent_start'), undefined)
  assert.equal(h.ackCalls().length, 0)
  assert.match(h.diagnostics.join('\n'), /branch API unavailable/)
  for (const mode of [undefined, 'rpc', 'json', 'print']) {
    const other = harness(); other.ctx.mode = mode; await other.start(); assert.equal(other.calls.length, 0)
  }
})

test('compact reentry refreshes guidance at a completed boundary in the same outer run', async () => {
  const h = harness(); await h.start(); const initial = await h.emit('before_agent_start'); h.store(initial); await h.context(); await h.flush()
  await h.emit('session_compact')
  const r = await h.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} })
  assert.match(r?.entries[0].content ?? '', /fixed guidance/)
  assert.equal(r.continue, false, 'guidance alone must not force another request')
})

test('late lifecycle guidance cannot overwrite a newer branch revision', async () => {
  const waiting = new Map()
  const h = harness({ run: async (_argv, request) => {
    const event = JSON.parse(request.stdin || '{}')
    return ['tree', 'compact'].includes(event.source) ? new Promise(resolve => waiting.set(event.source, resolve)) : { code: 0, stdout: '' }
  } })
  await h.start(); const tree = h.emit('session_tree'), compact = h.emit('session_compact')
  waiting.get('compact')({ code: 0, stdout: 'new compact guidance' }); await compact
  waiting.get('tree')({ code: 0, stdout: 'stale tree guidance' }); await tree
  const r = await h.emit('before_agent_start')
  assert.match(r.message.content, /new compact guidance/); assert.doesNotMatch(r.message.content, /stale tree/)
})

test('failed compact admission retries its own source on the next native run', async () => {
  let compacts = 0
  const h = harness({ run: async (_argv, request) => {
    const event = JSON.parse(request.stdin || '{}')
    return event.source === 'compact' ? { code: ++compacts === 1 ? 1 : 0, stdout: 'compact guidance' } : { code: 0, stdout: '' }
  } })
  await h.start(); await h.emit('session_compact'); await h.emit('before_agent_start')
  assert.equal(compacts, 2)
})

test('alternating unchanged urgent keys cannot release rejected or error attention', async () => {
  for (const outcome of ['completed', 'error', 'aborted']) {
    const h = harness(); await h.start(); h.setIdle(false)
    await h.emit('before_agent_start', { messageType: 'user' })
    h.message('urgent-a', 0, { wakes: true }); h.message('urgent-b', 0, { wakes: true })
    const r = await h.emit('turn_end', { outcome, entries: [], continue: false, context: {} })
    if (outcome === 'completed') {
      h.store({ entries: r.entries.map(entry => ({ ...entry, content: 'later extension rejected both urgent frames' })) })
      await h.context()
    }
    h.setIdle(true); await h.emit('agent_settled', { aborted: outcome === 'aborted' })
    assert.equal(h.wakes.length, 0, outcome)
    h.line({ kind: 'status', id: 'status:duplicate', state: 'connected' })
    assert.equal(h.wakes.length, 0, 'repeated same-channel status is not a replacement')
    for (const id of ['urgent-a', 'urgent-b', 'urgent-a', 'urgent-b']) {
      h.message(id, 0, { wakes: true })
      assert.equal(h.wakes.length, 0, `${outcome}: unchanged ${id}`)
    }
    h.message('genuinely-new-urgent', 0, { wakes: true })
    assert.equal(h.wakes.length, 1, `${outcome}: new attention may wake`)
    await h.emit('session_shutdown')
  }
})

test('unchanged urgent keys cannot release no-progress wake suspension', async () => {
  const h = harness(); await h.start(); h.setDraft('hold wake')
  h.message('urgent-a', 0, { wakes: true }); h.message('urgent-b', 0, { wakes: true })
  h.setDraft(''); await h.emit('agent_settled'); assert.equal(h.wakes.length, 1)
  await h.tick(30000)
  for (const id of ['urgent-a', 'urgent-b', 'urgent-a']) h.message(id, 0, { wakes: true })
  assert.equal(h.wakes.length, 1)
  h.message('new-urgent', 0, { wakes: true }); assert.equal(h.wakes.length, 2)
  await h.emit('session_shutdown')
})

test('shutdown joins a watch detached by overflow replacement before reporting completion', async () => {
  let releaseStop, closed = false
  const stopped = new Promise(resolve => { releaseStop = resolve })
  const h = harness({ stop: () => stopped }); await h.start()
  for (let i = 0; i < 257; i++) h.message(`overflow-${i}`)
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  assert.equal(h.watches[0].stops, 1)
  const closing = h.emit('session_shutdown', { reason: 'quit' }).then(() => { closed = true })
  await h.flush()
  try {
    assert.equal(closed, false, 'predecessor watch has not exited yet')
    h.watches[0].line({ schema: 1, id: 'old-urgent', kind: 'attention', attention_version: 1, text: 'stale' })
    assert.equal(h.wakes.length, 0)
    assert.equal(h.watches.length, 1)
  } finally { releaseStop(); await closing; await h.flush() }
  assert.equal(closed, true); assert.equal(h.watches.length, 1, 'closed replacement cannot start a successor')
})

test('reload waits for a detached predecessor watch, including an already closing owner', async () => {
  for (const shutdownFirst of [false, true]) {
    let releaseStop, reloaded = false
    const stopped = new Promise(resolve => { releaseStop = resolve })
    const h = harness({ stop: watch => watch === h.watches[0] ? stopped : Promise.resolve() }); await h.start()
    for (let i = 0; i < 257; i++) h.message(`overflow-${i}`)
    const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
    assert.equal(h.watches[0].stops, 1)
    const closing = shutdownFirst ? h.emit('session_shutdown', { reason: 'reload' }) : Promise.resolve()
    const reloading = h.start('reload').then(() => { reloaded = true }); await h.flush()
    try {
      assert.equal(reloaded, false, `shutdownFirst=${shutdownFirst}`)
      assert.equal(h.calls.filter(call => call.argv.at(-1) === 'SessionStart').length, 1)
      assert.equal(h.watches.length, 1)
    } finally { releaseStop(); await closing; await reloading; await h.flush() }
    assert.equal(reloaded, true); assert.equal(h.watches.length, 2)
    assert.equal(h.watches[0].stops, 1, 'one owner reaps the predecessor exactly once')
    await h.emit('session_shutdown', { reason: 'quit' })
  }
})

test('saturated discarded urgent history cannot turn old or ambiguous keys into new wake authority', async () => {
  for (const outcome of ['error', 'no-progress']) {
    const h = harness(); await h.start(); h.setDraft('hold idle wake')
    if (outcome === 'error') { h.setIdle(false); await h.emit('before_agent_start', { messageType: 'user' }) }
    // Keep 256 better passive records. All lower-ranked admitted urgent bodies
    // are discarded; their waking identities must still fence later reoffers.
    for (let i = 0; i < 256; i++) h.message(`retained-${i}`, 0)
    for (let i = 0; i < 300; i++) h.message(`discarded-${i}`, 9, { wakes: true })
    if (outcome === 'error') {
      await h.emit('turn_end', { outcome: 'error', entries: [], continue: false, context: {} })
      h.setIdle(true); h.setDraft(''); await h.emit('agent_settled')
    } else {
      h.setDraft(''); await h.emit('agent_settled'); assert.equal(h.wakes.length, 1)
      await h.tick(30000)
    }
    const before = h.wakes.length
    for (const i of [0, 299, 1, 298, 0, 299]) h.message(`discarded-${i}`, 9, { wakes: true })
    for (let i = 300; i < 600; i++) h.message(`discarded-${i}`, 9, { wakes: true })
    assert.equal(h.wakes.length, before, `${outcome}: bounded history cannot classify ambiguous keys as new`)
    await h.emit('agent_settled'); assert.equal(h.wakes.length, before, 'settlement alone does not reset saturated history')
    // An explicitly permitted user transition starts a fresh bounded history.
    h.setIdle(false); await h.emit('before_agent_start', { messageType: 'user' })
    h.message('after-user-transition', 9, { wakes: true })
    h.setIdle(true); await h.emit('agent_settled')
    assert.equal(h.wakes.length, before + 1)
    await h.emit('session_shutdown')
  }
})

test('canonical urgent replay re-arms a recovered unobserved frame only for the successor owner', async () => {
  const first = harness(); await first.start(); first.setIdle(false)
  const guidance = await first.emit('before_agent_start', { messageType: 'user' }); first.store(guidance); await first.context()
  first.message('urgent-recovered', 0, { wakes: true })
  const r = await first.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} })
  first.store(r)
  first.setIdle(true); await first.emit('agent_settled', { aborted: true })
  await first.emit('session_shutdown', { reason: 'reload' })
  assert.equal(first.ackCalls().length, 0, 'accepted branch entry is not context evidence')
  const next = harness({ registry: first.registry, branch: first.branch(), env: first.env })
  await next.start('reload')
  assert.equal(next.wakes.length, 0, 'recovery metadata alone has no canonical wake authority')
  next.message('urgent-recovered', 0, { wakes: false })
  assert.equal(next.wakes.length, 0, 'a current passive replay cannot inherit old wake eligibility')
  next.message('urgent-recovered', 0, { wakes: true })
  assert.equal(next.wakes.length, 1)
  assert.deepEqual(next.wakes[0], [{ customType: 'herdr-threads-wake', content: 'herdr-threads: wake up', display: false }, { triggerTurn: true }])
  next.message('urgent-recovered', 0, { wakes: true }); assert.equal(next.wakes.length, 1)
  assert.equal(next.ackCalls().length, 0)
  const startup = await next.emit('before_agent_start', { messageType: 'custom' })
  assert.doesNotMatch(startup?.message?.content ?? '', /body urgent-recovered/)
  next.store(startup)
  assert.equal(next.branch().filter(entry => String(entry.content).includes('body urgent-recovered')).length, 1)
  assert.equal(next.ackCalls().length, 0)
  await next.context(); await next.flush()
  assert.deepEqual(next.ackCalls()[0].argv, ['herdr-threads', 'watch', 'ack', '--session', 'native-session-1', '--via', 'context', 'urgent-recovered'])
  await next.emit('session_shutdown', { reason: 'quit' })
})
