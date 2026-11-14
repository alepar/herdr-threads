// Model-free logic tests for the herdr-threads opencode plugin (design D2-D5, D8, D9).
// Every test drives the plugin with a fake opencode `client`, a fake process runner and spawner,
// and a fake clock. Run with `node --test integrations/opencode/plugin/tests/`
// (scripts/test-opencode-plugin; the cargo test `opencode_plugin_js` runs it when node exists).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '../../../..')
const pluginPath = join(root, 'integrations/opencode/plugin/herdr-threads.js')
const source = readFileSync(pluginPath, 'utf8')
// The plugin is an ES module in a `.js` file with no package.json; a data: URL loads it as one.
const mod = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'))
const { HerdrThreads } = mod
const { createCore, makePlugin, frame, batchOrder, appendBatch, markerRecovery, launchRecovery, grantPermissions, constants } = HerdrThreads.internals

const A = 'ses_AAAAAAAAAAAAAAAAAAAAAAAAAA'
const B = 'ses_BBBBBBBBBBBBBBBBBBBBBBBBBB'
const C = 'ses_CCCCCCCCCCCCCCCCCCCCCCCCCC'
const D = 'ses_DDDDDDDDDDDDDDDDDDDDDDDDDD'
const WAKE = 'herdr-threads: wake up'
const CHECKIN = '<check-in context>'

const flush = async () => {
  for (let i = 0; i < 5; i++) {
    await new Promise((r) => setImmediate(r))
  }
}

const msg = (id, extra = {}) => ({
  schema: 1,
  id,
  kind: 'message',
  thread: 'T1',
  thread_name: 'plans',
  sender: 'S1',
  sender_name: 'alice',
  body: `body of ${id}`,
  body_len: 10,
  truncated: false,
  ack_required: false,
  ...extra,
})
const waking = (id, extra = {}) => msg(id, { wakes: true, priority: 0, ...extra })
const connected = { schema: 1, id: 'status:0', kind: 'status', state: 'connected' }
const statusLine = (state, reason, exit) => ({ schema: 1, id: 'status:1', kind: 'status', state, reason, exit })

/** A fake opencode client, runner and spawner around one core. */
function harness(opts = {}) {
  const h = {
    t: 1_000_000,
    runs: [],
    spawns: [],
    calls: [], // ordered: append / wake
    toasts: [],
    ledger: [],
    timers: [], // io.after(ms) promises, fired by h.expire()
    statusMap: {}, // client.session.status(): non-idle sessions only
    sessions: {}, // client.session.get data by id
    seq: 0,
    appendResult: null, // (body) => result, overrides the stored message
    wakeResult: null,
    hookReply: (event) => ({ code: 0, stdout: event === 'UserPromptSubmit' ? '' : CHECKIN }),
    ackReply: (argv) => ({
      code: 0,
      stdout: argv.slice(argv.indexOf('--via') + 2).map((id) => JSON.stringify({ id, result: 'settled' })).join('\n'),
    }),
  }
  const nextMsg = () => `msg_${String(++h.seq).padStart(10, '0')}`
  h.nextMsg = nextMsg
  const session = {
    get: async ({ path: { id } }) => (h.sessions[id] ? { data: h.sessions[id] } : { error: { name: 'NotFoundError' } }),
    prompt: async ({ path, body }) => {
      const id = nextMsg()
      h.calls.push({ kind: 'append', sid: path.id, text: body.parts[0].text, body, id })
      // opencode fires chat.message for the stored message before the call returns (probe).
      await h.core.onChatMessage({ sessionID: path.id }, { message: { id, role: 'user' }, parts: body.parts.map((p) => ({ ...p })) })
      if (h.appendResult) return h.appendResult(body)
      return { data: { info: { id, role: 'user' }, parts: [{ id: 'prt_1', type: 'text', synthetic: true }] } }
    },
    promptAsync: async ({ path, body }) => {
      h.calls.push({ kind: 'wake', sid: path.id, text: body.parts[0].text, body })
      if (h.wakeResult) return h.wakeResult()
      return { data: {} }
    },
    status: async () => ({ data: h.statusMap }),
  }
  h.client = { session, tui: { showToast: async ({ body }) => (h.toasts.push(body), { data: true }) } }
  h.io = {
    now: () => h.t,
    after: (ms) => new Promise((fire) => h.timers.push({ ms, fire })),
    pid: 4242,
    env: { PATH: '/bin', OPENCODE_PID: '4242', ...(opts.env || {}) },
    argv: opts.argv ?? null,
    directory: '/work/repo',
    client: h.client,
    run: async (argv, o) => {
      h.runs.push({ argv, ...o })
      if (argv.includes('hook-event')) return h.hookReply(argv[argv.length - 1], JSON.parse(o.stdin))
      if (argv.includes('ack')) return h.ackReply(argv)
      return { code: 1, stdout: '' }
    },
    spawn: (argv, o, cb) => {
      const c = { argv, env: o.env, cb, stopped: false, stop() { this.stopped = true } }
      h.spawns.push(c)
      return c
    },
    ledger: (e) => h.ledger.push(e),
    heartbeat: () => {},
  }
  h.core = createCore(h.io)
  h.hooks = () => h.runs.filter((r) => r.argv.includes('hook-event'))
  h.hookEvents = () => h.hooks().map((r) => {
    const p = JSON.parse(r.stdin)
    return p.source ? `${p.event}:${p.source}` : p.event
  })
  h.acks = () => h.runs.filter((r) => r.argv.includes('ack'))
  /** Every bounded SDK call pending now runs out of time. */
  h.expire = async () => {
    for (const t of h.timers.splice(0)) t.fire()
    await flush()
  }
  /** A model step of `sid` whose message list ended with user message `parentID`. */
  h.step = (sid, parentID) => h.event('message.updated', { info: { id: h.nextMsg(), role: 'assistant', sessionID: sid, parentID } })
  h.appends = () => h.calls.filter((c) => c.kind === 'append')
  h.wakes = () => h.calls.filter((c) => c.kind === 'wake')
  h.watch = () => h.spawns[h.spawns.length - 1]
  h.line = async (o) => {
    h.watch().cb.line(o)
    await flush()
  }
  h.event = async (type, properties) => {
    h.core.onEvent({ type, properties })
    await flush()
  }
  h.created = (sid, parentID) => h.event('session.created', { sessionID: sid, info: { id: sid, title: `title ${sid}`, ...(parentID ? { parentID } : {}) } })
  h.top = (sid) => {
    h.sessions[sid] = { id: sid }
  }
  h.turn = async (sid, text = 'hi', message = {}) => {
    const output = { message: { id: nextMsg(), role: 'user', ...message }, parts: [{ type: 'text', text }] }
    await h.core.onChatMessage({ sessionID: sid, agent: 'build' }, output)
    await flush()
    return output
  }
  h.idle = (sid) => h.event('session.status', { sessionID: sid, status: { type: 'idle' } })
  h.busy = (sid) => h.event('session.status', { sessionID: sid, status: { type: 'busy' } })
  /** A bound session with a connected watch, idle. */
  h.boot = async (sid = A) => {
    h.top(sid)
    await h.created(sid)
    await h.turn(sid)
    await h.idle(sid)
    await h.line(connected)
    return h
  }
  return h
}

// ---- D2: shell.env ---------------------------------------------------------------------------

test('shell.env sets all three variables on every top-level shell call', async () => {
  const h = harness()
  h.top(A)
  for (const callID of ['call_1', 'call_2']) {
    const out = { env: { HERDR_THREADS_OPENCODE_SESSION: 'ses_stale', HERDR_THREADS_OPENCODE_PARENT: 'ses_stale' } }
    await h.core.onShellEnv({ sessionID: A, callID, cwd: '/work' }, out)
    assert.deepEqual(out.env, {
      HERDR_THREADS_OPENCODE_SESSION: A,
      HERDR_THREADS_OPENCODE_PARENT: '',
      HERDR_THREADS_OPENCODE_PID: '4242',
    })
  }
})

test('shell.env resolves a child, and a nested child, to the root top-level session', async () => {
  const h = harness()
  h.top(A)
  await h.created(B, A) // from the event
  h.sessions[C] = { id: C, parentID: B } // from client.session.get
  for (const [sid, parent] of [[B, A], [C, A]]) {
    const out = { env: {} }
    await h.core.onShellEnv({ sessionID: sid }, out)
    assert.equal(out.env.HERDR_THREADS_OPENCODE_SESSION, sid)
    assert.equal(out.env.HERDR_THREADS_OPENCODE_PARENT, parent)
    assert.equal(out.env.HERDR_THREADS_OPENCODE_PID, '4242')
  }
})

test('shell.env marks a failed parent lookup with ?', async () => {
  const h = harness()
  h.sessions[B] = { id: B, parentID: D } // D's lookup fails
  for (const sid of [C, B]) {
    const out = { env: {} }
    await h.core.onShellEnv({ sessionID: sid }, out)
    assert.equal(out.env.HERDR_THREADS_OPENCODE_PARENT, '?')
    assert.equal(out.env.HERDR_THREADS_OPENCODE_SESSION, sid)
  }
})

test('shell.env leaves a call with no session (a PTY) alone', async () => {
  const h = harness()
  const out = { env: { X: '1' } }
  await h.core.onShellEnv({ cwd: '/work' }, out)
  assert.deepEqual(out.env, { X: '1' })
})

// ---- D3: lifecycle ---------------------------------------------------------------------------

test('first turn checks in with SessionStart startup, later turns run UserPromptSubmit', async () => {
  const h = harness()
  h.top(A)
  await h.created(A)
  const first = await h.turn(A)
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup'])
  // the check-in is a synthetic part of the same user message
  const part = first.parts[1]
  assert.equal(part.text, CHECKIN)
  assert.equal(part.synthetic, true)
  assert.equal(part.type, 'text')
  assert.equal(part.sessionID, A)
  assert.equal(part.messageID, first.message.id)
  assert.match(part.id, /^prt_[0-9a-z]{26}$/)
  // the watch child runs for the bound session with the claim variables
  assert.equal(h.spawns.length, 1)
  assert.deepEqual(h.watch().argv, ['herdr-threads', 'watch', '--harness', 'opencode', '--session', A])
  assert.equal(h.watch().env.HERDR_THREADS_OPENCODE_SESSION, A)
  assert.equal(h.watch().env.HERDR_THREADS_OPENCODE_PARENT, '')
  assert.equal(h.watch().env.HERDR_THREADS_OPENCODE_PID, '4242')
  h.hookReply = () => ({ code: 0, stdout: '<pending item>' })
  const second = await h.turn(A)
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit'])
  assert.equal(second.parts[1].text, '<pending item>')
  // a quiet turn injects nothing
  h.hookReply = () => ({ code: 0, stdout: '' })
  const third = await h.turn(A)
  assert.equal(third.parts.length, 1)
  assert.equal(h.spawns.length, 1)
})

test('the hook command and payload match the adapter fixtures', async () => {
  const h = harness({ argv: ['/opt/ht', '--state-dir', '/s', '--host-endpoint', '/h.sock'] })
  h.top(A)
  await h.created(A)
  await h.turn(A)
  await h.turn(A)
  await h.created(B, A)
  await h.turn(B)
  const fixtures = join(root, 'tests/fixtures/opencode/payloads')
  const shape = (f) => Object.keys(JSON.parse(readFileSync(join(fixtures, f), 'utf8'))).sort()
  const [start, prompt, sub] = h.hooks()
  assert.deepEqual(start.argv, ['/opt/ht', '--state-dir', '/s', '--host-endpoint', '/h.sock', 'hook-event', 'opencode', 'SessionStart'])
  assert.deepEqual(JSON.parse(start.stdin), { event: 'SessionStart', source: 'startup', session_id: A, cwd: '/work/repo' })
  assert.deepEqual(Object.keys(JSON.parse(start.stdin)).sort(), shape('session-start-startup.json'))
  assert.deepEqual(prompt.argv.slice(-3), ['hook-event', 'opencode', 'UserPromptSubmit'])
  assert.deepEqual(Object.keys(JSON.parse(prompt.stdin)).sort(), shape('user-prompt-submit.json'))
  assert.deepEqual(sub.argv.slice(-3), ['hook-event', 'opencode', 'SubagentStart'])
  assert.deepEqual(JSON.parse(sub.stdin), { event: 'SubagentStart', session_id: B, parent_session_id: A, cwd: '/work/repo' })
  assert.deepEqual(Object.keys(JSON.parse(sub.stdin)).sort(), shape('subagent-start.json'))
  assert.equal(sub.env.HERDR_THREADS_OPENCODE_PARENT, A)
})

test('a session older than the process starts with resume', async () => {
  const h = harness()
  h.top(A)
  await h.turn(A) // --continue: no session.created
  assert.deepEqual(h.hookEvents(), ['SessionStart:resume'])
})

test('/new and session switch: clear for a new session, resume for an older one', async () => {
  const h = harness()
  await h.boot(A)
  // /new: a session created in this process
  h.top(B)
  await h.created(B)
  await h.turn(B)
  // switch back to A, then to an older session C
  await h.turn(A)
  h.top(C)
  await h.turn(C)
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'SessionStart:clear', 'SessionStart:resume', 'SessionStart:resume'])
  // each rebind restarts the watch for the new session and stops the old one
  assert.deepEqual(h.spawns.map((s) => s.argv.at(-1)), [A, B, A, C])
  assert.deepEqual(h.spawns.map((s) => s.stopped), [true, true, true, false])
  assert.equal(h.core.snapshot().bound.sid, C)
})

test('compaction re-enters with SessionStart compact and appends its output', async () => {
  const h = await harness().boot(A)
  h.hookReply = (event, p) => ({ code: 0, stdout: p.source === 'compact' ? '<compact re-entry>' : '' })
  await h.event('session.compacted', { sessionID: A })
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'SessionStart:compact'])
  assert.equal(h.appends().length, 1)
  assert.equal(h.appends()[0].text, '<compact re-entry>')
  assert.equal(h.appends()[0].body.noReply, true)
  // the append's own chat.message ran no hook
  assert.equal(h.hooks().length, 2)
  assert.equal(h.acks().length, 0)
  // a compaction of another session is not ours
  await h.event('session.compacted', { sessionID: B })
  assert.equal(h.hooks().length, 2)
})

test('a failed check-in leaves the session unbound and the next turn retries it', async () => {
  for (const failure of [
    { code: 1, stdout: CHECKIN },
    { code: 0, stdout: '' },
    { code: 0, stdout: 'herdr-threads: check-in unavailable (daemon_unavailable). Tool use continues normally; …' },
    { code: -1, stdout: '' },
  ]) {
    const h = harness()
    h.top(A)
    await h.created(A)
    h.hookReply = () => failure
    const out = await h.turn(A)
    assert.equal(h.core.snapshot().bound, null)
    assert.equal(h.spawns.length, 0)
    // the shared unavailable notice is still shown; a failed exit injects nothing
    assert.equal(out.parts.length, failure.code === 0 && failure.stdout ? 2 : 1)
    h.hookReply = () => ({ code: 0, stdout: CHECKIN })
    await h.turn(A)
    assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'SessionStart:startup'])
    assert.equal(h.core.snapshot().bound.sid, A)
    assert.equal(h.spawns.length, 1)
  }
})

test('the plugin\'s own append and wake are not user turns', async () => {
  const h = await harness().boot(A)
  await h.line(waking('m1'))
  // the append raised chat.message (fake client) and ran no hook; the wake ran the turn hook
  assert.deepEqual(h.calls.map((c) => c.kind), ['append', 'wake'])
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup'])
  await h.core.onChatMessage({ sessionID: A }, { message: { id: h.nextMsg() }, parts: [{ type: 'text', text: WAKE }] })
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit'])
})

test('a wake into a session whose check-in was lost runs the turn hook, never a lifecycle', async () => {
  const h = await harness().boot(A)
  await h.line(waking('m1'))
  await h.line(statusLine('refused', 'no_binding', 2)) // marks A for re-check-in
  await h.core.onChatMessage({ sessionID: A }, { message: { id: h.nextMsg() }, parts: [{ type: 'text', text: WAKE }] })
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit'])
  // a real user turn then checks in again
  await h.turn(A)
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit', 'SessionStart:resume'])
})

test('one plugin instance per process: a second stays inert until the first is disposed', async () => {
  const guard = {}
  const sys = {
    spawn: () => { throw new Error('no spawn in this test') },
    fs: { mkdirSync() {}, writeFileSync() {}, renameSync() {}, appendFileSync() {} },
    now: () => 1,
    env: () => ({ OPENCODE_PID: '7' }),
    pid: () => 7,
    every: () => () => {},
    guard,
  }
  const client = { session: { get: async () => ({ data: { id: A } }) } }
  const first = await makePlugin({ client, directory: '/w' }, sys, { argv: null, grant: false })
  const second = await makePlugin({ client, directory: '/w' }, sys, { argv: null, grant: false })
  const env = async (p) => {
    const out = { env: {} }
    await p.hooks['shell.env']({ sessionID: A }, out)
    return out.env.HERDR_THREADS_OPENCODE_SESSION
  }
  assert.equal(await env(first), A)
  assert.equal(await env(second), undefined)
  await first.hooks.dispose()
  assert.equal(await env(first), undefined)
  assert.equal(await env(second), A)
})

test('the heartbeat is written at the end of init into the state dir', async () => {
  const writes = []
  const fs = {
    mkdirSync: (d) => writes.push(['mkdir', d]),
    writeFileSync: (f, data) => writes.push(['write', f, data]),
    renameSync: (a, b) => writes.push(['rename', a, b]),
    appendFileSync() {},
  }
  const sys = { spawn() {}, fs, now: () => 99, env: () => ({}), pid: () => 7, every: () => () => {}, guard: {} }
  const client = { session: { prompt() {}, promptAsync() {}, status() {}, get() {} }, tui: { showToast() {} } }
  const p = await makePlugin({ client, directory: '/w' }, sys, { argv: ['/x/ht', '--state-dir', '/state', '--host-endpoint', '/h'], grant: true })
  assert.deepEqual(writes[0], ['mkdir', '/state/opencode'])
  assert.deepEqual(writes[2], ['rename', '/state/opencode/plugin-heartbeat.json.7.tmp', '/state/opencode/plugin-heartbeat.json'])
  const record = JSON.parse(writes[1][2])
  assert.deepEqual(record, {
    schema: 1,
    plugin: 'herdr-threads',
    pid: 7,
    role: 'active',
    directory: '/w',
    launch: true,
    permissions: 'consented',
    capabilities: { append: true, wake: true, status: true, session_get: true, toast: true },
    loaded_at: 99,
    at: 99,
  })
  // a later write (the consent result) keeps the earlier fields
  await p.hooks.config({ permission: { bash: { '*': 'ask' } } })
  const later = JSON.parse(writes.filter((w) => w[0] === 'write').at(-1)[2])
  assert.equal(later.loaded_at, 99)
  assert.deepEqual(later.permission, { granted: ['herdr-threads *', 'ht *'], denied: [] })
  // no state dir known (no rendered launch): no heartbeat
  writes.length = 0
  await makePlugin({ client, directory: '/w' }, { ...sys, guard: {} }, { argv: null, grant: false })
  assert.equal(writes.length, 0)
})

// ---- D4: delivery ----------------------------------------------------------------------------

test('a watch line is appended and acked with the stored message id', async () => {
  const h = await harness().boot(A)
  await h.line(msg('m1', { ack_required: true, wakes: true, priority: 0 }))
  const [a] = h.appends()
  assert.equal(a.sid, A)
  assert.deepEqual(a.body, { agent: 'build', noReply: true, parts: [{ type: 'text', text: a.text, synthetic: true }] })
  assert.equal(a.text, '[herdr-threads] new messages:\n\n[herdr-threads] message m1 in T1 "plans" from S1 "alice" [p0]:\n  body of m1')
  const acks = h.acks()
  assert.equal(acks.length, 1)
  const row = acks[0].argv[acks[0].argv.indexOf('--row') + 1]
  assert.match(row, /^msg_/)
  assert.deepEqual(acks[0].argv, ['herdr-threads', 'watch', 'ack', '--session', A, '--row', row, '--via', 'append', 'm1'])
  assert.equal(acks[0].env.HERDR_THREADS_OPENCODE_SESSION, A)
  assert.deepEqual(h.core.snapshot().unacked, [])
  // a re-streamed delivered item is not appended again
  await h.line(msg('m1', { ack_required: true, wakes: true }))
  assert.equal(h.appends().length, 1)
})

test('append and wake carry the agent, model and variant of the newest real user message', async () => {
  const h = harness()
  h.top(A)
  await h.created(A)
  const plan = { agent: 'plan', model: { providerID: 'p1', modelID: 'm-one', variant: 'high' } }
  await h.turn(A, 'hi', plan)
  await h.idle(A)
  await h.line(connected)
  await h.line(waking('m1'))
  const fields = { agent: 'plan', model: { providerID: 'p1', modelID: 'm-one' }, variant: 'high' }
  const [a] = h.appends()
  const [w] = h.wakes()
  assert.deepEqual(a.body, { ...fields, noReply: true, parts: [{ type: 'text', text: a.text, synthetic: true }] })
  assert.deepEqual(w.body, { ...fields, parts: [{ type: 'text', text: WAKE }] })
  // the plugin's own wake turn and its append never replace the record
  await h.core.onChatMessage({ sessionID: A }, { message: { id: h.nextMsg(), role: 'user', agent: 'build' }, parts: [{ type: 'text', text: WAKE }] })
  await h.idle(A)
  await h.line(waking('m2'))
  assert.deepEqual(h.appends()[1].body.agent, 'plan')
  assert.deepEqual(h.wakes()[1].body.model, { providerID: 'p1', modelID: 'm-one' })
  // a new real message (the user switched agent) replaces it; no variant stays absent
  await h.turn(A, 'hi', { agent: 'build', model: { providerID: 'p2', modelID: 'm-two' } })
  await h.idle(A)
  await h.line(waking('m3'))
  assert.deepEqual(h.appends()[2].body, { agent: 'build', model: { providerID: 'p2', modelID: 'm-two' }, noReply: true, parts: h.appends()[2].body.parts })
  assert.deepEqual(h.wakes()[2].body, { agent: 'build', model: { providerID: 'p2', modelID: 'm-two' }, parts: [{ type: 'text', text: WAKE }] })
})

test('the agent record is per session: a rebound session uses its own', async () => {
  const h = harness()
  h.top(A)
  h.top(B)
  await h.created(A)
  await h.turn(A, 'hi', { agent: 'plan', model: { providerID: 'p', modelID: 'a' } })
  await h.created(B)
  await h.turn(B, 'hi', { agent: 'review', model: { providerID: 'p', modelID: 'b' } })
  assert.equal(h.core.snapshot().chanSid, B)
  await h.idle(B)
  await h.line(connected)
  await h.line(waking('m1'))
  assert.deepEqual(h.appends()[0].body.agent, 'review')
  assert.deepEqual(h.wakes()[0].body.model, { providerID: 'p', modelID: 'b' })
  // back to A: A's record, not B's
  await h.turn(A)
  assert.equal(h.core.snapshot().chanSid, A)
  await h.idle(A)
  await h.line(connected)
  await h.line(waking('m2'))
  assert.deepEqual(h.appends()[1].body.agent, 'build') // A's newest real message had input agent build only
  assert.equal(h.appends()[1].body.model, undefined)
})

test('a lazy row is acked without a row id; a truncated item is never acked', async () => {
  const h = await harness().boot(A)
  await h.line(msg('m1', { kind: 'lazy' }))
  await h.line(msg('m2', { truncated: true, body: 'cut…truncated; next: herdr-threads inbox' }))
  assert.equal(h.appends().length, 2)
  assert.deepEqual(h.acks().map((r) => r.argv.slice(3)), [['--session', A, '--via', 'append', 'm1']])
  assert.match(h.appends()[1].text, /cut…truncated\n\nnext: herdr-threads inbox$/)
})

test('the recovery line is the daemon-spelled inbox command, never the absolute launch argv', async () => {
  const argv = ['/opt/ht', '--state-dir', '/s', '--host-endpoint', '/h.sock']
  const h = await harness({ argv }).boot(A)
  // a truncated body carries the watch's own marker: its command is the recovery line
  await h.line(msg('m1', { truncated: true, body: 'cut…truncated; next: herdr-threads --state-dir /s inbox' }))
  assert.match(h.appends()[0].text, /cut…truncated\n\nnext: herdr-threads --state-dir \/s inbox$/)
  assert.doesNotMatch(h.appends()[0].text, /\/opt\/ht/)
  // and stays the line for a later page that only overflows
  const items = Array.from({ length: 33 }, (_, i) => ({ id: `x${i}`, kind: 'message', thread: 'T', body: 'b', priority: 1 }))
  assert.match(appendBatch(items, 'next: herdr-threads --state-dir /s inbox').text, /\n\nnext: herdr-threads --state-dir \/s inbox$/)
})

test('recovery markers: only a herdr-threads inbox command is taken; the fallbacks need no absolute path', () => {
  assert.equal(markerRecovery('cut…truncated; next: herdr-threads inbox', '…truncated; next: '), 'next: herdr-threads inbox')
  assert.equal(markerRecovery('herdr-threads: attention pending; run herdr-threads --host-endpoint /h inbox', 'herdr-threads: attention pending; run '),
    'next: herdr-threads --host-endpoint /h inbox')
  assert.equal(markerRecovery('cut…truncated; next: herdr-threads accept t1', '…truncated; next: '), null)
  assert.equal(markerRecovery('cut…truncated; next: /opt/ht inbox', '…truncated; next: '), null)
  assert.equal(markerRecovery('no marker', '…truncated; next: '), null)
  // before any marker: the launch's selectors after the bare name; no launch: the bare command
  assert.equal(launchRecovery(['/opt/ht', '--state-dir', '/s s', '--host-endpoint', '/h.sock']),
    "next: herdr-threads --state-dir '/s s' --host-endpoint /h.sock inbox")
  assert.equal(launchRecovery(null), 'next: herdr-threads inbox')
})

test('an attention marker sets the recovery line of a later overflow page', async () => {
  const h = await harness({ argv: ['/opt/ht', '--state-dir', '/s'] }).boot(A)
  h.appendResult = () => new Promise(() => {}) // the first append hangs: later items queue up
  await h.line(attention(1, { text: 'herdr-threads: attention pending; run herdr-threads --state-dir /s inbox' }))
  for (let i = 0; i < 40; i++) await h.line(msg(`q${i}`))
  await h.expire() // the hung append times out; the queue is offered after the backoff
  h.appendResult = null
  h.t += 31000
  await h.core.onTick()
  await flush()
  const page = h.appends().find((a) => a.text.includes('q0'))
  assert.ok(page, 'an overflowing page was appended')
  assert.match(page.text, /\n\nnext: herdr-threads --state-dir \/s inbox$/)
})

test('the marker strings match the daemon source', () => {
  const watch = readFileSync(join(root, 'src/protocol/watch.rs'), 'utf8')
  assert.ok(watch.includes('"…truncated; next: {}"'))
  assert.ok(watch.includes('"herdr-threads: attention pending; run {}"'))
})

test('no ack when the append fails; the item is offered again after the backoff', async () => {
  const h = await harness().boot(A)
  h.appendResult = () => ({ error: { name: 'BadRequest' } })
  await h.line(msg('m1', { ack_required: true }))
  assert.equal(h.appends().length, 1)
  assert.equal(h.acks().length, 0)
  assert.equal(h.wakes().length, 0)
  assert.deepEqual(h.core.snapshot().queue, ['m1'])
  h.appendResult = null
  await h.core.onTick()
  await flush()
  assert.equal(h.appends().length, 1) // still backing off
  h.t += 30_000
  await h.core.onTick()
  await flush()
  assert.equal(h.appends().length, 2)
  assert.equal(h.acks().length, 1)
})

test('an append that throws is a failure too', async () => {
  const h = await harness().boot(A)
  h.client.session.prompt = async () => { throw new Error('boom') }
  await h.line(msg('m1', { ack_required: true }))
  assert.equal(h.acks().length, 0)
  assert.deepEqual(h.core.snapshot().queue, ['m1'])
})

test('busy session: a waking item is appended, never woken while busy', async () => {
  const h = await harness().boot(A)
  await h.turn(A) // a user turn starts
  await h.busy(A)
  h.statusMap = { [A]: { type: 'busy' } }
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
  assert.ok(h.core.snapshot().wakePending)
  // the turn's next model step read the append (its newest user message): that turn reads it
  await h.step(A, h.appends()[0].id)
  assert.equal(h.core.snapshot().wakePending, null)
  h.statusMap = {}
  await h.idle(A)
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
})

test('a step that began before the append does not clear the wake-pending mark', async () => {
  const h = await harness().boot(A)
  await h.busy(A)
  h.statusMap = { [A]: { type: 'busy' } }
  const userMsg = h.nextMsg()
  const earlier = h.nextMsg()
  await h.line(waking('m1'))
  await h.event('message.updated', { info: { id: earlier, role: 'assistant', sessionID: A, parentID: userMsg } })
  assert.ok(h.core.snapshot().wakePending)
})

test('E1: a step created after the append from a message list read before it still owes the wake', async () => {
  const h = await harness().boot(A)
  await h.busy(A)
  h.statusMap = { [A]: { type: 'busy' } }
  const userMsg = h.nextMsg() // the newest user message when the step read its list
  await h.line(waking('m1'))
  // opencode created the step's assistant message after the append was stored (a later id), but
  // its parent is the older user message: the step's context lacks the append
  await h.step(A, userMsg)
  assert.ok(h.core.snapshot().wakePending)
  // the loop then ends (its last assistant is newer than every user message): the wake goes out
  h.statusMap = {}
  await h.idle(A)
  assert.deepEqual(h.calls.map((c) => c.kind), ['append', 'wake'])
  // an assistant message without a parent proves nothing either
  const h2 = await harness().boot(A)
  await h2.busy(A)
  h2.statusMap = { [A]: { type: 'busy' } }
  await h2.line(waking('m1'))
  await h2.event('message.updated', { info: { id: h2.nextMsg(), role: 'assistant', sessionID: A } })
  assert.ok(h2.core.snapshot().wakePending)
})

test('E2: a user message whose turn has not started keeps the session busy against a status read', async () => {
  const h = await harness().boot(A)
  let release
  h.hookReply = (event) => (event === 'UserPromptSubmit'
    ? new Promise((r) => { release = () => r({ code: 0, stdout: '' }) })
    : { code: 0, stdout: CHECKIN })
  const output = { message: { id: h.nextMsg(), role: 'user' }, parts: [{ type: 'text', text: 'hi' }] }
  const turn = h.core.onChatMessage({ sessionID: A, agent: 'build' }, output)
  await flush()
  // the server has not started the turn yet, so its status map says idle
  h.statusMap = {}
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
  release()
  await turn
  // the turn starts and its first step reads the append: no wake is owed
  await h.busy(A)
  await h.step(A, h.appends()[0].id)
  await h.idle(A)
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
  // with no status event at all the mark expires and the status read decides again
  const h2 = await harness().boot(A)
  h2.hookReply = () => ({ code: 0, stdout: '' })
  await h2.turn(A)
  h2.t += 18_000
  h2.statusMap = {}
  await h2.line(waking('m2'))
  assert.deepEqual(h2.calls.map((c) => c.kind), ['append', 'wake'])
})

test('idle session with a waking item: append, then the content-free wake', async () => {
  const h = await harness().boot(A)
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => [c.kind, c.sid]), [['append', A], ['wake', A]])
  assert.equal(h.wakes()[0].text, WAKE)
  assert.equal(h.core.snapshot().wakePending, null)
  assert.equal(h.core.snapshot().pluginTurn, A)
  // an ACK request wakes too, whatever its priority marker
  const h2 = await harness().boot(A)
  await h2.line(msg('m2', { ack_required: true }))
  assert.deepEqual(h2.calls.map((c) => c.kind), ['append', 'wake'])
})

test('C1: the turn ends between the append and the next step, so the wake is sent on idle', async () => {
  const h = await harness().boot(A)
  await h.busy(A)
  h.statusMap = { [A]: { type: 'busy' } }
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
  // the turn ended on its final text (or an interrupt, or a rejected permission): no new step
  h.statusMap = {}
  await h.idle(A)
  assert.deepEqual(h.calls.map((c) => c.kind), ['append', 'wake'])
})

test('C1: an append that lands just after the turn ended is woken by the status re-read', async () => {
  const h = await harness().boot(A)
  await h.busy(A) // the local view still says busy
  h.statusMap = {} // but the session is already idle
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.kind), ['append', 'wake'])
})

test('rule 2: a non-waking item on an idle session is appended with no wake', async () => {
  const h = await harness().boot(A)
  await h.line(msg('m1')) // p1, no wakes, no ACK
  await h.line(msg('m2', { kind: 'invitation', wakes: false }))
  await h.line({ schema: 1, id: 'w1', kind: 'notice', warning: 'w1', thread: 'T1', body: 'note' })
  await h.core.onTick()
  await h.idle(A)
  assert.deepEqual(h.calls.map((c) => c.kind), ['append', 'append', 'append'])
  assert.equal(h.core.snapshot().wakePending, null)
})

test('no wake while a permission or question is pending, and a toast for a plugin-started turn', async () => {
  const h = await harness().boot(A)
  await h.line(waking('m1'))
  assert.equal(h.wakes().length, 1)
  await h.busy(A)
  await h.event('permission.asked', { id: 'per_1', sessionID: A, permission: 'bash' })
  assert.equal(h.toasts.length, 1)
  assert.match(h.toasts[0].message, new RegExp(A))
  assert.equal(h.toasts[0].variant, 'info')
  // a second waking item while the prompt waits: appended, no wake
  h.statusMap = {}
  await h.event('session.status', { sessionID: A, status: { type: 'busy' } })
  await h.line(waking('m2'))
  assert.equal(h.wakes().length, 1)
  await h.event('permission.replied', { sessionID: A, requestID: 'per_1', reply: 'reject' })
  await h.idle(A)
  assert.equal(h.wakes().length, 2)
  // a permission in a user's own turn shows no toast
  const u = await harness().boot(A)
  await u.turn(A)
  await u.event('question.asked', { id: 'que_1', sessionID: A })
  assert.equal(u.toasts.length, 0)
})

test('a failed wake backs off and keeps the wake pending', async () => {
  const h = await harness().boot(A)
  h.wakeResult = () => ({ error: { name: 'ServerError' } })
  await h.line(waking('m1'))
  assert.equal(h.wakes().length, 1)
  assert.ok(h.core.snapshot().wakePending)
  h.wakeResult = null
  await h.core.onTick()
  assert.equal(h.wakes().length, 1)
  h.t += 30_000
  await h.core.onTick()
  assert.equal(h.wakes().length, 2)
  assert.equal(h.core.snapshot().wakePending, null)
})

test('the frame of a batch follows the daemon\'s priority order', async () => {
  const fixture = JSON.parse(readFileSync(join(root, 'tests/fixtures/priority_order_mixed.json'), 'utf8'))
  const h = await harness().boot(A)
  await h.busy(A) // collect the whole set into one append
  h.statusMap = { [A]: { type: 'busy' } }
  h.core.onLine({ ...connected })
  const lines = fixture.items.map((o) => ({ schema: 1, ...o }))
  // hold the pump until every line is queued
  h.core.onEvent({ type: 'noop' })
  const prompt = h.client.session.prompt
  let release
  const gate = new Promise((r) => { release = r })
  let firstHeld = false
  h.client.session.prompt = async (args) => {
    if (!firstHeld) {
      firstHeld = true
      await gate
    }
    return prompt(args)
  }
  await h.line({ schema: 1, id: 'm-first', kind: 'lazy', thread: 'T1', body: 'first' })
  for (const o of lines) h.watch().cb.line(o)
  release()
  await flush()
  const second = h.appends()[1].text
  const ids = [...second.matchAll(/^\[herdr-threads\] (?:message|invitation) (\S+) in /gm)].map((m) => m[1])
  assert.deepEqual(ids, fixture.expected)
})

test('watch no_binding or session_mismatch makes the next turn check in again', async () => {
  for (const reason of ['no_binding', 'session_mismatch']) {
    const h = await harness().boot(A)
    await h.line(statusLine('refused', reason, 2))
    assert.equal(h.core.snapshot().bound.ok, false)
    h.watch().cb.exit(2)
    await h.turn(A)
    assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'SessionStart:resume'])
    assert.equal(h.core.snapshot().bound.ok, true)
    // the successful check-in restarts the watch at once
    assert.equal(h.spawns.length, 2)
  }
})

test('the watch restarts with backoff after an exit and stops for a terminal reason', async () => {
  const h = await harness().boot(A)
  h.watch().cb.exit(0)
  assert.equal(h.spawns.length, 1)
  h.t += 1000
  await h.core.onTick()
  assert.equal(h.spawns.length, 2)
  await h.line(statusLine('closing', 'replaced', 3))
  h.watch().cb.exit(3)
  h.t += 60_000
  await h.core.onTick()
  assert.equal(h.spawns.length, 2)
  assert.equal(h.core.snapshot().stopped, true)
})

test('channel down: the turn hook delivers at turn start, with no append and no wake', async () => {
  const h = await harness({ env: { HERDR_THREADS_MOD_DELIVERY: 'off' } }).boot(A)
  await h.line(statusLine('refused', 'env_disabled', 3))
  h.watch().cb.exit(3)
  assert.equal(h.core.snapshot().live, false)
  h.hookReply = (event) => ({ code: 0, stdout: event === 'UserPromptSubmit' ? '[herdr-threads] new messages:\n\n<pending>' : CHECKIN })
  const out = await h.turn(A)
  assert.equal(out.parts[1].text, '[herdr-threads] new messages:\n\n<pending>')
  await h.idle(A)
  h.t += 60_000
  await h.core.onTick()
  assert.equal(h.calls.length, 0)
  assert.equal(h.spawns.length, 1)
})

test('an append API that returns no stored id stops the channel and falls back to the turn hook', async () => {
  const h = await harness().boot(A)
  h.appendResult = () => ({ data: {} })
  await h.line(waking('m1'))
  assert.equal(h.acks().length, 0)
  assert.equal(h.wakes().length, 0)
  assert.equal(h.core.snapshot().channelless, true)
  assert.deepEqual(h.core.snapshot().degraded, ['append_row'])
  assert.equal(h.watch().stopped, true)
  await h.turn(A)
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit'])
})

test('no promptAsync or no prompt: no channel at all, so the daemon sees no wake path', async () => {
  for (const [missing, capability] of [['promptAsync', 'wake'], ['prompt', 'append']]) {
    const h = harness()
    delete h.client.session[missing]
    h.top(A)
    await h.created(A)
    await h.turn(A)
    // bound, but no watch was ever started: no registration, so senders read wake=no_wake
    assert.equal(h.core.snapshot().bound.sid, A)
    assert.equal(h.spawns.length, 0)
    assert.equal(h.core.snapshot().channelless, true)
    assert.deepEqual(h.core.snapshot().degraded, [capability])
    // the turn hook delivers from now on, and nothing is appended or woken
    h.hookReply = (event) => ({ code: 0, stdout: event === 'UserPromptSubmit' ? '<pending>' : CHECKIN })
    const out = await h.turn(A)
    assert.equal(out.parts[1].text, '<pending>')
    h.t += 60_000
    await h.core.onTick()
    assert.equal(h.spawns.length, 0)
    assert.equal(h.calls.length, 0)
    assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit'])
  }
})

test('promptAsync gone after the channel started: the channel stops at the first wake', async () => {
  const h = await harness().boot(A)
  delete h.client.session.promptAsync
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
  assert.equal(h.core.snapshot().channelless, true)
  assert.deepEqual(h.core.snapshot().degraded, ['wake'])
  assert.equal(h.watch().stopped, true)
})

// ---- E3: bounded SDK calls ----------------------------------------------------------------------

const hang = () => new Promise(() => {})

test('a hung append times out into the failed-append path and is offered again', async () => {
  const h = await harness().boot(A)
  const prompt = h.client.session.prompt
  h.client.session.prompt = hang
  await h.line(msg('m1', { ack_required: true }))
  assert.equal(h.acks().length, 0)
  await h.expire()
  assert.ok(h.ledger.some((e) => e.reason === 'append:timeout'))
  assert.deepEqual(h.core.snapshot().queue, ['m1'])
  h.client.session.prompt = prompt
  h.t += 30_000
  await h.core.onTick()
  await flush()
  assert.equal(h.appends().length, 1)
  assert.equal(h.acks().length, 1)
})

test('a hung wake times out, keeps the wake pending and retries after the backoff', async () => {
  const h = await harness().boot(A)
  const promptAsync = h.client.session.promptAsync
  let hung = 0
  h.client.session.promptAsync = () => (hung++, hang())
  await h.line(waking('m1'))
  assert.equal(hung, 1)
  await h.expire()
  assert.ok(h.core.snapshot().wakePending)
  assert.ok(h.ledger.some((e) => e.reason === 'wake_drop:timeout'))
  // if the timed-out wake does start its turn later, that turn is still recognised as the plugin's
  await h.core.onChatMessage({ sessionID: A }, { message: { id: h.nextMsg() }, parts: [{ type: 'text', text: WAKE }] })
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'UserPromptSubmit'])
  await h.idle(A)
  h.client.session.promptAsync = promptAsync
  h.t += 30_000
  await h.core.onTick()
  assert.equal(h.wakes().length, 1)
})

test('a hung session.get never blocks a turn or a shell call', async () => {
  const h = harness()
  h.client.session.get = hang
  const output = { message: { id: h.nextMsg(), role: 'user' }, parts: [{ type: 'text', text: 'hi' }] }
  const turn = h.core.onChatMessage({ sessionID: A, agent: 'build' }, output)
  const env = { env: {} }
  const shell = h.core.onShellEnv({ sessionID: A }, env)
  await h.expire()
  await turn
  await shell
  // the role is unknown: no hook this turn, retried next message; the shell gets parent '?'
  assert.equal(h.hooks().length, 0)
  assert.equal(output.parts.length, 1)
  assert.equal(env.env.HERDR_THREADS_OPENCODE_PARENT, '?')
  assert.equal(env.env.HERDR_THREADS_OPENCODE_SESSION, A)
  h.client.session.get = async () => ({ data: { id: A } })
  await h.turn(A)
  assert.deepEqual(h.hookEvents(), ['SessionStart:resume'])
})

test('a hung status read leaves the local status alone', async () => {
  const h = await harness().boot(A)
  await h.busy(A)
  h.client.session.status = hang
  await h.line(waking('m1'))
  await h.expire()
  assert.deepEqual(h.calls.map((c) => c.kind), ['append'])
  assert.ok(h.core.snapshot().wakePending)
})

// ---- ACK error branches ----------------------------------------------------------------------

const ackWith = (result) => (argv) => ({
  code: 0,
  stdout: argv.slice(argv.indexOf('--via') + 2).map((id) => JSON.stringify({ id, result })).join('\n'),
})
const ackArgs = (h) => h.acks().map((r) => r.argv.slice(3))

test('a retryable ack keeps the item unacked and is retried every 30 s', async () => {
  for (const reply of [() => ({ code: 1, stdout: '' }), ackWith('retryable'), () => ({ code: 0, stdout: '' })]) {
    const h = await harness().boot(A)
    const settle = h.ackReply
    h.ackReply = reply
    await h.line(msg('m1', { ack_required: true }))
    assert.equal(h.acks().length, 1)
    assert.deepEqual(h.core.snapshot().unacked, ['m1'])
    h.t += 1000
    await h.core.onTick()
    assert.equal(h.acks().length, 1)
    h.ackReply = settle
    h.t += 30_000
    await h.core.onTick()
    await flush()
    assert.equal(h.acks().length, 2)
    // the retry carries the stored row again
    const [first, second] = ackArgs(h)
    assert.deepEqual(second, first)
    assert.ok(second.includes('--row'))
    assert.deepEqual(h.core.snapshot().unacked, [])
    assert.ok(h.ledger.some((e) => e.kind === 'acked' && e.ids[0] === 'm1'))
  }
})

test('an ack refused for its arguments (exit 2, an older CLI) is sent again without --row', async () => {
  const h = await harness().boot(A)
  const settle = h.ackReply
  h.ackReply = (argv) => (argv.includes('--row') ? { code: 2, stdout: '' } : settle(argv))
  await h.line(msg('m1', { ack_required: true }))
  await flush()
  const args = ackArgs(h)
  assert.equal(args.length, 2)
  assert.ok(args[0].includes('--row'))
  assert.deepEqual(args[1], ['--session', A, '--via', 'append', 'm1'])
  assert.deepEqual(h.core.snapshot().unacked, [])
  assert.ok(h.ledger.some((e) => e.reason === 'ack:row_refused'))
  // exit 2 without a row is an ordinary retryable failure, not another fallback
  const h2 = await harness().boot(A)
  h2.ackReply = () => ({ code: 2, stdout: '' })
  await h2.line(msg('m2', { kind: 'lazy' }))
  await flush()
  assert.equal(h2.acks().length, 1)
  assert.deepEqual(h2.core.snapshot().unacked, ['m2'])
})

test('refused_terminal drops the ack; stale_generation also forgets the delivery so it is appended again', async () => {
  const h = await harness().boot(A)
  h.ackReply = ackWith('refused_terminal')
  await h.line(msg('m1', { ack_required: true }))
  assert.deepEqual(h.core.snapshot().unacked, [])
  assert.deepEqual(h.core.snapshot().delivered, ['m1'])
  assert.ok(h.ledger.some((e) => e.reason === 'ack:refused_terminal'))
  await h.line(msg('m1', { ack_required: true }))
  assert.equal(h.appends().length, 1)
  h.t += 30_000
  await h.core.onTick()
  assert.equal(h.acks().length, 1)

  const g = await harness().boot(A)
  g.ackReply = ackWith('stale_generation')
  await g.line(msg('m1'))
  assert.deepEqual(g.core.snapshot().unacked, [])
  assert.deepEqual(g.core.snapshot().delivered, [])
  assert.ok(g.ledger.some((e) => e.reason === 'ack:stale_generation'))
  g.ackReply = ackWith('settled')
  await g.line(msg('m1'))
  assert.equal(g.appends().length, 2)
  assert.deepEqual(g.core.snapshot().unacked, [])
})

// ---- attention lines -------------------------------------------------------------------------

const attention = (epoch, extra = {}) => ({ schema: 1, id: `attention:${epoch}`, kind: 'attention', presentation_epoch: String(epoch), attention_version: 3, text: 'herdr-threads: attention pending; run herdr-threads inbox', ...extra })
const cleared = (epoch) => ({ schema: 1, id: `attention_cleared:${epoch}`, kind: 'attention_cleared', presentation_epoch: String(epoch), attention_version: 3 })

test('an attention line is appended unacked, wakes an idle session, and is deduped within a run', async () => {
  const h = await harness().boot(A)
  await h.line(attention(7))
  assert.deepEqual(h.calls.map((c) => c.kind), ['append', 'wake'])
  assert.equal(h.appends()[0].text, '[herdr-threads] new messages:\n\n[herdr-threads] attention attention:7:\n  herdr-threads: attention pending; run herdr-threads inbox')
  assert.equal(h.acks().length, 0)
  // the same attention again in this run: nothing new
  await h.line(attention(7))
  assert.equal(h.appends().length, 1)
  // a restarted watch is a new run: it is presented again
  h.watch().cb.exit(0)
  h.t += 1000
  await h.core.onTick()
  await h.line(connected)
  await h.idle(A)
  await h.line(attention(7))
  assert.equal(h.appends().length, 2)
})

test('an attention line with a non-canonical presentation_epoch is ignored', async () => {
  for (const epoch of ['07', 'x', '', '18446744073709551616', 7]) {
    const h = await harness().boot(A)
    await h.line(attention(7, { presentation_epoch: epoch }))
    await h.line({ ...cleared(7), presentation_epoch: epoch })
    assert.equal(h.calls.length, 0, String(epoch))
  }
})

test('attention_cleared retracts a queued attention of its epoch only, and resets the dedup', async () => {
  const h = await harness().boot(A)
  // hold the queue: the first append fails and backs off
  h.appendResult = () => ({ error: { name: 'BadRequest' } })
  await h.line(msg('m0'))
  h.appendResult = null
  await h.line(attention(7))
  await h.line(attention(8))
  assert.deepEqual(h.core.snapshot().queue, ['m0', 'attention:7', 'attention:8'])
  await h.line(cleared(7))
  assert.deepEqual(h.core.snapshot().queue, ['m0', 'attention:8'])
  assert.ok(h.ledger.some((e) => e.reason === 'attention_cleared' && e.ids.join() === 'attention:7'))
  h.t += 30_000
  await h.core.onTick()
  await flush()
  assert.equal(h.appends().length, 2)
  assert.doesNotMatch(h.appends()[1].text, /attention:7/)
  assert.match(h.appends()[1].text, /attention attention:8:/)
  // delivered, then cleared: the same epoch may be presented again in this run
  await h.line(attention(8))
  assert.equal(h.appends().length, 2)
  await h.line(cleared(8))
  await h.idle(A)
  await h.line(attention(8))
  assert.equal(h.appends().length, 3)
  // a clear without an epoch retracts every queued attention
  h.appendResult = () => ({ error: { name: 'BadRequest' } })
  await h.line(msg('m9'))
  h.appendResult = null
  await h.line(attention(9))
  const { presentation_epoch: _, ...bare } = cleared(9)
  await h.line(bare)
  assert.deepEqual(h.core.snapshot().queue, ['m9'])
})

test('after /new the previous session stays bound and is still the wake target', async () => {
  const h = await harness().boot(A)
  // /new returns to the home screen; no event, no prompt yet
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.sid), [A, A])
})

// ---- D5: subagents ---------------------------------------------------------------------------

test('a subagent session gets SubagentStart once and never a channel', async () => {
  const h = await harness().boot(A)
  h.hookReply = (event) => ({ code: 0, stdout: event === 'SubagentStart' ? '<child restriction>' : '' })
  await h.created(B, A)
  const first = await h.turn(B)
  const second = await h.turn(B)
  assert.deepEqual(h.hookEvents(), ['SessionStart:startup', 'SubagentStart'])
  assert.equal(first.parts[1].text, '<child restriction>')
  assert.equal(second.parts.length, 1)
  assert.equal(h.spawns.length, 1)
  assert.equal(h.core.snapshot().bound.sid, A)
  // a waking item still goes to the parent, never to the child
  await h.line(waking('m1'))
  assert.deepEqual(h.calls.map((c) => c.sid), [A, A])
})

test('a session whose role cannot be read is treated as a child: no hook until it resolves', async () => {
  const h = harness()
  await h.turn(C) // client.session.get fails
  assert.equal(h.hooks().length, 0)
  assert.equal(h.spawns.length, 0)
  h.top(C)
  await h.turn(C)
  assert.deepEqual(h.hookEvents(), ['SessionStart:resume'])
})

// ---- D6: permission consent ------------------------------------------------------------------

test('without consent the config hook adds nothing', async () => {
  const sys = { spawn() {}, fs: { mkdirSync() {}, writeFileSync() {}, renameSync() {} }, now: () => 1, env: () => ({}), pid: () => 1, every: () => () => {}, guard: {} }
  const p = await makePlugin({ client: {}, directory: '/w' }, sys, { argv: null, grant: false })
  const cfg = { permission: { bash: { '*': 'ask' } } }
  await p.hooks.config(cfg)
  assert.deepEqual(cfg, { permission: { bash: { '*': 'ask' } } })
  const consented = await makePlugin({ client: {}, directory: '/w' }, { ...sys, guard: {} }, { argv: null, grant: true })
  await consented.hooks.config(cfg)
  assert.deepEqual(Object.entries(cfg.permission.bash), [['*', 'ask'], ['herdr-threads *', 'allow'], ['ht *', 'allow']])
})

test('the grant lands last, respects an exact user deny and stays behind narrower user rules', () => {
  const cases = [
    [{ permission: { bash: { '*': 'ask' } } }, [['*', 'ask'], ['herdr-threads *', 'allow'], ['ht *', 'allow']], ['herdr-threads *', 'ht *'], []],
    // probe C1: an existing allow key is moved last, not overwritten in place
    [{ permission: { bash: { 'herdr-threads *': 'ask', '*': 'ask' } } }, [['*', 'ask'], ['herdr-threads *', 'allow'], ['ht *', 'allow']], ['herdr-threads *', 'ht *'], []],
    // an exact user deny is kept where it is and that grant is skipped
    [{ permission: { bash: { 'herdr-threads *': 'deny', '*': 'ask' } } }, [['herdr-threads *', 'deny'], ['*', 'ask'], ['ht *', 'allow']], ['ht *'], ['herdr-threads *']],
    // probe C2: a narrower user rule keeps winning over the grant
    [{ permission: { bash: { '*': 'ask', 'herdr-threads probe *': 'deny', 'ht x': 'ask' } } }, [['*', 'ask'], ['herdr-threads *', 'allow'], ['ht *', 'allow'], ['herdr-threads probe *', 'deny'], ['ht x', 'ask']], ['herdr-threads *', 'ht *'], []],
    // probe C3: a string rule becomes {"*": rule}
    [{ permission: { bash: 'ask' } }, [['*', 'ask'], ['herdr-threads *', 'allow'], ['ht *', 'allow']], ['herdr-threads *', 'ht *'], []],
    [{}, [['herdr-threads *', 'allow'], ['ht *', 'allow']], ['herdr-threads *', 'ht *'], []],
  ]
  for (const [cfg, want, granted, denied] of cases) {
    const r = grantPermissions(cfg)
    assert.deepEqual(Object.entries(cfg.permission.bash), want)
    assert.deepEqual(r, { granted, denied })
  }
  // a whole-permission string is the user's choice and is left alone
  const s = { permission: 'ask' }
  assert.deepEqual(grantPermissions(s), { granted: [], denied: [] })
  assert.deepEqual(s, { permission: 'ask' })
})

// ---- parity with the Rust core ---------------------------------------------------------------

test('the wake prompt is the core WAKE_UP and the failure marker is the core notice', () => {
  const policy = readFileSync(join(root, 'src/notification/policy.rs'), 'utf8')
  assert.ok(policy.includes(`pub const WAKE_UP: &str = ${JSON.stringify(constants.WAKE_PROMPT)};`))
  assert.equal(Buffer.byteLength(constants.WAKE_PROMPT), 22)
  const hook = readFileSync(join(root, 'src/cli/hook.rs'), 'utf8')
  assert.ok(hook.includes(`{}${constants.UNAVAILABLE}{reason})`))
  const witness = readFileSync(join(root, 'src/harness/opencode/witness.rs'), 'utf8')
  for (const v of [constants.SESSION_VAR, constants.PARENT_VAR, constants.PID_VAR]) assert.ok(witness.includes(`"${v}"`), v)
})

test('the frame matches the delivery goldens', () => {
  const dir = join(root, 'tests/fixtures/delivery')
  const cases = readdirSync(dir).filter((f) => /^frame-.*\.json$/.test(f))
  assert.ok(cases.length >= 8)
  for (const f of cases) {
    const items = JSON.parse(readFileSync(join(dir, f), 'utf8'))
    const want = readFileSync(join(dir, f.replace(/\.json$/, '.txt')), 'utf8')
    assert.equal(frame(items), want, f)
  }
  assert.deepEqual(batchOrder([{ kind: 'lazy', id: 'a' }, { kind: 'message', id: 'b' }]).map((i) => i.id), ['b', 'a'])
})

test('the module has exactly one export, the plugin function', () => {
  assert.deepEqual(Object.keys(mod), ['HerdrThreads'])
  assert.equal(typeof HerdrThreads, 'function')
  assert.equal(source.split('\n').filter((l) => l.startsWith('const LAUNCH = null // herdr-threads:launch')).length, 1)
  assert.equal(source.split('\n').filter((l) => l.startsWith('const GRANT_PERMISSIONS = false // herdr-threads:permissions')).length, 1)
})

// ---- the node wiring ---------------------------------------------------------------------------

test('the process runner passes stdin and env and reads stdout and the exit code', async () => {
  const sys = { spawn: (await import('node:child_process')).spawn, fs: {}, now: () => 1, env: () => ({ PATH: process.env.PATH }), pid: () => 1, every: () => () => {}, guard: {} }
  const p = await makePlugin({ client: {}, directory: '/w' }, sys, { argv: null, grant: false })
  const script = 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{process.stdout.write(s+"|"+process.env.HT_X);process.exit(4)})'
  const r = await p.io.run([process.execPath, '-e', script], { stdin: '{"a":1}', env: { ...process.env, HT_X: 'y' }, timeoutMs: 10000 })
  assert.deepEqual(r, { code: 4, stdout: '{"a":1}|y' })
  const slow = await p.io.run([process.execPath, '-e', 'setTimeout(()=>{}, 10000)'], { stdin: '', env: process.env, timeoutMs: 200 })
  assert.equal(slow.code, -1)
  const missing = await p.io.run(['/nonexistent/herdr-threads'], { stdin: '', env: process.env })
  assert.equal(missing.code, -1)
})

test('the watch spawner splits JSON lines and reports the exit code', async () => {
  const sys = { spawn: (await import('node:child_process')).spawn, fs: {}, now: () => 1, env: () => ({}), pid: () => 1, every: () => () => {}, guard: {} }
  const p = await makePlugin({ client: {}, directory: '/w' }, sys, { argv: null, grant: false })
  const lines = []
  const code = await new Promise((resolve) => {
    p.io.spawn([process.execPath, '-e', 'process.stdout.write(\'{"id":"a"}\\nnot json\\n{"id":\'); setTimeout(()=>{process.stdout.write(\'"b"}\\n\'); process.exit(2)}, 50)'], { env: process.env }, {
      line: (o) => lines.push(o.id),
      exit: resolve,
    })
  })
  assert.deepEqual(lines, ['a', 'b'])
  assert.equal(code, 2)
})
