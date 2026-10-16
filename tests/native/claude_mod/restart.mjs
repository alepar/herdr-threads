// Model-free bridge for mod_delivery's isolated-daemon restart regression.
// The Rust owner tags/owns this process group, including every watch child.
import { spawn } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { createInterface } from 'node:readline'

// The mod is a self-contained ES module, installed without a Node package.json.
const source = await readFile(new URL('../../../integrations/claude/mod/hooks/register.js', import.meta.url))
const { createCore } = await import(`data:text/javascript;base64,${source.toString('base64')}`)

const [bin, state, host, sid] = process.argv.slice(2)
const emit = (line) => process.stdout.write(JSON.stringify(line) + '\n')
const store = new Map()
let injectNext = false
let injected = false
const core = createCore({
  argv: [bin, '--state-dir', state, '--host-endpoint', host],
  now: () => Date.now(),
  apis: async () => true,
  sessionId: async () => sid,
  state: { get: async () => undefined, set: async () => {} },
  store: {
    get: async (key) => store.get(key),
    set: async (key, value) => { store.set(key, value) },
    delete: async (key) => { store.delete(key) },
  },
  promptRead: async () => ({ text: '' }),
  append: async (text) => { emit({ kind: 'append', text }); return {} },
  submit: async () => ({}),
  // Keep receipts pending: re-stream after reconnect must still be possible.
  run: async () => ({ code: 1, stdout: '' }),
  log: () => {},
  ledger: (entry) => emit({ kind: 'ledger', ...entry }),
  spawn: (argv, callbacks) => {
    if (injectNext && !injected) {
      injected = true
      // An older watch binary failed before printing a status in the restart
      // window. This is the general CLI's unavailable exit, not a stop.
      queueMicrotask(() => {
        emit({ kind: 'startup_failure', code: 3 })
        callbacks.exit(3)
      })
      return { stop() {} }
    }
    const child = spawn(argv[0], argv.slice(1), { stdio: ['ignore', 'pipe', 'inherit'] })
    let connected = false
    const lines = createInterface({ input: child.stdout })
    lines.on('line', (text) => {
      const line = JSON.parse(text)
      if (line.kind === 'status' && line.state === 'connected') connected = true
      callbacks.line(line)
      emit(line)
    })
    child.on('error', (error) => { throw error })
    child.on('close', (code) => {
      if (connected) injectNext = true
      callbacks.exit(code)
    })
    return { stop() { child.kill() } }
  },
})
await core.onLoad()
const timer = setInterval(() => { void core.onTick() }, 25)
process.on('SIGTERM', () => {
  clearInterval(timer)
  core.dispose()
})
