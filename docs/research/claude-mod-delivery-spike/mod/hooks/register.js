// Spike mod: follows a message feed from a child process and delivers each
// message into the session, either between tool calls (attached context),
// as a quiet appended row, or as a new turn once the session is idle.
// Every step is recorded in $HT_SPIKE_DIR/events.jsonl for the driver.

let dir = null
let loadId = 0
let pending = []
let isBusy = false
let isSubmitting = false
let draftGuard = true
const ledger = []
let writing = Promise.resolve()

function record($, kind, fields) {
  const row = { at: Date.now(), kind, ...fields }
  ledger.push(row)
  $.ui.log('ht-spike ' + JSON.stringify(row), { to: 'debug' })
  // Writes replace the whole file, so run them one at a time in order
  if (dir) writing = writing.then(() => $.fs.write(dir + '/events-' + loadId + '.jsonl', ledger.map(r => JSON.stringify(r)).join('\n') + '\n')).catch(() => {})
}

function frame(msgs) {
  return msgs.map(m => '[herdr-threads] message ' + m.id + ' from ' + m.from + ': ' + m.body).join('\n')
}

async function deliverIdle($, reason, force = false) {
  if ((isBusy && !force) || isSubmitting) return
  const batch = pending.filter(m => m.mode === 'submit' || m.mode === 'auto')
  if (batch.length === 0) return
  if (draftGuard) {
    const box = await $.prompt.read()
    if (box.text !== '') {
      record($, 'held-for-draft', { ids: batch.map(m => m.id), draft: box.text })
      return
    }
  }
  isSubmitting = true
  pending = pending.filter(m => !batch.includes(m))
  const asUser = batch.some(m => m.asUser)
  record($, 'submit-call', { ids: batch.map(m => m.id), reason, asUser })
  try {
    const r = await $.prompt.submit(asUser ? { text: frame(batch), asUser: true } : { text: frame(batch) })
    record($, 'submit-resolved', { ids: batch.map(m => m.id), drop: r.drop ?? null })
  } catch (err) {
    record($, 'submit-rejected', { ids: batch.map(m => m.id), error: String(err) })
  } finally {
    isSubmitting = false
  }
}

async function onMessage($, m) {
  record($, 'received', { id: m.id, mode: m.mode, busy: isBusy })
  if (m.control === 'draft-guard') { draftGuard = !!m.value; return }
  // Spike only: block the hooks worker thread to see how Claude Code recovers
  if (m.control === 'hang') { const until = Date.now() + m.ms; while (Date.now() < until) {} return }
  if (m.mode === 'append') {
    try {
      const r = await $.session.append({ message: { type: 'user', content: [{ type: 'text', text: frame([m]) }] } })
      record($, 'append-resolved', { id: m.id, busy: isBusy, deny: r && r.deny ? r.deny : null })
    } catch (err) {
      record($, 'append-rejected', { id: m.id, error: String(err) })
    }
    return
  }
  pending.push(m)
  void deliverIdle($, m.force ? 'forced' : 'message', !!m.force)
}

function follow($, feed) {
  void (async () => {
    let buf = ''
    const child = $.process.spawn({ argv: ['tail', '-n', '0', '-F', feed] })
    record($, 'feed-started', { feed })
    for await (const chunk of child) {
      if (chunk.stream !== 'stdout') continue
      buf += chunk.text
      let nl
      while ((nl = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, nl).trim()
        buf = buf.slice(nl + 1)
        if (!line) continue
        try { await onMessage($, JSON.parse(line)) } catch (err) { record($, 'bad-line', { line, error: String(err) }) }
      }
    }
    record($, 'feed-ended', {})
  })()
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    dir = await $.env.get('HT_SPIKE_DIR')
    loadId = await $.clock.now()
    const pane = await $.env.get('HERDR_PANE_ID')
    record($, 'session-start', { pane, session: await $.session.id(), version: (await $.session.version()).version })
    if (dir) follow($, dir + '/feed.jsonl')
    return started
  })

  on('session.end', async ($, e, next) => {
    record($, 'session-end', { reason: e.reason })
    return next(e)
  })

  on('turn.start', async ($, e, next) => {
    if (!e.agentId) { isBusy = true; record($, 'turn-start', { turn: e.turnId, session: await $.session.id() }) }
    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    if (!e.agentId) {
      isBusy = false
      record($, 'turn-complete', { turn: e.turnId, aborted: !!e.isAborted })
      void deliverIdle($, 'turn-complete')
    }
    return r
  })

  // Mid-turn: attach waiting messages after a main-conversation tool result.
  on('tool.call', async ($, e, next) => {
    const r = await next(e)
    if (e.agentId) return r
    const batch = pending.filter(m => m.mode === 'context' || m.mode === 'auto')
    if (batch.length === 0 || !r || r.deny !== undefined) return r
    pending = pending.filter(m => !batch.includes(m))
    record($, 'context-attached', { ids: batch.map(m => m.id), tool: e.tool })
    return { ...r, context: [...(r.context ?? []), frame(batch)] }
  })

  on('prompt.submit', async ($, e, next) => {
    record($, 'prompt-submit-seen', { origin: e.origin, text: e.text.slice(0, 120) })
    return next(e)
  })

  on('session.receive', async ($, e, next) => {
    record($, 'session-receive', { origin: e.origin, text: e.text.slice(0, 120) })
    return next(e)
  })
}
