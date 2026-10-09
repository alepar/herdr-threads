// Seeded randomized stress for the herdr-threads Claude mod (spec D10). Each
// schedule interleaves turn events, tool calls, prompt-box text, stream items,
// submit/append/ack outcomes, child exits, reloads and session changes against
// createCore with fakes, and checks the delivery invariants as calls happen.
// The runner gives a test no environment, so the base seed is the constant
// below; a failure reports seed, schedule and step so it can be replayed.
import { test, expect } from 'claude-code/testing'
import { createCore } from '../hooks/register.js'

type Any = any

const BASE_SEED = 20261009
const SCHEDULES = 600
const STEPS = 60
const HOLD_MS = 120_000
// Set to a failing schedule's seed to print its steps and calls when replaying.
const TRACE_SEED: number | null = null

const flush = async () => {
  for (let i = 0; i < 60; i++) await Promise.resolve()
}

function prng(seed: number) {
  let a = seed >>> 0
  const next = () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
  return {
    next,
    int: (n: number) => Math.floor(next() * n),
    chance: (p: number) => next() < p,
    pick: <T>(xs: readonly T[]): T => xs[Math.floor(next() * xs.length)],
  }
}

const totals = { submits: 0, contexts: 0, appends: 0, acks: 0, drops: 0, holdBlocks: 0, reloads: 0, channelLost: 0, schedules: 0 }

function runSchedule(seed: number) {
  const r = prng(seed)
  const world: Any = {
    t: 1_000_000,
    sid: 's0',
    sidN: 0,
    box: '',
    stateVal: r.chance(0.5) ? { open: [], assumedBusy: false, abortHoldSince: null } : undefined,
    store: new Map<string, Any>(),
    spawns: [] as Any[],
    pendingSubmits: [] as Any[],
    ackChecks: [] as Any[],
    pendingReads: [] as Array<() => void>,
    slowReads: r.chance(0.5),
    manualSubmit: r.chance(0.4),
    dropRate: r.pick([0, 0.2, 0.5]),
    denyRate: r.pick([0, 0.3]),
  }
  // model of what the engine told the mod
  const m: Any = { mainOpen: null as string | null, lingering: [] as string[], hold: false, holdSince: 0 }
  const items = new Map<string, Any>() // streamed items by id
  const deliveredVia = new Map<string, string>() // `${sid}|${id}` -> via
  const settled = new Set<string>()
  const okVia = { submit: new Set<string>(), append: new Set<string>(), context: new Set<string>() }
  const key = (sid: string, id: string) => `${sid}|${id}`
  let tn = 0
  let mn = 0
  let core: Any
  let sidAtCall = 'unknown'

  // The core swallows exceptions thrown by io, so violations are also recorded
  // here and checked after every step.
  const violations: string[] = []
  const fail = (what: string): never => {
    violations.push(what)
    throw new Error(what)
  }

  // which spawn index last streamed each id
  const streamedBy = new Map<string, number>()
  const checkRun = (ids: string[]) => {
    const c = world.spawns[world.spawns.length - 1]
    if (!c || !c.connected || c.ended || c.stopped) fail('delivery while the watch run is not connected')
    for (const id of ids) {
      if (streamedBy.get(id) !== world.spawns.length - 1) fail(`${id} delivered from a run that ended`)
    }
  }

  const idsOf = (text: string) => [...text.matchAll(/\[herdr-threads\] (?:message|lazy) (\S+) in /g)].map((x) => x[1])

  const io: Any = {
    bin: 'herdr-threads',
    apis: async () => true,
    now: () => world.t,
    sessionId: async () => world.sid,
    promptRead: async () => {
      const text = world.box
      if (world.slowReads && r.chance(0.4)) await new Promise<void>((res) => world.pendingReads.push(res))
      return { text }
    },
    submit: (text: string) => {
      if (seed === TRACE_SEED) console.log('  submit', idsOf(text).join(','), 'sid', core.snapshot().sid)
      if (m.mainOpen !== null) fail(`submit while main turn ${m.mainOpen} is open`)
      if (m.hold) fail('submit within the post-abort hold')
      totals.submits++
      const ids = idsOf(text)
      checkRun(ids)
      const done = (res: Any) => {
        if (res.drop === undefined) for (const id of ids) okVia.submit.add(key(sidAtCall, id))
        else totals.drops++
        return res
      }
      const sidNow = core.snapshot().sid
      sidAtCall = sidNow
      if (world.manualSubmit) {
        return new Promise((res, rej) => world.pendingSubmits.push({ res, rej, ids, sid: sidNow, text }))
      }
      const roll = r.next()
      if (roll < 0.05) return Promise.reject(new Error('submit rejected'))
      if (roll < 0.05 + world.dropRate) return Promise.resolve({ drop: 'hook blocked' })
      for (const id of ids) okVia.submit.add(key(sidNow, id))
      return Promise.resolve({})
    },
    append: (text: string) => {
      totals.appends++
      const ids = idsOf(text)
      checkRun(ids)
      const sidNow = core.snapshot().sid
      if (r.chance(world.denyRate)) return Promise.resolve({ deny: 'denied' })
      for (const id of ids) okVia.append.add(key(sidNow, id))
      return Promise.resolve({})
    },
    run: async (argv: string[]) => {
      totals.acks++
      if (seed === TRACE_SEED) console.log('  ack', argv.slice(4).join(' '))
      const sid = argv[argv.indexOf('--session') + 1]
      const via = argv[argv.indexOf('--via') + 1]
      const ids = argv.slice(argv.indexOf('--via') + 2)
      for (const id of ids) {
        const k = key(sid, id)
        if (id.startsWith('attention:')) fail(`attention ${id} acked`)
        const it = items.get(id)
        if (!it) fail(`acked unknown id ${id}`)
        if (it.truncated) fail(`truncated ${id} acked`)
        if (deliveredVia.get(k) !== via) fail(`${id} acked via ${via} but delivered via ${deliveredVia.get(k)}`)
        // the context result is registered when the tool.call step returns it
        world.ackChecks.push({ via, k, id })
        if (settled.has(k)) fail(`${id} acked again after a settling result`)
      }
      const roll = r.next()
      if (roll < 0.1) return { code: 1, stdout: '' }
      if (roll < 0.15) throw new Error('ack spawn failed')
      const lines: string[] = []
      for (const id of ids) {
        const x = r.next()
        let result = 'settled'
        if (x < 0.15) result = 'retryable'
        else if (x < 0.2) result = 'already_settled'
        else if (x < 0.25) result = 'refused_terminal'
        else if (x < 0.3) result = 'stale_generation'
        if (x >= 0.35 || x < 0.3) lines.push(JSON.stringify({ id, result }))
        const k = key(sid, id)
        if (result === 'settled' || result === 'already_settled') settled.add(k)
        if (result === 'stale_generation') deliveredVia.delete(k)
        if (result === 'refused_terminal') settled.add(k)
        if (x >= 0.3 && x < 0.35) {
          // a line the daemon never printed: the mod must treat it as retryable
          if (result === 'settled' || result === 'already_settled') settled.delete(k)
        }
      }
      return { code: 0, stdout: lines.join('\n') }
    },
    log: () => {},
    store: {
      get: async (k: string) => world.store.get(k),
      set: async (k: string, v: Any) => void world.store.set(k, JSON.parse(JSON.stringify(v))),
      delete: async (k: string) => void world.store.delete(k),
    },
    state: {
      get: async () => world.stateVal,
      set: async (v: Any) => void (world.stateVal = JSON.parse(JSON.stringify(v))),
    },
    ledger: (e: Any) => {
      if (seed === TRACE_SEED) console.log('  ledger', e.kind, e.ids.join(','), e.via ?? '', e.reason ?? '')
      if (e.kind === 'delivered') {
        const sid = core.snapshot().sid
        for (const id of e.ids) {
          if (id.startsWith('attention:')) continue
          const k = key(sid, id)
          if (deliveredVia.has(k)) fail(`${id} delivered twice in session ${sid}`)
          deliveredVia.set(k, e.via)
        }
      }
      if (e.kind === 'held') totals.holdBlocks++
      if (e.kind === 'refused' && String(e.reason).startsWith('channel_lost:')) totals.channelLost++
    },
    spawn: (argv: string[], cb: Any) => {
      const c = { argv, cb, stopped: false, connected: false, ended: false, stop() { c.stopped = true } }
      world.spawns.push(c)
      return c
    },
  }

  const child = () => world.spawns[world.spawns.length - 1]
  const newCore = async () => {
    core = createCore(io)
    await core.onLoad()
  }

  const mirrorTick = () => {
    if (m.hold && m.mainOpen === null && world.t - m.holdSince >= HOLD_MS && world.box === '') m.hold = false
  }
  const advance = async (ms: number) => {
    world.t += ms
    mirrorTick()
    let done = false
    const tick = core.onTick().then(() => void (done = true))
    // the tick awaits the prompt box itself: let slow reads finish
    while (!done) {
      await flush()
      for (const release of world.pendingReads.splice(0)) release()
    }
    await tick
  }
  const settleAll = () => {
    for (const p of world.pendingSubmits.splice(0)) {
      if (r.chance(0.2)) p.rej(new Error('late reject'))
      else if (r.chance(0.3)) {
        totals.drops++
        p.res({ drop: 'late drop' })
      } else {
        for (const id of p.ids) okVia.submit.add(key(p.sid, id))
        p.res({})
      }
    }
  }

  const newItem = (kind: string) => {
    mn++
    const id = `m${seed % 1000}-${mn}`
    const truncated = kind !== 'attention' && r.chance(0.12)
    const it: Any = {
      schema: 1,
      id,
      kind,
      thread: 'T',
      thread_name: 'plans',
      sender: 'S',
      sender_name: 'alice',
      body: truncated ? 'cut…truncated; run herdr-threads body ' + id : `body ${id}`,
      body_len: 10,
      truncated,
      ack_required: kind === 'message',
    }
    return it
  }

  const steps: Array<[number, string, () => Promise<void>]> = [
    [8, 'turn.start', async () => {
      const id = `t${++tn}`
      if (m.mainOpen !== null) m.lingering.push(m.mainOpen)
      m.mainOpen = id
      core.onTurnStart({ turnId: id })
    }],
    [9, 'turn.complete', async () => {
      const pool: string[] = []
      if (m.mainOpen !== null) pool.push(m.mainOpen, m.mainOpen)
      pool.push(...m.lingering)
      pool.push(`unknown${r.int(5)}`)
      const id = r.pick(pool)
      // A predecessor submit this core holds reads a completion of a turn it never saw start as the
      // submitted turn (rule b); the stress model does not know which one that is, so it waits.
      const held = core.snapshot().pred
      if (held && !held.sawStart) return
      const aborted = r.chance(0.35)
      m.lingering = m.lingering.filter((x: string) => x !== id)
      if (id === m.mainOpen) m.mainOpen = null
      if (aborted) {
        m.hold = true
        m.holdSince = world.t
      } else m.hold = false
      core.onTurnComplete({ turnId: id, isAborted: aborted })
    }],
    [3, 'subagent turn', async () => {
      core.onTurnStart({ turnId: `sub${r.int(5)}`, agentId: 'a1' })
      if (r.chance(0.5)) core.onTurnComplete({ turnId: `sub${r.int(5)}`, agentId: 'a1', isAborted: r.chance(0.5) })
    }],
    [18, 'tool.call', async () => {
      const sub = r.chance(0.2)
      const kind = r.pick(['answered', 'answered', 'denied', 'error'])
      const input: Any =
        kind === 'answered' ? { result: { ok: 1 }, text: 'x', context: r.chance(0.3) ? ['pre'] : undefined } : kind === 'denied' ? { deny: 'no' } : { isError: true, text: 'boom' }
      const e: Any = sub ? { tool: 'Bash', agentId: 'a1' } : { tool: 'Bash' }
      const out = await core.onToolCall(e, input)
      if (out !== input) {
        if (sub || kind !== 'answered') fail(`context attached to a ${sub ? 'subagent' : kind} result`)
        const ctx = out.context[out.context.length - 1]
        totals.contexts++
        checkRun(idsOf(ctx))
        for (const id of idsOf(ctx)) okVia.context.add(key(core.snapshot().sid, id))
      }
    }],
    [5, 'box', async () => {
      world.box = r.chance(0.5) ? '' : 'draft text'
    }],
    [14, 'stream item', async () => {
      const c = child()
      if (c.ended) return
      if (!c.connected && r.chance(0.9)) {
        c.connected = true
        c.cb.line({ schema: 1, id: 'status:1', kind: 'status', state: 'connected' })
      }
      const kind = r.pick(['message', 'message', 'lazy', 'attention'])
      if (kind === 'attention') {
        const v = 1 + r.int(4)
        child().cb.line({ schema: 1, id: `attention:${v}`, kind: 'attention', attention_version: v, text: `marker ${v}` })
        return
      }
      const it = newItem(kind)
      items.set(it.id, it)
      streamedBy.set(it.id, world.spawns.length - 1)
      c.cb.line(it)
    }],
    [4, 'restream', async () => {
      const c = child()
      if (c.ended) return
      const ids = [...items.keys()]
      if (!ids.length) return
      if (!c.connected && r.chance(0.9)) {
        c.connected = true
        c.cb.line({ schema: 1, id: 'status:1', kind: 'status', state: 'connected' })
      }
      const id = r.pick(ids)
      streamedBy.set(id, world.spawns.length - 1)
      c.cb.line(items.get(id))
    }],
    [5, 'status', async () => {
      const c = child()
      if (c.ended) return
      c.connected = true
      c.cb.line({ schema: 1, id: 'status:1', kind: 'status', state: 'connected' })
    }],
    [10, 'resolve submit', async () => {
      const p = world.pendingSubmits.shift()
      if (!p) return
      const x = r.next()
      if (x < 0.15) p.rej(new Error('rejected'))
      else if (x < 0.4) {
        totals.drops++
        p.res({ drop: 'dropped' })
      } else {
        for (const id of p.ids) okVia.submit.add(key(p.sid, id))
        p.res({})
      }
    }],
    [10, 'advance', async () => advance(r.pick([300, 1000, 5000, 31_000, 121_000]))],
    [4, 'child exit', async () => {
      const c = child()
      c.ended = true
      c.cb.exit(r.int(4))
    }],
    [3, 'close', async () => {
      const c = child()
      if (c.ended) return
      const reason = r.pick(['stalled', 'retired', 'disabled', 'replaced', 'stream_ended'])
      const exit = reason === 'disabled' || reason === 'replaced' ? 3 : 0
      c.cb.line({ schema: 1, id: 'status:9', kind: 'status', state: 'closing', reason, exit })
      c.ended = true
      c.cb.exit(exit)
    }],
    [2, 'reload', async () => {
      // A reload with a submit in flight no longer delivers twice: half the reloads settle every
      // in-flight submit first, the rest leave them unresolved and the successor holds the ids.
      if (r.chance(0.5)) {
        for (let i = 0; i < 20 && (world.pendingSubmits.length || world.pendingReads.length); i++) {
          for (const release of world.pendingReads.splice(0)) release()
          settleAll()
          await flush()
        }
        totals.reloads++
        core.dispose()
        await newCore()
        return
      }
      for (const release of world.pendingReads.splice(0)) release()
      await flush()
      // a disposed core's promise never resolves for it
      const inflight = world.pendingSubmits.splice(0)
      totals.reloads++
      core.dispose()
      await newCore()
      for (const p of inflight) {
        // the disposed core's late resolution reaches nobody: it must change no delivery
        if (r.chance(0.5)) p.res({})
        // never produced a turn: the ids arrive again through the stream steps
        if (r.chance(0.3)) continue
        const held = core.snapshot().pred
        if (!held || held.ids.join(',') !== p.ids.join(',')) continue
        // produced a turn whose start the successor sees, with the frame as its prompt
        for (const id of p.ids) okVia.submit.add(key(p.sid, id))
        if (m.mainOpen !== null) m.lingering.push(m.mainOpen)
        m.mainOpen = `t${++tn}`
        core.onTurnStart({ turnId: m.mainOpen, text: p.text })
      }
    }],
    [2, 'session end', async () => {
      const reason = r.pick(['clear', 'resume', 'branch'])
      core.onSessionEnd(reason === 'branch' ? 'resume' : reason)
      if (reason !== 'resume') world.sid = `s${++world.sidN}`
      await advance(0)
    }],
  ]
  const total = steps.reduce((a, s) => a + s[0], 0)

  return (async () => {
    await newCore()
    await flush()
    for (let i = 0; i < STEPS; i++) {
      let roll = r.int(total)
      let step = steps[0]
      for (const s of steps) {
        if (roll < s[0]) {
          step = s
          break
        }
        roll -= s[0]
      }
      try {
        if (seed === TRACE_SEED) console.log('STEP', i, step[1])
        const slow = world.pendingReads.splice(0)
        await step[2]()
        // a read of the prompt box that began earlier completes only after this event
        for (const release of slow) release()
        await flush()
        for (const c of world.ackChecks.splice(0)) {
          if (!(okVia as Any)[c.via]?.has(c.k)) fail(`${c.id} acked via ${c.via} though its delivered predicate never held`)
        }
        if (violations.length) throw new Error(violations[0])
        const snap = core.snapshot()
        for (const id of Object.keys(snap.rec.unacked)) {
          if (snap.rec.delivered[id] === undefined) fail(`${id} unacked but not delivered`)
          if (items.get(id)?.truncated) fail(`truncated ${id} kept for ack`)
        }
        if (snap.turns.open.length > 1) fail('more than one open main turn id')
      } catch (err) {
        throw new Error(`seed=${seed} step=${i} op=${step[1]}: ${String(err).replace(/^Error: /, '')}`)
      }
    }
    for (const release of world.pendingReads.splice(0)) release()
    settleAll()
    await advance(31_000)
    await flush()
    core.dispose()
    totals.schedules++
  })()
}

test('randomized stress: no submit during a turn or hold, ids delivered and acked at most once', async () => {
  for (let k = 0; k < SCHEDULES; k++) {
    try {
      await runSchedule(BASE_SEED + k)
    } catch (err) {
      throw new Error(`schedule ${k} (base seed ${BASE_SEED}): ${String(err)}`)
    }
  }
  expect(totals.schedules).toBe(SCHEDULES)
  // the schedules must have exercised every path, or the invariants were vacuous
  expect(totals.submits).toBeGreaterThan(300)
  expect(totals.contexts).toBeGreaterThan(300)
  expect(totals.appends).toBeGreaterThan(100)
  expect(totals.acks).toBeGreaterThan(300)
  expect(totals.drops).toBeGreaterThan(50)
  expect(totals.holdBlocks).toBeGreaterThan(100)
  expect(totals.reloads).toBeGreaterThan(100)
  expect(totals.channelLost).toBeGreaterThan(50)
})
