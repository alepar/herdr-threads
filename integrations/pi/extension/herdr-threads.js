import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'

// Setup replaces this one marked constant, and owns this entire extension directory.
const LAUNCH = null // herdr-threads:launch
const OWNER = Symbol.for('herdr-threads.pi.owner')
const RECOVERY = Symbol.for('herdr-threads.pi.recovery')
const PREFIX = 'HERDR_THREADS_PI_'
const WAKE = 'herdr-threads: wake up'
const HEADER = '[herdr-threads] new messages:'
const MAX_RECORDS = 256
const MAX_BYTES = 2 * 1024 * 1024
// Keep room for a new lifecycle guidance result even when all retained mail is
// already proposed and cannot be evicted.
const RETAINED_BYTES = MAX_BYTES - 4096
const PAGE_BYTES = 64 * 1024
const BACKOFF = [1000, 2000, 5000, 10000, 30000]
const size = text => Buffer.byteLength(text, 'utf8')
const oneLine = text => String(text).replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ')
const indent = text => String(text).split(/\r\n|[\n\r\u000b\u000c\u0085\u2028\u2029]/).map(s => s ? `  ${s}` : '').join('\n')
const label = (id, name) => !name || name === id ? oneLine(id || 'unknown')
  : id ? `${oneLine(id)} ${JSON.stringify(oneLine(name))}` : JSON.stringify(oneLine(name))

// Mirrors src/delivery/frame.rs. Watch CLI negotiates presentation leases and
// confirms nonordinary output after stdout flush, independently of mail ACK.
export function frame(item) {
  if (item.kind === 'attention') return `[herdr-threads] attention ${oneLine(item.id)}:\n${indent(item.text)}`
  let markers = ''
  if (Number.isInteger(item.priority) && item.priority >= 0 && item.priority <= 9 && item.priority !== 1) markers += ` [p${item.priority}]`
  if (item.author_role === 'human') markers += ' [human]'
  if (item.relays_user === true) markers += ' [relays user]'
  if (['query', 'request', 'rule'].includes(item.user_intent)) markers += ` [${item.user_intent}]`
  return `[herdr-threads] ${item.kind === 'lazy' ? 'message' : oneLine(item.kind)} ${oneLine(item.id)} in ${label(item.thread, item.thread_name)} from ${item.sender_label ? oneLine(item.sender_label) : label(item.sender, item.sender_name)}${markers}:\n${indent(item.body)}`
}

const compare = (a, b) => a.priority - b.priority || a.deadline - b.deadline || a.order - b.order

/** Register native callbacks. IO isolates process/clock operations, never pi persistence. */
export function register(pi, io) {
  const inherited = Object.fromEntries(['SESSION', 'ROLE', 'MODE', 'PID'].map(key => [key, io.env[PREFIX + key]]))
  const forbidden = (inherited.PID !== undefined && inherited.PID !== String(io.pid))
    || (inherited.ROLE !== undefined && inherited.ROLE !== 'top_level')
    || (inherited.MODE !== undefined && inherited.MODE !== 'tui')
    || (inherited.SESSION !== undefined && inherited.PID !== String(io.pid))
  const token = {}
  let owner = null
  let sequence = 0
  let lifecycle = 0
  const cmd = (...args) => [...(io.argv ?? ['herdr-threads']), ...args]
  const diagnostic = reason => { try { io.diagnostic(`herdr-threads: pi ${reason}`) } catch {} }
  const current = o => owner === o && !o.closed && io.registry[OWNER] === token
    && o.ctx.mode === 'tui' && o.ctx.sessionManager.getSessionId() === o.sid
  const callbackOwner = ctx => owner && current(owner) && ctx?.mode === 'tui'
    && ctx.sessionManager?.getSessionId() === owner.sid ? owner : null
  function branch(o) {
    try { const entries = o.ctx.sessionManager.getBranch(); if (Array.isArray(entries)) { o.branchMissing = false; return entries } } catch {}
    if (!o.branchMissing) diagnostic('branch API unavailable')
    o.branchMissing = true
    return null
  }
  const exact = (entry, p) => entry.type === 'custom_message' && entry.customType === p.customType && entry.content === p.text
  const claims = o => ({ event: '', session_id: o.sid, role: 'top_level', mode: 'tui', pid: io.pid, cwd: o.ctx.cwd ?? io.cwd })
  const childEnv = o => ({ ...io.env, ...o.env })
  function run(o, argv, options) {
    const task = Promise.resolve(io.run(argv, options))
    o.calls.add(task)
    task.then(() => o.calls.delete(task), () => o.calls.delete(task))
    return task
  }
  function later(o, ms, fn) {
    let cancel
    cancel = io.schedule(ms, () => { o.timers.delete(cancel); if (current(o)) fn() })
    o.timers.add(cancel)
    return cancel
  }
  async function hook(o, event, source) {
    try {
      const result = await run(o, cmd('hook-event', 'pi', event), {
        stdin: JSON.stringify({ ...claims(o), event, ...(source ? { source } : {}) }),
        env: childEnv(o), timeoutMs: 8000, maxBytes: 4096,
        signal: event === 'SessionEnd' ? undefined : o.abort.signal,
      })
      if (result.code !== 0 || typeof result.stdout !== 'string' || size(result.stdout) > 4096
          || result.stdout.includes('herdr-threads: check-in unavailable (')) throw new Error('unavailable')
      return result.stdout
    } catch { if (current(o)) diagnostic('check-in unavailable'); return null }
  }
  async function admit(o) {
    if (!current(o)) return false
    if (o.bound) return true
    if (o.admission) return o.admission
    o.admission = (async () => {
      const guidance = await hook(o, 'SessionStart', o.source)
      if (!current(o) || guidance === null) return false
      o.bound = true; o.guidance = guidance
      startWatch(o)
      if (['startup', 'reload'].includes(o.source)) o.idle = o.ctx.isIdle() === true
      return true
    })()
    try { return await o.admission } finally { o.admission = null }
  }
  function count(o) { return o.pending.size + o.proposals.reduce((n, p) => n + Math.max(1, p.items.length), 0) }
  function bytes(o) {
    return [...o.pending.values()].reduce((n, r) => n + size(r.rendered), 0)
      + o.proposals.reduce((n, p) => n + size(p.text), size(o.guidance))
  }
  function overflow(o) { o.replayDebt = true }
  function trim(o) {
    while (count(o) > MAX_RECORDS || bytes(o) > RETAINED_BYTES) {
      const worst = [...o.pending.values()].sort(compare).at(-1)
      if (!worst) break
      o.pending.delete(worst.key); overflow(o)
    }
  }
  function retain(o, record) { o.pending.set(record.key, record); trim(o) }
  function ownStop(o, promise) {
    const task = Promise.resolve(promise)
    o.stops.add(task)
    task.then(() => o.stops.delete(task), () => o.stops.delete(task))
    return task
  }
  function stopWatch(o, child) {
    if (!child) return Promise.resolve()
    return ownStop(o, Promise.resolve().then(() => child.stop()).catch(() => diagnostic('watch cleanup failed')))
  }
  // A connected replacement pays debt only after actual capacity release. Merely
  // being full never creates a reconnect loop asking for the same excess bodies.
  function released(o) {
    if (o.replayDebt && !o.replaying && count(o) < MAX_RECORDS && current(o)) {
      o.replaying = true
      void ownStop(o, replaceWatch(o))
    }
  }
  async function replaceWatch(o) {
    o.live = false; ++o.watchRun
    const child = o.child; o.child = null
    await stopWatch(o, child)
    if (current(o)) startWatch(o)
  }
  function startWatch(o) {
    if (!current(o) || !o.bound || o.child) return
    const run = ++o.watchRun
    try {
      o.child = io.spawn(cmd('watch', '--harness', 'pi', '--session', o.sid), { env: childEnv(o), maxBytes: PAGE_BYTES }, {
        line: line => { if (current(o) && run === o.watchRun) onLine(o, line) },
        exit: code => {
          if (!current(o) || run !== o.watchRun) return
          o.child = null; o.live = false
          if (o.replaying) o.replayDebt = true
          diagnostic('watch unavailable')
          if (code === 3) { o.bound = false; o.replaying = false; return }
          later(o, BACKOFF[Math.min(o.backoff++, BACKOFF.length - 1)], () => startWatch(o))
        },
      })
    } catch {
      o.child = null; o.live = false; o.replayDebt ||= o.replaying
      diagnostic('watch unavailable')
      later(o, BACKOFF[Math.min(o.backoff++, BACKOFF.length - 1)], () => startWatch(o))
    }
  }
  function progress(o) {
    o.wakeCancel?.(); o.wakeCancel = null; o.waking = false; o.suspended = false
  }
  function resetAttentionHistory(o) {
    o.attentionKeys.clear(); o.attentionHistoryBlocked = false
  }
  function blockAttention(o) {
    if (o.intent && o.attentionKeys.size === MAX_RECORDS) o.attentionHistoryBlocked = true
    if (o.intent && !o.intent.blocked) { o.intent.blocked = true; diagnostic('delivery deferred') }
  }
  function wake(o) {
    // No await between final owner/idle/UI/intent checks and the content-free call.
    if (!current(o) || !o.bound || !o.live || !o.idle || o.active || o.compacting || o.prompts
        || o.branchMissing || o.waking || o.suspended || o.attentionHistoryBlocked || !o.intent || o.intent.blocked || o.ctx.isIdle() !== true) return
    try {
      if (typeof o.ctx.ui?.getEditorText !== 'function' || typeof pi.sendMessage !== 'function') {
        if (!o.wakeMissing) { o.wakeMissing = true; diagnostic('wake API unavailable') }
        return
      }
      if (o.ctx.ui.getEditorText() !== '') return
      o.waking = true
      o.wakeCancel = later(o, 30000, () => {
        o.waking = false; o.suspended = true
        if (o.attentionKeys.size === MAX_RECORDS) o.attentionHistoryBlocked = true
        diagnostic('wake unreceived')
      })
      pi.sendMessage({ customType: 'herdr-threads-wake', content: WAKE, display: false }, { triggerTurn: true })
    } catch { o.wakeCancel?.(); o.waking = false; o.suspended = true; diagnostic('wake unavailable') }
  }
  function onLine(o, line) {
    if (!line || line.schema !== 1 || typeof line.id !== 'string' || size(line.id) > 128) return
    if (line.kind === 'status') {
      if (line.state === 'connected') {
        if (!o.live) {
          resetAttentionHistory(o); o.suspended = false
          if (o.intent) o.intent.blocked = false
        }
        o.live = true
        if (o.replaying) { o.replaying = false; o.replayDebt = false }
        retryAcks(o); wake(o)
      } else if (['closing', 'refused'].includes(line.state)) {
        o.live = false
        if (['no_binding', 'session_mismatch', 'binding_changed'].includes(line.reason)) o.bound = false
      }
      return
    }
    if (line.kind === 'attention_cleared') {
      for (const [key, record] of o.pending) if (record.kind === 'attention'
          && (line.presentation_epoch === undefined || record.epoch === line.presentation_epoch)) o.pending.delete(key)
      if (o.intent?.kind === 'attention' && (line.presentation_epoch === undefined || o.intent.epoch === line.presentation_epoch)) o.intent = null
      released(o); return
    }
    if (!['message', 'lazy', 'invitation', 'attention', 'notice'].includes(line.kind)) return
    const body = line.kind === 'attention' ? line.text : line.body
    if (typeof body !== 'string' || size(body) > PAGE_BYTES) { overflow(o); return }
    const key = JSON.stringify([line.kind, line.id, line.kind === 'invitation' ? [line.topic_revision ?? null, line.command_context_epoch ?? null] : line.kind === 'attention' ? line.presentation_epoch ?? line.attention_version : null])
    if (o.done.has(key)) return
    const waking = line.wakes === true || line.kind === 'attention'
    const existing = o.pending.get(key) ?? o.proposals.flatMap(p => p.items).find(r => r.key === key)
    // Reoffers of any known urgent ID belong to the same attention generation,
    // not just reoffers of its last key. Retained records keep this witness even
    // when the bounded cache of discarded waking keys is full. At that bound,
    // unknown keys cannot release a rejection/timeout fence by evicting history.
    if (waking && !existing?.waking && !o.attentionKeys.has(key) && o.intent?.key !== key) {
      if (o.attentionKeys.size === MAX_RECORDS && (o.suspended || o.intent?.blocked)) o.attentionHistoryBlocked = true
      if (!o.attentionHistoryBlocked) {
        if (o.attentionKeys.size < MAX_RECORDS) o.attentionKeys.add(key)
        o.intent = { key, kind: line.kind, epoch: line.presentation_epoch }
        o.suspended = false
      }
    }
    if (existing) { existing.waking ||= waking; wake(o); return }
    // Coalesced invitations are presentation, never automatic acceptance or mail ACK.
    if (line.kind === 'invitation') for (const [oldKey, r] of o.pending) if (r.kind === 'invitation' && r.id === line.id) o.pending.delete(oldKey)
    retain(o, {
      key, id: line.id, kind: line.kind, epoch: line.presentation_epoch,
      waking,
      priority: Number.isInteger(line.priority) && line.priority >= 0 && line.priority <= 9 ? line.priority : line.kind === 'lazy' ? 1 : 0,
      deadline: Number.isFinite(line.deadline_at) ? line.deadline_at : Infinity,
      order: sequence++, truncated: line.truncated === true, rendered: frame(line),
    })
    wake(o)
  }
  function remember(o, key) { o.done.add(key); while (o.done.size > MAX_RECORDS) o.done.delete(o.done.values().next().value) }
  function reconcile(o, entries = branch(o)) {
    if (!entries) return false
    for (const p of [...o.proposals]) {
      if (entries.some(e => exact(e, p))) continue
      o.proposals.splice(o.proposals.indexOf(p), 1)
      if (p.items.some(r => r.key === o.intent?.key)) blockAttention(o)
      if (p.guidance) o.guidance ||= p.text.slice(0, p.guidance)
      for (const r of p.items) { const [start, end] = p.frames.get(r.key); retain(o, { ...r, rendered: p.text.slice(start, end) }) }
    }
    return true
  }
  function proposal(o) {
    const items = [], frames = new Map(), guidance = o.guidanceAttempted ? '' : o.guidance
    const render = () => [guidance, items.length ? HEADER + '\n\n' + items.map(r => frames.get(r.key)).join('\n\n') : ''].filter(Boolean).join('\n\n')
    for (const r of [...o.pending.values()].sort(compare)) {
      if (r.attemptedRun === o.run) continue
      if (items.length === 32) break
      items.push(r); frames.set(r.key, r.rendered)
      if (size(render()) > PAGE_BYTES) { items.pop(); frames.delete(r.key); break }
    }
    if (!items.length && !guidance) return null
    const text = render()
    // Offsets share the finalized frame string. Rejection recovery slices it only
    // after releasing the proposal, so bodies never occupy two retained copies.
    let offset = (guidance ? guidance.length + (items.length ? 2 : 0) : 0) + (items.length ? HEADER.length + 2 : 0)
    for (const r of items) { frames.set(r.key, [offset, offset + r.rendered.length]); offset += r.rendered.length + 2 }
    const extra = size(text) - size(guidance) - items.reduce((n, r) => n + size(r.rendered), 0)
    // Reserve the framing overhead before moving records; proposed pages cannot
    // be displaced. If they own the whole budget, wait for context/ACK release.
    const selected = new Set(items.map(r => r.key))
    for (const r of [...o.pending.values()].sort(compare).reverse()) {
      if (bytes(o) + extra <= RETAINED_BYTES) break
      if (!selected.has(r.key)) { o.pending.delete(r.key); overflow(o) }
    }
    if (bytes(o) + extra > RETAINED_BYTES || (!items.length && count(o) >= MAX_RECORDS)) return null
    const p = { customType: `herdr-threads:${o.execution}:${io.nonce()}`, text, guidance: guidance.length, items: items.map(({ rendered, ...r }) => ({ ...r, attemptedRun: o.run })), frames,
      revision: o.revision, evidenced: false, acking: false, retryCancel: null }
    o.proposals.push(p)
    if (guidance) { o.guidance = ''; o.guidanceAttempted = true }
    for (const r of items) o.pending.delete(r.key)
    trim(o)
    return { message: { customType: p.customType, content: p.text, display: false }, mail: items.length > 0 }
  }
  async function acknowledge(o, p) {
    if (!current(o) || !o.live || p.acking || !p.evidenced || p.revision !== o.revision || !branch(o)?.some(e => exact(e, p))) return
    const ids = p.items.filter(r => !r.truncated && ['message', 'lazy'].includes(r.kind)).map(r => r.id)
    p.acking = true
    const revision = o.revision
    const attempt = Symbol('ack')
    p.attempt = attempt
    const settled = new Set()
    if (ids.length) {
      try {
        const r = await run(o, cmd('watch', 'ack', '--session', o.sid, '--via', 'context', ...ids), { stdin: '', env: childEnv(o), timeoutMs: 15000, maxBytes: PAGE_BYTES, signal: o.abort.signal })
        if (r.code === 0) for (const line of r.stdout.split('\n')) {
          try { const row = JSON.parse(line); if (ids.includes(row.id) && ['settled', 'already_settled'].includes(row.result)) settled.add(row.id) } catch {}
        }
      } catch {}
    }
    if (!current(o) || revision !== o.revision || p.attempt !== attempt || !o.proposals.includes(p)) return
    p.acking = false
    const complete = p.items.filter(r => !ids.includes(r.id) || settled.has(r.id))
    for (const r of complete) { remember(o, r.key); p.frames.delete(r.key); if (o.intent?.key === r.key) o.intent = null }
    p.items = p.items.filter(r => !complete.includes(r))
    if (!p.items.length) o.proposals.splice(o.proposals.indexOf(p), 1)
    else { diagnostic('ACK pending'); p.retryCancel = later(o, 30000, () => { p.retryCancel = null; void acknowledge(o, p) }) }
    if (complete.length || !p.items.length) released(o)
  }
  function retryAcks(o) { for (const p of o.proposals) if (!p.retryCancel) void acknowledge(o, p) }
  function close(o, releaseOwner = true, reload = false) {
    if (!o) return Promise.resolve()
    if (o.closing) {
      // A same-instance successor reserves the owner while joining the already
      // running shutdown; it still cannot start until that shutdown has reaped.
      if (!releaseOwner) o.releaseOwner = false
      return o.closing
    }
    o.closed = true
    o.releaseOwner = releaseOwner
    o.abort.abort()
    if (reload) io.registry[RECOVERY] = { sid: o.sid, proposals: o.proposals }
    else if (releaseOwner) delete io.registry[RECOVERY]
    for (const cancel of o.timers) cancel()
    o.timers.clear(); ++o.watchRun
    const stopping = stopWatch(o, o.child); o.child = null; o.live = false
    // Form the old-owner event/env before cleanup; no successor fields are removed.
    const ending = o.bound ? hook(o, 'SessionEnd') : Promise.resolve()
    for (const [key, value] of Object.entries(o.env)) if (io.env[key] === value) delete io.env[key]
    o.closing = (async () => {
      await stopping
      await ending
      await Promise.allSettled([...o.calls, ...o.stops])
      if (o.releaseOwner && io.registry[OWNER] === token) delete io.registry[OWNER]
    })()
    return o.closing
  }
  pi.on('session_start', async (event, ctx) => {
    if (forbidden || ctx.mode !== 'tui' || (io.registry[OWNER] && io.registry[OWNER] !== token)) return
    if (typeof ctx.sessionManager?.getSessionId !== 'function' || typeof ctx.sessionManager?.getBranch !== 'function' || typeof ctx.isIdle !== 'function') { diagnostic('boundary API unavailable'); return }
    const sid = ctx.sessionManager.getSessionId()
    if (typeof sid !== 'string' || !sid || size(sid) > 128 || /[\x00-\x20\x7f]/.test(sid)) { diagnostic('session unavailable'); return }
    // Reserve synchronously: two duplicate callbacks may otherwise both check in
    // before either async admission observes the other's owner.
    io.registry[OWNER] = token
    const ticket = ++lifecycle
    await close(owner, false, event.reason === 'reload')
    if (ticket !== lifecycle || io.registry[OWNER] !== token) return
    const recovery = event.reason === 'reload' && io.registry[RECOVERY]?.sid === sid ? io.registry[RECOVERY] : null
    delete io.registry[RECOVERY]
    owner = { sid, ctx, execution: io.nonce(), source: ['startup', 'reload', 'new', 'resume', 'fork'].includes(event.reason) ? event.reason : null,
      env: { [PREFIX + 'SESSION']: sid, [PREFIX + 'ROLE']: 'top_level', [PREFIX + 'MODE']: 'tui', [PREFIX + 'PID']: String(io.pid) },
      bound: false, closed: false, admission: null, idle: false, active: false, compacting: false, prompts: 0, abort: new AbortController(),
      pending: new Map(), proposals: recovery?.proposals ?? [], done: new Set(), guidance: '', guidanceAttempted: false, run: 0, revision: 0,
      timers: new Set(), calls: new Set(), stops: new Set(), closing: null, child: null, watchRun: 0, live: false, backoff: 0, intent: null, attentionKeys: new Set(), attentionHistoryBlocked: false,
      replayDebt: false, replaying: false, waking: false, suspended: false }
    if (!owner.source) { await close(owner); return }
    for (const p of owner.proposals) {
      p.evidenced = false; p.acking = false; p.retryCancel = null; p.attempt = null; p.revision = 0
      for (const r of p.items) r.attemptedRun = 0
    }
    Object.assign(io.env, owner.env)
    await admit(owner)
  })
  pi.on('before_agent_start', async (event, ctx) => {
    const o = callbackOwner(ctx)
    if (!o) return
    o.ctx = ctx; o.idle = false; o.active = true; o.run++; o.guidanceAttempted = false; progress(o)
    if (event.messageType === 'user') { resetAttentionHistory(o); if (o.intent) o.intent.blocked = false }
    if (!await admit(o)) return
    const live = await hook(o, 'TurnStart')
    if (!current(o) || live === null) return
    if (!reconcile(o)) return
    const p = proposal(o)
    return p ? { message: p.message } : undefined
  })
  for (const name of ['turn_end', 'agent_before_settle']) pi.on(name, (event, ctx) => {
    const o = callbackOwner(ctx)
    if (!o || !o.bound || !o.active || o.compacting) return
    if (event.outcome !== 'completed') { if (['error', 'aborted'].includes(event.outcome)) blockAttention(o); return }
    if (!Array.isArray(event.entries) || typeof event.continue !== 'boolean' || !event.context) { diagnostic('boundary API unavailable'); return }
    o.ctx = ctx
    if (!branch(o)) return
    const p = proposal(o)
    return p ? { entries: [...event.entries, { type: 'custom_message', ...p.message }], continue: event.continue || p.mail } : undefined
  })
  pi.on('context', (event, ctx) => {
    const o = callbackOwner(ctx)
    if (!o || !o.bound || !Array.isArray(event.messages)) return
    o.ctx = ctx; progress(o)
    const entries = branch(o)
    if (!reconcile(o, entries)) return
    for (const p of o.proposals) if (entries.some(e => exact(e, p)) && event.messages.some(m => m.role === 'custom' && m.customType === p.customType && m.content === p.text)) {
      p.evidenced = true; p.revision = o.revision
      for (const r of p.items) o.attentionKeys.delete(r.key)
      if (p.items.some(r => r.key === o.intent?.key)) o.intent = null
      void acknowledge(o, p)
    }
  })
  pi.on('agent_start', (_event, ctx) => { const o = callbackOwner(ctx); if (o) { o.ctx = ctx; o.active = true; o.idle = false; progress(o) } })
  pi.on('agent_settled', (event, ctx) => {
    const o = callbackOwner(ctx)
    if (o) {
      if (event.aborted || [...o.pending.values(), ...o.proposals.flatMap(p => p.items)].some(r => r.key === o.intent?.key && r.attemptedRun === o.run)) blockAttention(o)
      o.ctx = ctx; o.active = false; o.idle = ctx.isIdle() === true; progress(o); wake(o)
    }
  })
  pi.on('ui_prompt_start', (_event, ctx) => { const o = callbackOwner(ctx); if (o) o.prompts++ })
  pi.on('ui_prompt_end', (_event, ctx) => { const o = callbackOwner(ctx); if (o) { o.prompts = Math.max(0, o.prompts - 1); wake(o) } })
  pi.on('session_before_compact', (_event, ctx) => { const o = callbackOwner(ctx); if (o) { o.compacting = true; o.idle = false } })
  pi.on('session_compact_failed', (_event, ctx) => { const o = callbackOwner(ctx); if (o) o.compacting = false })
  async function reenter(source, ctx) {
    const o = callbackOwner(ctx)
    if (!o) return
    o.ctx = ctx; o.revision++; o.compacting = false
    const revision = o.revision
    for (const p of o.proposals) { p.retryCancel?.(); p.retryCancel = null; p.evidenced = false; p.acking = false }
    reconcile(o)
    const guidance = await hook(o, 'SessionStart', source)
    if (!current(o) || revision !== o.revision) return
    if (guidance === null) { o.bound = false; o.source = source; return }
    o.guidance = guidance; o.guidanceAttempted = false
  }
  pi.on('session_tree', (_event, ctx) => reenter('tree', ctx))
  pi.on('session_compact', (_event, ctx) => reenter('compact', ctx))
  pi.on('session_shutdown', (event, ctx) => {
    const o = callbackOwner(ctx)
    if (o) { lifecycle++; return close(o, true, event.reason === 'reload') }
  })
}

/** Bounded JSON-line child transport; stop waits for this owned child's exit. */
export function nodeIO(argv = LAUNCH?.argv) {
  const env = process.env
  function child(argv, options, handlers = {}) {
    const proc = spawn(argv[0], argv.slice(1), { env: options.env, cwd: process.cwd(), stdio: ['pipe', 'pipe', 'ignore'] })
    let buffer = '', stopped = false, killTimer, finished = false
    let resolveExit
    const exited = new Promise(resolve => { resolveExit = resolve })
    const stop = () => {
      if (!stopped && !finished) { stopped = true; proc.kill('SIGTERM'); killTimer = setTimeout(() => proc.kill('SIGKILL'), 250); killTimer.unref?.() }
      return exited
    }
    proc.stdin.on('error', () => {})
    proc.stdout.setEncoding('utf8')
    proc.stdout.on('data', chunk => {
      buffer += chunk
      if (options.lines) {
        let at
        while ((at = buffer.indexOf('\n')) >= 0) {
          const line = buffer.slice(0, at); buffer = buffer.slice(at + 1)
          if (size(line) > options.maxBytes) { void stop(); return }
          try { handlers.line?.(JSON.parse(line)) } catch {}
        }
      }
      if (size(buffer) > options.maxBytes) { buffer = ''; void stop() }
    })
    const finish = code => {
      if (finished) return
      finished = true; clearTimeout(killTimer); options.signal?.removeEventListener('abort', stop)
      handlers.exit?.(stopped ? 1 : code, buffer)
      resolveExit()
    }
    proc.on('error', () => finish(1))
    proc.on('close', code => finish(code ?? 1))
    options.signal?.addEventListener('abort', stop, { once: true })
    if (options.signal?.aborted) void stop()
    return { stop, proc }
  }
  return {
    argv, env, pid: process.pid, cwd: process.cwd(), registry: globalThis, nonce: randomUUID,
    schedule: (ms, fn) => { const timer = setTimeout(fn, ms); timer.unref?.(); return () => clearTimeout(timer) },
    diagnostic: text => process.stderr.write(text + '\n'),
    spawn: (argv, options, handlers) => {
      const owned = child(argv, { ...options, lines: true }, handlers)
      owned.proc.stdin.end()
      return { stop: owned.stop }
    },
    run: (argv, options) => new Promise(resolve => {
      let timer
      const owned = child(argv, options, { exit: (code, stdout) => { clearTimeout(timer); resolve({ code, stdout }) } })
      timer = setTimeout(() => { void owned.stop() }, options.timeoutMs)
      timer.unref?.()
      owned.proc.stdin.end(options.stdin)
    }),
  }
}

export default function herdrThreads(pi) { register(pi, nodeIO()) }
