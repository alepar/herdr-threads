import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { register } from '../herdr-threads.js'

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

  for (const mode of ['turn', 'settle', 'error', 'aborted', 'cancel-at-settle', 'transformed', 'transformed-p0', 'self-wake', 'reload', 'reload-urgent-unobserved']) await t.test(mode, async () => {
    let session, watch, calls = 0, starts = 0, injected = false, nonce = 0, stopped = 0, reloaded = false
    const commands = [], providerContexts = [], errors = []
    const privateRoot = path.join(root, mode); fs.mkdirSync(privateRoot, { recursive: true })
    const send = (waking = false) => {
      injected = true
      for (const [id, priority] of [['third', 3], ['first', 0], ['second', 1]]) watch.line({ schema: 1, kind: 'message', id, priority, body: `exact ${id}`, body_len: 11, thread: 'thread', sender: 'sender', ack_required: true, wakes: waking && priority === 0, truncated: false })
    }
    const io = {
      env: {}, registry: {}, pid: process.pid, cwd: privateRoot, nonce: () => `${mode}-${++nonce}`,
      run: async argv => {
        commands.push(argv)
        if (argv.includes('ack')) return { code: 0, stdout: argv.slice(argv.indexOf('context') + 1).map(id => JSON.stringify({ id, result: 'settled' })).join('\n') }
        return { code: 0, stdout: argv.at(-1) === 'SessionStart' ? 'isolated SDK guidance' : '' }
      },
      spawn: (_argv, _options, handlers) => {
        watch = handlers
        queueMicrotask(() => handlers.line({ schema: 1, kind: 'status', id: 'status:0', state: 'connected' }))
        return { stop: async () => { stopped++ } }
      },
      schedule: (ms, fn) => { const timer = setTimeout(fn, ms); timer.unref(); return () => clearTimeout(timer) },
      diagnostic: text => errors.push(text),
    }
    const loader = new DefaultResourceLoader({ cwd: privateRoot, agentDir: process.env.PI_CODING_AGENT_DIR, settingsManager: SettingsManager.inMemory({}), noContextFiles: true, noSkills: true, noThemes: true, noPromptTemplates: true,
      extensionFactories: [pi => {
        register(pi, io)
        pi.on('before_agent_start', () => { starts++ })
        pi.on('turn_end', e => {
          if (['settle', 'cancel-at-settle', 'reload-urgent-unobserved'].includes(mode)) { if (!injected) send(mode === 'reload-urgent-unobserved') }
          if (mode.startsWith('transformed')) return { entries: e.entries.map(entry => entry.type === 'custom_message' && String(entry.content).includes('exact first') ? { ...entry, content: 'later extension replaced frame' } : entry) }
        })
        if (mode === 'cancel-at-settle') pi.on('agent_before_settle', async (_e, ctx) => { ctx.abort() })
        if (mode === 'reload-urgent-unobserved') pi.on('agent_before_settle', async (_e, ctx) => { if (!reloaded) ctx.abort() })
      }] })
    await loader.reload()
    ;({ session } = await createAgentSession({ cwd: privateRoot, agentDir: process.env.PI_CODING_AGENT_DIR, settingsManager: SettingsManager.inMemory({}), resourceLoader: loader, sessionManager: SessionManager.create(privateRoot, path.join(privateRoot, 'sessions')), tools: [],
      model: { id: 'selected-model', name: 'Selected', api: 'openai-completions', provider: 'selected-provider', baseUrl: 'http://invalid.local', reasoning: false, input: ['text'], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 10000, maxTokens: 1000 } }))
    try {
      session.modelRuntime.hasConfiguredAuth = () => true
      await session.bindExtensions({ mode: 'tui', onError: error => errors.push(error.error) })
      session.agent.streamFunction = (model, context) => {
        calls++; providerContexts.push(JSON.stringify(context.messages))
        assert.equal(model.id, 'selected-model'); assert.equal(model.provider, 'selected-provider')
        if (!injected && !['settle', 'cancel-at-settle', 'reload-urgent-unobserved'].includes(mode)) send(mode === 'transformed-p0')
        const stream = createAssistantMessageEventStream()
        queueMicrotask(() => {
          const reason = mode === 'error' ? 'error' : mode === 'aborted' ? 'aborted' : 'stop'
          const message = { role: 'assistant', content: [{ type: 'text', text: 'scripted' }], api: model.api, provider: model.provider, model: model.id, usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } }, stopReason: reason, timestamp: Date.now(), ...(reason === 'error' ? { errorMessage: 'synthetic non-retryable error' } : {}) }
          stream.push(reason === 'stop' ? { type: 'done', reason, message } : { type: 'error', reason, error: message })
        })
        return stream
      }
      if (mode === 'self-wake') {
        let timer
        const settled = new Promise((resolve, reject) => {
          session.subscribe(event => { if (event.type === 'agent_settled') { clearTimeout(timer); resolve() } })
          timer = setTimeout(() => reject(new Error('self-wake did not settle')), 3000)
        })
        send(true)
        await settled
      } else await session.prompt('isolated boundary test')
      const successful = ['turn', 'settle', 'self-wake', 'reload'].includes(mode)
      assert.equal(calls, ['turn', 'settle', 'transformed', 'transformed-p0', 'reload'].includes(mode) ? 2 : 1)
      assert.equal(starts, 1, 'native continuation stays within original outer run')
      const acks = commands.filter(argv => argv.includes('ack'))
      assert.equal(acks.length, successful ? 1 : 0)
      if (successful) {
        assert.deepEqual(acks[0].slice(acks[0].indexOf('context') + 1), ['first', 'second', 'third'])
        const received = providerContexts.at(-1)
        assert.ok(received.indexOf('exact first') < received.indexOf('exact second'))
        assert.ok(received.indexOf('exact second') < received.indexOf('exact third'))
      }
      assert.deepEqual(errors, ['transformed-p0', 'reload-urgent-unobserved'].includes(mode) ? ['herdr-threads: pi delivery deferred'] : [])
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
        let timer
        const settled = new Promise((resolve, reject) => {
          session.subscribe(event => { if (event.type === 'agent_settled') { clearTimeout(timer); resolve() } })
          timer = setTimeout(() => reject(new Error('replayed urgent frame did not wake the reloaded owner')), 3000)
        })
        send(true); send(true)
        await settled
        assert.equal(calls, 2); assert.equal(starts, 2, 'canonical replay starts exactly one successor run')
        assert.equal(storedFrames().length, 1, 'recovery does not duplicate native body drafts')
        const after = commands.filter(argv => argv.includes('ack'))
        assert.equal(after.length, 1)
        assert.deepEqual(after[0].slice(after[0].indexOf('context') + 1), ['first', 'second', 'third'])
      }
    } finally {
      await session.extensionRunner.emit({ type: 'session_shutdown', reason: 'quit' })
      await session.dispose()
    }
    assert.equal(stopped, mode.startsWith('reload') ? 2 : 1, 'native shutdown reaps its watch')
  })
})
