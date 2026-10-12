import assert from 'node:assert/strict'
import fs from 'node:fs'
import { stripTypeScriptTypes } from 'node:module'
import { pathToFileURL } from 'node:url'
const tests = []
globalThis.taskTest = (name, fn) => { if (/^(bounded invitation|framed append batches|acks are capped)|whole same-revision goal arriving|changed command context arriving/.test(name)) tests.push({ name, fn }) }
globalThis.taskExpect = (actual) => {
  const checks = (negate = false) => {
    const check = (ok, expected) => assert.ok(negate ? !ok : ok, `${JSON.stringify(actual)} ${negate ? 'not ' : ''}${expected}`)
    return {
      toBe: expected => negate ? assert.notStrictEqual(actual, expected) : assert.strictEqual(actual, expected),
      toEqual: expected => negate ? assert.notDeepStrictEqual(actual, expected) : assert.deepStrictEqual(actual, expected),
      toContain: expected => check(actual.includes(expected), `contains ${JSON.stringify(expected)}`),
      toBeDefined: () => check(actual !== undefined, 'is defined'),
      toBeLessThanOrEqual: expected => check(actual <= expected, `<= ${expected}`),
      get not() { return checks(!negate) },
    }
  }
  return checks()
}
let source = fs.readFileSync('integrations/claude/mod/tests/delivery.test.ts', 'utf8')
source = source.replace("import { test, expect, mock } from 'claude-code/testing'", 'const test = globalThis.taskTest; const expect = globalThis.taskExpect; const mock = {}')
source = source.replace("'../hooks/register.js'", JSON.stringify(pathToFileURL(process.cwd() + '/integrations/claude/mod/hooks/register.js').href))
source = source.replace("import activationCapture from './fixtures/watch-activation.js'", 'const activationCapture = {}')
await import('data:text/javascript;base64,' + Buffer.from(stripTypeScriptTypes(source)).toString('base64'))
let failed = 0
for (const { name, fn } of tests) {
  try { await fn(); console.log('PASS', name) } catch (error) { failed++; console.log('FAIL', name, error.message) }
}
console.log(`${tests.length - failed} passed, ${failed} failed, ${tests.length} selected`)
process.exitCode = failed ? 1 : 0
