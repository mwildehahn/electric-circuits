import { type ChildProcess, execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { createConnection } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { afterEach, describe, expect, it } from 'vitest'

import { cleanupOwnedAdapterResources } from './electric-adapter-cleanup.js'

const repo = dirname(dirname(dirname(dirname(fileURLToPath(import.meta.url)))))
const runs: AdapterRun[] = []

interface CleanupManifest {
  adapterPid?: number
  enginePid?: number
  dsPid?: number
  dsUrl?: string
  dsDataDir?: string
  engineBin?: string
  pgCtl?: string
  pgData?: string
}

interface AdapterRun {
  child: ChildProcess
  manifestPath: string
  manifest: CleanupManifest
  descendants: number[]
}

function isAlive(pid: number | undefined): boolean {
  if (!pid) return false
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function descendantsOf(pid: number): number[] {
  const rows = execFileSync('ps', ['-axo', 'pid=,ppid='], { encoding: 'utf8' })
    .trim()
    .split('\n')
    .flatMap((line) => {
      const [child, parent] = line.trim().split(/\s+/, 2).map(Number)
      return Number.isInteger(child) && Number.isInteger(parent) ? [[child!, parent!] as const] : []
    })
  const byParent = new Map<number, number[]>()
  for (const [child, parent] of rows) byParent.set(parent, [...(byParent.get(parent) ?? []), child])
  const found: number[] = []
  const pending = [pid]
  while (pending.length) {
    const parent = pending.pop()!
    for (const child of byParent.get(parent) ?? []) {
      found.push(child)
      pending.push(child)
    }
  }
  return found
}

async function waitForExit(child: ChildProcess, timeoutMs = 10_000): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`adapter ${child.pid} did not exit within ${timeoutMs}ms`)), timeoutMs)
    child.once('exit', () => {
      clearTimeout(timer)
      resolve()
    })
  })
}

async function waitForPidExit(pid: number, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (isAlive(pid)) {
    if (Date.now() >= deadline) throw new Error(`adapter ${pid} did not exit within ${timeoutMs}ms`)
    await new Promise((resolve) => setTimeout(resolve, 25))
  }
}

async function startAdapter(): Promise<AdapterRun> {
  const root = mkdtempSync(join(tmpdir(), 'electric-adapter-cleanup-test-'))
  const manifestPath = join(root, 'manifest.json')
  const child = spawn('pnpm', ['--filter', '@electric-circuits/bench', 'exec', 'tsx', 'src/electric-adapter.ts'], {
    cwd: repo,
    env: {
      ...process.env,
      ADAPTER_CLEANUP_FILE: manifestPath,
      ADAPTER_LONGPOLL_MS: '50',
      // `pnpm engine:test` builds this debug artifact; do not make the product test gate depend
      // on a separate release build.
      ELECTRIC_CIRCUITS_ADAPTER_ENGINE_BIN: join(repo, 'target', 'debug', 'electric-circuits-engine'),
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let output = ''
  const ready = new Promise<void>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`adapter did not become ready:\n${output}`)), 30_000)
    const read = (chunk: Buffer) => {
      output += chunk.toString()
      if (output.includes('ADAPTER_LISTENING ')) {
        clearTimeout(timeout)
        resolve()
      }
    }
    child.stdout?.on('data', read)
    child.stderr?.on('data', read)
    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      reject(new Error(`adapter exited before ready (code=${code}, signal=${signal}):\n${output}`))
    })
  })
  await ready
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as CleanupManifest
  const run = { child, manifestPath, manifest, descendants: descendantsOf(child.pid!) }
  runs.push(run)
  return run
}

async function stopTestRun(run: AdapterRun): Promise<void> {
  if (isAlive(run.manifest.adapterPid)) process.kill(run.manifest.adapterPid!, 'SIGTERM')
  try {
    await waitForPidExit(run.manifest.adapterPid!, 5_000)
  } catch {
    if (isAlive(run.manifest.adapterPid)) process.kill(run.manifest.adapterPid!, 'SIGKILL')
  }
  if (isAlive(run.child.pid)) run.child.kill('SIGTERM')
  await waitForExit(run.child, 5_000).catch(() => run.child.kill('SIGKILL'))
  for (const pid of [run.manifest.enginePid, ...run.descendants]) {
    if (isAlive(pid)) {
      try {
        process.kill(pid!, 'SIGKILL')
      } catch {
        // Exact descendant may have exited between the liveness check and signal.
      }
    }
  }
  if (run.manifest.pgCtl && run.manifest.pgData) {
    try {
      execFileSync(run.manifest.pgCtl, ['-D', run.manifest.pgData, '-m', 'immediate', '-w', 'stop'], { stdio: 'ignore' })
    } catch {
      // The adapter may already have stopped its own ephemeral Postgres.
    }
    rmSync(dirname(run.manifest.pgData), { recursive: true, force: true })
  }
  rmSync(dirname(run.manifestPath), { recursive: true, force: true })
}

afterEach(async () => {
  while (runs.length) await stopTestRun(runs.pop()!)
})

describe('electric adapter durable-stream ownership', () => {
  it('awaits graceful shutdown until its owned durable-stream process, listener, and data root are gone', async () => {
    const run = await startAdapter()

    // This is the public ownership handoff used by the runner's forced-exit fallback. The base
    // adapter records only its engine and Postgres, so this assertion is the intended red proof.
    expect(run.manifest.dsPid).toEqual(expect.any(Number))
    expect(run.manifest.dsUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
    expect(run.manifest.dsDataDir).toEqual(expect.any(String))
    expect(run.manifest.engineBin).toBe(join(repo, 'target', 'debug', 'electric-circuits-engine'))
    expect(existsSync(run.manifest.dsDataDir!)).toBe(true)

    process.kill(run.manifest.adapterPid!, 'SIGTERM')
    await waitForPidExit(run.manifest.adapterPid!)
    await waitForExit(run.child)

    expect(isAlive(run.manifest.dsPid)).toBe(false)
    expect(existsSync(run.manifest.dsDataDir!)).toBe(false)
    const url = new URL(run.manifest.dsUrl!)
    const listening = await new Promise<boolean>((resolve) => {
      const socket = createConnection({ host: url.hostname, port: Number(url.port) })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => resolve(false))
    })
    expect(listening).toBe(false)
  })

  it('removes only its recorded durable-stream resources after the adapter is forced down', async () => {
    const run = await startAdapter()
    expect(run.manifest.dsPid).toEqual(expect.any(Number))
    expect(run.manifest.dsUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
    expect(run.manifest.dsDataDir).toEqual(expect.any(String))
    expect(run.manifest.engineBin).toBe(join(repo, 'target', 'debug', 'electric-circuits-engine'))

    process.kill(run.manifest.adapterPid!, 'SIGKILL')
    await waitForPidExit(run.manifest.adapterPid!)
    await cleanupOwnedAdapterResources(run.manifest)

    expect(isAlive(run.manifest.dsPid)).toBe(false)
    expect(existsSync(run.manifest.dsDataDir!)).toBe(false)
    const url = new URL(run.manifest.dsUrl!)
    const listening = await new Promise<boolean>((resolve) => {
      const socket = createConnection({ host: url.hostname, port: Number(url.port) })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => resolve(false))
    })
    expect(listening).toBe(false)
  })
})
