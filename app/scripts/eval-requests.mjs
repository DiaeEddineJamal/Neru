#!/usr/bin/env node
// How many model requests Neru spends on a set of scripted tasks, checked against a saved baseline.
// Free plans run out of requests per day before anything else, so a change that makes Neru send more
// of them (an extra helper call, a retry that should not happen, an edit that needs a second try) fails
// here. A change that sends fewer prints the improvement; run with --update to lower the baseline.
//
//   node app/scripts/eval-requests.mjs <path to neru-cli> [--update]
//
// The model is a scripted OpenAI-compatible server in this script, as in smoke.mjs: it answers each
// scenario the same way every time, so the counts measure what Neru adds around the model (helper
// requests, retries, repairs, switches), not how well a model works. Each scenario runs twice: as a
// local model (lean prompt and tools) and as a large one (the full prompt and tool set).

import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const cli = resolve(process.argv[2] ?? '')
const update = process.argv.includes('--update')
if (!process.argv[2] || !existsSync(cli)) {
  console.error('Usage: node app/scripts/eval-requests.mjs <path to neru-cli> [--update]')
  process.exit(2)
}
const baselinePath = join(dirname(fileURLToPath(import.meta.url)), 'eval-requests.baseline.json')

const DONE = 'EVAL-DONE'
const read = path => ({ tool: 'read_file', args: { path } })
const answer = { text: DONE }

// Each step is one model reply in the agent loop. `fail` makes the first request of the scenario fail.
const SCENARIOS = {
  answer: { steps: [answer] },
  read: { steps: [read('a.txt'), answer] },
  'parallel-read': { steps: [{ tools: [read('a.txt'), read('b.txt'), { tool: 'search_text', args: { query: 'beta' } }] }, answer] },
  edit: { steps: [{ tool: 'propose_edit', args: { path: 'a.txt', old_string: 'alpha', new_string: 'alpha one' } }, answer] },
  'edit-miss': { steps: [{ tool: 'propose_edit', args: { path: 'a.txt', old_string: 'alpah', new_string: 'alpha one' } }, { tool: 'propose_edit', args: { path: 'a.txt', old_string: 'alpha', new_string: 'alpha one' } }, answer] },
  'bad-json': { steps: [{ tool: 'read_file', rawArgs: '{"path": "a.txt"' }, answer] },
  'text-protocol': { steps: [{ text: '<tool_call>{"name": "read_file", "arguments": {"path": "a.txt"}}</tool_call>' }, answer] },
  'rate-limit': { fail: { status: 429, headers: { 'retry-after': '1' }, body: { error: { message: 'Rate limit reached for requests per minute', type: 'rate_limit_error' } } }, steps: [answer] },
  'server-error': { fail: { status: 500, headers: {}, body: { error: { message: 'Internal server error' } } }, steps: [answer] },
  'multi-step': { steps: [{ tool: 'list_directory', args: { path: '.' } }, read('a.txt'), { tool: 'propose_edit', args: { path: 'a.txt', old_string: 'alpha', new_string: 'alpha one' } }, { tool: 'run_shell_command', args: { command: 'echo eval-ok' } }, answer] },
  // A model calling a parameter by another name: `pattern` for search_text's `query`.
  'alias-args': { steps: [{ tool: 'search_text', args: { pattern: 'beta' } }, answer] },
  // bad-json with Ollama running beside the cloud model: the repair goes to its small local model.
  'local-helper': { ollama: true, expectLocal: 1, expectSchema: true, steps: [{ tool: 'read_file', rawArgs: '{"path": "a.txt"' }, answer] },
}

// A scripted Ollama (native API), for scenarios with `ollama: true`. The others point OLLAMA_HOST at a
// closed port, so an Ollama running on the machine cannot change their counts.
let local = []
// Whether each repair Ollama received came with a JSON schema to hold the reply to.
let schemas = []
const ollama = createServer((request, response) => {
  let body = ''
  request.on('data', part => { body += part })
  request.on('end', () => {
    const json = value => { response.writeHead(200, { 'content-type': 'application/json' }); response.end(JSON.stringify(value)) }
    if (request.method === 'GET' && request.url.startsWith('/v1/models')) return json({ object: 'list', data: [{ id: 'gemma3:4b', object: 'model' }, { id: 'qwen3-coder:30b', object: 'model' }] })
    if (request.url.startsWith('/api/show')) return json({ capabilities: ['completion', 'tools'], model_info: { 'gemma3.context_length': 131072 } })
    if (request.method !== 'POST' || !request.url.startsWith('/api/chat')) return response.writeHead(404).end()
    const payload = JSON.parse(body || '{}')
    const system = (payload.messages ?? []).find(message => message.role === 'system')?.content ?? ''
    const repair = system.startsWith('You repair tool-call arguments')
    const reply = repair ? '{"path": "a.txt"}' : 'Eval'
    if (repair) schemas.push(typeof payload.format === 'object' && payload.format !== null)
    local.push(payload.model)
    response.writeHead(200, { 'content-type': 'application/x-ndjson' })
    response.write(`${JSON.stringify({ model: payload.model, message: { role: 'assistant', content: reply }, done: false })}\n`)
    response.end(`${JSON.stringify({ model: payload.model, message: { role: 'assistant', content: '' }, done: true, prompt_eval_count: 40, eval_count: 8 })}\n`)
  })
})

// edit-miss-block: a function whose first line the model miscopies. Like a real model, the script
// retries at once only when the error showed it every line it has to copy; otherwise it reads the file.
const BLOCK = ['function total(items) {', '  let sum = 0', '  for (const item of items) {', '    sum += item.price * item.quantity', '  }', '  return sum', '}']
const blockEdit = first => ({ tool: 'propose_edit', args: { path: 'c.js', old_string: [first, ...BLOCK.slice(1)].join('\n'), new_string: BLOCK.join('\n').replace('sum = 0', 'sum = 0 // running total') } })
SCENARIOS['edit-miss-block'] = {
  files: { 'c.js': `// cart\n\n${BLOCK.join('\n')}\n\nexport { total }\n` },
  script: messages => {
    const last = messages.filter(message => message.role === 'tool').map(text).at(-1)
    if (last === undefined) return blockEdit('export async function computeTotal(cartItems = []) {')
    if (last.startsWith('Applied')) return answer
    if (last.startsWith('Error')) return BLOCK.every(line => last.includes(line)) ? blockEdit(BLOCK[0]) : read('c.js')
    // The file was read: now the model can copy the block.
    return blockEdit(BLOCK[0])
  },
}

// find-edit: a project too large to send whole, and a request naming a function. Like a real model,
// the script edits at once when the function's code is already in the conversation; otherwise it
// searches for it, reads the file, then edits.
const DISCOUNT = '  return total - total * rate'
const moduleFiles = Object.fromEntries(Array.from({ length: 30 }, (_, n) => [`src/module${n}.js`, `// Module ${n}\nexport function helper${n}(value) {\n${'  value = value + 1\n'.repeat(40)}  return value\n}\n`]))
SCENARIOS['find-edit'] = {
  prompt: 'EVAL-find-edit: make applyDiscount round the result to cents',
  files: { ...moduleFiles, 'src/billing/invoice.js': `import { tax } from './tax'\n\nexport function applyDiscount(total, rate) {\n${DISCOUNT}\n}\n` },
  script: messages => {
    const results = messages.filter(message => message.role === 'tool').map(text)
    const edit = { tool: 'propose_edit', args: { path: 'src/billing/invoice.js', old_string: DISCOUNT, new_string: '  return Math.round((total - total * rate) * 100) / 100' } }
    if (results.at(-1)?.startsWith('Applied')) return answer
    if (messages.some(message => message.role !== 'tool' && text(message).includes(DISCOUNT))) return edit
    if (!results.length) return { tool: 'search_text', args: { query: 'applyDiscount' } }
    if (results.length === 1) return read('src/billing/invoice.js')
    return edit
  },
}

// Large: a name the lean check does not treat as small. Local servers otherwise always run lean.
const PROFILES = { lean: 'eval-model', full: 'eval-coder-70b' }

const text = message => typeof message.content === 'string' ? message.content : JSON.stringify(message.content ?? '')
let scenario = null
let failed = false
let seen = []
// The most tools any request of the run offered: the lean set is much smaller than the full one.
let offered = 0

/** The scripted reply for a request: a repair, a chore, or the scenario's next step. */
function reply(payload) {
  const messages = payload.messages ?? []
  const system = messages.find(message => message.role === 'system')
  if (system && text(system).startsWith('You repair tool-call arguments')) return { text: '{"path": "a.txt"}', kind: 'repair' }
  if (!payload.tools?.length && !messages.some(message => text(message).includes('EVAL-'))) return { text: 'Eval', kind: 'chore' }
  const step = messages.filter(message => message.role === 'assistant').length
  const next = step > 12 ? null : SCENARIOS[scenario].script?.(messages) ?? SCENARIOS[scenario].steps[step]
  return { ...(next ?? { text: `EVAL-OVERRAN at step ${step}` }), kind: 'main' }
}

function chunk(delta, finish = null) {
  return `data: ${JSON.stringify({ id: 'eval', object: 'chat.completion.chunk', created: 0, model: 'eval', choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`
}

const toolCalls = step => (step.tools ?? (step.tool ? [step] : [])).map((call, index) => ({ index, id: `call_${seen.length}_${index}`, type: 'function', function: { name: call.tool, arguments: call.rawArgs ?? JSON.stringify(call.args) } }))

const server = createServer((request, response) => {
  let body = ''
  request.on('data', part => { body += part })
  request.on('end', () => {
    if (request.method === 'GET' && request.url.startsWith('/v1/models')) {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ object: 'list', data: Object.values(PROFILES).map(id => ({ id, object: 'model', context_length: 131_072 })) }))
      return
    }
    if (request.method !== 'POST' || !request.url.startsWith('/v1/chat/completions')) {
      response.writeHead(404).end()
      return
    }
    const payload = JSON.parse(body || '{}')
    const step = reply(payload)
    seen.push(step.kind)
    offered = Math.max(offered, payload.tools?.length ?? 0)
    const fail = SCENARIOS[scenario].fail
    if (fail && step.kind === 'main' && !failed) {
      failed = true
      response.writeHead(fail.status, { 'content-type': 'application/json', ...fail.headers })
      response.end(JSON.stringify(fail.body))
      return
    }
    const calls = toolCalls(step)
    if (!payload.stream) {
      const message = calls.length ? { role: 'assistant', content: null, tool_calls: calls.map(({ index: _, ...call }) => call) } : { role: 'assistant', content: step.text }
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ id: 'eval', object: 'chat.completion', model: 'eval', choices: [{ index: 0, message, finish_reason: calls.length ? 'tool_calls' : 'stop' }], usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } }))
      return
    }
    response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' })
    response.write(chunk({ role: 'assistant', content: '' }))
    if (calls.length) {
      response.write(chunk({ tool_calls: calls }))
      response.write(chunk({}, 'tool_calls'))
    } else {
      response.write(chunk({ content: step.text }))
      response.write(chunk({}, 'stop'))
    }
    response.end('data: [DONE]\n\n')
  })
})

function neru(args, cwd, env) {
  return new Promise((done, fail) => {
    const child = spawn(cli, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', part => { stdout += part })
    child.stderr.on('data', part => { stderr += part })
    const timer = setTimeout(() => { child.kill(); fail(new Error(`neru ${args.join(' ')} timed out\n${stdout}\n${stderr}`)) }, 180_000)
    child.on('error', fail)
    child.on('close', code => { clearTimeout(timer); done({ code, stdout, stderr }) })
  })
}

const baseline = existsSync(baselinePath) ? JSON.parse(readFileSync(baselinePath, 'utf8')) : {}
const measured = {}
const problems = []
const tools = { lean: 0, full: 0 }
const scratch = mkdtempSync(join(tmpdir(), 'neru-eval-'))
try {
  await new Promise(ready => server.listen(0, '127.0.0.1', ready))
  await new Promise(ready => ollama.listen(0, '127.0.0.1', ready))
  const { port } = server.address()
  const ollamaPort = ollama.address().port
  console.log('scenario                    requests  rounds  failed edits  bad calls  retries  switches  waited  local')
  for (const [profile, model] of Object.entries(PROFILES)) {
    for (const name of Object.keys(SCENARIOS)) {
      const key = `${profile}/${name}`
      scenario = name
      failed = false
      seen = []
      local = []
      schemas = []
      offered = 0
      // A fresh project and data folder each time, so no health, quota or session carries over.
      const project = join(scratch, `${profile}-${name}`, 'project')
      const data = join(scratch, `${profile}-${name}`, 'data')
      mkdirSync(project, { recursive: true })
      mkdirSync(data, { recursive: true })
      writeFileSync(join(project, 'a.txt'), 'alpha\n')
      writeFileSync(join(project, 'b.txt'), 'beta\n')
      for (const [file, content] of Object.entries(SCENARIOS[name].files ?? {})) {
        mkdirSync(dirname(join(project, file)), { recursive: true })
        writeFileSync(join(project, file), content)
      }
      writeFileSync(join(data, 'settings.json'), JSON.stringify({ providerId: 'local', apiFormat: 'openai-chat', baseUrl: `http://127.0.0.1:${port}/v1`, model }))
      const run = await neru(['-p', SCENARIOS[name].prompt ?? `EVAL-${name}: do the scripted task`, '--permission-mode', 'bypass', '--output-format', 'json'], project, { ...process.env, NERU_DATA_DIR: data, NO_COLOR: '1', OLLAMA_HOST: SCENARIOS[name].ollama ? `127.0.0.1:${ollamaPort}` : '127.0.0.1:9' })
      let result = {}
      try { result = JSON.parse(run.stdout.trim().split('\n').at(-1) ?? '{}') } catch { /* reported below */ }
      const task = result.task
      const log = `exit ${run.code}\nstdout:\n${run.stdout.slice(-2000)}\nstderr:\n${run.stderr.slice(-2000)}\nserver saw: ${seen.join(', ')}`
      if (run.code !== 0 || !String(result.result ?? '').includes(DONE) || !task) {
        problems.push(`${key}: the task did not finish\n${log}`)
        console.log(`${key.padEnd(28)}FAIL`)
        continue
      }
      measured[key] = { requests: task.requests }
      tools[profile] = Math.max(tools[profile], offered)
      // Requests that ran on this computer (Ollama) rather than on the cloud model.
      const onDevice = Object.entries(task.models ?? {}).filter(([model]) => model.startsWith('ollama/')).reduce((sum, [, count]) => sum + count, 0)
      const cells = [task.requests, task.rounds, task.failedEdits, task.parseFailures, task.retries, task.switches, `${(task.waitedMs / 1000).toFixed(1)}s`, onDevice]
      console.log(`${key.padEnd(28)}${String(cells[0]).padEnd(10)}${String(cells[1]).padEnd(8)}${String(cells[2]).padEnd(14)}${String(cells[3]).padEnd(11)}${String(cells[4]).padEnd(9)}${String(cells[5]).padEnd(10)}${String(cells[6]).padEnd(8)}${cells[7]}`)
      // The servers' counts are the ground truth for what Neru's own counter reports.
      if (seen.length + local.length !== task.requests) problems.push(`${key}: Neru counted ${task.requests} requests but the models received ${seen.length + local.length} (${[...seen, ...local.map(model => `ollama ${model}`)].join(', ')})`)
      if (local.length !== onDevice) problems.push(`${key}: Neru counted ${onDevice} local requests but Ollama received ${local.length}`)
      if (SCENARIOS[name].expectSchema && (!schemas.length || schemas.includes(false))) problems.push(`${key}: the repair reached Ollama without a JSON schema (format)`)
      if ((SCENARIOS[name].expectLocal ?? 0) > onDevice) problems.push(`${key}: expected ${SCENARIOS[name].expectLocal} request(s) on the local model, got ${onDevice}\n${log}`)
      const before = baseline[key]?.requests
      if (before === undefined) continue
      if (task.requests > before) problems.push(`${key}: ${task.requests} requests, up from ${before}\n${log}`)
      else if (task.requests < before) console.log(`  ↓ ${key} improved from ${before} to ${task.requests} requests (run with --update to save)`)
    }
  }
  if (tools.full <= tools.lean) problems.push(`The full profile offered ${tools.full} tools and the lean one ${tools.lean}: both ran lean, so the full prompt went untested`)
} catch (error) {
  problems.push(String(error?.stack ?? error))
} finally {
  server.close()
  ollama.close()
  rmSync(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 })
}

const total = Object.values(measured).reduce((sum, entry) => sum + entry.requests, 0)
console.log(`\n${Object.keys(measured).length} scenarios, ${total} requests in all (most tools offered: lean ${tools.lean}, full ${tools.full})`)
if (update && Object.keys(measured).length) {
  writeFileSync(baselinePath, `${JSON.stringify(measured, null, 2)}\n`)
  console.log(`Saved the baseline to ${baselinePath}`)
}
if (problems.length) {
  console.error(`\n${problems.length} problem(s):\n\n${problems.join('\n\n')}`)
  process.exit(1)
}
