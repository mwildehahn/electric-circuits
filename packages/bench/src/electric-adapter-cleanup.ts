// Invocation-scoped fallback cleanup for `electric-conformance/run.sh`. The adapter normally
// performs the graceful path itself; this handles only resources it recorded before a forced
// adapter exit. It deliberately accepts neither process-name searches nor arbitrary directories.

import { readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export interface AdapterCleanupManifest {
  enginePid?: number
  dsPid?: number
  dsDataDir?: string
}

function validPid(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 1
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

async function waitForExit(pid: number, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs
  while (alive(pid)) {
    if (Date.now() >= deadline) return false
    await new Promise((resolve) => setTimeout(resolve, 25))
  }
  return true
}

async function terminateRecordedProcess(pid: unknown, label: string): Promise<void> {
  if (!validPid(pid) || !alive(pid)) return
  process.kill(pid, 'SIGTERM')
  if (await waitForExit(pid, 3_000)) return
  process.kill(pid, 'SIGKILL')
  if (!(await waitForExit(pid, 1_000))) throw new Error(`${label} PID ${pid} survived fallback cleanup`)
}

function removeRecordedDsDataDir(value: unknown): void {
  if (typeof value !== 'string') return
  const dir = resolve(value)
  const expectedParent = resolve(tmpdir())
  if (dirname(dir) !== expectedParent || !basename(dir).startsWith('el-econf-ds-')) {
    throw new Error(`refusing to remove non-owned durable-stream directory: ${value}`)
  }
  rmSync(dir, { recursive: true, force: true })
}

export function readAdapterCleanupManifest(path: string): AdapterCleanupManifest {
  return JSON.parse(readFileSync(path, 'utf8')) as AdapterCleanupManifest
}

/** Stop only the exact engine/DS PIDs and DS root recorded by one adapter invocation. */
export async function cleanupOwnedAdapterResources(manifest: AdapterCleanupManifest): Promise<void> {
  await terminateRecordedProcess(manifest.enginePid, 'engine')
  await terminateRecordedProcess(manifest.dsPid, 'durable-streams')
  removeRecordedDsDataDir(manifest.dsDataDir)
}

async function main(): Promise<void> {
  const manifestPath = process.argv[2]
  if (!manifestPath) throw new Error('usage: electric-adapter-cleanup.ts <manifest-path>')
  await cleanupOwnedAdapterResources(readAdapterCleanupManifest(manifestPath))
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  void main().catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
}
