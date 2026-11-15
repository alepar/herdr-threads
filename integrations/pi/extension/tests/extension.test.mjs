import test from 'node:test'
import assert from 'node:assert/strict'
import { register, nodeIO, frame } from '../herdr-threads.js'
import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'

// Controlled transport and clock only: callbacks, native draft results and receipt decisions
// execute the real registration/state machine. Native persistence is explicitly simulated.
let harnessId = 0
function harness(options = {}) {
  const instance = ++harnessId
  const callbacks = new Map(), calls = [], watches = [], timers = [], wakes = [], diagnostics = []
  let branch = options.branch ?? [], sid = 'native-session-1', idle = true, draft = '', counter = 0
  const env = { ...options.env }, registry = options.registry ?? {}
  const io = {
    env, pid: 420, cwd: '/isolated/project', registry,
    nonce: () => `nonce-${instance}-${++counter}`,
    run: async (argv, request) => {
      calls.push({ argv, ...request })
      if (options.run) return options.run(argv, request)
      if (argv.includes('presentation-confirm')) return { code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}\n' }
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
    isIdle: () => idle, ui: { getEditorText: () => draft, notify: text => diagnostics.push(text) },
  }
  register(pi, io)
  const emit = async (name, event = {}) => callbacks.get(name)?.({ type: name, ...event }, ctx)
  const start = async (reason = 'startup') => { await emit('session_start', { reason }); watches.at(-1)?.line({ schema: 1, id: 'status:0', kind: 'status', state: 'connected' }) }
  const line = (item, rawBytes) => watches.at(-1)?.line({ schema: 1, ...item }, rawBytes)
  const message = (id, priority = 1, extras = {}) => line({ id, kind: 'message', body: `body ${id}`, body_len: 8, thread: 'thread', sender: 'sender', truncated: false, ack_required: true, wakes: false, priority, ...extras })
  const store = (result) => { const entries = result?.message ? [{ type: 'custom_message', ...result.message }] : result?.entries ?? []; branch.push(...entries.map((entry, i) => ({ id: `row-${branch.length + i}`, ...entry }))) }
  const context = () => emit('context', { messages: branch.filter(x => x.type === 'custom_message').map(x => ({ role: 'custom', customType: x.customType, content: x.content })) })
  const ackCalls = () => calls.filter(x => x.argv.includes('ack'))
  const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve() }
  const tick = async (ms) => { const due = timers.filter(x => !x.canceled && x.ms <= ms); for (const t of due) { t.canceled = true; t.fn() } await flush() }
  return { callbacks, calls, watches, wakes, diagnostics, env, registry, ctx, emit, start, line, message, store, context, ackCalls, flush, tick,
    setBranch: b => { branch = b }, branch: () => branch, setSid: s => { sid = s }, setIdle: v => { idle = v }, setDraft: s => { draft = s } }
}

const notice = (id, body = `notice ${id}`) => ({ schema: 1, kind: 'notice', id: `notice:${id}`, warning: id, thread: 'thread', body })
const slot = n => n.toString(16).padStart(64, '0')
const packet = (items, keys = items.map((_, i) => slot(i + 1)), receipt = 'opaque receipt with 18446744073709551615') => ({ schema: 1, kind: 'presentation_page', items, keys, receipt })
const confirmations = h => h.calls.filter(c => c.argv.includes('presentation-confirm'))

test('manual presentation protects a notice behind a full higher-priority ordinary cache', async () => {
  const h = harness(); await h.start()
  assert.ok(h.watches[0].argv.includes('--manual-presentation'))
  for (let i = 0; i < 300; i++) h.message(`predecessor-${i}`, 0)
  h.line(packet([notice('protected')]))
  let observed = 0, ordinary = 0
  for (let i = 0; i < 12; i++) {
    const r = await h.emit('before_agent_start'); if (!r?.message) break
    if (r.message.content.includes('notice protected')) { observed++; assert.equal(ordinary, 224) }
    ordinary += (r.message.content.match(/\[herdr-threads\] message /g) ?? []).length
    h.store(r); await h.context(); await h.flush()
    if (!observed) assert.equal(h.watches.length, 1, 'ordinary ACK cannot replace a parked page')
  }
  assert.equal(observed, 1); assert.equal(confirmations(h).length, 1)
  assert.equal(confirmations(h)[0].stdin, 'opaque receipt with 18446744073709551615')
  assert.equal(h.watches.length, 2)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('manual whole-page confirmation waits for exact observed context of every emitted slot', async () => {
  const h = harness(); await h.start()
  h.line(packet([notice('a', 'a\n'.repeat(10000)), notice('b', 'b\n'.repeat(10000))]))
  const first = await h.emit('before_agent_start'); h.store(first)
  assert.equal(confirmations(h).length, 0)
  await h.context(); await h.flush(); assert.equal(confirmations(h).length, 0)
  const second = await h.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} })
  h.store(second); assert.equal(confirmations(h).length, 0)
  await h.context(); await h.flush(); assert.equal(confirmations(h).length, 1)
  assert.equal(h.ackCalls().length, 0)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('reload requires canonical replay and fresh context but reuses an exact slot acceptance hint', async () => {
  const first = harness(); await first.start(); first.line(packet([notice('reload')]))
  const r = await first.emit('before_agent_start'); first.store(r)
  assert.match(r?.message?.content ?? '', /notice reload/)
  await first.emit('session_shutdown', { reason: 'reload' })
  const next = harness({ registry: first.registry, branch: first.branch() }); await next.start('reload')
  await next.context(); await next.flush(); assert.equal(confirmations(next).length, 0)
  next.line(packet([notice('reload')], [slot(1)], 'fresh lease opaque'))
  assert.equal(confirmations(next).length, 0)
  const startup = await next.emit('before_agent_start')
  assert.doesNotMatch(startup?.message?.content ?? '', /notice reload/)
  next.store(startup); await next.context(); await next.flush()
  assert.equal(confirmations(next).length, 1); assert.equal(confirmations(next)[0].stdin, 'fresh lease opaque')
  await next.emit('session_shutdown', { reason: 'quit' })
})

test('admission uses native identity and refuses inherited child/nonterminal claims', async () => {
  for (const options of [{ env: { HERDR_THREADS_PI_PID: '419' } }, { env: { HERDR_THREADS_PI_ROLE: 'child' } }, { env: { HERDR_THREADS_PI_ROLE: 'mystery' } }, { mode: 'rpc' }, { mode: undefined, env: { HERDR_THREADS_PI_PID: 'bad' } }]) {
    const h = harness(options); await h.start(); assert.equal(h.watches.length, 0); assert.equal(h.calls.length, 0)
  }
  const h = harness(); await h.start()
  assert.equal(h.watches.length, 1)
  assert.deepEqual(JSON.parse(h.calls[0].stdin), { event: 'SessionStart', source: 'startup', session_id: 'native-session-1', role: 'top_level', mode: 'tui', pid: 420, cwd: '/isolated/project' })
  assert.deepEqual(h.watches[0].argv, ['herdr-threads', 'watch', '--harness', 'pi', '--session', 'native-session-1', '--manual-presentation'])
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

test('watch delivery never starts a native run; the host owns idle wake', async () => {
  const h = harness(); await h.start(); h.message('passive'); assert.equal(h.wakes.length, 0)
  h.setDraft('human draft'); h.message('urgent', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  h.setDraft(''); await h.emit('ui_prompt_start'); await h.emit('agent_settled'); assert.equal(h.wakes.length, 0)
  await h.emit('ui_prompt_end'); assert.equal(h.wakes.length, 0)
  h.message('urgent2', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  await h.tick(30000); await h.emit('agent_settled'); assert.equal(h.wakes.length, 0)
  const offered = await h.emit('before_agent_start', { messageType: 'user', prompt: 'herdr-threads: wake up' })
  assert.match(offered.message.content, /body urgent/)
})

test('TUI diagnostics use native notifications instead of corrupting the composer with stderr', async () => {
  const h = harness(), notifications = []
  h.ctx.ui.notify = (...args) => notifications.push(args)
  await h.start(); h.watches[0].exit(2)
  assert.deepEqual(notifications, [['herdr-threads: pi watch unavailable', 'warning']])
  assert.equal(h.diagnostics.length, 0)
  await h.emit('session_shutdown')
})

test('missing or throwing TUI notification does not fall back to unsafe terminal writes', async () => {
  for (const notify of [undefined, () => { throw new Error('UI unavailable') }]) {
    const h = harness(); h.ctx.ui.notify = notify
    await h.start(); h.watches[0].exit(2)
    assert.equal(h.diagnostics.length, 0)
    await h.emit('session_shutdown')
  }
})

test('named wake edges use explicit startup idle and actual settlement, with no inferred idle', async () => {
  const h = harness(); await h.start()
  const edges = () => h.calls.filter(c => c.argv.includes('TurnEnd')).map(c => JSON.parse(c.stdin))
  assert.equal(edges().length, 1); assert.equal(edges()[0].source, 'startup'); assert.equal(edges()[0].idle, true)
  h.setIdle(false); await h.emit('before_agent_start', { messageType: 'user' })
  await h.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} })
  await h.emit('agent_end'); assert.equal(edges().length, 1)
  await h.emit('agent_settled'); assert.equal(edges().length, 1, 'busy is not an idle witness')
  h.setIdle(true); await h.emit('agent_settled'); assert.equal(edges().length, 2)
  assert.equal(edges()[1].source, 'settled'); assert.equal(edges()[1].idle, true)
  const unknown = harness(); delete unknown.ctx.isIdle; await unknown.start()
  assert.equal(unknown.calls.filter(c => c.argv.includes('TurnEnd')).length, 0)
  await h.emit('session_shutdown'); await unknown.emit('session_shutdown')
})

test('queued idle is owner fenced and cannot overtake the next busy call', async () => {
  let finishIdle, block = false
  const h = harness({ run: async argv => {
    if (argv.includes('TurnEnd') && block) await new Promise(resolve => { finishIdle = resolve })
    return { code: 0, stdout: argv.at(-1) === 'SessionStart' ? 'guidance' : '' }
  } })
  await h.start(); block = true
  const settled = h.emit('agent_settled'); await h.flush()
  assert.ok(finishIdle)
  h.setIdle(false); const starting = h.emit('before_agent_start', { messageType: 'user' }); await h.flush()
  assert.equal(h.calls.filter(c => c.argv.includes('TurnStart')).length, 0)
  finishIdle(); await settled; await starting
  assert.equal(h.calls.at(-1).argv.at(-1), 'TurnStart')
  await h.emit('session_shutdown')
})

test('shutdown fences an idle report still queued behind the previous owner busy call', async () => {
  let finishBusy
  const h = harness({ run: async argv => {
    if (argv.includes('TurnStart')) await new Promise(resolve => { finishBusy = resolve })
    return { code: 0, stdout: '' }
  } })
  await h.start(); h.setIdle(false)
  const starting = h.emit('before_agent_start'); await h.flush()
  h.setIdle(true); const settled = h.emit('agent_settled'); const closing = h.emit('session_shutdown')
  finishBusy(); await Promise.all([starting, settled, closing])
  assert.equal(h.calls.filter(c => c.argv.includes('TurnEnd')).length, 1, 'only admitted startup supplied Idle')
  assert.equal(h.calls.at(-1).argv.at(-1), 'SessionEnd')
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
  assert.equal([...a.calls, ...b.calls].filter(c => c.argv.includes('SessionStart')).length, 1)
})

test('context delivery retries failed ACK without starting runs on settlement or replay', async () => {
  const h = harness({ run: async argv => ({ code: argv.includes('ack') ? 1 : 0, stdout: '' }) })
  await h.start(); h.message('m', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  await h.emit('agent_settled'); assert.equal(h.wakes.length, 0)
  const done = harness(); await done.start(); done.message('m', 0, { wakes: true })
  const first = await done.emit('before_agent_start'); done.store(first); await done.context(); await done.flush(); await done.emit('agent_settled')
  done.message('m', 0, { wakes: true }); assert.equal(done.wakes.length, 0)
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

test('clear application and invitation presentation never ACK truncated mail or accept an invitation', async () => {
  const h = harness(); await h.start()
  h.line(packet([
    { schema: 1, kind: 'attention_cleared', id: 'clear:1', presentation_epoch: '11' },
    { schema: 1, kind: 'attention', id: 'attention:2', text: 'attention two', presentation_epoch: '12' },
    { schema: 1, kind: 'invitation', id: 'invite', thread: 't', body: 'accept manually', topic_revision: '1' },
  ]))
  h.message('cut', 1, { truncated: true, body: 'prefix\n…truncated; next: herdr-threads inbox' })
  assert.equal(confirmations(h).length, 0, 'clear cannot cover other bodies')
  const r = await h.emit('before_agent_start'); assert.match(r.message.content, /attention two/)
  assert.match(r.message.content, /next: herdr-threads inbox/)
  h.store(r); await h.context(); await h.flush()
  assert.equal(h.ackCalls().length, 0); assert.equal(confirmations(h).length, 1)
  assert.equal(h.calls.some(c => c.argv.includes('accept')), false)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('foreign-session callbacks cannot consume or shut down the successor owner', async () => {
  const h = harness(); await h.start(); const oldCtx = { ...h.ctx, sessionManager: { getSessionId: () => 'old-session', getBranch: () => [] } }
  await h.callbacks.get('session_shutdown')({ type: 'session_shutdown' }, oldCtx)
  h.message('m'); assert.ok((await h.emit('before_agent_start'))?.message)
})

test('boundary delivery needs no editor API; manual compaction prevents continuation', async () => {
  const h = harness(); await h.start(); delete h.ctx.ui.getEditorText
  h.message('m', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
  assert.equal(h.diagnostics.length, 0)
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

test('reload requeue keeps a retained slot before newly observed rows with the same priority and deadline', async () => {
  const first = harness(); await first.start()
  for (let i = 0; i < 10; i++) first.message(`earlier-${i}`, 9)
  const initial = await first.emit('before_agent_start'); first.store(initial); await first.context(); await first.flush()
  first.line(packet([notice('retained-tie')]))
  const accepted = await first.emit('before_agent_start'); first.store(accepted)
  await first.emit('session_shutdown', { reason: 'reload' })
  const h = harness({ registry: first.registry, branch: first.branch() }); await h.start('reload')
  h.line(packet([notice('retained-tie')]))
  h.message('new-tie', 0)
  // Native history still contains the accepted hint, but the model projection
  // omits it. Requeue must preserve its first-observed ordinal across owners.
  await h.emit('context', { messages: [] })
  const r = await h.emit('before_agent_start')
  assert.ok(r.message.content.indexOf('notice retained-tie') < r.message.content.indexOf('body new-tie'))
  h.store(r); await h.context(); await h.flush()
  assert.equal(confirmations(h).length, 1)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('pre-F1 reload drops legacy 256-record hints before fresh canonical 224 plus 32 admission', async () => {
  const proposals = [], branch = []
  // Exact pre-F1 recovery shape: no immutable arena/protected discriminator.
  // This is optional local acceptance metadata, never a canonical notice source.
  for (let page = 0; page < 8; page++) {
    const items = [], frames = new Map(), texts = []
    let offset = '[herdr-threads] new messages:\n\n'.length
    for (let i = 0; i < 32; i++) {
      const id = `legacy-${page * 32 + i}`, key = JSON.stringify(['message', id, null])
      const text = frame({ kind: 'message', id, thread: 't', sender: 's', body: `legacy body ${id}` })
      items.push({ key, id, kind: 'message', priority: 0, deadline: Infinity, order: page * 32 + i, truncated: false })
      frames.set(key, [offset, offset + text.length]); offset += text.length + 2; texts.push(text)
    }
    const p = { customType: `legacy-proposal-${page}`, text: '[herdr-threads] new messages:\n\n' + texts.join('\n\n'), guidance: 0, items, frames, revision: 0 }
    // The first F1 bridge could default this flag on old records; the current
    // record discriminator must also be present before reusing its hints.
    if (page === 0) p.ordinaryArena = true
    proposals.push(p); branch.push({ type: 'custom_message', customType: p.customType, content: p.text })
  }
  const registry = { [Symbol.for('herdr-threads.pi.recovery')]: { sid: 'native-session-1', proposals } }
  const h = harness({ registry, branch }); await h.start('reload')
  await h.context(); await h.flush()
  assert.equal(h.ackCalls().length, 0, 'unsupported legacy hints cannot import current receipt proof')
  assert.equal(confirmations(h).length, 0)
  for (let i = 0; i < 224; i++) h.message(`canonical-${i}`, 0)
  h.line(packet(Array.from({ length: 32 }, (_, i) => notice(`fresh-${i}`))))
  let ordinary = 0, protectedSlots = 0
  for (let i = 0; i < 10 && !confirmations(h).length; i++) {
    const r = await h.emit('before_agent_start'); assert.ok(r?.message)
    assert.doesNotMatch(r.message.content, /legacy body/)
    ordinary += (r.message.content.match(/\[herdr-threads\] message /g) ?? []).length
    protectedSlots += (r.message.content.match(/\[herdr-threads\] notice /g) ?? []).length
    h.store(r); await h.context(); await h.flush()
  }
  assert.equal(ordinary, 224); assert.equal(protectedSlots, 32)
  assert.equal(confirmations(h).length, 1)
  assert.equal(h.ackCalls().flatMap(c => c.argv.slice(c.argv.indexOf('context') + 1)).length, 224)
  await h.emit('session_shutdown', { reason: 'quit' })
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

async function assertWatchConnected(io, env, script, firstRowMs = 3000) {
  let watcher, timer, resolveLine, rejectLine, seen = false, exited = false
  try {
    const line = new Promise((resolve, reject) => { resolveLine = resolve; rejectLine = reject })
    timer = setTimeout(() => rejectLine(new Error('watch first row timeout')), firstRowMs)
    watcher = io.spawn([process.execPath, '-e', script], { env, maxBytes: 65536 }, {
      line: row => { seen = true; resolveLine(row) },
      exit: code => { exited = true; if (!seen) rejectLine(new Error(`watch exited before first row (${code})`)) },
    })
    assert.equal((await line).state, 'connected')
  } finally { clearTimeout(timer); await watcher?.stop() }
  assert.equal(exited, true)
}

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
    await assertWatchConnected(io, env, 'console.log(JSON.stringify({id:"status:0",schema:1,kind:"status",state:"connected"}));setInterval(()=>{},1000)')
  } finally { fs.rmSync(root, { recursive: true, force: true }) }
})

test('native watch fixture reaps on assertion failure, early exit and first-row timeout', async t => {
  const root = fs.mkdtempSync(path.join(import.meta.dirname, '.scratch-watch-failure-'))
  const native = nodeIO()
  const env = { HOME: root, PATH: process.env.PATH, HT_LEAK_RUN_ID: process.env.HT_LEAK_RUN_ID ?? 'pi-watch-failure' }
  try {
    for (const scenario of [
      { name: 'wrong-state assertion', script: 'console.log(JSON.stringify({state:"wrong"}));setInterval(()=>{},1000)', deadline: 3000, expected: { code: 'ERR_ASSERTION' } },
      { name: 'early child exit', script: 'process.exit(2)', deadline: 3000, expected: /watch exited before first row \(2\)/ },
      { name: 'no first row', script: 'setInterval(()=>{},1000)', deadline: 100, expected: /watch first row timeout/ },
    ]) {
      await t.test(scenario.name, async () => {
        let watcher, exited = false, guard
        const io = { ...native, spawn: (argv, options, handlers) => {
          watcher = native.spawn(argv, options, { ...handlers, exit: (...args) => { exited = true; handlers.exit(...args) } })
          return watcher
        } }
        try {
          const fixture = assertWatchConnected(io, env, scenario.script, scenario.deadline)
          const bounded = Promise.race([fixture, new Promise((_resolve, reject) => {
            guard = setTimeout(() => reject(new Error('outer first-row guard')), 5000)
          })])
          await assert.rejects(bounded, scenario.expected)
          assert.equal(exited, true, 'fixture must reap before propagating its failure')
        } finally { clearTimeout(guard); await watcher?.stop() }
      })
    }
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
  h.message('new-attention', 0, { wakes: true }); assert.equal(h.wakes.length, 0)
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

test('missing mode uses native capability warning without enrollment or stderr', async t => {
  for (const mode of [undefined, null]) await t.test(String(mode), async () => {
    const h = harness(), notices = []
    h.ctx.mode = mode; h.ctx.ui.notify = (...args) => notices.push(args)
    await h.start()
    assert.deepEqual(notices, [['herdr-threads: pi boundary API unavailable', 'warning']])
    assert.equal(h.diagnostics.length, 0, 'pre-owner warning must not use raw diagnostic output')
    assert.equal(h.calls.length, 0); assert.equal(h.watches.length, 0)
    assert.deepEqual(h.env, {}); assert.deepEqual(Reflect.ownKeys(h.registry), [])
  })
})

test('explicit non-tui modes and forbidden or duplicate missing-mode instances stay quiet', async () => {
  for (const mode of ['rpc', 'json', 'print', 'future-mode']) {
    const h = harness(); h.ctx.mode = mode; await h.start()
    assert.equal(h.diagnostics.length, 0); assert.equal(h.calls.length, 0); assert.equal(h.watches.length, 0)
    assert.deepEqual(h.env, {}); assert.deepEqual(Reflect.ownKeys(h.registry), [])
  }
  for (const env of [{ HERDR_THREADS_PI_ROLE: 'child' }, { HERDR_THREADS_PI_PID: '419' }]) {
    const h = harness({ env }); h.ctx.mode = undefined; await h.start()
    assert.equal(h.diagnostics.length, 0); assert.equal(h.calls.length, 0); assert.equal(h.watches.length, 0)
    assert.deepEqual(h.env, env); assert.deepEqual(Reflect.ownKeys(h.registry), [])
  }
  const first = harness(); await first.start()
  const duplicate = harness({ registry: first.registry }); duplicate.ctx.mode = undefined; await duplicate.start()
  assert.equal(duplicate.diagnostics.length, 0); assert.equal(duplicate.calls.length, 0); assert.equal(duplicate.watches.length, 0)
  assert.deepEqual(duplicate.env, {})
  await first.emit('session_shutdown')
})

test('missing mode with absent or throwing native notification stays inert', async () => {
  for (const notify of [undefined, null, 'unsupported', () => { throw new Error('native notification unavailable') }]) {
    const h = harness(); h.ctx.mode = undefined; h.ctx.ui.notify = notify; await h.start()
    assert.equal(h.diagnostics.length, 0, 'missing notification must never fall back to raw stderr')
    assert.equal(h.calls.length, 0); assert.equal(h.watches.length, 0)
    assert.deepEqual(h.env, {}); assert.deepEqual(Reflect.ownKeys(h.registry), [])
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
  await h.start(); const tree = h.emit('session_tree'), compact = h.emit('session_compact'); await h.flush()
  assert.equal(waiting.has('compact'), false, 'lifecycle children are serialized')
  waiting.get('tree')({ code: 0, stdout: 'stale tree guidance' }); await tree; await h.flush()
  waiting.get('compact')({ code: 0, stdout: 'new compact guidance' }); await compact
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

test('canonical urgent replay retains one recovered frame until the successor host turn observes it', async () => {
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
  assert.equal(next.wakes.length, 0)
  next.message('urgent-recovered', 0, { wakes: true }); assert.equal(next.wakes.length, 0)
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

test('observer refusal sentinel never admits an owner or starts watch, and retries lifecycle', async () => {
  let refused = true
  const h = harness({ run: async argv => ({ code: 0, stdout: argv.at(-1) === 'SessionStart' && refused ? 'herdr-threads: check-in unavailable (hook refused).\n' : '' }) })
  await h.start()
  await h.emit('before_agent_start')
  assert.equal(h.watches.length, 0)
  assert.equal(h.calls.filter(call => call.argv.at(-1) === 'SessionStart').length, 2)
  assert.equal(h.wakes.length, 0)
  assert.equal(h.ackCalls().length, 0)
  refused = false
  await h.emit('before_agent_start')
  assert.equal(h.watches.length, 1)
})


test('manual clear-only applies owner-local state without a model run or ordinary ACK', async () => {
  const h = harness(); await h.start()
  const clear = { schema: 1, kind: 'attention_cleared', id: 'clear:max', presentation_epoch: '18446744073709551615' }
  h.line(packet([clear], [slot(8)], 'exact clear receipt 18446744073709551615'))
  await h.flush()
  assert.equal(confirmations(h).length, 1)
  assert.equal(confirmations(h)[0].stdin, 'exact clear receipt 18446744073709551615')
  assert.equal(h.wakes.length, 0); assert.equal(h.ackCalls().length, 0); assert.equal(h.watches.length, 2)
  h.watches[0].line(packet([clear])); await h.flush()
  assert.equal(confirmations(h).length, 1, 'detached owner/watch cannot apply a late clear')
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('manual malformed and unknown pages remain unconfirmed with bounded retry and no fallback', async () => {
  for (const bad of [
    packet([notice('bad')], [null]), packet([notice('bad')], ['not-a-key']),
    packet([notice('bad'), notice('duplicate')], [slot(1), slot(1)]),
    packet([{ schema: 1, kind: 'future_control', id: 'future' }]),
    packet([notice('bad')], [slot(1)], 'x'.repeat(4097)),
    { schema: 1, kind: 'notice', id: 'unframed', body: 'legacy flush-confirm is forbidden' },
  ]) {
    const h = harness(); await h.start(); h.line(bad); await h.flush()
    assert.equal(confirmations(h).length, 0)
    assert.doesNotMatch((await h.emit('before_agent_start'))?.message?.content ?? '', /notice bad|legacy flush/)
    await h.tick(1000); assert.equal(h.watches.length, 2)
    assert.ok(h.watches[1].argv.includes('--manual-presentation'))
    await h.emit('session_shutdown', { reason: 'quit' })
  }
  const h = harness(); await h.start()
  // An unknown hidden page has no emitted packet: the CLI owns finite scan work.
  await h.tick(30000); assert.equal(h.watches.length, 1); assert.equal(confirmations(h).length, 0)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('new opaque key cannot inherit an accepted old-token body or pre-replay context', async () => {
  const first = harness(); await first.start(); first.line(packet([notice('same')], [slot(1)]))
  const old = await first.emit('before_agent_start'); first.store(old)
  await first.emit('session_shutdown', { reason: 'reload' })
  const h = harness({ registry: first.registry, branch: first.branch() }); await h.start('reload')
  await h.context(); h.line(packet([notice('same')], [slot(2)])); await h.flush()
  assert.equal(confirmations(h).length, 0)
  const fresh = await h.emit('before_agent_start'); assert.match(fresh.message.content, /notice same/)
  assert.notEqual(fresh.message.customType, old.message.customType)
  h.store(fresh); await h.context(); await h.flush(); assert.equal(confirmations(h).length, 1)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('protected proposal projection absence requeues only at a later qualified run', async () => {
  const h = harness(); await h.start(); h.line(packet([notice('projection')]))
  const first = await h.emit('before_agent_start'); h.store(first)
  await h.emit('session_compact')
  // getBranch still contains history, but the supported projection excludes it.
  const boundary = { outcome: 'completed', entries: [], continue: false, context: { contextMessages: [] } }
  const sameRun = await h.emit('turn_end', boundary)
  assert.doesNotMatch(JSON.stringify(sameRun ?? {}), /notice projection/)
  assert.equal(confirmations(h).length, 0)
  const next = await h.emit('before_agent_start'); assert.match(next.message.content, /notice projection/)
  h.store(next); await h.context(); await h.flush(); assert.equal(confirmations(h).length, 1)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('revision clears partial coverage and actual-context absence cannot reuse historical branch proof', async () => {
  const h = harness(); await h.start()
  h.line(packet([notice('first', 'a\n'.repeat(10000)), notice('second', 'b\n'.repeat(10000))]))
  const a = await h.emit('before_agent_start'); h.store(a); await h.context(); await h.flush()
  assert.equal(confirmations(h).length, 0)
  await h.emit('session_tree')
  const b = await h.emit('turn_end', { outcome: 'completed', entries: [], continue: false, context: {} }); h.store(b)
  await h.emit('context', { messages: [{ role: 'custom', customType: b.entries.at(-1).customType, content: b.entries.at(-1).content }] })
  await h.flush(); assert.equal(confirmations(h).length, 0)
  const later = await h.emit('before_agent_start'); assert.match(later.message.content, /a\n  a/)
  h.store(later); await h.context(); await h.flush(); assert.equal(confirmations(h).length, 1)
  await h.emit('session_shutdown', { reason: 'quit' })
})

test('watch exit and malformed-page retry join an outstanding confirmation before successor startup', async () => {
  for (const cause of ['exit', 'bad-page']) {
    let resolveConfirm
    const held = new Promise(resolve => { resolveConfirm = resolve })
    const h = harness({ run: async argv => argv.includes('presentation-confirm') ? held : { code: 0, stdout: '' } })
    await h.start(); h.line(packet([notice('held')]))
    const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
    assert.equal(confirmations(h).length, 1)
    if (cause === 'exit') h.watches[0].exit(1)
    else h.line(packet([notice('conflict')], [slot(9)]))
    await h.tick(1000); assert.equal(h.watches.length, 1)
    resolveConfirm({ code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}' })
    await h.flush(); await h.flush(); assert.equal(h.watches.length, 2)
    await h.emit('session_shutdown', { reason: 'quit' })
  }
})

test('refusal re-admission on a genuine turn cannot bypass a held confirmation join', async () => {
  for (const refusal of ['exit3', 'status-and-exit3', 'status-only']) {
    let resolveConfirm
    const held = new Promise(resolve => { resolveConfirm = resolve })
    const h = harness({ run: async argv => argv.includes('presentation-confirm') ? held : { code: 0, stdout: '' } })
    await h.start(); h.line(packet([notice('readmission')]))
    const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
    assert.equal(confirmations(h).length, 1)
    if (refusal !== 'exit3') h.line({ schema: 1, kind: 'status', state: 'refused', reason: 'no_binding' })
    if (refusal !== 'status-only') h.watches[0].exit(3)
    try {
      await h.emit('before_agent_start', { messageType: 'user', prompt: 'genuine input after refusal' })
      await h.flush()
      assert.equal(h.calls.filter(c => c.argv.at(-1) === 'SessionStart').length, 2)
      assert.equal(h.watches.length, 1, refusal)
    } finally {
      resolveConfirm({ code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}' })
      await h.flush(); await h.flush()
      await h.emit('session_shutdown', { reason: 'quit' })
    }
    assert.equal(h.watches.length, 2, 'one successor starts after the old confirmation is joined')
  }
})

test('ambiguous confirmation reconciles once and shutdown joins both held confirmation and watch stop', async () => {
  const h = harness({ run: async argv => argv.includes('presentation-confirm') ? { code: 0, stdout: 'lost/truncated reply' } : { code: 0, stdout: '' } })
  await h.start(); h.line(packet([notice('ambiguous')]))
  const r = await h.emit('before_agent_start'); h.store(r); await h.context(); await h.flush()
  assert.equal(confirmations(h).length, 1); assert.equal(h.watches.length, 1)
  await h.tick(1000); assert.equal(h.watches.length, 2)
  h.line({ schema: 1, kind: 'status', state: 'connected' })
  h.line(packet([notice('ambiguous')], [slot(1)], 'canonical new lease'))
  await h.flush(); assert.equal(confirmations(h).length, 1, 'old context is not fresh replay proof')
  await h.context(); await h.flush(); assert.equal(confirmations(h).length, 2)
  await h.emit('session_shutdown', { reason: 'quit' })

  let resolveConfirm, releaseStop, closed = false
  const held = new Promise(resolve => { resolveConfirm = resolve }), stopped = new Promise(resolve => { releaseStop = resolve })
  const owned = harness({ run: async argv => argv.includes('presentation-confirm') ? held : { code: 0, stdout: '' }, stop: () => stopped })
  await owned.start(); owned.line(packet([notice('closing')]))
  const draft = await owned.emit('before_agent_start'); owned.store(draft); await owned.context(); await owned.flush()
  const closing = owned.emit('session_shutdown', { reason: 'reload' }).then(() => { closed = true })
  await owned.flush(); assert.equal(closed, false)
  resolveConfirm({ code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}' }); await owned.flush()
  assert.equal(closed, false)
  releaseStop(); await closing; assert.equal(owned.watches.length, 1)
})


test('mixed proposal strings keep their arena charge after ordinary ACK removes their ordinary records', async () => {
  const h = harness(); await h.start()
  h.line(packet(Array.from({ length: 32 }, (_, i) => notice(`pin-${i}`))))
  let retainedProposalText = 0, acknowledged = 0
  for (let boundary = 0; boundary < 40 && !confirmations(h).length; boundary++) {
    // Exercise byte invariants even if an IO transport supplies additional cached
    // ordinary lines while the presentation page is parked.
    for (let i = 0; i < 31; i++) h.message(`mixed-${boundary}-${i}`, 0, { body: 'x'.repeat(1900), deadline_at: 1 })
    const r = await h.emit('before_agent_start'); if (!r?.message) break
    retainedProposalText += Buffer.byteLength(r.message.content)
    assert.ok(retainedProposalText <= (1536 + 144) * 1024, `retained mixed proposal text: ${retainedProposalText}`)
    h.store(r); await h.context(); await h.flush()
    acknowledged = h.ackCalls().reduce((n, call) => n + call.argv.slice(call.argv.indexOf('context') + 1).length, 0)
  }
  assert.ok(acknowledged > 224, 'ordinary records were released while immutable mixed strings remained pinned')
  assert.ok(confirmations(h).length <= 1)
  assert.equal(h.watches.length, confirmations(h).length + 1, 'pressure never restarts a parked page')
  await h.emit('session_shutdown', { reason: 'quit' })
})


test('canonical manual pages drain more than 256 notices and 2 MiB without a local notice journal', async () => {
  let cursor = 0, confirmed = 0
  const total = 300, perPage = 6
  const h = harness({
    run: async (argv, request) => {
      if (!argv.includes('presentation-confirm')) return { code: 0, stdout: '' }
      assert.equal(request.stdin, `canonical receipt ${confirmed}`)
      confirmed++; return { code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}' }
    },
    spawn: (_argv, _request, handlers) => queueMicrotask(() => {
      handlers.line({ schema: 1, kind: 'status', state: 'connected' })
      if (cursor === total) return
      const start = cursor
      const items = Array.from({ length: perPage }, () => notice(`backlog-${cursor++}`, 'z'.repeat(8192)))
      handlers.line(packet(items, items.map((_, i) => slot(start + i + 1)), `canonical receipt ${confirmed}`))
    }),
  })
  await h.start()
  let observed = 0, textBytes = 0
  for (let i = 0; i < 100 && observed < total; i++) {
    const r = await h.emit('before_agent_start'); assert.ok(r?.message)
    observed += (r.message.content.match(/\[herdr-threads\] notice /g) ?? []).length
    textBytes += Buffer.byteLength(r.message.content)
    h.store(r); await h.context(); await h.flush(); await h.flush()
  }
  assert.equal(observed, total); assert.ok(textBytes > 2 * 1024 * 1024)
  assert.equal(confirmed, total / perPage); assert.equal(h.watches.length, confirmed + 1)
  assert.equal(h.ackCalls().length, 0); assert.equal(h.wakes.length, 0)
  await h.emit('session_shutdown', { reason: 'quit' })
})


const rustPackets = [
  ['manual-maximum.jsonl', 70886, 'b38c97768e05e3e1e90bb11c031e7a848b8b4c923775e9fbd590ade175daa60e'],
  ['manual-notice-clear.jsonl', 769, '718b5f490cac71a45e9bf3c2067cdd75c6ffc5b5993ac6d5c6736476339b0c6a'],
]

test('current production Rust packets preserve opaque u64 receipts and fit bounded native proposals', async () => {
  for (const [filename, length, hash] of rustPackets) {
    const raw = fs.readFileSync(new URL(`./fixtures/${filename}`, import.meta.url))
    assert.equal(raw.length, length); assert.equal(createHash('sha256').update(raw).digest('hex'), hash)
    const page = JSON.parse(raw.toString('utf8'))
    assert.ok(page.receipt.includes('18446744073709551615'))
    assert.ok(Buffer.byteLength(page.receipt) <= 4096)
    assert.ok(page.items.filter(i => i.kind !== 'attention_cleared').reduce((n, i) => n + Buffer.byteLength(frame(i)), 0) <= 128 * 1024)
    if (filename.includes('maximum')) assert.equal(Buffer.byteLength(page.receipt), 2802)
    const h = harness(); await h.start(); h.line(page, raw.length)
    assert.equal(confirmations(h).length, 0, 'applied clear cannot stand in for the emitted body slots')
    let seen = 0
    for (let i = 0; i < 10 && !confirmations(h).length; i++) {
      const r = await h.emit('before_agent_start'); assert.ok(r?.message)
      assert.ok(Buffer.byteLength(r.message.content) <= 65536)
      seen += (r.message.content.match(/\[herdr-threads\] (attention|notice) /g) ?? []).length
      h.store(r); await h.context(); await h.flush()
    }
    assert.equal(seen, page.items.filter(i => i.kind !== 'attention_cleared').length)
    assert.equal(confirmations(h).length, 1); assert.equal(confirmations(h)[0].stdin, page.receipt)
    assert.equal(h.ackCalls().length, 0)
    await h.emit('session_shutdown', { reason: 'quit' })
  }
})

test('native byte parser handles fragmented and coalesced Rust packets and rejects overlong frames', async () => {
  const root = fs.mkdtempSync(path.join(import.meta.dirname, '.scratch-manual-io-'))
  const io = nodeIO(), env = { HOME: root, PATH: process.env.PATH, HT_LEAK_RUN_ID: process.env.HT_LEAK_RUN_ID ?? 'pi-manual-io' }
  const fixture = new URL('./fixtures/manual-maximum.jsonl', import.meta.url).pathname
  let child, timer
  try {
    const rows = []
    let resolveRows, rejectRows
    const complete = new Promise((resolve, reject) => { resolveRows = resolve; rejectRows = reject })
    timer = setTimeout(() => rejectRows(new Error('fragmented packet deadline')), 3000)
    // Tiny fragments split escaped strings; the final write coalesces a UTF-8
    // a status row with another complete large packet in one stdout sequence.
    const script = 'const fs=require("node:fs");const b=fs.readFileSync(process.argv[1]);let i=0;function send(){if(i<b.length){process.stdout.write(b.subarray(i,i+=113));setImmediate(send)}else{process.stdout.write(JSON.stringify({schema:1,kind:"status",id:"status:🧭",state:"connected"})+"\\n");process.stdout.write(b);setInterval(()=>{},1000)}}send()'
    child = io.spawn([process.execPath, '-e', script, fixture], { env, maxBytes: 73728 }, {
      line: (row, bytes) => { rows.push({ row, bytes }); if (rows.length === 3) resolveRows() },
      exit: code => { if (rows.length < 3) rejectRows(new Error(`early fixture exit ${code}`)) },
    })
    await complete
    assert.equal(rows[0].bytes, 70886); assert.equal(rows[2].bytes, 70886)
    assert.equal(rows[1].row.id, 'status:🧭')
    assert.equal(rows[0].row.receipt, rows[2].row.receipt)
    await child.stop(); child = null; clearTimeout(timer)
    const overlong = await io.run([process.execPath, '-e', 'process.stdout.write("x".repeat(4097));setInterval(()=>{},1000)'], { env, stdin: '', timeoutMs: 1000, maxBytes: 4096 })
    assert.equal(overlong.code, 1)
    let badLines = 0, resolveExit
    const exited = new Promise(resolve => { resolveExit = resolve })
    timer = setTimeout(() => resolveExit('timeout'), 3000)
    child = io.spawn([process.execPath, '-e', 'process.stdout.write("x".repeat(73728));setInterval(()=>{},1000)'], { env, maxBytes: 73728 }, { line: () => { badLines++ }, exit: () => resolveExit('exited') })
    assert.equal(await exited, 'exited'); assert.equal(badLines, 0)
  } finally { clearTimeout(timer); await child?.stop(); fs.rmSync(root, { recursive: true, force: true }) }
})
