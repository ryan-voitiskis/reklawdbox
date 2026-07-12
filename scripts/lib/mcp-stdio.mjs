import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'

const STDERR_LIMIT = 64 * 1024
const PROTOCOL_VIOLATION_LIMIT = 100

export function decodeProtocolLine(line) {
  if (!line.trim()) return { kind: 'blank' }

  try {
    const message = JSON.parse(line)
    if (!message || typeof message !== 'object' || Array.isArray(message)) {
      return { kind: 'violation', line }
    }
    return { kind: 'message', message }
  } catch {
    return { kind: 'violation', line }
  }
}

export function routeProtocolLine(line, pending, protocolViolations) {
  const decoded = decodeProtocolLine(line)
  if (decoded.kind === 'violation') {
    if (protocolViolations.length < PROTOCOL_VIOLATION_LIMIT) {
      protocolViolations.push(decoded.line)
    }
    return decoded
  }
  if (decoded.kind !== 'message') return decoded

  const { message } = decoded
  if (message.id != null && pending.has(message.id)) {
    const { resolve, timer } = pending.get(message.id)
    pending.delete(message.id)
    clearTimeout(timer)
    resolve(message.error ? { transportError: message.error } : message.result)
  }
  return decoded
}

export class McpStdioClient {
  constructor({
    bin,
    args = ['mcp'],
    cwd = process.cwd(),
    env = process.env,
    timeoutMs = 20_000,
  }) {
    if (!bin) throw new Error('MCP binary path is required')
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
      throw new Error('MCP timeout must be a positive number')
    }

    this.timeoutMs = timeoutMs
    this.pending = new Map()
    this.protocolViolations = []
    this.nextId = 1
    this.stderr = ''
    this.exited = false
    this.exitResult = null

    this.child = spawn(bin, args, {
      cwd,
      env: { ...env },
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    this.exitPromise = new Promise((resolve) => {
      this.resolveExit = resolve
    })

    this.readline = createInterface({ input: this.child.stdout })
    this.readline.on('line', (line) => {
      routeProtocolLine(line, this.pending, this.protocolViolations)
    })

    this.child.stderr.on('data', (chunk) => {
      this.stderr = `${this.stderr}${chunk.toString()}`.slice(-STDERR_LIMIT)
    })

    this.child.on('error', (error) => {
      this.finishExit({ spawnError: error.message })
    })
    this.child.on('exit', (code, signal) => {
      this.finishExit({ code, signal })
    })
  }

  finishExit(result) {
    if (this.exited) return
    this.exited = true
    this.exitResult = result
    for (const { resolve, timer } of this.pending.values()) {
      clearTimeout(timer)
      resolve({ childExit: result.spawnError ?? result.code ?? result.signal })
    }
    this.pending.clear()
    this.resolveExit(result)
  }

  request(method, params) {
    const id = this.nextId
    this.nextId += 1
    const message = { jsonrpc: '2.0', id, method }
    if (params !== undefined) message.params = params

    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        if (!this.pending.delete(id)) return
        resolve({ timeout: method, stderr: this.stderrTail() })
      }, this.timeoutMs)
      this.pending.set(id, { resolve, timer })
      this.child.stdin.write(`${JSON.stringify(message)}\n`, (error) => {
        if (!error || !this.pending.delete(id)) return
        clearTimeout(timer)
        resolve({ childExit: error.message })
      })
    })
  }

  notify(method, params) {
    const message = { jsonrpc: '2.0', method }
    if (params !== undefined) message.params = params
    this.child.stdin.write(`${JSON.stringify(message)}\n`)
  }

  listTools() {
    return this.request('tools/list')
  }

  callTool(name, args = {}) {
    return this.request('tools/call', { name, arguments: args })
  }

  stderrTail(limit = 2_000) {
    return this.stderr.slice(-limit)
  }

  async close() {
    this.readline.close()
    if (this.exited) return this.exitResult

    this.child.stdin.end()
    const graceful = await Promise.race([
      this.exitPromise.then(() => true),
      delay(250).then(() => false),
    ])
    if (graceful) return this.exitResult

    this.child.kill('SIGTERM')
    const terminated = await Promise.race([
      this.exitPromise.then(() => true),
      delay(500).then(() => false),
    ])
    if (!terminated && !this.exited) this.child.kill('SIGKILL')
    await Promise.race([this.exitPromise, delay(500)])
    return this.exitResult
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
