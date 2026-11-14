// herdr-threads plugin for opencode (epic ht-vxdp, design D1-D5, D8, D9).
//
// One dependency-free module, loaded by opencode's embedded Bun from `plugins/`. It
//   - adds the session claim variables to every shell call (`shell.env`, D2);
//   - runs one registered hook command per top-level user message (`chat.message`, D3):
//     `<cli> hook-event opencode <Event>` with one JSON object on stdin,
//     `{"event", "source"?, "session_id", "parent_session_id"?, "cwd"}`
//     (src/harness/opencode/decode.rs); stdout is the context text itself, added to that user
//     message as one synthetic text part; empty stdout or a non-zero exit injects nothing;
//   - runs `<cli> watch --harness opencode --session <sid>` for the bound session and appends
//     each page as a stored synthetic message (`client.session.prompt` with `noReply`), acks it
//     with the stored message id, and wakes an idle session with `client.session.promptAsync`
//     (D4). The watch line contract is src/protocol/watch.rs.
// `createCore(io)` is the state machine, acting only through `io`; `HerdrThreads` wires it to
// opencode. The module has exactly one export, the plugin function, because opencode's legacy
// loader treats every export as a plugin; the tests reach the internals through
// `HerdrThreads.internals`.

import { spawn as nodeSpawn } from 'node:child_process'
import nodeFs from 'node:fs'
import nodePath from 'node:path'

// `setup opencode` replaces the next line with the hooks' invocation prefix
// ({"argv": [absolute executable, "--state-dir", S, "--host-endpoint", H]});
// `null` means `herdr-threads` from PATH, with no heartbeat (no state dir known).
const LAUNCH = null // herdr-threads:launch (setup opencode writes the invocation here)
// `setup opencode --with-permissions` replaces `false` with `true` on the next line (D6).
const GRANT_PERMISSIONS = false // herdr-threads:permissions (setup opencode writes the consent here)

const HEADER = '[herdr-threads] new messages:'
// The content-free wake prompt (R18), pinned to src/notification/policy.rs WAKE_UP by the tests.
const WAKE_PROMPT = 'herdr-threads: wake up'
// The shared notice a lifecycle hook prints when the check-in failed (src/cli/hook.rs
// `unavailable_text`); the hook still exits 0, so this text is the failure signal.
const UNAVAILABLE = 'herdr-threads: check-in unavailable ('
const SESSION_VAR = 'HERDR_THREADS_OPENCODE_SESSION'
const PARENT_VAR = 'HERDR_THREADS_OPENCODE_PARENT'
const PID_VAR = 'HERDR_THREADS_OPENCODE_PID'
const LEDGER_ENV = 'HERDR_THREADS_MOD_LEDGER'
const HEARTBEAT_FILE = ['opencode', 'plugin-heartbeat.json']
const GRANT_KEYS = ['herdr-threads *', 'ht *']

const BACKOFF_S = [1, 2, 5, 10, 30]
const DROP_BACKOFF_MS = 30000
const ACK_RETRY_MS = 30000
const ACK_BATCH_MAX = 100 // MAX_BATCH_ITEMS in src/protocol/commands.rs
const CONNECTED_RESET_MS = 60000
const HOOK_TIMEOUT_MS = 8000
const ACK_TIMEOUT_MS = 15000
const PARENT_DEPTH = 8
// An own message's chat.message may arrive after its call returned (promptAsync is async).
const APPEND_MARK_GRACE_MS = 5000
const WAKE_MARK_GRACE_MS = 30000
// After a wake was accepted the session counts as busy until its next status event, or this long.
const WAKE_SETTLE_MS = 10000
// Bounds on opencode SDK calls (the in-process client never times out by itself): a call past
// its bound is a failure and takes the call's existing retry or fallback path.
const SDK_WRITE_TIMEOUT_MS = 15000 // session.prompt (append), session.promptAsync (wake)
const SDK_READ_TIMEOUT_MS = 5000 // session.status, session.get
// A timed-out append or wake may still be stored later: its mark lasts this long.
const LATE_MARK_MS = 600000
// A top-level user message counts as a turn about to start until its first status event, or this
// long (its turn hook can take HOOK_TIMEOUT_MS before opencode starts the turn).
const TURN_START_MS = 18000
const APPEND_MAX_ITEMS = 32
const APPEND_MAX_BYTES = 64 * 1024

const SESSION_ID = /^ses_[0-9A-Za-z]{1,64}$/
const ROW_ID = /^[A-Za-z0-9_-]{1,128}$/
const INTENTS = new Set(['query', 'request', 'rule'])
const STOP_REASONS = ['not_claude', 'disabled', 'replaced', 'no_pane', 'env_disabled', 'unsupported']

// ---- framing (a third copy of src/delivery/frame.rs and the Claude mod's frame(); pinned by
// tests/fixtures/delivery goldens) ----------------------------------------------------------

const canonicalEpoch = (v) => typeof v === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(v)
  && (v.length < 20 || v <= '18446744073709551615')
const oneLine = (s) => String(s).replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g, ' ')
const LINE_BREAK = /\r\n|[\n\r\u000b\u000c\u0085\u2028\u2029]/
const indent = (body) => String(body).split(LINE_BREAK).map((l) => (l === '' ? '' : `  ${l}`)).join('\n')

function markers(it) {
  let out = ''
  if (Number.isInteger(it.priority) && it.priority !== 1 && it.priority >= 0 && it.priority <= 9) out += ` [p${it.priority}]`
  if (it.author_role === 'human') out += ' [human]'
  if (it.relays_user === true) out += ' [relays user]'
  if (INTENTS.has(it.user_intent)) out += ` [${it.user_intent}]`
  return out
}

const label = (id, name) => {
  const i = id ? oneLine(id) : ''
  const n = name ? oneLine(name) : ''
  if (!n || n === i) return i || 'unknown'
  return i ? `${i} ${JSON.stringify(n)}` : JSON.stringify(n)
}

function frame(items) {
  const blocks = items.map((it) =>
    it.kind === 'attention'
      ? `[herdr-threads] attention ${oneLine(it.id)}:\n${indent(it.body)}`
      : `[herdr-threads] ${it.kind === 'lazy' ? 'message' : oneLine(it.kind)} ${oneLine(it.id)} in ${label(it.thread, it.threadName)} from ${it.senderLabel ? oneLine(it.senderLabel) : label(it.sender, it.senderName)}${markers(it)}:\n${indent(it.body)}`,
  )
  return `${HEADER}\n\n${blocks.join('\n\n')}`
}

// src/delivery/assemble.rs `batch_key`: priority asc (absent: 1 for lazy, else 0), deadline asc
// with none last; the stable sort keeps arrival order for the rest.
const orderKey = (it) => [it.priority ?? (it.kind === 'lazy' ? 1 : 0), it.deadline ?? Infinity]
const batchOrder = (items) => [...items].sort((a, b) => {
  const [pa, da] = orderKey(a)
  const [pb, db] = orderKey(b)
  return pa !== pb ? pa - pb : da === db ? 0 : da < db ? -1 : 1
})

const shellArg = (arg) => /^[a-zA-Z0-9_@%+=:,./-]+$/.test(arg)
  ? arg : "'" + arg.replace(/'/g, "'\\''") + "'"

// The daemon's own continuation markers (src/protocol/watch.rs `truncation_marker_for`,
// `attention_marker_for`) spell the inbox command the session's shell needs: `herdr-threads` plus
// only the selectors its own detection would not reach (src/cli/hook.rs `cli_prefix`). The plugin's
// recovery line repeats the newest one seen, so it matches the consented `herdr-threads *` grant.
const TRUNCATION_MARK = '…truncated; next: '
const ATTENTION_MARK = 'herdr-threads: attention pending; run '
const BARE_RECOVERY = 'next: herdr-threads inbox'

/** The `next: herdr-threads … inbox` line a daemon marker in `text` spells, or null. */
function markerRecovery(text, mark) {
  const s = String(text ?? '')
  const at = s.lastIndexOf(mark)
  if (at < 0) return null
  const command = s.slice(at + mark.length).split(LINE_BREAK)[0].trim()
  return /^herdr-threads( .+)? inbox$/.test(command) ? `next: ${command}` : null
}

/**
 * The recovery line before any marker was seen: the launch's selectors after the bare program
 * name (they reach the right instance, if not always the shortest spelling); `herdr-threads inbox`
 * with no rendered launch.
 */
function launchRecovery(argv) {
  if (!argv || !argv.length) return BARE_RECOVERY
  return 'next: ' + ['herdr-threads', ...argv.slice(1), 'inbox'].map(shellArg).join(' ')
}

/** A bounded complete prefix of `items`, with the recovery line inside the byte budget. */
function appendBatch(items, recovery) {
  const batch = []
  batch.deferred = []
  const encoder = new TextEncoder()
  const render = (selected, more) => {
    const truncated = selected.some((it) => it.truncated)
    const framed = selected.length ? frame(selected.map((it) => {
      if (!it.truncated || (it.kind !== 'message' && it.kind !== 'lazy')) return it
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

// Passive or waking follows the daemon's `wakes` (priority <= the seat's threshold) and the ACK need.
const passive = (item) => item.kind === 'notice' || (item.kind === 'invitation' && !item.wakes)
  || ((item.kind === 'message' || item.kind === 'lazy') && !item.wakes && item.ackRequired !== true)

const emptyRec = () => ({ delivered: {}, unacked: {}, rows: {}, attentionVersions: [], invitationVersions: {} })

// ---- permission grant (D6) -----------------------------------------------------------------

/**
 * Adds `permission.bash["herdr-threads *"]` and `["ht *"] = "allow"`, removed and re-added so
 * they are the last rules (last match wins). A user `deny` on either exact key is kept and that
 * grant is skipped. A user rule narrower than a grant (`herdr-threads probe *`) is moved after
 * the grants, in its own order, so the grant never overrides it. A string `bash` rule becomes
 * `{"*": rule}`. Returns the keys granted and the keys refused by the user.
 */
function grantPermissions(cfg) {
  const out = { granted: [], denied: [] }
  if (!cfg || typeof cfg !== 'object') return out
  if (cfg.permission == null) cfg.permission = {}
  // A whole-permission string (every tool) is the user's choice: left alone.
  if (typeof cfg.permission !== 'object') return out
  let bash = cfg.permission.bash
  if (typeof bash === 'string') bash = { '*': bash }
  else if (bash == null || typeof bash !== 'object' || Array.isArray(bash)) bash = {}
  const narrower = (k) => !GRANT_KEYS.includes(k) && GRANT_KEYS.some((g) => k.startsWith(g.slice(0, -1)))
  const moved = Object.entries(bash).filter(([k]) => narrower(k))
  for (const [k] of moved) delete bash[k]
  for (const key of GRANT_KEYS) {
    if (bash[key] === 'deny') {
      out.denied.push(key)
      continue
    }
    delete bash[key]
    bash[key] = 'allow'
    out.granted.push(key)
  }
  for (const [k, v] of moved) bash[k] = v
  cfg.permission.bash = bash
  return out
}

// ---- the core --------------------------------------------------------------------------------

/**
 * io: { now(), pid, env, argv|null, directory, client, run(argv, {stdin, env, timeoutMs}) ->
 * Promise<{code, stdout}>, spawn(argv, {env}, {line, exit}) -> {stop()}, ledger(entry),
 * heartbeat(record), after(ms) -> Promise (resolves once ms have passed) }.
 */
function createCore(io) {
  const S = {
    disposed: false,
    parents: new Map(), // sid -> direct parent id | null
    titles: new Map(),
    created: new Set(), // sessions created in this process (session.created seen)
    status: new Map(), // sid -> idle | busy | retry
    turnStart: null, // { sid, until }: a user message whose turn has not reported a status yet
    asks: new Map(), // sid -> Set of pending permission/question ids
    children: new Set(), // child sessions that got their SubagentStart
    everBound: new Set(),
    bound: null, // { sid, ok }: the top-level session whose last lifecycle check-in succeeded
    marks: [], // own in-flight messages: { sid, text, kind, until }
    ownIds: new Set(),
    // sid -> the agent, model and variant of its newest real top-level user message: opencode runs
    // each step with the newest user message's agent and model, and a message without them gets the
    // default agent, so every append and wake repeats them
    prompts: new Map(),
    hookChain: Promise.resolve(),
    // the channel
    chanSid: null,
    child: null,
    run: 0,
    live: false,
    stopped: false,
    stopReason: null,
    restartAt: null,
    backoffIdx: 0,
    connectedAt: null,
    queue: [],
    runAttention: new Set(),
    recs: new Map(),
    rec: emptyRec(),
    appendInflight: false,
    appendBackoffUntil: 0,
    wakePending: null, // { sid, after: stored id of the append }
    wakeInflight: false,
    wakeBackoffUntil: 0,
    wakeSettling: null, // { sid, until }
    pluginTurn: null, // a session running a turn the plugin's wake started
    lastAckRetry: 0,
    acking: new Set(),
    ackChain: Promise.resolve(),
    gen: 0,
    degraded: new Set(),
    channelless: false,
    recovery: null, // the newest daemon-spelled `next: … inbox` line
  }

  const cmd = (...rest) => [...(io.argv || ['herdr-threads']), ...rest]
  const TIMEOUT = Symbol('timeout')
  const after = io.after || ((ms) => new Promise((resolve) => {
    const t = setTimeout(resolve, ms)
    t.unref?.()
  }))
  /** `promise`, or TIMEOUT once `ms` have passed. */
  const bounded = (promise, ms) => Promise.race([promise, after(ms).then(() => TIMEOUT)])
  const client = () => io.client || {}
  const hasFn = (obj, name) => !!obj && typeof obj[name] === 'function'
  const sessionApi = () => client().session

  function ledger(kind, fields = {}) {
    try {
      io.ledger({
        at: io.now(),
        kind,
        ids: fields.ids ?? [],
        ...(fields.via ? { via: fields.via } : {}),
        ...(fields.reason ? { reason: fields.reason } : {}),
        ...(fields.row ? { row: fields.row } : {}),
      })
    } catch {}
  }

  function degrade(capability) {
    if (S.degraded.has(capability)) return
    S.degraded.add(capability)
    ledger('refused', { reason: `degraded:${capability}` })
    try {
      io.heartbeat({ degraded: [...S.degraded] })
    } catch {}
  }

  // ---- sessions ------------------------------------------------------------------------------

  async function directParent(sid) {
    if (S.parents.has(sid)) return S.parents.get(sid)
    const api = sessionApi()
    if (!hasFn(api, 'get')) throw new Error('session.get missing')
    const r = await bounded(api.get({ path: { id: sid } }), SDK_READ_TIMEOUT_MS)
    if (r === TIMEOUT) throw new Error('session.get timed out')
    if (!r || r.error || !r.data || typeof r.data !== 'object') throw new Error('session.get failed')
    const p = typeof r.data.parentID === 'string' && r.data.parentID ? r.data.parentID : null
    S.parents.set(sid, p)
    if (typeof r.data.title === 'string') S.titles.set(sid, r.data.title)
    return p
  }

  /** '' for a top-level session, the root top-level id for a child, '?' when unknown. */
  async function rootOf(sid) {
    try {
      let cur = sid
      for (let i = 0; i < PARENT_DEPTH; i++) {
        const p = await directParent(cur)
        if (p == null) return cur === sid ? '' : cur
        cur = p
      }
    } catch {}
    return '?'
  }

  /** The agent/model/variant body fields of a real user message (its stored info, else its input). */
  function promptFields(input, message) {
    const out = {}
    const str = (v) => typeof v === 'string' && v !== ''
    const agent = str(message?.agent) ? message.agent : str(input?.agent) ? input.agent : null
    if (agent) out.agent = agent
    const stored = message?.model && typeof message.model === 'object' ? message.model : null
    const model = stored ?? (input?.model && typeof input.model === 'object' ? input.model : null)
    if (model && str(model.providerID) && str(model.modelID)) out.model = { providerID: model.providerID, modelID: model.modelID }
    const variant = stored ? stored.variant : input?.variant
    if (str(variant)) out.variant = variant
    return out
  }

  function recordPrompt(sid, input, message) {
    S.prompts.delete(sid)
    S.prompts.set(sid, promptFields(input, message))
    if (S.prompts.size > 64) S.prompts.delete(S.prompts.keys().next().value)
  }

  const childEnv = (sid, root) => ({ ...io.env, [SESSION_VAR]: sid, [PARENT_VAR]: root, [PID_VAR]: String(io.pid) })

  // ---- own messages --------------------------------------------------------------------------

  function addMark(sid, text, kind) {
    const mark = { sid, text, kind, until: Infinity }
    S.marks.push(mark)
    if (S.marks.length > 64) S.marks.shift()
    return mark
  }

  function takeMark(sid, text, msgId) {
    const now = io.now()
    S.marks = S.marks.filter((m) => m.until >= now)
    if (msgId && S.ownIds.has(msgId)) return { kind: 'append' }
    const i = S.marks.findIndex((m) => m.sid === sid && m.text === text)
    if (i < 0) return null
    const [mark] = S.marks.splice(i, 1)
    return mark
  }

  const partsText = (parts) => (Array.isArray(parts) ? parts : [])
    .filter((p) => p && p.type === 'text' && typeof p.text === 'string')
    .map((p) => p.text)
    .join('\n')

  let partSeq = 0
  function partId() {
    partSeq = (partSeq + 1) % 1e6
    const tail = (Math.random().toString(36).slice(2, 10) + partSeq.toString(36)).padEnd(14, '0').slice(0, 14)
    return `prt_ffffffffffff${tail}`
  }

  function inject(sid, output, text) {
    if (!text || !output || !Array.isArray(output.parts)) return
    output.parts.push({
      id: partId(),
      sessionID: sid,
      messageID: output.message?.id,
      type: 'text',
      text,
      synthetic: true,
    })
  }

  // ---- hook commands -------------------------------------------------------------------------

  function hook(event, payload, sid, root) {
    const job = S.hookChain.then(async () => {
      let r = { code: -1, stdout: '' }
      try {
        r = await io.run(cmd('hook-event', 'opencode', event), {
          stdin: JSON.stringify(payload),
          env: childEnv(sid, root),
          timeoutMs: HOOK_TIMEOUT_MS,
        })
      } catch {}
      const text = r && r.code === 0 ? String(r.stdout ?? '') : ''
      return { ok: text.trim() !== '' && !text.includes(UNAVAILABLE), text }
    })
    S.hookChain = job.catch(() => {})
    return job.catch(() => ({ ok: false, text: '' }))
  }

  /** The lifecycle source for a top-level session that is not the bound one (D3 table). */
  function sourceFor(sid) {
    if (S.everBound.has(sid)) return 'resume'
    if (!S.created.has(sid)) return 'resume'
    return S.everBound.size === 0 ? 'startup' : 'clear'
  }

  async function lifecycle(sid, source, output) {
    const r = await hook('SessionStart', { event: 'SessionStart', source, session_id: sid, cwd: io.directory }, sid, '')
    if (S.disposed) return r
    inject(sid, output, r.text)
    if (r.ok) bind(sid)
    return r
  }

  async function onChatMessage(input, output) {
    const sid = input?.sessionID
    if (S.disposed || typeof sid !== 'string' || !SESSION_ID.test(sid)) return
    const mark = takeMark(sid, partsText(output?.parts), output?.message?.id)
    // An append is the plugin's own delivery: no hook, no turn.
    if (mark && mark.kind === 'append') return
    const root = await rootOf(sid)
    if (root === '?') return // unknown role fails toward child: nothing now, retried next message
    if (root !== '') {
      if (S.children.has(sid)) return
      const r = await hook('SubagentStart', { event: 'SubagentStart', session_id: sid, parent_session_id: root, cwd: io.directory }, sid, root)
      if (S.disposed) return
      if (r.text) S.children.add(sid)
      inject(sid, output, r.text)
      return
    }
    // The plugin's own wake carries the recorded fields already: it never replaces them.
    if (!(mark && mark.kind === 'wake')) recordPrompt(sid, input, output?.message)
    S.status.set(sid, 'busy')
    S.turnStart = { sid, until: io.now() + TURN_START_MS }
    const bound = S.bound && S.bound.sid === sid
    // A wake prompt runs the bound session's turn hook and is never a lifecycle trigger.
    if ((bound && S.bound.ok) || (mark && mark.kind === 'wake')) {
      const r = await hook('UserPromptSubmit', { event: 'UserPromptSubmit', session_id: sid, cwd: io.directory }, sid, '')
      if (!S.disposed) inject(sid, output, r.text)
      return
    }
    await lifecycle(sid, sourceFor(sid), output)
  }

  /** `session.compacted` of the bound session: the compact re-entry, appended (D3, D4). */
  async function onCompacted(sid) {
    if (!S.bound || S.bound.sid !== sid) return
    const r = await hook('SessionStart', { event: 'SessionStart', source: 'compact', session_id: sid, cwd: io.directory }, sid, '')
    if (S.disposed) return
    if (r.ok && S.bound && S.bound.sid === sid) S.bound.ok = true
    if (r.text && S.prompts.has(sid)) {
      const a = await append(sid, r.text)
      if (a.id) S.ownIds.add(a.id)
    }
  }

  function bind(sid) {
    const prev = S.bound ? S.bound.sid : null
    S.everBound.add(sid)
    S.bound = { sid, ok: true }
    if (S.channelless) return
    if (prev !== sid || S.chanSid !== sid) return switchChannel(sid)
    if (!S.child) {
      S.stopped = false
      S.backoffIdx = 0
      startChild()
    }
  }

  // ---- the channel ---------------------------------------------------------------------------

  function dropQueued(why) {
    S.live = false
    const dropped = S.queue.filter((q) => !q.inflight).map((q) => q.id)
    S.queue = S.queue.filter((q) => q.inflight)
    if (dropped.length) ledger('refused', { ids: dropped, reason: `channel_lost:${why}` })
  }

  function discardStale(items, why) {
    const stale = items.filter((it) => it.run !== S.run || !S.live)
    if (!stale.length) return items
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

  function switchChannel(sid) {
    stopChild()
    S.gen++
    for (const it of S.queue) it.inflight = false
    S.queue = []
    S.appendInflight = false
    S.wakeInflight = false
    S.wakePending = null
    S.wakeSettling = null
    S.pluginTurn = null
    S.chanSid = sid
    if (!S.recs.has(sid)) S.recs.set(sid, emptyRec())
    S.rec = S.recs.get(sid)
    S.acking = new Set()
    S.stopped = false
    S.stopReason = null
    S.backoffIdx = 0
    startChild()
  }

  /** The SDK call a channel cannot run without, or null (D8, detected before any watch runs). */
  function channelMissing() {
    const api = sessionApi()
    if (!hasFn(api, 'prompt')) return 'append'
    if (!hasFn(api, 'promptAsync')) return 'wake'
    return null
  }

  function startChild() {
    if (S.disposed || S.channelless || !S.chanSid) return
    // No channel the daemon would count on: it then sees no wake path (the sender reads
    // wake=no_wake) and the turn hook delivers.
    const missing = channelMissing()
    if (missing) return goChannelless(missing)
    S.run++
    const run = S.run
    const sid = S.chanSid
    S.runAttention = new Set()
    S.connectedAt = null
    S.live = false
    S.restartAt = null
    S.stopReason = null
    try {
      S.child = io.spawn(cmd('watch', '--harness', 'opencode', '--session', sid), { env: childEnv(sid, '') }, {
        line: (o) => onLine(o, run),
        exit: (c) => onChildExit(c, run),
      })
    } catch {
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
    if (code === 3 && S.stopReason) {
      S.stopped = true
      return
    }
    if (wasConnected != null && io.now() - wasConnected >= CONNECTED_RESET_MS) S.backoffIdx = 0
    const delay = BACKOFF_S[Math.min(S.backoffIdx, BACKOFF_S.length - 1)]
    S.backoffIdx++
    S.restartAt = io.now() + delay * 1000
  }

  function onLine(o, run) {
    if (run !== undefined && run !== S.run) return
    if (S.disposed || !o || typeof o !== 'object' || typeof o.id !== 'string') return
    if (o.kind === 'status') {
      if (o.state === 'connected') {
        S.stopReason = null
        S.connectedAt = io.now()
        S.lastAckRetry = io.now()
        S.live = true
        retryAcks()
        void pump()
      } else if (o.state === 'closing' || o.state === 'refused') {
        const named = !!(io.env?.HERDR_THREADS_SEAT_NAME || io.env?.HC_TERMINAL_ID)
        S.stopReason = o.exit === 3 && STOP_REASONS.includes(o.reason) && !(named && o.reason === 'no_pane')
          ? o.reason : null
        // The binding is gone or names another session: the next turn checks in again (D3).
        if (o.state === 'refused' && (o.reason === 'no_binding' || o.reason === 'session_mismatch')
            && S.bound && S.bound.sid === S.chanSid) S.bound.ok = false
        dropQueued(`${o.state}:${o.reason ?? ''}`)
      }
      return
    }
    const hasEpoch = Object.prototype.hasOwnProperty.call(o, 'presentation_epoch')
    if ((o.kind === 'attention' || o.kind === 'attention_cleared') && hasEpoch
        && !canonicalEpoch(o.presentation_epoch)) return
    if (o.kind === 'attention_cleared') {
      const retract = (q) => q.kind === 'attention' && !q.inflight
        && (!hasEpoch || q.presentationEpoch === o.presentation_epoch)
      const gone = S.queue.filter(retract).map((q) => q.id)
      S.queue = S.queue.filter((q) => !retract(q))
      if (hasEpoch) S.runAttention.delete(`attention:${o.presentation_epoch}`)
      else S.runAttention = new Set()
      if (gone.length) ledger('refused', { ids: gone, reason: 'attention_cleared' })
      return
    }
    if (!['message', 'lazy', 'invitation', 'attention', 'notice'].includes(o.kind)) return
    ledger('received', { ids: [o.id] })
    const learned = o.kind === 'attention' ? markerRecovery(o.text, ATTENTION_MARK)
      : (o.kind === 'message' || o.kind === 'lazy') && o.truncated === true ? markerRecovery(o.body, TRUNCATION_MARK)
        : null
    if (learned) S.recovery = learned
    if (o.kind === 'attention') {
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
        version: o.attention_version,
        presentationEpoch: hasEpoch ? o.presentation_epoch : null,
        run: S.run,
        ackable: false,
      })
    } else {
      const revision = o.topic_revision == null ? null : String(o.topic_revision)
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
      if (o.kind === 'invitation') S.queue = S.queue.filter((q) => q.id !== o.id || q.inflight)
      const truncated = o.kind !== 'notice' && o.truncated === true
      S.queue.push({
        id: o.id,
        kind: o.kind,
        thread: o.thread || 'unknown',
        threadName: o.thread_name ?? null,
        sender: o.sender ?? null,
        senderName: o.sender_name ?? null,
        senderLabel: o.sender_label ?? null,
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
    void pump()
  }

  // ---- delivery ------------------------------------------------------------------------------

  function releaseBatch(batch) {
    for (const it of batch) it.inflight = false
    const newest = new Map()
    for (const q of S.queue) {
      if (q.kind === 'invitation' && !q.inflight) newest.set(q.id, q)
    }
    S.queue = S.queue.filter((q) => q.kind !== 'invitation' || q.inflight || newest.get(q.id) === q)
  }

  function complete(items, row) {
    const ids = items.map((i) => i.id)
    for (const it of items) {
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
          const keys = Object.keys(S.rec.invitationVersions)
          if (keys.length > 1024) delete S.rec.invitationVersions[keys[0]]
        }
      } else {
        S.rec.delivered[it.id] = 'append'
        if (it.ackable) {
          S.rec.unacked[it.id] = 'append'
          if (row && it.kind === 'message') S.rec.rows[it.id] = row
          else delete S.rec.rows[it.id]
        }
      }
    }
    ledger('delivered', { ids, via: 'append', row })
    return ackIds(items.filter((i) => i.ackable).map((i) => i.id))
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
    const sid = S.chanSid
    const rec = S.rec
    const acking = S.acking
    const jobs = []
    for (const { via, row, ids: all } of Object.values(groups)) {
      for (let at = 0; at < all.length; at += ACK_BATCH_MAX) {
        const chunk = all.slice(at, at + ACK_BATCH_MAX)
        const job = S.ackChain.then(() => doAck(via, row, chunk, sid, rec, acking))
        S.ackChain = job.catch(() => {})
        jobs.push(job)
      }
    }
    return Promise.all(jobs).then(() => {})
  }

  async function doAck(via, row, ids, sid, rec, acking) {
    let code = 1
    let stdout = ''
    try {
      const r = await io.run(cmd('watch', 'ack', '--session', sid, ...(row ? ['--row', row] : []), '--via', via, ...ids), {
        stdin: '',
        env: childEnv(sid, ''),
        timeoutMs: ACK_TIMEOUT_MS,
      })
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
    const live = !S.disposed
    // Exit 2 is the CLI refusing its arguments (an older binary without `--row`): the ids go
    // again without the row.
    const rowRefused = row !== null && code === 2
    for (const id of ids) {
      acking.delete(id)
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
    if (live && rowRefused && rec === S.rec) {
      ledger('refused', { ids, via, row, reason: 'ack:row_refused' })
      void ackIds(ids)
    }
  }

  function retryAcks() {
    if (S.disposed || !S.chanSid) return
    void ackIds(Object.keys(S.rec.unacked))
  }

  /** Stores `text` in `sid` as a synthetic user message with no turn; `{id}` on success. */
  async function append(sid, text) {
    const api = sessionApi()
    if (!hasFn(api, 'prompt')) return { missing: true }
    const mark = addMark(sid, text, 'append')
    let late = false
    try {
      const r = await bounded(
        api.prompt({ path: { id: sid }, body: { ...S.prompts.get(sid), noReply: true, parts: [{ type: 'text', text, synthetic: true }] } }),
        SDK_WRITE_TIMEOUT_MS,
      )
      if (r === TIMEOUT) {
        late = true
        return { error: 'timeout' }
      }
      if (!r || r.error) return { error: String(r?.error?.name ?? r?.error ?? 'no result') }
      const id = r.data?.info?.id
      if (typeof id !== 'string' || !ROW_ID.test(id)) return { noId: true }
      S.ownIds.add(id)
      if (S.ownIds.size > 512) S.ownIds.delete(S.ownIds.values().next().value)
      return { id }
    } catch (err) {
      return { error: String(err) }
    } finally {
      mark.until = io.now() + (late ? LATE_MARK_MS : APPEND_MARK_GRACE_MS)
    }
  }

  /**
   * The SDK cannot deliver or wake as designed: stop the channel; the turn hook delivers from now
   * on, and with no channel the daemon reports the seat wake=no_wake (D8).
   */
  function goChannelless(capability) {
    degrade(capability)
    S.channelless = true
    stopChild()
    S.gen++
    S.queue = []
    S.appendInflight = false
    S.wakePending = null
  }

  async function refreshStatus(sid) {
    const api = sessionApi()
    if (!hasFn(api, 'status')) return
    try {
      const r = await bounded(api.status(), SDK_READ_TIMEOUT_MS)
      if (r !== TIMEOUT && r && !r.error && r.data && typeof r.data === 'object') {
        // The map lists only non-idle sessions.
        const st = r.data[sid]
        const type = st && typeof st.type === 'string' ? st.type : 'idle'
        // A user message whose turn has not started yet reads idle on the server: keep it busy.
        const starting = S.turnStart && S.turnStart.sid === sid && io.now() < S.turnStart.until
        if (type !== 'idle' || !starting) S.status.set(sid, type)
      }
    } catch {}
  }

  async function pump() {
    if (S.disposed || S.channelless || !S.live || S.appendInflight || !S.chanSid) return
    if (io.now() < S.appendBackoffUntil) return
    const sid = S.chanSid
    // No real message of this session seen yet (unreachable: a session binds at one): nothing is
    // sent with opencode's default agent; the items wait for the turn hook.
    if (!S.prompts.has(sid)) return
    const batch = appendBatch(batchOrder(S.queue.filter((q) => !q.inflight && !q.recoveryOffered && q.run === S.run)), S.recovery ?? launchRecovery(io.argv))
    if (!batch.text) {
      void maybeWake()
      return
    }
    const gen = S.gen
    for (const it of batch) it.inflight = true
    S.appendInflight = true
    const ids = batch.map((i) => i.id)
    const waking = batch.some((it) => !passive(it))
    const r = await append(sid, batch.text)
    if (S.disposed || gen !== S.gen) return
    S.appendInflight = false
    releaseBatch(batch)
    if (r.missing) return goChannelless('append')
    if (r.noId) return goChannelless('append_row')
    if (r.error !== undefined) {
      discardStale(batch, 'append_error')
      S.appendBackoffUntil = io.now() + DROP_BACKOFF_MS
      ledger('refused', { ids, via: 'append', reason: `append:${r.error}` })
      return
    }
    for (const item of batch.deferred) item.recoveryOffered = true
    // A waking item is guaranteed a turn: cleared by a model step starting after this append,
    // else the wake is sent once the session is idle (C1).
    if (waking) S.wakePending = { sid, after: r.id }
    void complete(batch, r.id)
    // An append that landed just after the turn ended starts no turn: re-read the status.
    if (waking) await refreshStatus(sid)
    if (S.disposed || gen !== S.gen) return
    await maybeWake()
    void pump()
  }

  function idle(sid) {
    const st = S.status.get(sid)
    if (st === 'busy' || st === 'retry') return false
    if (S.asks.get(sid)?.size) return false
    if (S.wakeSettling && S.wakeSettling.sid === sid && io.now() < S.wakeSettling.until) return false
    return true
  }

  async function maybeWake() {
    const w = S.wakePending
    if (!w || S.disposed || S.wakeInflight || !S.live || S.channelless) return
    if (!S.bound || w.sid !== S.bound.sid || w.sid !== S.chanSid) return
    if (io.now() < S.wakeBackoffUntil || !idle(w.sid) || !S.prompts.has(w.sid)) return
    const api = sessionApi()
    // No wake on this opencode: no channel either, so senders are told (D8).
    if (!hasFn(api, 'promptAsync')) return goChannelless('wake')
    S.wakeInflight = true
    const gen = S.gen
    ledger('submit', { via: 'submit', reason: 'wake' })
    const mark = addMark(w.sid, WAKE_PROMPT, 'wake')
    S.pluginTurn = w.sid
    let r
    try {
      r = await bounded(api.promptAsync({ path: { id: w.sid }, body: { ...S.prompts.get(w.sid), parts: [{ type: 'text', text: WAKE_PROMPT }] } }), SDK_WRITE_TIMEOUT_MS)
    } catch (err) {
      r = { error: String(err) }
    }
    const late = r === TIMEOUT
    if (late) r = { error: 'timeout' }
    mark.until = io.now() + (late ? LATE_MARK_MS : WAKE_MARK_GRACE_MS)
    if (S.disposed || gen !== S.gen) return
    S.wakeInflight = false
    if (!r || r.error) {
      // A timed-out wake may still start its turn: its mark stays, so that turn is still ours.
      if (!late) S.marks = S.marks.filter((m) => m !== mark)
      S.pluginTurn = null
      S.wakeBackoffUntil = io.now() + DROP_BACKOFF_MS
      ledger('refused', { via: 'submit', reason: `wake_drop:${String(r?.error?.name ?? r?.error ?? 'no result')}` })
      return
    }
    if (S.wakePending === w) S.wakePending = null
    S.wakeSettling = { sid: w.sid, until: io.now() + WAKE_SETTLE_MS }
  }

  // ---- events --------------------------------------------------------------------------------

  function toast(sid, what) {
    const tui = client().tui
    if (!hasFn(tui, 'showToast')) return
    const title = S.titles.get(sid)
    const message = `herdr-threads: session ${title ? JSON.stringify(title) + ' ' : ''}(${sid}) is waiting for ${what}; open it from the session list`
    try {
      const p = tui.showToast({ body: { message, variant: 'info' } })
      if (p && typeof p.catch === 'function') p.catch(() => {})
    } catch {}
  }

  function addAsk(sid, id, what) {
    if (typeof sid !== 'string') return
    if (!S.asks.has(sid)) S.asks.set(sid, new Set())
    S.asks.get(sid).add(typeof id === 'string' ? id : `anon:${io.now()}`)
    if (S.pluginTurn === sid) toast(sid, what)
  }

  function removeAsk(sid, id) {
    const set = S.asks.get(sid)
    if (!set) return
    if (typeof id === 'string') set.delete(id)
    else set.clear()
  }

  function onEvent(event) {
    if (S.disposed || !event || typeof event.type !== 'string') return
    const p = event.properties || {}
    const sid = p.sessionID ?? p.info?.sessionID
    switch (event.type) {
      case 'session.created':
      case 'session.updated': {
        const info = p.info
        if (!info || typeof info.id !== 'string') return
        if (event.type === 'session.created') S.created.add(info.id)
        S.parents.set(info.id, typeof info.parentID === 'string' && info.parentID ? info.parentID : null)
        if (typeof info.title === 'string') S.titles.set(info.id, info.title)
        return
      }
      case 'session.status':
      case 'session.idle': {
        if (typeof sid !== 'string') return
        const type = event.type === 'session.idle' ? 'idle' : p.status?.type
        if (typeof type !== 'string') return
        S.status.set(sid, type)
        if (S.turnStart && S.turnStart.sid === sid) S.turnStart = null
        if (S.wakeSettling && S.wakeSettling.sid === sid) S.wakeSettling = null
        if (type === 'idle') {
          if (S.pluginTurn === sid) S.pluginTurn = null
          S.asks.delete(sid)
          void maybeWake()
          void pump()
        }
        return
      }
      case 'session.compacted':
        if (typeof sid === 'string') void onCompacted(sid)
        return
      case 'message.updated': {
        const info = p.info
        const w = S.wakePending
        // A model step whose message list held the append: that step reads it, so no wake is owed.
        // opencode reads the list, then creates the step's assistant message with `parentID` the
        // newest user message read, so a step id after the append proves nothing; its parent does.
        if (w && info && info.role === 'assistant' && info.sessionID === w.sid
            && typeof info.parentID === 'string' && info.parentID >= w.after) S.wakePending = null
        return
      }
      case 'permission.asked':
        return addAsk(sid, p.id, 'a permission')
      case 'question.asked':
        return addAsk(sid, p.id, 'an answer')
      case 'permission.replied':
      case 'question.replied':
      case 'question.rejected':
        return removeAsk(sid, p.requestID ?? p.id)
      default:
    }
  }

  async function onShellEnv(input, output) {
    const sid = input?.sessionID
    if (S.disposed || typeof sid !== 'string' || !sid || !output) return
    if (!output.env || typeof output.env !== 'object') output.env = {}
    const root = await rootOf(sid)
    output.env[SESSION_VAR] = sid
    output.env[PARENT_VAR] = root
    output.env[PID_VAR] = String(io.pid)
  }

  async function onTick() {
    if (S.disposed) return
    const now = io.now()
    if (!S.child && !S.stopped && !S.channelless && S.restartAt != null && now >= S.restartAt) startChild()
    if (S.connectedAt != null && now - S.lastAckRetry >= ACK_RETRY_MS) {
      S.lastAckRetry = now
      retryAcks()
    }
    await pump()
    await maybeWake()
  }

  function dispose() {
    S.disposed = true
    stopChild()
  }

  return {
    onChatMessage,
    onShellEnv,
    onEvent,
    onLine,
    onChildExit,
    onTick,
    dispose,
    snapshot: () => ({
      bound: S.bound ? { ...S.bound } : null,
      chanSid: S.chanSid,
      live: S.live,
      stopped: S.stopped,
      childRunning: S.child != null,
      restartAt: S.restartAt,
      queue: S.queue.map((q) => q.id),
      unacked: Object.keys(S.rec.unacked),
      delivered: Object.keys(S.rec.delivered),
      wakePending: S.wakePending ? { ...S.wakePending } : null,
      pluginTurn: S.pluginTurn,
      degraded: [...S.degraded],
      channelless: S.channelless,
    }),
  }
}

// ---- opencode wiring -------------------------------------------------------------------------

function launchArgv() {
  return LAUNCH && Array.isArray(LAUNCH.argv) && LAUNCH.argv.length && LAUNCH.argv.every((a) => typeof a === 'string')
    ? LAUNCH.argv : null
}

function stateDirOf(argv) {
  if (!argv) return null
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--state-dir' && typeof argv[i + 1] === 'string') return argv[i + 1]
    if (argv[i].startsWith('--state-dir=')) return argv[i].slice('--state-dir='.length)
  }
  return null
}

const DEFAULT_SYS = {
  spawn: nodeSpawn,
  fs: nodeFs,
  now: () => Date.now(),
  env: () => process.env,
  pid: () => process.pid,
  every: (ms, fn) => {
    const t = setInterval(fn, ms)
    return () => clearInterval(t)
  },
  after: (ms) => new Promise((resolve) => {
    const t = setTimeout(resolve, ms)
    t.unref?.()
  }),
  guard: globalThis,
}

const MAX_STDOUT = 1024 * 1024

/** The core's io over node APIs (`sys` is injectable for tests). */
function buildIo(input, sys, argv, heartbeatPath) {
  const env = sys.env()
  const ledgerFile = env[LEDGER_ENV] || null
  let heartbeatBase = null
  const io = {
    now: sys.now,
    after: sys.after,
    pid: sys.pid(),
    env,
    argv,
    directory: typeof input?.directory === 'string' ? input.directory : process.cwd(),
    client: input?.client,
    run: (cmdArgv, opts = {}) => new Promise((resolve) => {
      let child
      try {
        child = sys.spawn(cmdArgv[0], cmdArgv.slice(1), { env: opts.env, stdio: ['pipe', 'pipe', 'ignore'] })
      } catch {
        resolve({ code: -1, stdout: '' })
        return
      }
      let out = ''
      let done = false
      const finish = (code) => {
        if (done) return
        done = true
        clearTimeout(timer)
        resolve({ code, stdout: out })
      }
      const timer = setTimeout(() => {
        try {
          child.kill('SIGKILL')
        } catch {}
        finish(-1)
      }, opts.timeoutMs ?? ACK_TIMEOUT_MS)
      child.stdout?.setEncoding?.('utf8')
      child.stdout?.on('data', (d) => {
        if (out.length < MAX_STDOUT) out += d
      })
      child.on('error', () => finish(-1))
      child.on('close', (code) => finish(typeof code === 'number' ? code : -1))
      try {
        child.stdin?.on?.('error', () => {})
        child.stdin?.end(opts.stdin ?? '')
      } catch {}
    }),
    spawn: (cmdArgv, opts, cb) => {
      const child = sys.spawn(cmdArgv[0], cmdArgv.slice(1), { env: opts.env, stdio: ['ignore', 'pipe', 'ignore'] })
      let stopped = false
      let buf = ''
      child.stdout?.setEncoding?.('utf8')
      child.stdout?.on('data', (chunk) => {
        if (stopped) return
        buf += chunk
        let nl
        while ((nl = buf.indexOf('\n')) >= 0) {
          const line = buf.slice(0, nl).trim()
          buf = buf.slice(nl + 1)
          if (!line) continue
          let o
          try {
            o = JSON.parse(line)
          } catch {
            continue
          }
          try {
            cb.line(o)
          } catch {}
        }
      })
      let exited = false
      const exit = (code) => {
        if (exited || stopped) return
        exited = true
        cb.exit(code)
      }
      child.on('error', () => exit(1))
      child.on('close', (code) => exit(typeof code === 'number' ? code : 1))
      return {
        stop() {
          stopped = true
          try {
            child.kill('SIGTERM')
          } catch {}
        },
      }
    },
    ledger: (entry) => {
      if (!ledgerFile) return
      try {
        sys.fs.appendFileSync(ledgerFile, JSON.stringify(entry) + '\n')
      } catch {}
    },
    // Fields accumulate: a later write (consent result, degraded capability) keeps the earlier ones.
    heartbeat: (extra) => {
      if (!heartbeatPath || !heartbeatBase) return
      heartbeatBase = { ...heartbeatBase, ...extra }
      writeHeartbeat(sys.fs, heartbeatPath, { ...heartbeatBase, at: sys.now() })
    },
    setHeartbeatBase: (base) => {
      heartbeatBase = base
    },
  }
  return io
}

function writeHeartbeat(fs, file, record) {
  try {
    fs.mkdirSync(nodePath.dirname(file), { recursive: true, mode: 0o700 })
    const tmp = `${file}.${record.pid}.tmp`
    fs.writeFileSync(tmp, JSON.stringify(record) + '\n', { mode: 0o600 })
    fs.renameSync(tmp, file)
  } catch {}
}

/** What this opencode's plugin input offers, feature-detected (D8, recorded in the heartbeat). */
function capabilities(client) {
  const fn = (o, k) => !!o && typeof o[k] === 'function'
  return {
    append: fn(client?.session, 'prompt'),
    wake: fn(client?.session, 'promptAsync'),
    status: fn(client?.session, 'status'),
    session_get: fn(client?.session, 'get'),
    toast: fn(client?.tui, 'showToast'),
  }
}

const GUARD = Symbol.for('herdr-threads.opencode.plugin')

/** Builds one plugin instance; `sys` replaces the node APIs in tests. */
async function makePlugin(input, sys = DEFAULT_SYS, options = {}) {
  const argv = options.argv !== undefined ? options.argv : launchArgv()
  const grant = options.grant !== undefined ? options.grant : GRANT_PERMISSIONS
  const stateDir = stateDirOf(argv)
  const heartbeatPath = stateDir ? nodePath.join(stateDir, ...HEARTBEAT_FILE) : null
  const io = buildIo(input, sys, argv, heartbeatPath)
  // One instance per process (D3): a later instance or a second installed copy stays inert until
  // the owner is disposed, then the first hook call of a waiting instance takes over.
  const slot = (sys.guard[GUARD] ||= { owner: null })
  const me = { disposed: false }
  const owns = () => {
    if (me.disposed) return false
    if (!slot.owner || slot.owner.disposed) slot.owner = me
    return slot.owner === me
  }
  const core = createCore(io)
  const active = owns()
  // The heartbeat names the instance that owns now: one that loaded inert and took over (a reload
  // whose new instance loaded before the old one was disposed) rewrites it as active.
  let role = active ? 'active' : 'inert'
  const owning = () => {
    if (!owns()) return false
    if (role !== 'active') {
      role = 'active'
      try {
        io.heartbeat({ role })
      } catch {}
    }
    return true
  }
  const wrap = (fn) => async (...args) => {
    try {
      if (owning()) await fn(...args)
    } catch {}
  }
  const stopTimer = sys.every(1000, () => {
    if (me.disposed || slot.owner !== me) return
    core.onTick().catch(() => {})
  })
  let permission = { granted: [], denied: [] }
  const hooks = {
    config: wrap(async (cfg) => {
      if (!grant) return
      permission = grantPermissions(cfg)
      io.heartbeat({ permission })
    }),
    'shell.env': wrap((i, o) => core.onShellEnv(i, o)),
    'chat.message': wrap((i, o) => core.onChatMessage(i, o)),
    event: wrap(async ({ event } = {}) => core.onEvent(event)),
    dispose: async () => {
      try {
        me.disposed = true
        stopTimer()
        core.dispose()
        if (slot.owner === me) slot.owner = null
      } catch {}
    },
  }
  // The heartbeat comes last: code before a failing init also runs, so only this write proves
  // the hooks were set up (D6).
  io.setHeartbeatBase({
    schema: 1,
    plugin: 'herdr-threads',
    pid: io.pid,
    role,
    directory: io.directory,
    launch: !!argv,
    permissions: grant ? 'consented' : 'not_consented',
    capabilities: capabilities(input?.client),
  })
  io.heartbeat({ loaded_at: sys.now() })
  return { hooks, core, io }
}

export const HerdrThreads = async (input) => (await makePlugin(input)).hooks

HerdrThreads.internals = {
  createCore,
  makePlugin,
  frame,
  markers,
  batchOrder,
  appendBatch,
  markerRecovery,
  launchRecovery,
  grantPermissions,
  stateDirOf,
  constants: { WAKE_PROMPT, UNAVAILABLE, HEADER, SESSION_VAR, PARENT_VAR, PID_VAR, HEARTBEAT_FILE, GRANT_KEYS, GUARD },
}
