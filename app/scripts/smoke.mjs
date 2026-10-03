#!/usr/bin/env node
// End-to-end smoke test of the neru CLI against a scripted model: a first run on a fresh project
// that edits a file and runs a command, then a resumed session that still remembers it. It catches
// first-run failures (settings, data folder, sandbox, shell) that unit tests do not reach.
//
//   node app/scripts/smoke.mjs <path to neru-cli>
//
// The model is a local OpenAI-compatible server in this script, so no key or network is needed.
// Neru's data folder is a temporary one (NERU_DATA_DIR), so the run never touches real settings.

import { spawn } from 'node:child_process'
import { createServer } from 'node:http'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const cli = resolve(process.argv[2] ?? '')
if (!process.argv[2] || !existsSync(cli)) {
  console.error('Usage: node app/scripts/smoke.mjs <path to neru-cli>')
  process.exit(2)
}

const MARKER = 'smoke-command-ok'
const requests = []

/** What the scripted model does next, from the conversation it is sent. */
function nextStep(messages) {
  const last = messages.at(-1) ?? {}
  const text = message => typeof message.content === 'string' ? message.content : JSON.stringify(message.content ?? '')
  const users = messages.filter(message => message.role === 'user').map(text)
  if (last.role === 'user' && text(last).includes('SMOKE-RESUME')) {
    // The resumed session must still hold the first request and its tool results.
    const remembers = users.some(content => content.includes('SMOKE-EDIT')) && messages.some(message => message.role === 'tool' && text(message).includes(MARKER))
    return { text: remembers ? 'SMOKE-RESUMED' : 'SMOKE-LOST' }
  }
  if (last.role === 'user' && text(last).includes('SMOKE-EDIT')) {
    return { tool: 'propose_edit', args: { path: 'hello.txt', old_string: 'Hello', new_string: 'Hello, smoke' } }
  }
  if (last.role === 'tool' && text(last).startsWith('Applied')) {
    return { tool: 'run_shell_command', args: { command: `echo ${MARKER}` } }
  }
  if (last.role === 'tool' && text(last).includes(MARKER)) return { text: 'SMOKE-DONE' }
  if (last.role === 'tool') return { text: `SMOKE-UNEXPECTED: ${text(last).slice(0, 300)}` }
  // Anything else (a title, a summary) gets a plain answer.
  return { text: 'Smoke test' }
}

function chunk(delta, finish = null) {
  return `data: ${JSON.stringify({ id: 'smoke', object: 'chat.completion.chunk', created: 0, model: 'smoke-model', choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`
}

const server = createServer((request, response) => {
  let body = ''
  request.on('data', part => { body += part })
  request.on('end', () => {
    if (request.method === 'GET' && request.url.startsWith('/v1/models')) {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ object: 'list', data: [{ id: 'smoke-model', object: 'model' }] }))
      return
    }
    if (request.method !== 'POST' || !request.url.startsWith('/v1/chat/completions')) {
      response.writeHead(404).end()
      return
    }
    const payload = JSON.parse(body || '{}')
    const step = nextStep(payload.messages ?? [])
    requests.push({ last: payload.messages?.at(-1)?.role, step: step.tool ?? step.text })
    if (!payload.stream) {
      const message = step.tool
        ? { role: 'assistant', content: null, tool_calls: [{ id: `call_${requests.length}`, type: 'function', function: { name: step.tool, arguments: JSON.stringify(step.args) } }] }
        : { role: 'assistant', content: step.text }
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ id: 'smoke', object: 'chat.completion', model: 'smoke-model', choices: [{ index: 0, message, finish_reason: step.tool ? 'tool_calls' : 'stop' }], usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } }))
      return
    }
    response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' })
    response.write(chunk({ role: 'assistant', content: '' }))
    if (step.tool) {
      response.write(chunk({ tool_calls: [{ index: 0, id: `call_${requests.length}`, type: 'function', function: { name: step.tool, arguments: JSON.stringify(step.args) } }] }))
      response.write(chunk({}, 'tool_calls'))
    } else {
      response.write(chunk({ content: step.text }))
      response.write(chunk({}, 'stop'))
    }
    response.end('data: [DONE]\n\n')
  })
})

/** Runs the CLI in the project and resolves with its exit code and output. */
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

const failures = []
function check(ok, what, detail = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`)
  if (!ok) failures.push(detail ? `${what}\n${detail}` : what)
}

const scratch = mkdtempSync(join(tmpdir(), 'neru-smoke-'))
const project = join(scratch, 'project')
const data = join(scratch, 'data')
try {
  await new Promise(ready => server.listen(0, '127.0.0.1', ready))
  const { port } = server.address()
  // A brand-new project and a first run: no settings but the model, nothing saved before.
  mkdirSync(project)
  mkdirSync(data)
  writeFileSync(join(project, 'hello.txt'), 'Hello\n')
  writeFileSync(join(data, 'settings.json'), JSON.stringify({ providerId: 'local', apiFormat: 'openai-chat', baseUrl: `http://127.0.0.1:${port}/v1`, model: 'smoke-model' }))
  const env = { ...process.env, NERU_DATA_DIR: data, NO_COLOR: '1' }

  const first = await neru(['-p', 'SMOKE-EDIT: change hello.txt, then run a command', '--permission-mode', 'bypass', '--output-format', 'json'], project, env)
  let result = {}
  try { result = JSON.parse(first.stdout.trim().split('\n').at(-1) ?? '{}') } catch { /* checked below */ }
  const log = `exit ${first.code}\nstdout:\n${first.stdout}\nstderr:\n${first.stderr}\nmodel saw: ${JSON.stringify(requests)}`
  check(first.code === 0, 'first run exits cleanly', log)
  check(result.subtype === 'success' && String(result.result).includes('SMOKE-DONE'), 'the agent finished its task', log)
  check(readFileSync(join(project, 'hello.txt'), 'utf8').includes('Hello, smoke'), 'the edit was written to the file', log)
  check(requests.some(request => request.step === 'SMOKE-DONE'), 'the command ran and its output reached the model', log)
  check(typeof result.session_id === 'string' && result.session_id.length > 0, 'the run reports its session', log)

  const resumed = await neru(['-c', '-p', 'SMOKE-RESUME: what did you do?', '--output-format', 'json'], project, env)
  let again = {}
  try { again = JSON.parse(resumed.stdout.trim().split('\n').at(-1) ?? '{}') } catch { /* checked below */ }
  const resumeLog = `exit ${resumed.code}\nstdout:\n${resumed.stdout}\nstderr:\n${resumed.stderr}`
  check(resumed.code === 0, 'resumed run exits cleanly', resumeLog)
  check(again.session_id === result.session_id, '-c continues the same session', resumeLog)
  check(String(again.result).includes('SMOKE-RESUMED'), 'the resumed session remembers the first run', resumeLog)
} catch (error) {
  failures.push(String(error?.stack ?? error))
} finally {
  server.close()
  rmSync(scratch, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 })
}

if (failures.length) {
  console.error(`\n${failures.length} smoke check(s) failed:\n\n${failures.join('\n\n')}`)
  process.exit(1)
}
console.log('\nSmoke test passed.')
