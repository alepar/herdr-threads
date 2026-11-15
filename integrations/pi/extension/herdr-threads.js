import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'

// Setup replaces this one marked constant, and owns this entire extension directory.
const LAUNCH = null // herdr-threads:launch
const OWNER = Symbol.for('herdr-threads.pi.owner')
const RECOVERY = Symbol.for('herdr-threads.pi.recovery')
const PREFIX = 'HERDR_THREADS_PI_'
const HEADER = '[herdr-threads] new messages:'
const MAX_RECORDS = 256
const ORDINARY_RECORDS = 224
const RESERVE_BYTES = 512 * 1024
const TRANSPORT_BYTES = 72 * 1024
const MAX_BYTES = 2 * 1024 * 1024
// Parser, the pinned canonical page and bounded acceptance hints own the reserve.
const RETAINED_BYTES = MAX_BYTES - RESERVE_BYTES
const PAGE_BYTES = 64 * 1024
const BACKOFF = [1000, 2000, 5000, 10000, 30000]
const size = text => Buffer.byteLength(text, 'utf8')
const oneLine = text => String(text).replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ')
const indent = text => String(text).split(/\r\n|[\n\r\u000b\u000c\u0085\u2028\u2029]/).map(s => s ? `  ${s}` : '').join('\n')
const label = (id, name) => !name || name === id ? oneLine(id || 'unknown')
  : id ? `${oneLine(id)} ${JSON.stringify(oneLine(name))}` : JSON.stringify(oneLine(name))

// Mirrors src/delivery/frame.rs. Manual watch leaves nonordinary presentation
// pending until this owner proves the whole emitted page in native context.
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
  const diagnostic = (reason, o = owner) => {
    try {
      const text = `herdr-threads: pi ${reason}`
      if (o) {
        if (!current(o)) return
        // Raw stderr bypasses Pi's renderer and can land in its composer.
        // Missing/throwing native notification never falls back to that path.
        if (o.ctx.mode === 'tui') { o.ctx.ui?.notify?.(text, 'warning'); return }
      }
      io.diagnostic(text)
    } catch {}
  }
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
  function hook(o, event, source) {
    // Preserve native event order even if a lifecycle or Idle child is slow.
    // Check a queued Idle against the current native state at actual emission.
    const task = (o.hooks ?? Promise.resolve()).then(async () => {
      if (event !== 'SessionEnd' && !current(o)) return null
      try {
        if (event === 'TurnEnd' && o.ctx.isIdle?.() !== true) return null
        const result = await run(o, cmd('hook-event', 'pi', event), {
          stdin: JSON.stringify({ ...claims(o), event, ...(source ? { source } : {}), ...(event === 'TurnEnd' ? { idle: true } : {}) }),
          env: childEnv(o), timeoutMs: 8000, maxBytes: 4096,
          signal: event === 'SessionEnd' ? undefined : o.abort.signal,
        })
        if (result.code !== 0 || typeof result.stdout !== 'string' || size(result.stdout) > 4096
            || result.stdout.includes('herdr-threads: check-in unavailable (')) throw new Error('unavailable')
        return result.stdout
      } catch { if (current(o)) diagnostic('check-in unavailable'); return null }
    })
    o.hooks = task
    return task
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
      await hook(o, 'TurnEnd', 'startup')
      return true
    })()
    try { return await o.admission } finally { o.admission = null }
  }
  const records = o => [...o.pending.values(), ...o.proposals.flatMap(p => p.items)]
  const metadata = r => size(r.key) + size(r.id) + size(r.epoch ?? '')
  function count(o) { return records(o).filter(r => !r.protected).length }
  function bytes(o) {
    return [...o.pending.values()].filter(r => !r.protected).reduce((n, r) => n + size(r.rendered) + metadata(r), 0)
      + o.proposals.filter(p => p.ordinaryArena).reduce((n, p) => n + size(p.text) + size(p.customType) + p.items.reduce((m, r) => m + metadata(r), 0), size(o.guidance))
      + [...o.done].reduce((n, key) => n + size(key), 0)
  }
  function protectedBytes(o) {
    return [...o.pending.values()].filter(r => r.protected).reduce((n, r) => n + size(r.rendered) + metadata(r), 0)
      + o.proposals.filter(p => !p.ordinaryArena).reduce((n, p) => n + size(p.text) + size(p.customType), 0)
      + records(o).filter(r => r.protected && !o.pending.has(r.key)).reduce((n, r) => n + metadata(r), 0)
      + size(o.page?.receipt ?? '')
  }
  function overflow(o) { o.replayDebt = true }
  function trim(o) {
    while (count(o) > ORDINARY_RECORDS || bytes(o) > RETAINED_BYTES - 4096) {
      const worst = [...o.pending.values()].filter(r => !r.protected).sort(compare).at(-1)
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
    return ownStop(o, Promise.resolve().then(() => child.stop()).catch(() => diagnostic('watch cleanup failed', o)))
  }
  // A connected replacement pays debt only after actual capacity release. Merely
  // being full never creates a reconnect loop asking for the same excess bodies.
  function released(o) {
    if (o.replayDebt && !o.page && !o.confirming && !o.replaying && count(o) < ORDINARY_RECORDS && current(o)) {
      o.replaying = true
      void ownStop(o, replaceWatch(o))
    }
  }
  function replaceWatch(o) {
    if (o.replacing) return o.replacing
    o.live = false; ++o.watchRun
    if (o.replayDebt) o.replaying = true
    const child = o.child; o.child = null
    const confirming = o.confirming
    const task = (async () => {
      await stopWatch(o, child)
      await confirming
      discardPage(o)
      if (current(o)) startWatch(o, task)
    })()
    o.replacing = task
    task.then(() => { if (o.replacing === task) o.replacing = null }, () => { if (o.replacing === task) o.replacing = null })
    return task
  }
  function startWatch(o, replacement) {
    if (!current(o) || !o.bound) return
    // Admission after binding refusal is another successor-start path. It must
    // join the same old confirmation/replacement as backoff and ACK recovery.
    if (o.confirming || (o.replacing && o.replacing !== replacement)) {
      if (!o.replacing) void ownStop(o, replaceWatch(o))
      return
    }
    if (o.child) return
    const run = ++o.watchRun
    try {
      o.child = io.spawn(cmd('watch', '--harness', 'pi', '--session', o.sid, '--manual-presentation'), { env: childEnv(o), maxBytes: TRANSPORT_BYTES }, {
        line: (line, rawBytes) => { if (current(o) && run === o.watchRun) onLine(o, line, rawBytes) },
        exit: code => {
          if (!current(o) || run !== o.watchRun) return
          o.child = null; o.live = false
          discardPage(o)
          if (o.replaying) o.replayDebt = true
          diagnostic('watch unavailable')
          if (code === 3) { o.bound = false; o.replaying = false; return }
          later(o, BACKOFF[Math.min(o.backoff++, BACKOFF.length - 1)], () => { void ownStop(o, replaceWatch(o)) })
        },
      })
    } catch {
      o.child = null; o.live = false; o.replayDebt ||= o.replaying
      diagnostic('watch unavailable')
      later(o, BACKOFF[Math.min(o.backoff++, BACKOFF.length - 1)], () => { void ownStop(o, replaceWatch(o)) })
    }
  }
  function ordinary(line) { return ['message', 'lazy'].includes(line.kind) }
  function record(line, key, protectedSlot = false) {
    if (!line || line.schema !== 1 || typeof line.id !== 'string' || size(line.id) > 128) return null
    if (!['message', 'lazy', 'invitation', 'attention', 'notice', 'attention_cleared'].includes(line.kind)) return null
    const clear = line.kind === 'attention_cleared'
    const body = line.kind === 'attention' ? line.text : line.body
    if (!clear && (typeof body !== 'string' || size(body) > PAGE_BYTES)) return null
    if (['attention', 'attention_cleared'].includes(line.kind) && typeof line.presentation_epoch !== 'string') return null
    if (line.presentation_epoch !== undefined && (typeof line.presentation_epoch !== 'string' || size(line.presentation_epoch) > 128)) return null
    const rendered = clear ? '' : frame(line)
    // Never truncate a required slot and then claim that its body was observed.
    if (size(rendered) + size(HEADER) + 2 > PAGE_BYTES) return null
    return { key, id: line.id, kind: line.kind, epoch: line.presentation_epoch,
      protected: protectedSlot, covered: false, applied: false,
      priority: Number.isInteger(line.priority) && line.priority >= 0 && line.priority <= 9 ? line.priority : line.kind === 'lazy' ? 1 : 0,
      deadline: Number.isFinite(line.deadline_at) ? line.deadline_at : Infinity,
      order: sequence++, truncated: line.truncated === true, rendered }
  }
  function proposalFor(o, r) { return o.proposals.find(p => p.items.includes(r)) }
  function discardPage(o) {
    o.page = null
    for (const r of records(o)) if (r.protected) {
      r.covered = false; r.applied = false
      if (o.pending.get(r.key) === r) o.pending.delete(r.key)
    }
  }
  function pageFailure(o) {
    if (o.pageFailure) return
    o.pageFailure = true; o.live = false
    diagnostic('presentation unavailable')
    // Contract failures stay canonical and retry with bounded existing backoff.
    later(o, BACKOFF[Math.min(o.backoff++, BACKOFF.length - 1)], () => {
      o.pageFailure = false; discardPage(o); void ownStop(o, replaceWatch(o))
    })
  }
  function applyClears(o) {
    if (!o.page) return
    for (const clear of o.page.required.values()) if (clear.kind === 'attention_cleared') {
      for (const [key, r] of o.pending) if (!r.protected && r.kind === 'attention' && r.epoch === clear.epoch) {
        o.pending.delete(key)
      }
      clear.applied = true
    }
  }
  function onPage(o, line, rawBytes) {
    if (o.page || !Array.isArray(line.items) || !Array.isArray(line.keys)
        || line.items.length < 1 || line.items.length > 32 || line.items.length !== line.keys.length
        || typeof line.receipt !== 'string' || !line.receipt || size(line.receipt) > 4096) { pageFailure(o); return }
    // The transport measures raw bytes before JSON decoding. Never reserialize
    // decoded u64 fields (which JavaScript may already have rounded).
    if (rawBytes > TRANSPORT_BYTES) { pageFailure(o); return }
    const required = new Map(), mail = []
    let renderedBytes = 0
    for (let i = 0; i < line.items.length; i++) {
      const item = line.items[i], key = line.keys[i]
      if (!item || (ordinary(item) ? key !== null : typeof key !== 'string' || !/^[0-9a-f]{64}$/.test(key) || required.has(key))) { pageFailure(o); return }
      const r = record(item, ordinary(item) ? JSON.stringify([item.kind, item.id, null]) : key, !ordinary(item))
      if (!r) { pageFailure(o); return }
      renderedBytes += size(r.rendered)
      if (r.protected) required.set(key, r); else mail.push(r)
    }
    if (!required.size || renderedBytes > 128 * 1024) { pageFailure(o); return }
    // Old proposals are hints only. Drop mismatching token/slot associations,
    // merge exact ones, then atomically pin the fresh canonical page.
    for (const p of [...o.proposals]) {
      p.items = p.items.filter(r => {
        if (!r.protected) return true
        const fresh = required.get(r.key), span = p.frames.get(r.key)
        if (!fresh || !span || p.text.slice(...span) !== fresh.rendered) { p.frames.delete(r.key); return false }
        r.covered = false; r.applied = false
        required.set(r.key, r)
        return true
      })
      if (!p.items.length) o.proposals.splice(o.proposals.indexOf(p), 1)
    }
    o.page = { required, receipt: line.receipt, revision: o.revision }
    for (const r of required.values()) if (!proposalFor(o, r)) o.pending.set(r.key, r)
    // A surviving hint may retain stale whole-proposal text or old guidance.
    // It is optional: release that hint before canonical data could exceed the
    // reserved payload budget. Fresh slot frames remain pinned for at-least-once
    // delivery, never silently discarded to preserve a deduplication hint.
    for (const p of [...o.proposals]) if (!p.ordinaryArena && protectedBytes(o) > 144 * 1024) requeue(o, p)
    applyClears(o)
    for (const r of mail) if (!o.done.has(r.key) && !records(o).some(old => old.key === r.key)) retain(o, r)
    void confirmPage(o)
  }
  function onLine(o, line, rawBytes) {
    if (!line || line.schema !== 1 || o.pageFailure) return
    if (line.kind === 'presentation_page') { onPage(o, line, rawBytes); return }
    if (line.kind === 'status') {
      if (line.state === 'connected') {
        o.live = true
        if (o.replaying) { o.replaying = false; o.replayDebt = false }
        retryAcks(o)
      } else if (['closing', 'refused'].includes(line.state)) {
        o.live = false
        if (['no_binding', 'session_mismatch', 'binding_changed'].includes(line.reason)) o.bound = false
      }
      return
    }
    if (!ordinary(line)) { pageFailure(o); return }
    const key = JSON.stringify([line.kind, line.id, null])
    if (o.done.has(key) || records(o).some(r => r.key === key)) return
    const r = record(line, key)
    if (!r) { overflow(o); return }
    retain(o, r)
  }
  function pageCovered(o, page) {
    if (!current(o) || !o.live || o.page !== page || page.revision !== o.revision) return false
    const entries = branch(o)
    return [...page.required.values()].every(r => r.kind === 'attention_cleared' ? r.applied
      : r.covered && (() => { const p = proposalFor(o, r); return p && p.revision === o.revision && entries?.some(e => exact(e, p)) })())
  }
  function confirmPage(o) {
    const page = o.page
    if (!page || o.confirming || !pageCovered(o, page)) return
    const revision = o.revision
    const task = (async () => {
      let confirmed = false
      try {
        // Capture exact opaque bytes before yielding. Neither a successful ACK
        // nor accepted native drafts substitute for whole-page evidence.
        const result = await run(o, cmd('watch', 'presentation-confirm', '--session', o.sid), {
          stdin: page.receipt, env: childEnv(o), timeoutMs: 15000, maxBytes: 4096, signal: o.abort.signal,
        })
        if (result.code === 0 && typeof result.stdout === 'string' && size(result.stdout) <= 4096) {
          const value = JSON.parse(result.stdout)
          confirmed = value.emitted_page_confirmed === true && ['confirmed', 'already_confirmed'].includes(value.status)
        }
      } catch {}
      if (!current(o) || o.page !== page) return
      // A lost response (or revision during the call) may already have committed.
      // Reconcile once through canonical replay, never infer a failed mutation.
      if (confirmed && revision === o.revision && pageCovered(o, page)) {
        const keys = new Set(page.required.keys())
        for (const key of keys) o.pending.delete(key)
        for (const p of [...o.proposals]) {
          for (const key of keys) p.frames.delete(key)
          p.items = p.items.filter(r => !keys.has(r.key))
          if (!p.items.length) o.proposals.splice(o.proposals.indexOf(p), 1)
        }
      }
      return confirmed
    })()
    o.confirming = task
    ownStop(o, task).then(confirmed => {
      if (o.confirming === task) o.confirming = null
      if (!current(o)) return
      // An existing replacement owns the replay even if a revision/exit already
      // invalidated this page. It is waiting for this exact flight to finish.
      if (o.replacing) return
      if (o.page !== page) return
      discardPage(o)
      if (confirmed) void ownStop(o, replaceWatch(o))
      else {
        diagnostic('presentation pending')
        later(o, BACKOFF[Math.min(o.backoff++, BACKOFF.length - 1)], () => { void ownStop(o, replaceWatch(o)) })
      }
    }).catch(() => {})
    return task
  }
  function remember(o, key) { o.done.add(key); while (o.done.size > MAX_RECORDS) o.done.delete(o.done.values().next().value) }
  function requeue(o, p) {
    o.proposals.splice(o.proposals.indexOf(p), 1)
    if (p.guidance) o.guidance ||= p.text.slice(0, p.guidance)
    for (const r of p.items) {
      r.covered = false
      if (r.protected && o.page?.required.get(r.key) !== r) continue
      const [start, end] = p.frames.get(r.key); r.rendered = p.text.slice(start, end); retain(o, r)
    }
  }
  function reconcile(o, entries = branch(o), projection) {
    if (!entries) return false
    for (const p of [...o.proposals]) {
      const required = p.items.some(r => r.protected && o.page?.required.get(r.key) === r)
      const absent = required && Array.isArray(projection) && !projection.some(m => m.role === 'custom' && m.customType === p.customType && m.content === p.text)
      if (entries.some(e => exact(e, p)) && !absent) continue
      // Projection only proves absence. Keep attemptedRun when requeueing, so a
      // rejected/compacted frame cannot cause an automatic continuation loop.
      requeue(o, p)
    }
    return true
  }
  function proposal(o) {
    if (o.proposals.length >= MAX_RECORDS) return null
    const items = [], frames = new Map(), guidance = o.guidanceAttempted ? '' : o.guidance
    const render = () => [guidance, items.length ? HEADER + '\n\n' + items.map(r => frames.get(r.key)).join('\n\n') : ''].filter(Boolean).join('\n\n')
    for (const r of [...o.pending.values()].sort(compare)) {
      if (r.kind === 'attention_cleared' || (r.protected && o.page?.required.get(r.key) !== r) || r.attemptedRun === o.run) continue
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
    const customType = `herdr-threads:${o.execution}:${io.nonce()}`
    const ordinaryArena = !items.length || items.some(r => !r.protected)
    const extra = (ordinaryArena ? size(text) + size(customType) + items.reduce((n, r) => n + metadata(r), 0) : 0)
      - size(guidance) - items.filter(r => !r.protected).reduce((n, r) => n + size(r.rendered) + metadata(r), 0)
    const protectedExtra = ordinaryArena ? -items.filter(r => r.protected).reduce((n, r) => n + size(r.rendered), 0)
      : size(text) + size(customType) - items.reduce((n, r) => n + size(r.rendered), 0)
    // Reserve the framing overhead before moving records; proposed pages cannot
    // be displaced. If they own the whole budget, wait for context/ACK release.
    const selected = new Set(items.map(r => r.key))
    for (const r of [...o.pending.values()].sort(compare).reverse()) {
      if (bytes(o) + extra <= RETAINED_BYTES - 4096) break
      if (!r.protected && !selected.has(r.key)) { o.pending.delete(r.key); overflow(o) }
    }
    if (protectedBytes(o) + protectedExtra > 144 * 1024 || bytes(o) + extra > RETAINED_BYTES - 4096 || (!items.length && count(o) >= ORDINARY_RECORDS)) return null
    const p = { customType, ordinaryArena, text, guidance: guidance.length, items, frames,
      revision: o.revision, evidenced: false, acking: false, retryCancel: null }
    o.proposals.push(p)
    if (guidance) { o.guidance = ''; o.guidanceAttempted = true }
    for (const r of items) { r.attemptedRun = o.run; delete r.rendered; o.pending.delete(r.key) }
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
    const complete = p.items.filter(r => !r.protected && (!ids.includes(r.id) || settled.has(r.id)))
    for (const r of complete) { remember(o, r.key); p.frames.delete(r.key) }
    p.items = p.items.filter(r => !complete.includes(r))
    if (!p.items.length) o.proposals.splice(o.proposals.indexOf(p), 1)
    else if (p.items.some(r => !r.protected)) { diagnostic('ACK pending'); p.retryCancel = later(o, 30000, () => { p.retryCancel = null; void acknowledge(o, p) }) }
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
    discardPage(o)
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
    if (forbidden || (io.registry[OWNER] && io.registry[OWNER] !== token)) return
    if (ctx.mode === undefined || ctx.mode === null) {
      try {
        if (typeof ctx.ui?.notify === 'function') ctx.ui.notify('herdr-threads: pi boundary API unavailable', 'warning')
      } catch {}
      return
    }
    if (ctx.mode !== 'tui') return
    if (typeof ctx.sessionManager?.getSessionId !== 'function' || typeof ctx.sessionManager?.getBranch !== 'function') { diagnostic('boundary API unavailable'); return }
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
      bound: false, closed: false, admission: null, active: false, compacting: false, abort: new AbortController(),
      pending: new Map(), proposals: recovery?.proposals ?? [], done: new Set(), guidance: '', guidanceAttempted: false, run: 0, revision: 0,
      timers: new Set(), calls: new Set(), stops: new Set(), closing: null, child: null, watchRun: 0, live: false, backoff: 0,
      replayDebt: false, replaying: false, replacing: null, page: null, confirming: null, pageFailure: false }
    if (!owner.source) { await close(owner); return }
    for (const p of owner.proposals) {
      p.ordinaryArena ??= true
      p.evidenced = false; p.acking = false; p.retryCancel = null; p.attempt = null; p.revision = 0
      for (const r of p.items) {
        r.attemptedRun = 0
        // Retained hints keep their first-observed order if requeued. Fresh
        // canonical rows in this factory must sort after those stable ties.
        sequence = Math.max(sequence, r.order + 1)
      }
    }
    Object.assign(io.env, owner.env)
    await admit(owner)
  })
  pi.on('before_agent_start', async (event, ctx) => {
    const o = callbackOwner(ctx)
    if (!o) return
    o.ctx = ctx; o.active = true; o.run++; o.guidanceAttempted = false
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
    if (event.outcome !== 'completed') return
    if (!Array.isArray(event.entries) || typeof event.continue !== 'boolean' || !event.context) { diagnostic('boundary API unavailable'); return }
    o.ctx = ctx
    const projection = Array.isArray(event.context.contextMessages) ? event.context.contextMessages
      : Array.isArray(event.context.contextEntries) ? event.context.contextEntries.flatMap(e => e.messages ?? []) : undefined
    if (!reconcile(o, branch(o), projection)) return
    const p = proposal(o)
    return p ? { entries: [...event.entries, { type: 'custom_message', ...p.message }], continue: event.continue || p.mail } : undefined
  })
  pi.on('context', (event, ctx) => {
    const o = callbackOwner(ctx)
    if (!o || !o.bound || !Array.isArray(event.messages)) return
    o.ctx = ctx
    const entries = branch(o)
    if (!reconcile(o, entries, event.messages)) return
    for (const p of o.proposals) if (entries.some(e => exact(e, p)) && event.messages.some(m => m.role === 'custom' && m.customType === p.customType && m.content === p.text)) {
      p.evidenced = true; p.revision = o.revision
      for (const r of p.items) if (r.protected && o.page?.required.get(r.key) === r) r.covered = true
      void acknowledge(o, p)
    }
    void confirmPage(o)
  })
  pi.on('agent_start', (_event, ctx) => { const o = callbackOwner(ctx); if (o) { o.ctx = ctx; o.active = true } })
  pi.on('agent_settled', async (_event, ctx) => {
    const o = callbackOwner(ctx)
    if (o) {
      o.ctx = ctx; o.active = false
      if (o.bound) await hook(o, 'TurnEnd', 'settled')
    }
  })
  pi.on('session_before_compact', (_event, ctx) => { const o = callbackOwner(ctx); if (o) o.compacting = true })
  pi.on('session_compact_failed', (_event, ctx) => { const o = callbackOwner(ctx); if (o) o.compacting = false })
  async function reenter(source, ctx) {
    const o = callbackOwner(ctx)
    if (!o) return
    o.ctx = ctx; o.revision++; o.compacting = false
    if (o.page) {
      o.page.revision = o.revision
      for (const r of o.page.required.values()) { r.covered = false; r.applied = false }
      applyClears(o)
    }
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
    let stopped = false, killTimer, finished = false
    let resolveExit
    const exited = new Promise(resolve => { resolveExit = resolve })
    const stop = () => {
      if (!stopped && !finished) { stopped = true; proc.kill('SIGTERM'); killTimer = setTimeout(() => proc.kill('SIGKILL'), 250); killTimer.unref?.() }
      return exited
    }
    proc.stdin.on('error', () => {})
    // Read bounded binary chunks rather than concatenating an arbitrary data
    // event containing many pages. Clear the accumulator before decoding a line.
    // At normalization: <=2 transport buffers + decoded JSON + bounded frames.
    let lineBuffer = Buffer.alloc(0)
    proc.stdout.on('readable', () => {
      let chunk
      while ((chunk = proc.stdout.read(Math.min(proc.stdout.readableLength, options.maxBytes) || 1)) !== null) {
        if (!options.lines) {
          if (lineBuffer.length + chunk.length > options.maxBytes) { lineBuffer = Buffer.alloc(0); void stop(); return }
          lineBuffer = Buffer.concat([lineBuffer, chunk]); continue
        }
        let from = 0
        while (from < chunk.length) {
          const end = chunk.indexOf(10, from), until = end < 0 ? chunk.length : end
          if (lineBuffer.length + until - from + 1 > options.maxBytes) { lineBuffer = Buffer.alloc(0); void stop(); return }
          lineBuffer = Buffer.concat([lineBuffer, chunk.subarray(from, until)])
          from = until + 1
          if (end >= 0) {
            const rawBytes = lineBuffer.length + 1
            let text = lineBuffer.toString('utf8'); lineBuffer = Buffer.alloc(0)
            try {
              const parsed = JSON.parse(text); text = null
              handlers.line?.(parsed, rawBytes)
            } catch {}
          }
        }
      }
    })
    const finish = code => {
      if (finished) return
      finished = true; clearTimeout(killTimer); options.signal?.removeEventListener('abort', stop)
      handlers.exit?.(stopped ? 1 : code, options.lines ? '' : lineBuffer.toString('utf8'))
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
