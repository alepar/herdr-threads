// A named seat (HERDR_THREADS_SEAT_NAME, no HERDR_PANE_ID): the live mod's watch child carries the
// name (and the read-only marker when set) and the mod never stops on `no_pane`. `claude plugin test`
// runs this file.
import { test, expect, mock } from 'claude-code/testing'
import { createCore, register } from '../hooks/register.js'

type Any = any

const connected = { schema: 1, id: 'status:1', kind: 'status', state: 'connected' }
const status = (state: string, reason: string, exit: number) => ({ schema: 1, id: 'status:9', kind: 'status', state, reason, exit })

// Starts the production register() under the engine with `env` as the session environment and
// returns what the watch child was spawned with. The child prints `lines`, then exits 3.
async function startMod($: Any, on: Any, env: Record<string, string>, lines: Any[]) {
  mock.clock(on, { now: 5_000 })
  mock.store(on)
  mock.env(on, { HERDR_THREADS_BIN: 'ht-named', ...env })
  const spawned: Any[] = []
  on('session.start', async (_$: Any, e: Any) => ({ cwd: e.cwd }) as Any)
  on('session.id', async () => ({ value: 'named-1' }) as Any)
  on('fs.write', async () => ({ value: undefined }) as Any)
  on('state.get', async () => ({ value: { value: undefined, version: 0 } }) as Any)
  on('state.set', async () => ({ value: { isSet: true, version: 1 } }) as Any)
  on('process.spawn', async function* (_$: Any, e: Any) {
    spawned.push(e)
    for (const line of lines) yield { stream: 'stdout', text: JSON.stringify(line) + '\n' } as Any
    return { value: { code: 3, signal: null } } as Any
  })
  expect(typeof register).toBe('function')
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  for (let i = 0; i < 100 && spawned.length === 0; i++) await new Promise<void>((r) => setTimeout(r, 5))
  return spawned
}

test('named seat: the watch child gets the name and no pane is needed', async ($, on) => {
  const spawned = await startMod($, on, { HERDR_THREADS_SEAT_NAME: 'worker-1' }, [connected])
  expect(spawned.length).toBe(1)
  expect(spawned[0].argv).toEqual(['ht-named', 'watch', '--harness', 'claude', '--session', 'named-1'])
  expect(spawned[0].env).toEqual({ HERDR_THREADS_SEAT_NAME: 'worker-1' })
})

test('named seat: the read-only marker rides along when set', async ($, on) => {
  const spawned = await startMod(
    $,
    on,
    { HERDR_THREADS_SEAT_NAME: 'worker-1', HERDR_THREADS_SEAT_READONLY: '1' },
    [connected],
  )
  expect(spawned[0].env).toEqual({ HERDR_THREADS_SEAT_NAME: 'worker-1', HERDR_THREADS_SEAT_READONLY: '1' })
})

// A minimal io for createCore: only the watch child's lifecycle is observable.
async function coreWith(watchEnv: Any) {
  let t = 1_000_000
  const spawns: Any[] = []
  const io: Any = {
    bin: 'herdr-threads',
    argv: null,
    watchEnv,
    apis: async () => true,
    now: () => t,
    sessionId: async () => 'named-1',
    promptRead: async () => ({ text: '' }),
    submit: async () => ({}),
    append: async () => ({}),
    run: async () => ({ code: 0, stdout: '' }),
    log: () => {},
    store: { get: async () => undefined, set: async () => {}, delete: async () => {} },
    state: { get: async () => undefined, set: async () => {} },
    ledger: () => {},
    spawn: (argv: string[], cb: Any) => {
      const c = { argv, cb, stop() {} }
      spawns.push(c)
      return c
    },
  }
  const core = createCore(io)
  await core.onLoad()
  for (let i = 0; i < 60; i++) await Promise.resolve()
  return {
    core,
    spawns,
    async advance(ms: number) {
      t += ms
      await core.onTick()
      for (let i = 0; i < 60; i++) await Promise.resolve()
    },
  }
}

test('named seat: a no_pane refusal does not stop the mod and the child restarts', async () => {
  const h = await coreWith({ HERDR_THREADS_SEAT_NAME: 'worker-1' })
  h.spawns[0].cb.line(status('refused', 'no_pane', 3))
  h.spawns[0].cb.exit(3)
  await h.advance(60_000)
  expect(h.core.snapshot().stopped).toBe(false)
  expect(h.spawns.length).toBe(2)
  h.core.dispose()
})

test('a pane seat still stops permanently on no_pane', async () => {
  const h = await coreWith(null)
  h.spawns[0].cb.line(status('refused', 'no_pane', 3))
  h.spawns[0].cb.exit(3)
  await h.advance(60_000)
  expect(h.core.snapshot().stopped).toBe(true)
  expect(h.spawns.length).toBe(1)
  h.core.dispose()
})

test('a pane seat: the continuity key is not forwarded; the watch child keeps the argv-only spawn', async ($, on) => {
  const spawned = await startMod($, on, { HERDR_THREADS_CONTINUITY_KEY: 'clone-1' }, [connected])
  expect(spawned[0].argv).toEqual(['ht-named', 'watch', '--harness', 'claude', '--session', 'named-1'])
  expect(spawned[0].env).toBeUndefined()
})

test('a Herdr pane session spawns the child with the unchanged argv-only event', async ($, on) => {
  const spawned = await startMod($, on, {}, [connected])
  expect(spawned[0].argv).toEqual(['ht-named', 'watch', '--harness', 'claude', '--session', 'named-1'])
  expect(spawned[0].env).toBeUndefined()
})
