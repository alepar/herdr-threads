import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { register } from '../herdr-threads.js'

const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }
async function bounded(promise) {
  let timer
  try { return await Promise.race([promise, new Promise((_resolve, reject) => { timer = setTimeout(() => reject(new Error('SDK fixture barrier timed out')), 3000) })]) }
  finally { clearTimeout(timer) }
}

// Opt-in qualification against the recorded real SDK. No provider credentials,
// network, CLI/service process or real profile participates in these tests.
test('real SDK persists native drafts and continues only completed runs', async t => {
  const root = fs.mkdtempSync(path.join(import.meta.dirname, '.scratch-sdk-'))
  const previous = { HOME: process.env.HOME, PI_CODING_AGENT_DIR: process.env.PI_CODING_AGENT_DIR }
  process.env.HOME = path.join(root, 'home')
  process.env.PI_CODING_AGENT_DIR = path.join(root, 'agent')
  fs.mkdirSync(process.env.HOME, { recursive: true })
  const modules = process.env.HT_PI_SDK_MODULES ?? new URL('../../../../docs/superpowers/runs/2026-11-14-pi-harness/research/candidates/probe/node_modules/', import.meta.url).pathname
  t.after(() => {
    for (const [key, value] of Object.entries(previous)) if (value === undefined) delete process.env[key]; else process.env[key] = value
    fs.rmSync(root, { recursive: true, force: true })
  })
  const { createAgentSession, DefaultResourceLoader, SessionManager, SettingsManager } = await import(pathToFileURL(path.join(modules, '@earendil-works/pi-coding-agent/dist/index.js')))
  const { createAssistantMessageEventStream } = await import(pathToFileURL(path.join(modules, '@earendil-works/pi-ai/dist/index.js')))

  await t.test('native loader accepts the default factory with helper exports', async () => {
    const loader = new DefaultResourceLoader({ cwd: root, agentDir: process.env.PI_CODING_AGENT_DIR, settingsManager: SettingsManager.inMemory({}), noContextFiles: true, noSkills: true, noThemes: true, noPromptTemplates: true,
      additionalExtensionPaths: [new URL('../herdr-threads.js', import.meta.url).pathname] })
    await loader.reload()
    assert.equal(loader.getExtensions().errors.length, 0)
    assert.equal(loader.getExtensions().extensions.length, 1)
    // No bind/session_start: loader qualification must not execute a real ht CLI.
  })

  for (const mode of ['turn', 'settle', 'error', 'aborted', 'cancel-at-settle', 'transformed', 'transformed-p0', 'native-wake', 'reload', 'reload-urgent-unobserved', 'manual-partial', 'manual-transformed', 'manual-reload', 'manual-late-confirm']) await t.test(mode, async () => {
    let session, watch, calls = 0, starts = 0, injected = false, nonce = 0, stopped = 0, spawned = 0, settled = 0, reloaded = false
    const commands = [], providerContexts = [], errors = [], receipts = []
    const late = mode === 'manual-late-confirm' ? { confirm: deferred(), stopRequested: deferred(), stop: deferred(), successor: deferred(), drained: deferred(), contexts: [], order: [] } : null
    const privateRoot = path.join(root, mode); fs.mkdirSync(privateRoot, { recursive: true })
    const send = (waking = false) => {
      injected = true
      if (mode.startsWith('manual-')) {
        const items = ['manual-reload', 'manual-late-confirm'].includes(mode) ? [{ schema: 1, kind: 'notice', id: 'notice:first', warning: 'first', thread: 'thread', body: 'exact first notice' }]
          : ['first', 'second'].map(id => ({ schema: 1, kind: 'notice', id: `notice:${id}`, warning: id, thread: 'thread', body: `exact ${id} notice\n` + 'x\n'.repeat(10000) }))
        watch.line({ schema: 1, kind: 'presentation_page', items, keys: items.map((_, i) => (i + 1).toString(16).padStart(64, '0')), receipt: 'opaque SDK receipt 18446744073709551615' })
        return
      }
      for (const [id, priority] of [['third', 3], ['first', 0], ['second', 1]]) watch.line({ schema: 1, kind: 'message', id, priority, body: `exact ${id}`, body_len: 11, thread: 'thread', sender: 'sender', ack_required: true, wakes: waking && priority === 0, truncated: false })
    }
    const io = {
      env: {}, registry: {}, pid: process.pid, cwd: privateRoot, nonce: () => `${mode}-${++nonce}`,
      run: async (argv, request) => {
        commands.push(argv)
        if (argv.includes('presentation-confirm')) {
          receipts.push(request.stdin)
          if (late && receipts.length === 1) return late.confirm.promise
          return { code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}' }
        }
        if (argv.includes('ack')) return { code: 0, stdout: argv.slice(argv.indexOf('context') + 1).map(id => JSON.stringify({ id, result: 'settled' })).join('\n') }
        return { code: 0, stdout: argv.at(-1) === 'SessionStart' ? 'isolated SDK guidance' : '' }
      },
      spawn: (_argv, _options, handlers) => {
        watch = handlers; const generation = ++spawned
        queueMicrotask(() => {
          handlers.line({ schema: 1, kind: 'status', id: 'status:0', state: 'connected' })
          if (late && generation === 2) {
            late.order.push('successor-page')
            handlers.line({ schema: 1, kind: 'presentation_page', items: [{ schema: 1, kind: 'notice', id: 'notice:second', warning: 'second', thread: 'thread', body: 'exact second notice' }], keys: ['2'.padStart(64, '0')], receipt: 'opaque SDK successor receipt 18446744073709551615' })
            late.successor.resolve()
          }
          if (late && generation === 3) late.drained.resolve()
        })
        return { stop: async () => {
          stopped++
          if (late && generation === 1) { late.order.push('stop-requested'); late.stopRequested.resolve(); await late.stop.promise; late.order.push('stop-completed') }
        } }
      },
      schedule: (ms, fn) => { const timer = setTimeout(fn, ms); timer.unref(); return () => clearTimeout(timer) },
      diagnostic: text => errors.push(text),
    }
    const loader = new DefaultResourceLoader({ cwd: privateRoot, agentDir: process.env.PI_CODING_AGENT_DIR, settingsManager: SettingsManager.inMemory({}), noContextFiles: true, noSkills: true, noThemes: true, noPromptTemplates: true,
      extensionFactories: [pi => {
        register(pi, io)
        pi.on('before_agent_start', () => { starts++ })
        if (late) {
          pi.on('agent_settled', () => { settled++; late.order.push('settled') })
          pi.on('context', e => { late.contexts.push(e.messages.filter(m => m.role === 'custom').map(m => ({ customType: m.customType, content: m.content }))) })
        }
        pi.on('turn_end', e => {
          if (['settle', 'cancel-at-settle', 'reload-urgent-unobserved', 'manual-reload'].includes(mode)) { if (!injected) send(mode === 'reload-urgent-unobserved') }
          if (mode.includes('transformed')) return { entries: e.entries.map(entry => entry.type === 'custom_message' && String(entry.content).includes('exact first') ? { ...entry, content: 'later extension replaced frame' } : entry) }
        })
        if (mode === 'cancel-at-settle') pi.on('agent_before_settle', async (_e, ctx) => { ctx.abort() })
        if (['reload-urgent-unobserved', 'manual-reload'].includes(mode)) pi.on('agent_before_settle', async (_e, ctx) => { if (!reloaded) ctx.abort() })
      }] })
    await loader.reload()
    ;({ session } = await createAgentSession({ cwd: privateRoot, agentDir: process.env.PI_CODING_AGENT_DIR, settingsManager: SettingsManager.inMemory({}), resourceLoader: loader, sessionManager: SessionManager.create(privateRoot, path.join(privateRoot, 'sessions')), tools: [],
      model: { id: 'selected-model', name: 'Selected', api: 'openai-completions', provider: 'selected-provider', baseUrl: 'http://invalid.local', reasoning: false, input: ['text'], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: mode.startsWith('manual-') ? 1000000 : 10000, maxTokens: 1000 } }))
    try {
      session.modelRuntime.hasConfiguredAuth = () => true
      await session.bindExtensions({ mode: 'tui', onError: error => errors.push(error.error) })
      session.agent.streamFunction = (model, context) => {
        calls++; providerContexts.push(JSON.stringify(context.messages))
        assert.equal(model.id, 'selected-model'); assert.equal(model.provider, 'selected-provider')
        if (!injected && !['settle', 'cancel-at-settle', 'reload-urgent-unobserved', 'manual-reload'].includes(mode)) send(mode === 'transformed-p0')
        const stream = createAssistantMessageEventStream()
        queueMicrotask(() => {
          const reason = mode === 'error' ? 'error' : mode === 'aborted' ? 'aborted' : 'stop'
          const message = { role: 'assistant', content: [{ type: 'text', text: 'scripted' }], api: model.api, provider: model.provider, model: model.id, usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } }, stopReason: reason, timestamp: Date.now(), ...(reason === 'error' ? { errorMessage: 'synthetic non-retryable error' } : {}) }
          stream.push(reason === 'stop' ? { type: 'done', reason, message } : { type: 'error', reason, error: message })
        })
        return stream
      }
      if (mode === 'native-wake') {
        send(true)
        await session.prompt('herdr-threads: wake up')
      } else {
        const prompted = session.prompt('isolated boundary test')
        await (late ? bounded(prompted) : prompted)
      }
      const successful = ['turn', 'settle', 'native-wake', 'reload'].includes(mode)
      assert.equal(calls, ['manual-partial', 'manual-transformed'].includes(mode) ? 3 : ['turn', 'settle', 'transformed', 'transformed-p0', 'reload', 'manual-late-confirm'].includes(mode) ? 2 : 1)
      assert.equal(starts, 1, 'native continuation stays within original outer run')
      const acks = commands.filter(argv => argv.includes('ack'))
      assert.equal(acks.length, successful ? 1 : 0)
      if (successful) {
        assert.deepEqual(acks[0].slice(acks[0].indexOf('context') + 1), ['first', 'second', 'third'])
        const received = providerContexts.at(-1)
        assert.ok(received.indexOf('exact first') < received.indexOf('exact second'))
        assert.ok(received.indexOf('exact second') < received.indexOf('exact third'))
      }
      assert.deepEqual(errors, [])
      if (mode.startsWith('manual-')) {
        assert.equal(commands.filter(argv => argv.includes('ack')).length, 0)
        assert.equal(receipts.length, ['manual-partial', 'manual-late-confirm'].includes(mode) ? 1 : 0)
        if (mode === 'manual-partial') {
          assert.deepEqual(receipts, ['opaque SDK receipt 18446744073709551615'])
          assert.ok(providerContexts.at(-1).includes('exact first notice'))
          assert.ok(providerContexts.at(-1).includes('exact second notice'))
        }
        if (mode === 'manual-reload') {
          const stored = () => session.sessionManager.getBranch().filter(entry => entry.type === 'custom_message' && String(entry.content).includes('exact first notice'))
          assert.equal(stored().length, 1, 'native cancellation left accepted but unseen notice')
          reloaded = true; await session.reload(); send()
          assert.equal(receipts.length, 0, 'fresh replay alone is not context evidence')
          await session.prompt('after canonical replay')
          assert.equal(stored().length, 1, 'native reload reused exact opaque-key acceptance hint')
          assert.deepEqual(receipts, ['opaque SDK receipt 18446744073709551615'])
          assert.equal(calls, 2)
        }
      }
      if (late) {
        const frames = text => session.sessionManager.getBranch().filter(e => e.type === 'custom_message' && String(e.content).includes(text))
        const observed = entry => late.contexts.some(messages => messages.some(m => m.customType === entry.customType && m.content === entry.content))
        assert.equal(settled, 1, 'actual first native run settled while its confirmation remained held')
        assert.equal(frames('exact first notice').length, 1); assert.ok(observed(frames('exact first notice')[0]))
        assert.equal(spawned, 1); assert.equal(stopped, 0); assert.equal(frames('exact second notice').length, 0)
        late.order.push('confirm-released')
        late.confirm.resolve({ code: 0, stdout: '{"status":"confirmed","emitted_page_confirmed":true}' })
        await bounded(late.stopRequested.promise)
        assert.equal(spawned, 1, 'successor cannot start before its owned predecessor stop joins')
        late.stop.resolve(); await bounded(late.successor.promise)
        assert.deepEqual(late.order, ['settled', 'confirm-released', 'stop-requested', 'stop-completed', 'successor-page'])
        assert.equal(starts, 1); assert.equal(calls, 2); assert.equal(settled, 1, 'late page arrival never starts another run')
        assert.equal(receipts.length, 1); assert.equal(frames('exact second notice').length, 0)
        // Explicit host-input stimulus only: this proves extension uptake in the
        // next real SDK turn, NOT automatic scheduler/native-host wake liveness.
        await bounded(session.prompt('herdr-threads: wake up'))
        await bounded(late.drained.promise)
        assert.equal(starts, 2); assert.equal(calls, 3); assert.equal(settled, 2)
        assert.equal(frames('exact first notice').length, 1); assert.equal(frames('exact second notice').length, 1)
        assert.ok(observed(frames('exact second notice')[0]))
        assert.deepEqual(receipts, ['opaque SDK receipt 18446744073709551615', 'opaque SDK successor receipt 18446744073709551615'])
        assert.equal(commands.filter(argv => argv.includes('ack')).length, 0)
        assert.deepEqual(errors, [])
      }
      if (mode === 'reload') {
        await session.reload()
        assert.equal(stopped, 1, 'native reload closes its predecessor watch')
        assert.equal(commands.filter(argv => argv.at(-1) === 'SessionStart').length, 2)
        await session.prompt('after native reload')
        assert.equal(calls, 3); assert.equal(starts, 2)
        assert.equal(commands.filter(argv => argv.includes('ack')).length, 1)
      }
      if (mode === 'reload-urgent-unobserved') {
        const storedFrames = () => session.sessionManager.getBranch().filter(entry => entry.type === 'custom_message' && String(entry.content).includes('exact first'))
        assert.equal(storedFrames().length, 1, 'cancellation stored a frame without another model request')
        reloaded = true; await session.reload()
        assert.equal(stopped, 1); assert.equal(commands.filter(argv => argv.includes('ack')).length, 0)
        send(true); send(true)
        await session.prompt('herdr-threads: wake up')
        assert.equal(calls, 2); assert.equal(starts, 2, 'canonical replay starts exactly one successor run')
        assert.equal(storedFrames().length, 1, 'recovery does not duplicate native body drafts')
        const after = commands.filter(argv => argv.includes('ack'))
        assert.equal(after.length, 1)
        assert.deepEqual(after[0].slice(after[0].indexOf('context') + 1), ['first', 'second', 'third'])
      }
    } finally {
      late?.confirm.resolve({ code: 1, stdout: '' }); late?.stop.resolve()
      await session.extensionRunner.emit({ type: 'session_shutdown', reason: 'quit' })
      await session.dispose()
    }
    assert.equal(stopped, ['manual-reload', 'manual-late-confirm'].includes(mode) ? 3 : mode === 'manual-partial' || mode.startsWith('reload') ? 2 : 1, 'native shutdown reaps its watch')
  })
})
