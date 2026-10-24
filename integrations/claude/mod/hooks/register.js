// herdr-threads Claude Code mod (spec D1, D4-D6). One self-contained module:
// `createCore(io)` is the delivery state machine, driven by events and acting
// only through `io`; `register(on)` wires it to the engine. The contract with
// the `watch` child is in ../README.md and src/protocol/watch.rs.

// `setup claude` replaces the next line with the hooks' invocation (absolute
// executable, --state-dir, --host-endpoint); `null` means launch by name.
const LAUNCH = null // herdr-threads:launch (setup claude writes the hooks' invocation here)

const HEADER = '[herdr-threads] new messages:'
// The content-free wake prompt (R18); pinned to src/notification/policy.rs CHANNEL_WAKE_UP by tests/harness/claude_mod.rs.
const WAKE_PROMPT = 'wake up'
const BACKOFF_S = [1, 2, 5, 10, 30]
const ASSUMED_BUSY_IDLE_MS = 5000
const HOLD_IDLE_MS = 120000
const DRAFT_WAIT_MS = 120000
const DROP_BACKOFF_MS = 30000
const ACK_RETRY_MS = 30000
const ACK_BATCH_MAX = 100 // MAX_BATCH_ITEMS in src/protocol/commands.rs
const CONNECTED_RESET_MS = 60000
const LEDGER_CAP = 2000
// Match the delivery envelope, counting the actual framed UTF-8 text.
const APPEND_MAX_ITEMS = 32
const APPEND_MAX_BYTES = 64 * 1024

const INTENTS = new Set(['query', 'request', 'rule'])
const canonicalEpoch = (v) => typeof v === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(v)
  && (v.length < 20 || v <= '18446744073709551615')
const oneLine = (s) => String(s).replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ')
const LINE_BREAK = /\r\n|[\n\r\u000b\u000c\u0085\u2028\u2029]/
/** Every non-empty body line indented two spaces (src/protocol/output_compact.rs), so a body never starts a line at column 0. */
const indent = (body) => String(body).split(LINE_BREAK).map((l) => (l === '' ? '' : `  ${l}`)).join('\n')

/** The fixed markers every other read path shows (src/protocol/results.rs author_markers). */
export function markers(it) {
  let out = ''
  if (Number.isInteger(it.priority) && it.priority !== 1 && it.priority >= 0 && it.priority <= 9) out += ` [p${it.priority}]`
  if (it.author_role === 'human') out += ' [human]'
  if (it.relays_user === true) out += ' [relays user]'
  if (INTENTS.has(it.user_intent)) out += ` [${it.user_intent}]`
  return out
}

/** `<id> "<name>"`: the name quoted on the id's line so it cannot pass for the id or a marker; a name equal to the id is not repeated. */
const label = (id, name) => {
  const i = id ? oneLine(id) : ''
  const n = name ? oneLine(name) : ''
  if (!n || n === i) return i || 'unknown'
  return i ? `${i} ${JSON.stringify(n)}` : JSON.stringify(n)
}

/**
 * Frames peer text so a body can never pass for a header line (spec D4), with the thread and sender
 * names (quoted, after their ids) and the source and intent markers after the
 * service-generated fields; never starts with '/'. Body lines are indented
 * two spaces; only header lines start at column 0.
 */
export function frame(items) {
  const blocks = items.map((it) =>
    it.kind === 'attention'
      ? `[herdr-threads] attention ${oneLine(it.id)}:\n${indent(it.body)}`
      : `[herdr-threads] ${it.kind === 'lazy' ? 'message' : oneLine(it.kind)} ${oneLine(it.id)} in ${label(it.thread, it.threadName)} from ${label(it.sender, it.senderName)}${markers(it)}:\n${indent(it.body)}`,
  )
  return `${HEADER}\n\n${blocks.join('\n\n')}`
}

/**
 * The batch order key, mirroring Rust `src/delivery/assemble.rs` `batch_key`: priority asc (absent: 1 for a
 * `lazy` row, else 0, so a priority-less message, an ACK, an invitation or a notice is p0), then deadline asc
 * with none last. Arrival breaks every remaining tie because `Array.prototype.sort` is stable.
 */
const orderKey = (it) => [it.priority ?? (it.kind === 'lazy' ? 1 : 0), it.deadline ?? Infinity]
export const batchOrder = (items) => [...items].sort((a, b) => {
  const [pa, da] = orderKey(a)
  const [pb, db] = orderKey(b)
  return pa !== pb ? pa - pb : da === db ? 0 : da < db ? -1 : 1
})

const shellArg = (arg) => /^[a-zA-Z0-9_@%+=:,./-]+$/.test(arg)
  ? arg : "'" + arg.replace(/'/g, "'\\''") + "'"

/** A bounded complete prefix, with recovery included in the rendered byte budget.
 * An individually oversized item stays queued but yields later opportunities once
 * its recovery was emitted. Reconnect discards this local hint with the queue.
 */
function appendBatch(items, argv) {
  const batch = []
  batch.deferred = []
  const encoder = new TextEncoder()
  const recovery = 'next: ' + [...argv, 'inbox'].map(shellArg).join(' ')
  const render = (selected, more) => {
    const truncated = selected.some((it) => it.truncated)
    const framed = selected.length ? frame(selected.map((it) => {
      if (!it.truncated || (it.kind !== 'message' && it.kind !== 'lazy')) return it
      // The watch marker is service-authored and last; replace its per-body
      // recipe with the single batch continuation below.
      const cut = it.body.lastIndexOf('…truncated;')
      return cut < 0 ? it : { ...it, body: it.body.slice(0, cut) + '…truncated' }
    })) : ''
    return framed + (more || truncated ? (framed ? '\n\n' : '') + recovery : '')
  }
  for (const item of items) {
    if (batch.length === APPEND_MAX_ITEMS
        || encoder.encode(render([...batch, item], batch.length + 1 < items.length)).length > APPEND_MAX_BYTES) {
      if (!batch.length) batch.deferred.push(item)
      break
    }
    batch.push(item)
  }
  batch.text = render(batch, batch.length < items.length)
  return batch
}

const emptyRec = () => ({ delivered: {}, unacked: {}, rows: {}, attentionVersions: [], invitationVersions: {} })

// The id of the row the engine stored for an append (`{ message, uuid }`): the admission evidence the
// ACK carries (`watch ack --row`). A result without one still settles, as before, with no row evidence.
// Mirrors `mod_ack_row_valid` (src/protocol/commands.rs); a CLI that refuses a row anyway gets a row-less retry.
const rowOf = (r) => typeof r.uuid === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(r.uuid) ? r.uuid : null

/** A loaded record, its rows kept only for ids still unacked. */
const loadRec = (rec) => {
  if (!rec || typeof rec !== 'object') return emptyRec()
  const out = { ...emptyRec(), ...rec }
  out.rows = Object.fromEntries(Object.entries(out.rows || {}).filter(([id]) => out.unacked[id] !== undefined))
  return out
}

// Esc at a permission dialog (ht-j16.30, live stress D3): the engine ends that turn with a
// non-aborted turn.complete, so the hold cannot key on isAborted alone. On 2.1.309 a user
// reject and an ordinary failing tool reach tool.call alike (isError, no deny); only what
// follows differs: a failing tool is followed by a main turn.step, a reject by turn.complete
// (ht-mod-busy probe). So an unanswered last main result counts only if no main step followed.
const userRejected = (r) => !!r && (r.deny !== undefined || r.isError === true)

export function createCore(io) {
  const S = {
    inert: false,
    disposed: false,
    sid: null,
    rec: emptyRec(),
    turns: { open: [], assumedBusy: false, abortHoldSince: null, submitting: null, wake: null, provisional: [] },
    loadedAt: 0,
    sawStart: false,
    turnRejected: false,
    queue: [],
    // Batches attached to an isError main result ({ batch, turn }): settled by the next main turn.step of
    // that turn, released by a main turn boundary first (a user reject ends the turn and its context is never
    // read). Their ids are also kept in turns.provisional, so a reload's successor settles or releases them.
    provisional: [],
    // A predecessor's turns.provisional records ({ sid, turn, ids, ackable }), adopted at load.
    heldProv: [],
    submitInflight: false,
    appendInflight: false,
    submitBackoffUntil: 0,
    appendBackoffUntil: 0,
    pred: null,
    draftSince: null,
    lastHeld: '',
    child: null,
    run: 0,
    live: false,
    runAttention: new Set(),
    connectedAt: null,
    stopped: false,
    stopReason: null,
    restartAt: null,
    backoffIdx: 0,
    pendingRefresh: null,
    endReason: null,
    sidSuspect: false,
    gen: 0,
    ticking: false,
    pumping: false,
    lastAckRetry: 0,
    acking: new Set(),
    ackChain: Promise.resolve(),
    saveChain: Promise.resolve(),
    stateChain: Promise.resolve(),
  }

  const cmd = (...rest) => [...(io.argv || [io.bin || 'herdr-threads']), ...rest]
  const key = (sid) => `delivered:${sid}`
  const busy = () => S.turns.open.length > 0 || S.turns.assumedBusy || S.pred != null
  const predHas = (id) => (S.pred != null && S.pred.ids.includes(id)) || S.heldProv.some((h) => h.ids.includes(id))

  function ledger(kind, fields = {}) {
    const turn = S.turns.open.length ? S.turns.open[S.turns.open.length - 1] : null
    try {
      io.ledger({
        at: io.now(),
        kind,
        ids: fields.ids ?? [],
        via: fields.via ?? null,
        turn: fields.turn ?? turn,
        reason: fields.reason ?? null,
        ...(fields.row ? { row: fields.row } : {}),
      })
    } catch {}
  }

  function saveRec() {
    if (!S.sid) return
    const k = key(S.sid)
    const snapshot = JSON.parse(JSON.stringify(S.rec))
    S.saveChain = S.saveChain.then(() => io.store.set(k, snapshot)).catch(() => {})
  }

  function persistTurns() {
    const snapshot = JSON.parse(JSON.stringify(S.turns))
    S.stateChain = S.stateChain.then(() => io.state.set(snapshot)).catch(() => {})
  }

  // ---- child lifecycle -------------------------------------------------

  /** The watch run is gone: forget every queued item that is not in flight. */
  function dropQueued(why) {
    S.live = false
    const dropped = S.queue.filter((q) => !q.inflight).map((q) => q.id)
    S.queue = S.queue.filter((q) => q.inflight)
    S.draftSince = null
    S.lastHeld = ''
    if (dropped.length) ledger('refused', { ids: dropped, reason: `channel_lost:${why}` })
  }

  /** Removes items whose run ended while their call was in flight. */
  function discardStale(items, why) {
    const stale = items.filter((it) => it.run !== S.run || !S.live)
    if (!stale.length) return items
    // Invitation IDs outlive their context: retire only the obsolete queued object.
    const obsolete = new Set(stale)
    const gone = new Set(stale.filter((it) => it.kind !== 'invitation').map((it) => it.id))
    const retain = (it) => !obsolete.has(it) && !gone.has(it.id)
    S.queue = S.queue.filter(retain)
    ledger('refused', { ids: stale.map((it) => it.id), reason: `channel_lost:${why}` })
    return items.filter(retain)
  }

  function stopChild() {
    S.run++
    const c = S.child
    S.child = null
    S.connectedAt = null
    S.live = false
    if (c) {
      try {
        c.stop()
      } catch {}
    }
  }

  function startChild() {
    if (S.disposed || S.inert || !S.sid) return
    S.run++
    const run = S.run
    S.runAttention = new Set()
    S.connectedAt = null
    S.live = false
    S.restartAt = null
    S.stopReason = null
    try {
      S.child = io.spawn(cmd('watch', '--harness', 'claude', '--session', S.sid), {
        line: (o) => onLine(o, run),
        exit: (c) => onChildExit(c, run),
      })
    } catch (err) {
      if (err && err.apiMissing) return goInert()
      S.child = { stop() {} }
      onChildExit(1, run)
    }
  }

  function onChildExit(code, run) {
    if (run !== undefined && run !== S.run) return
    S.run++
    S.child = null
    if (S.disposed) return
    dropQueued(`exit:${code}`)
    ledger('restart', { reason: `exit:${code}` })
    const wasConnected = S.connectedAt
    S.connectedAt = null
    // Older binaries also used 3 for generic CLI availability errors before
    // watch started. Only this run's explicit terminal status can stop us.
    if (code === 3 && S.stopReason) {
      S.stopped = true
      return
    }
    if (wasConnected != null && io.now() - wasConnected >= CONNECTED_RESET_MS) S.backoffIdx = 0
    const delay = BACKOFF_S[Math.min(S.backoffIdx, BACKOFF_S.length - 1)]
    S.backoffIdx++
    S.restartAt = io.now() + delay * 1000
  }

  // ---- stream lines ----------------------------------------------------

  function onLine(o, run) {
    if (run !== undefined && run !== S.run) return
    if (S.inert || !o || typeof o !== 'object' || typeof o.id !== 'string') return
    if (o.kind === 'status') {
      if (o.state === 'connected') {
        S.stopReason = null
        S.sidSuspect = false
        S.connectedAt = io.now()
        S.lastAckRetry = io.now()
        S.live = true
        retryAcks()
        void pumpLazy()
        void pump()
      } else if (o.state === 'closing' || o.state === 'refused') {
        S.stopReason = o.exit === 3 &&
          ['not_claude', 'disabled', 'replaced', 'no_pane', 'env_disabled', 'unsupported'].includes(o.reason)
          ? o.reason : null
        if (
          (o.state === 'refused' && o.reason === 'session_mismatch') ||
          (o.state === 'closing' && o.reason === 'binding_changed')
        )
          S.sidSuspect = true
        dropQueued(`${o.state}:${o.reason ?? ''}`)
      }
      return
    }
    const hasEpoch = Object.prototype.hasOwnProperty.call(o, 'presentation_epoch')
    if ((o.kind === 'attention' || o.kind === 'attention_cleared') && hasEpoch
        && !canonicalEpoch(o.presentation_epoch)) return
    if (o.kind === 'attention_cleared') {
      // The watch found nothing pending that an earlier attention line pointed at (ht-j16.33): an unsent
      // attention item is stale. One already in a submit or attached as context is not recalled.
      const retract = (q) => q.kind === 'attention' && !q.inflight
        && (!hasEpoch || q.presentationEpoch === o.presentation_epoch)
      const gone = S.queue.filter(retract).map((q) => q.id)
      S.queue = S.queue.filter((q) => !retract(q))
      if (hasEpoch) S.runAttention.delete(`attention:${o.presentation_epoch}`)
      else S.runAttention = new Set()
      if (gone.length) {
        S.lastHeld = ''
        ledger('refused', { ids: gone, reason: 'attention_cleared' })
      }
      return
    }
    if (o.kind !== 'message' && o.kind !== 'lazy' && o.kind !== 'invitation' && o.kind !== 'attention' && o.kind !== 'notice') return
    ledger('received', { ids: [o.id] })
    if (o.kind === 'attention') {
      const v = o.attention_version
      const dup = S.queue.find((q) => q.id === o.id)
      if (dup) dup.run = S.run
      if (S.runAttention.has(o.id) || dup) {
        retryAcks()
        return
      }
      S.queue.push({
        id: o.id,
        kind: 'attention',
        body: String(o.text ?? ''),
        version: v,
        presentationEpoch: hasEpoch ? o.presentation_epoch : null,
        run: S.run,
        ackable: false,
      })
    } else {
      const revision = o.topic_revision == null ? null : String(o.topic_revision)
      // Retain historical scalar keys; modern contexts include exact decimal epochs.
      const contextVersion = o.command_context_epoch == null
        ? revision : JSON.stringify([revision, String(o.command_context_epoch)])
      if (o.kind === 'invitation') {
        if (Object.prototype.hasOwnProperty.call(S.rec.invitationVersions, o.id)
            && S.rec.invitationVersions[o.id] === contextVersion) return
      } else if (S.rec.delivered[o.id] !== undefined) {
        if (S.rec.unacked[o.id] !== undefined) retryAcks()
        return
      }
      const dup = S.queue.find((q) => q.id === o.id && (o.kind !== 'invitation'
        || ((q.run === S.run || !q.inflight) && q.contextVersion === contextVersion
          && (!q.truncated || o.truncated === true))))
      if (dup) {
        dup.run = S.run
        return
      }
      // An append owns its exact object. Only the newest pending context is useful;
      // the duplicate check above also prevents a same-version whole-to-partial downgrade.
      if (o.kind === 'invitation') {
        S.queue = S.queue.filter((q) => q.id !== o.id || q.inflight)
      }
      const truncated = o.kind !== 'notice' && o.truncated === true
      S.queue.push({
        id: o.id,
        kind: o.kind,
        thread: o.thread || 'unknown',
        threadName: o.thread_name ?? null,
        sender: o.sender ?? null,
        senderName: o.sender_name ?? null,
        author_role: o.author_role ?? null,
        relays_user: o.relays_user === true,
        user_intent: o.user_intent ?? null,
        body: String(o.body ?? ''),
        truncated,
        wakes: o.kind !== 'notice' && o.wakes === true,
        ackRequired: o.ack_required === true,
        priority: Number.isInteger(o.priority) ? o.priority : undefined,
        deadline: Number.isFinite(o.deadline_at) ? o.deadline_at : undefined,
        contextVersion,
        run: S.run,
        ackable: !truncated && (o.kind === 'lazy' || o.kind === 'message'),
      })
    }
    retryAcks()
    void pumpLazy()
    void pump()
  }

  // ---- delivery --------------------------------------------------------

  /** A refused append is pending again; discard it if a newer context replaced it. */
  function releaseBatch(batch) {
    for (const it of batch) it.inflight = false
    const newest = new Map()
    for (const q of S.queue) {
      if (q.kind === 'invitation' && !q.inflight) newest.set(q.id, q)
    }
    S.queue = S.queue.filter((q) => q.kind !== 'invitation' || q.inflight || newest.get(q.id) === q)
  }

  /** Marks `items` delivered through `via` (an append's stored `row`, if any), then acks the ackable ones. */
  function complete(items, via, row = null) {
    const ids = items.map((i) => i.id)
    for (const it of items) {
      // A successful whole append also settles an identical pending replay from a
      // successor run. A partial output, newer context or other inflight object does not.
      S.queue = S.queue.filter((q) => it.kind === 'invitation'
        ? q !== it && (it.truncated || q.inflight || q.kind !== 'invitation'
          || q.id !== it.id || q.contextVersion !== it.contextVersion)
        : q.id !== it.id)
      if (it.kind === 'attention') {
        S.runAttention.add(it.id)
        if (it.version !== undefined && !S.rec.attentionVersions.includes(it.version)) {
          S.rec.attentionVersions.push(it.version)
          if (S.rec.attentionVersions.length > 64) S.rec.attentionVersions.shift()
        }
      } else if (it.kind === 'invitation') {
        if (!it.truncated) {
          S.rec.invitationVersions[it.id] = it.contextVersion
          const ids = Object.keys(S.rec.invitationVersions)
          if (ids.length > 1024) delete S.rec.invitationVersions[ids[0]]
        }
      } else {
        S.rec.delivered[it.id] = via
        if (it.ackable) {
          S.rec.unacked[it.id] = via
          // Only an ordinary message's receipt records the row; a lazy completion keeps none.
          if (row && it.kind === 'message') S.rec.rows[it.id] = row
          else delete S.rec.rows[it.id]
        }
      }
    }
    saveRec()
    S.lastHeld = ''
    ledger('delivered', { ids, via, row })
    if (via !== 'submit') {
      // Claude Code already prefixes mod log lines with the plugin name.
      try {
        io.log(`delivered ${ids.length} message(s)`)
      } catch {}
    }
    const ackable = items.filter((i) => i.ackable).map((i) => i.id)
    // The context path is delivered once the hook has returned its result.
    if (via === 'context') return Promise.resolve().then(() => ackIds(ackable))
    return ackIds(ackable)
  }

  function completeBatch(batch, via, row = null) {
    for (const item of batch.deferred) item.recoveryOffered = true
    if (!batch.length) return Promise.resolve()
    return complete(batch, via, row)
  }

  function ackIds(ids) {
    const groups = {}
    for (const id of ids) {
      const via = S.rec.unacked[id]
      if (via === undefined || S.acking.has(id)) continue
      S.acking.add(id)
      const row = S.rec.rows[id] ?? null
      const group = (groups[`${via}\n${row ?? ''}`] ||= { via, row, ids: [] })
      group.ids.push(id)
    }
    const sid = S.sid
    const rec = S.rec
    const jobs = []
    for (const { via, row, ids: all } of Object.values(groups)) {
      for (let at = 0; at < all.length; at += ACK_BATCH_MAX) {
        const chunk = all.slice(at, at + ACK_BATCH_MAX)
        const job = S.ackChain.then(() => doAck(via, row, chunk, sid, rec))
        S.ackChain = job.catch(() => {})
        jobs.push(job)
      }
    }
    return Promise.all(jobs).then(() => {})
  }

  async function doAck(via, row, ids, sid, rec) {
    let code = 1
    let stdout = ''
    try {
      const r = await io.run(cmd('watch', 'ack', '--session', sid, ...(row ? ['--row', row] : []), '--via', via, ...ids))
      code = r.code
      stdout = r.stdout || ''
    } catch {}
    const byId = {}
    for (const line of stdout.split('\n')) {
      if (!line.trim()) continue
      try {
        const o = JSON.parse(line)
        if (o && typeof o.id === 'string' && typeof o.result === 'string') byId[o.id] = o.result
      } catch {}
    }
    const live = S.rec === rec && !S.disposed
    // Exit 2 is the CLI refusing its arguments (an older binary without `--row`): the ids
    // go again without the row, as before this evidence existed.
    const rowRefused = row !== null && code === 2
    for (const id of ids) {
      S.acking.delete(id)
      if (!live) continue
      if (rowRefused) delete rec.rows[id]
      const result = code === 0 ? (byId[id] ?? 'retryable') : 'retryable'
      if (result === 'settled' || result === 'already_settled') {
        delete rec.unacked[id]
        delete rec.rows[id]
        ledger('acked', { ids: [id], via, row, reason: result })
      } else if (result === 'refused_terminal' || result === 'stale_generation') {
        delete rec.unacked[id]
        delete rec.rows[id]
        if (result === 'stale_generation') delete rec.delivered[id]
        ledger('refused', { ids: [id], via, reason: `ack:${result}` })
      }
    }
    if (live) saveRec()
    if (live && rowRefused) {
      ledger('refused', { ids, via, row, reason: 'ack:row_refused' })
      void ackIds(ids)
    }
  }

  function retryAcks() {
    if (S.inert || !S.sid) return
    void ackIds(Object.keys(S.rec.unacked))
  }

  function held(reason, ids) {
    const k = `${reason}|${ids.join(',')}`
    if (S.lastHeld === k) return
    S.lastHeld = k
    ledger('held', { ids, reason })
  }

  /**
   * Idle path (R18): the framed batch is appended as hidden rows (no turn), then the content-free wake
   * prompt is submitted; the turn that wake starts sees the rows. Never while a main turn is open.
   */
  async function pump() {
    if (S.inert || S.disposed || S.pumping || !S.live) return
    S.pumping = true
    try {
      // a wake recorded for another session belongs to nobody now
      if (S.turns.wake && S.turns.wake.sid !== S.sid) {
        S.turns.wake = null
        persistTurns()
      }
      const items = S.queue.filter((q) => !passive(q) && !q.inflight && q.run === S.run && !predHas(q.id))
      if (!(items.length || S.turns.wake) || S.submitInflight) return
      if (busy()) return
      const ids = items.map((i) => i.id)
      if (io.now() < S.submitBackoffUntil) return held('backoff', ids)
      if (S.turns.abortHoldSince != null) return held('post_abort', ids)
      let box = { text: '' }
      try {
        box = await io.promptRead()
      } catch {}
      if (S.disposed || !S.live || S.submitInflight || busy()) return
      if (S.turns.abortHoldSince != null) return held('post_abort', ids)
      if (box && box.text !== '') {
        if (S.draftSince == null) S.draftSince = io.now()
        if (io.now() - S.draftSince < DRAFT_WAIT_MS) return held('draft', ids)
      }
      S.draftSince = null
      const batch = appendBatch(batchOrder(S.queue.filter((q) => !passive(q) && !q.inflight && !q.recoveryOffered && q.run === S.run && !predHas(q.id))), cmd())
      if (!batch.text && !S.turns.wake) return
      const gen = S.gen
      for (const it of batch) it.inflight = true
      S.submitInflight = true
      void deliverIdle(batch, gen)
    } finally {
      S.pumping = false
    }
  }

  /** Whether the idle gates still hold after an await; a bail leaves the wake (if any) pending for the next pump. */
  const idleOpen = (gen) => !S.disposed && gen === S.gen && S.live && !busy() && S.turns.abortHoldSince == null

  async function deliverIdle(batch, gen) {
    const bids = batch.map((i) => i.id)
    if (batch.text) {
      let p
      try {
        p = Promise.resolve(io.append(batch.text))
      } catch (err) {
        p = Promise.reject(err)
      }
      const r = await p.then(
        (v) => v || {},
        (err) => (err && err.apiMissing ? { missing: true } : { deny: `error:${String(err)}` }),
      )
      // a disposed core touches neither the record, the store nor acks
      if (S.disposed) return
      // a resolution from before a session change touches nothing
      if (gen !== S.gen) return
      releaseBatch(batch)
      if (r.missing) {
        S.submitInflight = false
        return goInert()
      }
      if (r.deny !== undefined) {
        S.submitInflight = false
        discardStale(batch, 'late_deny')
        S.appendBackoffUntil = io.now() + DROP_BACKOFF_MS
        S.submitBackoffUntil = S.appendBackoffUntil
        ledger('refused', { ids: bids, via: 'append', reason: `deny:${r.deny}` })
        return
      }
      // The append is the delivery: the items are settled by it, and the wake below only gives them a turn.
      void completeBatch(batch, 'append', rowOf(r))
      S.turns.wake = { sid: S.sid, at: io.now() }
      persistTurns()
      await S.stateChain
    }
    if (S.disposed || gen !== S.gen) return
    // Every gate the write may have changed is checked again. A bail submits nothing; the wake stays pending.
    // A main turn.start clears the wake: any turn sees the appended rows.
    if (!S.turns.wake || !idleOpen(gen)) {
      S.submitInflight = false
      if (S.turns.wake && !S.disposed && gen === S.gen) held(busy() ? 'busy' : 'post_abort', bids)
      return
    }
    const wake = S.turns.wake
    ledger('submit', { ids: [], via: 'submit', reason: 'wake' })
    let q
    try {
      q = Promise.resolve(io.submit(WAKE_PROMPT))
    } catch (err) {
      q = Promise.reject(err)
    }
    const r = await q.then(
      (v) => v || {},
      (err) => (err && err.apiMissing ? { missing: true } : { drop: `error:${String(err)}` }),
    )
    if (S.disposed || gen !== S.gen) return
    S.submitInflight = false
    if (r.missing) return goInert()
    if (r.drop !== undefined) {
      S.submitBackoffUntil = io.now() + DROP_BACKOFF_MS
      ledger('refused', { ids: [], via: 'submit', reason: `wake_drop:${r.drop}` })
    } else if (S.turns.wake === wake) {
      S.turns.wake = null
      persistTurns()
    }
    void pump()
  }

  // Passive or waking follows the daemon's `wakes` (priority <= the seat's threshold) and ACK need only.
  const passive = (item) => item.kind === 'notice' || (item.kind === 'invitation' && !item.wakes)
    || ((item.kind === 'message' || item.kind === 'lazy') && !item.wakes && item.ackRequired !== true)

  /** Passive items are appended at once, busy or idle, without a wake. */
  async function pumpLazy() {
    if (S.inert || S.disposed || S.appendInflight || !S.live) return
    if (io.now() < S.appendBackoffUntil) return
    const batch = appendBatch(batchOrder(S.queue.filter((q) => passive(q) && !q.inflight && !q.recoveryOffered && q.run === S.run)), cmd())
    if (!batch.text) return
    const gen = S.gen
    for (const it of batch) it.inflight = true
    S.appendInflight = true
    const ids = batch.map((i) => i.id)
    let p
    try {
      p = Promise.resolve(io.append(batch.text))
    } catch (err) {
      p = Promise.reject(err)
    }
    void p
      .then(
        (r) => r || {},
        (err) => (err && err.apiMissing ? { missing: true } : { deny: `error:${String(err)}` }),
      )
      .then((r) => {
        if (gen !== S.gen) return
        S.appendInflight = false
        releaseBatch(batch)
        if (r.missing) return goInert()
        if (r.deny !== undefined) {
          discardStale(batch, 'late_deny')
          S.appendBackoffUntil = io.now() + DROP_BACKOFF_MS
          ledger('refused', { ids, via: 'append', reason: `deny:${r.deny}` })
          return
        }
        return completeBatch(batch, 'append', rowOf(r))
      })
      .then(() => pumpLazy())
  }

  // ---- predecessor submit (reload mid-submit, ht-j16.29) -----------------

  /** Ids of the mod frames in a prompt, attention blocks included: only the mod's own header lines start at column 0. */
  function framedIds(text) {
    return Array.from(
      String(text ?? '').matchAll(/^\[herdr-threads\] (?:(?:message|lazy) (\S+) in |attention (\S+):$)/gm),
      (m) => m[1] ?? m[2],
    )
  }

  /** The predecessor's submit produced a turn that carried its ids: mark them delivered via submit and ack. */
  // why: 'turn_start' (rule a) or 'turn_complete' (rule b); kept for readers only
  function settlePred(why) {
    const pred = S.pred
    if (!pred) return
    S.pred = null
    if (pred.sid !== S.sid) return void pump()
    void why
    const gone = new Set(pred.ids)
    S.queue = S.queue.filter((q) => !gone.has(q.id))
    // A submit that resolved just before dispose was already recorded (and its
    // ack sent or kept in unacked) by the predecessor: record only the rest.
    const fresh = pred.ids.filter((id) => S.rec.delivered[id] === undefined)
    if (!fresh.length) return void pump()
    for (const id of fresh) {
      S.rec.delivered[id] = 'submit'
      if (pred.ackable.includes(id)) S.rec.unacked[id] = 'submit'
    }
    saveRec()
    ledger('delivered', { ids: fresh, via: 'submit', reason: 'predecessor_submit' })
    void ackIds(pred.ackable.filter((id) => fresh.includes(id)))
    void pump()
  }

  /** The predecessor's submit produced no turn: its ids are delivered normally when streamed. */
  function releasePred(reason = 'predecessor_no_turn') {
    const pred = S.pred
    if (!pred) return
    S.pred = null
    ledger('refused', { ids: pred.ids, reason })
    void pump()
  }

  // ---- events ----------------------------------------------------------

  function goInert() {
    if (S.inert) return
    S.inert = true
    stopChild()
    ledger('refused', { reason: 'api_missing' })
  }

  async function onLoad() {
    S.loadedAt = io.now()
    if (!(await io.apis())) return goInert()
    let recorded
    try {
      recorded = await io.state.get()
    } catch {}
    if (recorded && Array.isArray(recorded.open)) {
      S.turns = {
        open: recorded.open.slice(),
        assumedBusy: recorded.assumedBusy === true,
        abortHoldSince: recorded.abortHoldSince ?? null,
        submitting: null,
        wake: recorded.wake && typeof recorded.wake === 'object' && recorded.wake.sid ? recorded.wake : null,
        provisional: [],
      }
    } else {
      S.turns = { open: [], assumedBusy: true, abortHoldSince: null, submitting: null, wake: null, provisional: [] }
      persistTurns()
    }
    const priorProv = recorded && Array.isArray(recorded.provisional) ? recorded.provisional : []
    const prior = recorded && recorded.submitting
    const hasPrior = prior && typeof prior === 'object' && prior.sid && Array.isArray(prior.ids) && prior.ids.length > 0
    await loadSession()
    S.heldProv = priorProv.filter((h) => h && h.sid === S.sid && Array.isArray(h.ids))
    if (priorProv.length) persistTurns()
    if (hasPrior) {
      if (prior.sid === S.sid) S.pred = { ...prior, ackable: Array.isArray(prior.ackable) ? prior.ackable : [], issued: prior.issued !== false, since: io.now(), sawStart: false }
      // the record now belongs to this core as S.pred
      persistTurns()
    }
    startChild()
  }

  async function loadSession(fresh = false) {
    const sid = await io.sessionId()
    S.sid = sid
    let rec = null
    if (!fresh) {
      try {
        rec = await io.store.get(key(sid))
      } catch {}
    }
    S.rec = loadRec(rec)
  }

  /** Re-reads the engine's session id; adopts a changed one per the last session.end reason. */
  async function syncSession() {
    let sid
    try {
      sid = await io.sessionId()
    } catch {
      return false
    }
    if (!sid || sid === S.sid) return false
    const reason = S.endReason
    S.endReason = null
    S.sid = sid
    let rec = null
    try {
      rec = reason === 'clear' ? null : await io.store.get(key(sid))
    } catch {}
    S.rec = loadRec(rec)
    S.acking = new Set()
    S.sidSuspect = false
    return true
  }

  // A batch with no turn id (attached while assumed busy) belongs to whichever main turn steps next.
  const sameTurn = (turn, e) => turn == null || turn === e.turnId

  function recordProvisional() {
    S.turns.provisional = S.provisional.map(({ batch, turn }) => ({
      sid: S.sid,
      turn,
      ids: batch.filter((i) => i.kind === 'message' || i.kind === 'lazy').map((i) => i.id),
      ackable: batch.filter((i) => i.ackable).map((i) => i.id),
    })).filter((h) => h.ids.length)
    persistTurns()
  }

  /** A main step began: context attached to an errored result of this turn rides its request. */
  function onTurnStep(e) {
    if (S.inert || !e || e.agentId) return
    S.turnRejected = false
    const entries = S.provisional
    const held = S.heldProv
    if (!entries.length && !held.length) return
    S.provisional = []
    S.heldProv = []
    for (const { batch, turn } of entries) {
      if (!sameTurn(turn, e)) releaseOne(batch)
      else {
        for (const it of batch) it.inflight = false
        void completeBatch(batch, 'context')
      }
    }
    // A predecessor attached these: record them as its delivery, as settlePred does for a submit.
    const fresh = []
    const ackable = []
    for (const h of held) {
      if (!sameTurn(h.turn, e)) continue
      for (const id of h.ids) {
        if (S.rec.delivered[id] !== undefined || fresh.includes(id)) continue
        fresh.push(id)
        S.rec.delivered[id] = 'context'
        if (h.ackable.includes(id)) {
          S.rec.unacked[id] = 'context'
          ackable.push(id)
        }
      }
    }
    if (fresh.length) {
      S.queue = S.queue.filter((q) => !fresh.includes(q.id))
      saveRec()
      ledger('delivered', { ids: fresh, via: 'context', reason: 'predecessor_context' })
      void ackIds(ackable)
    }
    recordProvisional()
  }

  /** No main step of its turn followed an errored result's context: it was never read, so its items are pending again. */
  function releaseOne(batch) {
    releaseBatch(batch)
    const left = discardStale(batch, 'no_step')
    if (left.length) ledger('refused', { ids: left.map((i) => i.id), via: 'context', reason: 'no_step' })
  }

  function releaseProvisional() {
    if (!S.provisional.length && !S.heldProv.length) return
    const entries = S.provisional
    S.provisional = []
    S.heldProv = []
    for (const { batch } of entries) releaseOne(batch)
    recordProvisional()
  }

  function onTurnStart(e) {
    if (S.inert || !e || e.agentId) return
    releaseProvisional()
    S.sawStart = true
    S.turnRejected = false
    S.turns.open = [e.turnId]
    // any turn sees the appended rows: a pending wake has nothing left to do
    S.turns.wake = null
    persistTurns()
    const framed = framedIds(e.text)
    if (S.pred) {
      S.pred.sawStart = true
      S.pred.since = io.now()
      if (S.pred.ids.every((id) => framed.includes(id))) settlePred('turn_start')
    }
    const sub = S.turns.submitting
    if (sub && sub.ids.every((id) => framed.includes(id))) {
      sub.turnId = e.turnId
      persistTurns()
    }
  }

  function onTurnComplete(e) {
    if (S.inert || !e || e.agentId) return
    releaseProvisional()
    if (S.pred && (!S.pred.sawStart || S.pred.turnId === e.turnId)) {
      // rule (b) claims a delivery only for a submit that was issued (ht-j16.31)
      if (S.pred.issued) settlePred('turn_complete')
      else releasePred('predecessor_not_issued')
    }
    const i = S.turns.open.indexOf(e.turnId)
    if (i >= 0) S.turns.open = S.turns.open.slice(i + 1)
    S.turns.assumedBusy = false
    const interrupted = e.isAborted === true || e.reason === 'aborted' || S.turnRejected
    S.turnRejected = false
    S.turns.abortHoldSince = interrupted ? io.now() : null
    persistTurns()
    void pump()
  }

  // ctx.foreign: another plugin's own $.tool.call, whose result the engine gives no context;
  // ctx.aborted: the dispatch was abandoned (an interrupt), so no request will read this result.
  async function onToolCall(e, result, ctx = {}) {
    if (S.inert || !e || e.agentId || ctx.foreign) return result
    S.turnRejected = userRejected(result)
    if (!busy()) return result
    const answered =
      result && result.result !== undefined && result.deny === undefined && result.isError !== true
    // An errored result carries context too (a deny cannot), but it is settled only by the next main step.
    const errored = !!result && result.isError === true && result.deny === undefined
    if (!(answered || (errored && !ctx.aborted)) || !S.live) return result
    const batch = appendBatch(batchOrder(S.queue.filter((q) => !passive(q) && !q.inflight && !q.recoveryOffered && q.run === S.run && !predHas(q.id))), cmd())
    if (!batch.text) return result
    const output = { ...result, context: [...(result.context ?? []), batch.text] }
    if (answered) {
      void completeBatch(batch, 'context')
    } else {
      for (const it of batch) it.inflight = true
      S.provisional.push({ batch, turn: S.turns.open.length ? S.turns.open[S.turns.open.length - 1] : null })
      recordProvisional()
    }
    return output
  }

  function onSessionEnd(reason) {
    if (S.inert || S.disposed) return
    stopChild()
    S.gen++
    S.submitInflight = false
    S.appendInflight = false
    S.provisional = []
    S.heldProv = []
    S.turns.provisional = []
    for (const it of S.queue) it.inflight = false
    dropQueued(`session_end:${reason}`)
    S.stopped = false
    S.restartAt = null
    S.backoffIdx = 0
    S.pendingRefresh = reason
    S.endReason = reason
    if (reason === 'clear') {
      const old = S.sid
      S.rec = emptyRec()
      S.queue = []
      S.acking = new Set()
      if (old) S.saveChain = S.saveChain.then(() => io.store.delete(key(old))).catch(() => {})
    }
  }

  async function refresh() {
    S.pendingRefresh = null
    await syncSession()
    startChild()
  }

  async function onTick() {
    if (S.inert || S.disposed || S.ticking) return
    S.ticking = true
    try {
      const now = io.now()
      if (S.pendingRefresh) await refresh()
      else if (!S.child && !S.stopped && S.restartAt != null) {
        const changed = S.sidSuspect || now >= S.restartAt ? await syncSession() : false
        if (S.disposed || S.child) {
          // nothing: disposed or started meanwhile
        } else if (changed) {
          S.backoffIdx = 0
          startChild()
        } else if (now >= S.restartAt) startChild()
      }
      if (S.turns.assumedBusy && !S.sawStart && now - S.loadedAt >= ASSUMED_BUSY_IDLE_MS) {
        try {
          await io.promptRead()
          if (!S.sawStart) {
            S.turns.assumedBusy = false
            persistTurns()
          }
        } catch {}
      }
      if (S.turns.abortHoldSince != null && S.turns.open.length === 0 && now - S.turns.abortHoldSince >= HOLD_IDLE_MS) {
        try {
          const box = await io.promptRead()
          if (box.text === '' && S.turns.open.length === 0) {
            S.turns.abortHoldSince = null
            persistTurns()
          }
        } catch {}
      }
      if (S.pred && S.turns.open.length === 0 && now - S.pred.since >= HOLD_IDLE_MS) {
        try {
          const box = await io.promptRead()
          if (box.text === '' && S.pred && S.turns.open.length === 0 && io.now() - S.pred.since >= HOLD_IDLE_MS) releasePred()
        } catch {}
      }
      if (S.connectedAt != null && now - S.lastAckRetry >= ACK_RETRY_MS) {
        S.lastAckRetry = now
        retryAcks()
      }
      await pumpLazy()
      await pump()
    } finally {
      S.ticking = false
    }
  }

  function dispose() {
    S.disposed = true
    stopChild()
  }

  return {
    onLoad,
    onTurnStart,
    onTurnStep,
    onTurnComplete,
    onToolCall,
    onLine,
    onChildExit,
    onSessionEnd,
    onTick,
    dispose,
    snapshot: () => ({
      turns: JSON.parse(JSON.stringify(S.turns)),
      queue: S.queue.map((q) => q.id),
      rec: JSON.parse(JSON.stringify(S.rec)),
      sid: S.sid,
      pred: S.pred ? { ids: S.pred.ids.slice(), sawStart: S.pred.sawStart, issued: S.pred.issued } : null,
      inert: S.inert,
      live: S.live,
      stopped: S.stopped,
      childRunning: S.child != null,
      restartAt: S.restartAt,
    }),
  }
}

// ---- engine wiring -------------------------------------------------------

/** A TypeError from calling a member the engine lacks means the API is missing. */
function guard(fn) {
  try {
    const r = fn()
    return r && typeof r.catch === 'function' ? r.catch(rethrow) : r
  } catch (err) {
    return rethrow(err)
  }
}
function rethrow(err) {
  if (err instanceof TypeError) {
    const m = new Error('api_missing')
    m.apiMissing = true
    throw m
  }
  throw err
}

/** Builds the core's io from the engine interface `$`. */
function buildIo($, ledgerFile, binName) {
  const lines = []
  let writing = Promise.resolve()
  return {
    bin: binName,
    argv: Array.isArray(LAUNCH?.argv) && LAUNCH.argv.length ? LAUNCH.argv : null,
    // The engine's validator forbids reading `$` members as values, so a missing
    // API is detected by calling: only the store can be probed without effect.
    apis: async () => {
      try {
        await $.store.get('probe')
        return true
      } catch {
        return false
      }
    },
    now: () => Date.now(),
    sessionId: () => $.session.id(),
    promptRead: () => $.prompt.read(),
    submit: (text) => guard(() => $.prompt.submit({ text })),
    append: (text) =>
      guard(() => $.session.append({ message: { type: 'user', content: [{ type: 'text', text }] } })),
    run: async (argv) => {
      const r = await $.process.run(argv)
      return { code: r.exitCode, stdout: r.stdout }
    },
    log: (text) => $.ui.log(text, { dim: true }),
    store: {
      get: (k) => $.store.get(k),
      set: (k, v) => $.store.set(k, v),
      delete: (k) => $.store.delete(k),
    },
    state: {
      get: async () => (await $.state.get({ plugin: 'herdr-threads', key: 'turns' })).value,
      set: (v) => $.state.set({ plugin: 'herdr-threads', key: 'turns' }, v),
    },
    ledger: (entry) => {
      if (!ledgerFile) return
      lines.push(JSON.stringify(entry))
      if (lines.length > LEDGER_CAP) lines.splice(0, lines.length - LEDGER_CAP)
      // $.fs has no append and concurrent writes race: rewrite in order.
      writing = writing
        .then(() => $.fs.write(ledgerFile, lines.join('\n') + '\n'))
        .catch(() => {})
    },
    spawn: (argv, cb) => {
      const stream = guard(() => $.process.spawn({ argv }))
      let stopped = false
      void (async () => {
        let buf = ''
        try {
          for await (const chunk of stream) {
            if (stopped) break
            if (chunk.stream !== 'stdout') continue
            buf += chunk.text
            let nl
            while ((nl = buf.indexOf('\n')) >= 0) {
              const line = buf.slice(0, nl).trim()
              buf = buf.slice(nl + 1)
              if (!line) continue
              try {
                cb.line(JSON.parse(line))
              } catch {}
            }
          }
        } catch {}
        if (stopped) return
        let code = 1
        try {
          const r = await stream.result
          if (r && typeof r.code === 'number') code = r.code
        } catch {}
        cb.exit(code)
      })()
      return {
        stop() {
          stopped = true
          try {
            void stream.return(undefined)
          } catch {}
        },
      }
    },
  }
}

/** @type {import('claude-code').Register} */
export const register = (on) => {
  let core = null
  let timer = null

  on('session.start', async ($, e, next) => {
    const started = await next(e)
    if (timer) timer.cancel()
    if (core) core.dispose()
    const ledgerFile = (await $.env.get('HERDR_THREADS_MOD_LEDGER')) || null
    const binName = (await $.env.get('HERDR_THREADS_BIN')) || 'herdr-threads'
    core = createCore(buildIo($, ledgerFile, binName))
    await core.onLoad()
    timer = $.clock.every(1000, () => {
      if (core) void core.onTick()
    })
    return started
  })

  on('session.end', async ($, e, next) => {
    if (core) core.onSessionEnd(e.reason)
    const r = await next(e)
    // The handler has returned: the tick re-reads the session id and restarts watch.
    if (core) void core.onTick()
    return r
  })

  on('turn.start', async ($, e, next) => {
    if (core) core.onTurnStart(e)
    return next(e)
  })

  on('turn.step', async function* ($, e, next) {
    try {
      if (core) core.onTurnStep(e)
    } catch {}
    return yield* next(e)
  })

  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    if (core) core.onTurnComplete(e)
    return r
  })

  on('tool.call', async ($, e, next) => {
    const r = await next(e)
    try {
      const ctx = { foreign: next.origin?.plugin !== undefined && next.origin.plugin !== 'engine', aborted: next.signal?.aborted === true }
      return core ? await core.onToolCall(e, r, ctx) : r
    } catch {
      return r
    }
  })
}
