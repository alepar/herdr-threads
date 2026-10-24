const lines = []
let n = 0
let file = null
let w = Promise.resolve()
function log($, o) {
  lines.push(JSON.stringify({ at: Date.now(), ...o }))
  if (file) w = w.then(() => $.fs.write(file, lines.join('\n') + '\n')).catch(() => {})
}
const brief = (v) => { try { const s = JSON.stringify(v); return s && s.length > 400 ? s.slice(0, 400) + '…' : s } catch { return String(v) } }
export const register = (on) => {
  on('session.start', async ($, e, next) => {
    const r = await next(e)
    file = await $.env.get('HT_PROBE_LOG')
    log($, { ev: 'session.start' })
    return r
  })
  on('tool.call', async ($, e, next) => {
    const r = await next(e)
    n += 1
    const nonce = `PROBE-${n}-${Math.random().toString(36).slice(2, 8)}`
    log($, { ev: 'tool.call', nonce, tool: e && (e.name ?? e.tool ?? e.toolName), agentId: e && e.agentId, ekeys: e && Object.keys(e), rkeys: r && Object.keys(r), isError: r && r.isError, deny: r && r.deny, hasResult: r && r.result !== undefined, result: brief(r && r.result), input: brief(e && e.input) })
    if (e && e.agentId) return r
    return { ...r, context: [...((r && r.context) || []), `[probe] nonce ${nonce}`] }
  })
  on('turn.step', async function* ($, e, next) {
    log($, { ev: 'turn.step', index: e && e.index, turnId: e && e.turnId, agentId: e && e.agentId, messageCount: e && e.messageCount })
    return yield* next(e)
  })
  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    log($, { ev: 'turn.complete', keys: e && Object.keys(e), isAborted: e && e.isAborted, reason: e && e.reason, agentId: e && e.agentId })
    return r
  })
}
