import fs from 'node:fs'
import { pathToFileURL } from 'node:url'
import path from 'node:path'

const root = process.env.HT_PI_QUALIFY_ROOT
const modules = process.env.HT_PI_QUALIFY_MODULES
const { createAssistantMessageEventStream } = await import(pathToFileURL(path.join(modules, '@earendil-works/pi-ai/dist/index.js')))
const { matchesKey } = await import(pathToFileURL(path.join(modules, '@earendil-works/pi-tui/dist/index.js')))
export const log = data => fs.appendFileSync(path.join(root, 'native.jsonl'), JSON.stringify({ at: Date.now(), ...data }) + '\n')
const hold = async (file, signal) => { while (!signal?.aborted && !fs.existsSync(path.join(root, file))) await new Promise(resolve => setTimeout(resolve, 100)) }
export default function fixture(pi) {
  log({ event: 'fixture_loaded', pid: process.pid, named: process.env.HERDR_THREADS_SEAT_NAME, tmux: process.env.TMUX, tmuxPane: process.env.TMUX_PANE })
  const snapshot = ctx => ({ sid: ctx.sessionManager.getSessionId(), idle: ctx.isIdle(), editor: ctx.ui.getEditorText(), model: ctx.model?.id, provider: ctx.model?.provider })
  for (const event of ['session_start', 'session_tree', 'session_compact', 'session_compact_failed', 'before_agent_start', 'context', 'agent_start', 'turn_end', 'agent_before_settle', 'agent_settled', 'ui_prompt_start', 'ui_prompt_end', 'model_select', 'tool_execution_start', 'tool_execution_end', 'session_shutdown', 'before_provider_request', 'after_provider_response']) {
    pi.on(event, (data, ctx) => {
      log({ event, state: snapshot(ctx), data: event === 'after_provider_response' ? { status: data.status } : event === 'before_provider_request' ? { model: ctx.model?.id, provider: ctx.model?.provider } : data, branch: ctx.sessionManager.getBranch().filter(e => e.type === 'custom_message') })
    })
  }
  let timer
  pi.on('session_start', (_event, ctx) => {
    clearInterval(timer)
    const reloadFile = path.join(root, 'reload-hold.json')
    if (_event.reason === 'reload' && fs.existsSync(reloadFile)) {
      const request = JSON.parse(fs.readFileSync(reloadFile, 'utf8'))
      if (request.sid === snapshot(ctx).sid && typeof request.editor === 'string') {
        ctx.ui.setEditorText(request.editor)
        log({ event: 'reload_draft_held', state: snapshot(ctx), nonce: request.nonce })
      }
    }
    let previous = '', previousObservation, preparedNonce
    timer = setInterval(() => {
      try {
        const prepare = path.join(root, 'prepare-command.json')
        const command = fs.existsSync(prepare) ? JSON.parse(fs.readFileSync(prepare, 'utf8')) : null
        if (command?.sid === snapshot(ctx).sid && typeof command.nonce === 'string' && command.nonce !== preparedNonce) {
          preparedNonce = command.nonce
          const accepted = typeof command.expected === 'string' && typeof command.command === 'string'
            && /^\/probe-[a-z-]+(?: |$)/.test(command.command) && ctx.ui.getEditorText() === command.expected
          if (accepted) ctx.ui.setEditorText(command.command)
          log({ event: 'command_prepared', nonce: command.nonce, accepted, state: snapshot(ctx) })
        }
        const state = snapshot(ctx), encoded = JSON.stringify(state)
        const file = path.join(root, 'observe-state.json')
        const request = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : null
        const observation = request?.sid === state.sid && typeof request.nonce === 'string' ? request.nonce : undefined
        if (encoded !== previous || observation !== previousObservation) {
          previous = encoded; previousObservation = observation
          log({ event: 'state', state, observation })
        }
      } catch {}
    }, 100)
    timer.unref()
    ctx.ui.onTerminalInput(data => { log({ event: 'terminal_input', data }); return undefined })
  })
  pi.on('session_shutdown', () => clearInterval(timer))
  const command = async (pi, args) => {
    const [program, ...prefix] = JSON.parse(process.env.HT_PI_QUALIFY_CLI)
    const result = await pi.exec(program, [...prefix, ...args], { timeout: 15000 })
    log({ event: 'command', args, code: result.code, stdout: result.stdout, stderr: result.stderr })
    if (result.code !== 0) throw new Error('qualification command refused')
    return result
  }
  pi.registerCommand('probe-notice-cause', { description: 'private genuine agent receipt cause', handler: async (value, ctx) => {
    const data = JSON.parse(value)
    if (data.sid !== snapshot(ctx).sid || typeof data.target !== 'string' || typeof data.body !== 'string'
      || data.deadline !== 8 || typeof data.hold !== 'string') throw new Error('invalid owned notice cause')
    ctx.ui.setEditorText(data.hold)
    const result = await command(pi, ['send', '--seat', data.target, '--body', data.body, '--require-ack', data.target, '--deadline', String(data.deadline)])
    log({ event: 'notice_cause', state: snapshot(ctx), body: data.body, target: data.target, result: JSON.parse(result.stdout) })
  } })
  pi.registerCommand('probe-notice-invite', { description: 'private genuine agent invitation', handler: async (value, ctx) => {
    const data = JSON.parse(value)
    if (data.sid !== snapshot(ctx).sid || typeof data.target !== 'string' || typeof data.hold !== 'string') throw new Error('invalid owned invitation')
    ctx.ui.setEditorText(data.hold)
    const created = JSON.parse((await command(pi, ['thread', 'create', '--topic', 'Owned Pi notice page', '--goal', 'Qualification only'])).stdout)
    const thread = created.result.data
    const invited = JSON.parse((await command(pi, ['invite', thread, '--seat', data.target])).stdout)
    log({ event: 'notice_invitation', state: snapshot(ctx), target: data.target, thread, result: invited })
  } })
  pi.registerCommand('probe-notice-reload', { description: 'private public reload with owned draft guard', handler: async (_value, ctx) => { await ctx.reload() } })
  pi.registerCommand('probe-read-ack', { description: 'private full-body explicit receipt cleanup', handler: async value => {
    const id = value.trim()
    const read = await command(pi, ['body', id])
    if (!read.stdout.includes('TRUNCATED-END')) throw new Error('full synthetic body not read')
    await command(pi, ['ack', id])
  } })
  pi.registerCommand('probe-accept', { description: 'private owned invitation acceptance', handler: async value => { await command(pi, ['accept', value.trim()]) } })
  pi.registerCommand('probe-inbox', { description: 'private explicit inbox fallback', handler: async () => {
    const [program, ...prefix] = JSON.parse(process.env.HT_PI_QUALIFY_CLI)
    const args = prefix.filter(value => value !== '--json')
    const result = await pi.exec(program, [...args, 'inbox'], { timeout: 15000, maxBuffer: 65536 })
    log({ event: 'explicit_inbox', code: result.code, stdout: result.stdout })
    if (result.code !== 0) throw new Error('explicit inbox refused')
  } })
  pi.registerCommand('probe-handoff', { description: 'private top-level public handoff', handler: async () => {
    const [program, ...prefix] = JSON.parse(process.env.HT_PI_QUALIFY_CLI)
    const result = await pi.exec('env', ['HERDR_THREADS_PI_OPTS=' + process.env.HT_PI_QUALIFY_OPTS, program, ...prefix,
      'handoff', '--new-tab', 'pi-handoff', '--kind', 'pi', '--profile-dir', process.env.PI_CODING_AGENT_DIR,
      '--summary', 'Pi qualification handoff', '--', 'HANDOFF EXACT SYNTHETIC BODY'], { timeout: 30000, maxBuffer: 65536 })
    log({ event: 'handoff_result', code: result.code, stdout: result.stdout, stderr: result.stderr })
    if (result.code !== 0) throw new Error('native public handoff refused')
  } })
  pi.on('session_start', (_event, ctx) => {
    const file = ctx.sessionManager.getSessionFile()
    if (file && !fs.existsSync(path.join(root, 'first-session'))) fs.writeFileSync(path.join(root, 'first-session'), file)
  })
  pi.registerCommand('probe-resume', { description: 'private native resume qualification', handler: async (_args, ctx) => { await ctx.switchSession(fs.readFileSync(path.join(root, 'first-session'), 'utf8')); log({ event: 'resume_done' }) } })
  pi.registerCommand('probe-new', { description: 'private native session replacement', handler: async (_args, ctx) => { await ctx.newSession(); log({ event: 'new_done' }) } })
  pi.registerCommand('probe-compact', { description: 'private native compaction qualification', handler: async (_args, ctx) => {
    ctx.compact({ customInstructions: 'Summarize the synthetic qualification conversation briefly.',
      onComplete: () => log({ event: 'compact_done' }), onError: error => log({ event: 'compact_error', message: error.message }) })
  } })
  pi.registerCommand('probe-threshold', { description: 'private threshold qualification', handler: async (value) => {
    if (!/^-?[0-9]+$/.test(value.trim())) throw new Error('invalid qualification threshold')
    const [program, ...prefix] = JSON.parse(process.env.HT_PI_QUALIFY_CLI)
    const result = await pi.exec(program, [...prefix, 'nudge-threshold', value.trim()], { timeout: 10000 })
    log({ event: 'threshold', code: result.code, stdout: result.stdout, stderr: result.stderr })
    if (result.code !== 0) throw new Error('qualification threshold command refused')
  } })
  pi.on('turn_end', async (event, ctx) => {
    // Loaded after production: its completed-turn handler has already seen
    // the empty queue before the controller sends the cancellation target.
    if (fs.existsSync(path.join(root, 'settle-barrier')) && event.outcome === 'completed' && event.continue === false) {
      log({ event: 'settle_barrier', entries: event.entries, outcome: event.outcome, continue: event.continue })
      await hold('release-settle', ctx.signal)
      log({ event: 'settle_barrier_released' })
    }
    const transform = path.join(root, 'transform')
    if (fs.existsSync(transform)) {
      const target = JSON.parse(fs.readFileSync(transform, 'utf8'))
      const result = { entries: event.entries.map(entry => entry.type === 'custom_message' && entry.customType.startsWith('herdr-threads:') ? { ...entry, content: 'qualification replaced draft' } : entry) }
      log({ event: 'transform_applied', state: snapshot(ctx), target, before: event.entries, after: result.entries })
      return result
    }
  })
  pi.on('agent_before_settle', (event, ctx) => {
    if (fs.existsSync(path.join(root, 'cancel-settle'))) {
      log({ event: 'settle_abort', entries: event.entries, outcome: event.outcome, continue: event.continue })
      ctx.abort()
    }
  })
  pi.registerCommand('probe-fork', { description: 'private native fork qualification', handler: async (_args, ctx) => {
    const entry = ctx.sessionManager.getBranch().find(e => e.type === 'message' && e.message.role === 'user')
    if (!entry) throw new Error('no user entry to fork')
    await ctx.fork(entry.id, { position: 'at' })
    log({ event: 'fork_done' })
  } })
  pi.registerCommand('probe-tree', { description: 'private native branch qualification', handler: async (_args, ctx) => {
    const entry = ctx.sessionManager.getBranch().find(e => e.type === 'message' && e.message.role === 'user')
    if (!entry) throw new Error('no user entry to select')
    await ctx.navigateTree(entry.id)
    log({ event: 'tree_done' })
  } })
  pi.registerCommand('probe-select', { description: 'private inline selector probe', handler: async (_args, ctx) => { await ctx.ui.select('PROBE INLINE SELECT', ['First', 'Second']) } })
  pi.registerCommand('probe-overlay', { description: 'private overlay probe', handler: async (_args, ctx) => {
    await ctx.ui.custom((_tui, _theme, _bindings, done) => ({ render: () => ['PROBE CUSTOM OVERLAY', 'Escape closes this owned overlay'], invalidate() {}, handleInput(data) { if (matchesKey(data, 'escape')) done(undefined) } }), { overlay: true, blocked: { kind: 'question', message: 'PROBE CUSTOM OVERLAY' } })
  } })
  pi.registerTool({ name: 'probe_hold', label: 'Probe held tool', description: 'Private deterministic held tool', parameters: { type: 'object', properties: {}, required: [] },
    async execute(_id, _params, signal) { log({ event: 'tool_held' }); await hold('release-tool', signal); return { content: [{ type: 'text', text: 'PROBE TOOL RELEASED' }], details: {} } } })
  if (process.env.HT_PI_QUALIFY_LIVE !== '1') pi.registerProvider('ht-fixture', {
    baseUrl: 'http://127.0.0.1:1', apiKey: 'fixture-not-a-secret', api: 'ht-scripted',
    models: [{ id: 'scripted', name: 'Scripted probe', api: 'ht-scripted', reasoning: false, input: ['text'], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 100000, maxTokens: 1000 }],
    streamSimple(model, context, options) {
      const stream = createAssistantMessageEventStream()
      queueMicrotask(async () => {
        if (model.provider !== 'ht-fixture' || model.id !== 'scripted') throw new Error('fixture model mismatch')
        let payload = { messages: context.messages }
        payload = await options?.onPayload?.(payload, model) ?? payload
        await options?.onResponse?.({ status: 200, headers: {} }, model)
        log({ event: 'provider_request', model: model.id, provider: model.provider, payload })
        const planPath = path.join(root, 'mode.txt')
        const mode = fs.existsSync(planPath) ? fs.readFileSync(planPath, 'utf8').trim() : 'normal'
        fs.writeFileSync(planPath, 'normal')
        const message = { role: 'assistant', content: [{ type: 'text', text: 'PROBE COMPLETED' }], api: model.api, provider: model.provider, model: model.id,
          usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } }, stopReason: options?.signal?.aborted ? 'aborted' : 'stop', timestamp: Date.now() }
        if (mode === 'hold-text' || mode === 'hold-error') {
          message.content[0].text = ''
          stream.push({ type: 'start', partial: message }); stream.push({ type: 'text_start', contentIndex: 0, partial: message })
          message.content[0].text = 'PROBE STREAM HOLD'
          stream.push({ type: 'text_delta', contentIndex: 0, delta: 'PROBE STREAM HOLD', partial: message })
          log({ event: 'stream_held' }); await hold('release-text', options?.signal)
          stream.push({ type: 'text_end', contentIndex: 0, content: message.content[0].text, partial: message })
        }
        if (mode === 'error' || mode === 'hold-error') { message.stopReason = 'error'; message.errorMessage = 'synthetic nonretryable qualification failure' }
        if (mode === 'tool') { message.content = [{ type: 'toolCall', id: 'probe-held-tool', name: 'probe_hold', arguments: {} }]; message.stopReason = 'toolUse' }
        if (options?.signal?.aborted) stream.push({ type: 'error', reason: 'aborted', error: { ...message, stopReason: 'aborted' } })
        else if (mode === 'error' || mode === 'hold-error') stream.push({ type: 'error', reason: 'error', error: message })
        else if (mode === 'tool') stream.push({ type: 'done', reason: 'toolUse', message })
        else stream.push({ type: 'done', reason: 'stop', message })
      })
      return stream
    },
  })
}
