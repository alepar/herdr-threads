// Manual fan-out adapter: replay the canonical engine against persisted live-agent output.
// Missing agent calls persist their exact assembled prompts and remain unresolved; Node
// exits once no active handles remain. A later replay advances from the cached results.
import { readFileSync, writeFileSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const root = dirname(fileURLToPath(import.meta.url));
const args = JSON.parse(readFileSync(join(root, 'args.json'), 'utf8'));
const doc = readFileSync('/Users/alepar/.aisw/profiles/codex/codex-3/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar4.20/skills/super-roast/super-roast-workflow.md', 'utf8');
const start = doc.indexOf('```javascript\n');
const end = doc.indexOf('\n```\n', start);
const engine = doc.slice(start + '```javascript\n'.length, end).replace('export const meta =', 'const meta =');
mkdirSync(join(root, 'calls'), { recursive: true });
rmSync(join(root, 'pending.json'), { force: true });
const pending = [];
const logs = [];
async function agent(prompt, opts) {
  const key = opts.label.replace(/[^a-zA-Z0-9_-]/g, '_');
  const hash = createHash('sha256').update(prompt).digest('hex');
  const base = join(root, 'calls', key);
  const request = { key, hash, prompt, opts };
  writeFileSync(base + '.request.json', JSON.stringify(request, null, 2) + '\n');
  if (existsSync(base + '.response.json')) {
    const cached = JSON.parse(readFileSync(base + '.response.json', 'utf8'));
    if (cached.hash !== hash) throw new Error('Stale response for ' + key);
    return cached.result;
  }
  pending.push(request);
  writeFileSync(join(root, 'pending.json'), JSON.stringify(pending, null, 2) + '\n');
  return new Promise(() => {});
}
const parallel = tasks => Promise.all(tasks.map(async task => { try { return await task(); } catch (error) { logs.push(String(error)); return null; } }));
const log = value => logs.push(value);
const phase = () => {};
const AsyncFunction = Object.getPrototypeOf(async function() {}).constructor;
const run = new AsyncFunction('args', 'agent', 'parallel', 'log', 'phase', engine);
run(args, agent, parallel, log, phase).then(result => {
  writeFileSync(join(root, 'result.json'), JSON.stringify(result, null, 2) + '\n');
  writeFileSync(join(root, 'coverage.json'), JSON.stringify(result.coverage, null, 2) + '\n');
  writeFileSync(join(root, 'report.md'), result.reportMarkdown + '\n');
}).catch(error => { writeFileSync(join(root, 'error.txt'), String(error.stack)); process.exitCode = 1; });
process.on('exit', () => writeFileSync(join(root, 'engine-log.json'), JSON.stringify(logs, null, 2) + '\n'));
