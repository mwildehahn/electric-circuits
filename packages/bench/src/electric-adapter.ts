// Boots our stack (ephemeral Postgres? + durable-streams + engine with the Electric /v1/shape adapter)
// so Electric's oracle harness (or a curl smoke test) can drive shapes against our engine.
//
//   Standalone (seeds its own minimal standard schema, for curl testing):
//     pnpm --filter @electric-circuits/bench exec tsx src/electric-adapter.ts
//   Driven by Electric's Elixir harness (it provides the DB + schema):
//     ADAPTER_PG_URL=postgres://... ADAPTER_PG_TABLES=level_1,level_2,... \
//       tsx src/electric-adapter.ts     # prints ADAPTER_LISTENING <url>, stays up until killed

import { type ChildProcess, execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { DurableStreamTestServer } from '@electric-circuits/ds-rust'
import pgpkg from 'pg'

import { postgres18Tools } from '../../../scripts/postgres18.js'

const here = dirname(fileURLToPath(import.meta.url))
function repoRoot(): string {
  let d = here
  for (let i = 0; i < 8; i++) {
    if (existsSync(join(d, 'Cargo.toml'))) return d
    d = dirname(d)
  }
  throw new Error('repo root not found')
}
const SLOT = process.env.ADAPTER_PG_SLOT || 'electric_circuits_conformance'
const ADAPTER_ENGINE_BIN = 'ELECTRIC_CIRCUITS_ADAPTER_ENGINE_BIN'
let postgres: ReturnType<typeof postgres18Tools> | undefined

// Electric's full standard schema (level_1..4 + composite-PK *_tags side tables).
const STANDARD_DDL = [
  `CREATE TABLE IF NOT EXISTS level_1 (id TEXT PRIMARY KEY, active BOOLEAN NOT NULL DEFAULT true)`,
  `CREATE TABLE IF NOT EXISTS level_1_tags (level_1_id TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (level_1_id, tag))`,
  `CREATE TABLE IF NOT EXISTS level_2 (id TEXT PRIMARY KEY, level_1_id TEXT NOT NULL, active BOOLEAN NOT NULL DEFAULT true)`,
  `CREATE TABLE IF NOT EXISTS level_2_tags (level_2_id TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (level_2_id, tag))`,
  `CREATE TABLE IF NOT EXISTS level_3 (id TEXT PRIMARY KEY, level_2_id TEXT NOT NULL, active BOOLEAN NOT NULL DEFAULT true)`,
  `CREATE TABLE IF NOT EXISTS level_3_tags (level_3_id TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (level_3_id, tag))`,
  `CREATE TABLE IF NOT EXISTS level_4 (id TEXT PRIMARY KEY, level_3_id TEXT NOT NULL, value TEXT NOT NULL DEFAULT '')`,
]
const SEED = [
  `INSERT INTO level_1 VALUES ('l1-1', true), ('l1-2', false)`,
  `INSERT INTO level_1_tags VALUES ('l1-1','alpha'), ('l1-1','beta'), ('l1-2','gamma')`,
  `INSERT INTO level_2 VALUES ('l2-1','l1-1',true), ('l2-2','l1-2',true)`,
  `INSERT INTO level_2_tags VALUES ('l2-1','alpha'), ('l2-2','delta')`,
  `INSERT INTO level_3 VALUES ('l3-1','l2-1',true), ('l3-2','l2-2',false)`,
  `INSERT INTO level_3_tags VALUES ('l3-1','alpha'), ('l3-2','beta'), ('l3-1','gamma')`,
  `INSERT INTO level_4 VALUES ('l4-1','l3-1','alpha'), ('l4-2','l3-2','beta'), ('l4-3','l3-1','gamma')`,
]

let pgDir: string | undefined
let pgData: string | undefined
let engineProc: ChildProcess | undefined
let ds: DurableStreamTestServer | undefined
let dsDataDir: string | undefined
let selectedEngineBin: string | undefined
const cleanupFile = process.env.ADAPTER_CLEANUP_FILE
let shuttingDown: Promise<void> | undefined

function selectEngineBin(): string {
  const injected = process.env[ADAPTER_ENGINE_BIN]
  const binary = injected ?? join(repoRoot(), 'target', 'release', 'electric-circuits-engine')
  if (existsSync(binary)) return binary

  if (!injected) {
    // Keep the standalone adapter contract unchanged: its default remains the release engine.
    console.error('build first: cargo build --release -p electric-circuits-engine')
    process.exit(1)
  }
  throw new Error(`${ADAPTER_ENGINE_BIN} does not exist: ${binary}`)
}

function writeCleanupManifest(): void {
  if (!cleanupFile) return

  writeFileSync(
    cleanupFile,
    JSON.stringify({
      adapterPid: process.pid,
      enginePid: engineProc?.pid,
      dsPid: ds?.pid,
      dsUrl: ds?.url,
      // This is a fresh `el-econf-ds-*` root created by this invocation, never a caller path.
      // The oracle runner's forced-exit fallback validates that ownership marker before removal.
      dsDataDir,
      engineBin: selectedEngineBin,
      pgCtl: postgres?.pgCtl,
      pgData,
    }),
  )
}

function bootEphemeralPg(): string {
  postgres = postgres18Tools()
  pgDir = mkdtempSync(join(tmpdir(), 'el-econf-pg-'))
  pgData = join(pgDir, 'data')
  execFileSync(postgres.initdb, ['-D', pgData, '-U', 'postgres', '--auth=trust', '--no-sync'], { stdio: 'ignore' })
  let port = 0
  for (let attempt = 0; attempt < 8; attempt++) {
    port = 55000 + Math.floor(Math.random() * 4000)
    execFileSync('bash', ['-c', `printf '\\nwal_level=logical\\nmax_replication_slots=10\\nmax_wal_senders=10\\nmax_connections=1200\\nlisten_addresses=\\047127.0.0.1\\047\\nunix_socket_directories=\\047/tmp\\047\\nport=${port}\\nfsync=off\\n' >> ${pgData}/postgresql.conf`])
    try {
      execFileSync(postgres.pgCtl, ['-D', pgData, '-l', join(pgDir, 'log'), '-w', 'start'], { stdio: 'ignore' })
      return `postgres://postgres@127.0.0.1:${port}/postgres`
    } catch {
      /* retry */
    }
  }
  throw new Error('failed to start ephemeral postgres')
}

async function main() {
  // Select and validate before starting any fixture-owned process. Tests inject the debug binary
  // built by `pnpm engine:test`; standalone use still defaults to the release binary above.
  selectedEngineBin = selectEngineBin()

  let pgUrl = process.env.ADAPTER_PG_URL || ''
  let tables =
    process.env.ADAPTER_PG_TABLES ||
    'level_1,level_1_tags,level_2,level_2_tags,level_3,level_3_tags,level_4'
  // Two-phase mode: boot an ephemeral PG, announce its URL, then wait for an external driver (the Elixir
  // oracle harness) to create the schema before introspecting — lets the test apply StandardSchema's
  // exact DDL+seed so the property generators line up.
  const waitTable = process.env.ADAPTER_WAIT_TABLE
  if (!pgUrl && waitTable) {
    pgUrl = bootEphemeralPg()
    writeCleanupManifest()
    console.log(`ADAPTER_PG ${pgUrl}`)
    const client = new pgpkg.Client({ connectionString: pgUrl })
    await client.connect()
    const deadline = Date.now() + 60000
    while (Date.now() < deadline) {
      const r = await client.query(`SELECT to_regclass($1) AS t`, [`public.${waitTable}`])
      if (r.rows[0]?.t) break
      await new Promise((res) => setTimeout(res, 200))
    }
    await client.end()
    console.error(`schema present (${waitTable}) → ${pgUrl}`)
  } else if (!pgUrl) {
    pgUrl = bootEphemeralPg()
    writeCleanupManifest()
    const client = new pgpkg.Client({ connectionString: pgUrl })
    await client.connect()
    for (const ddl of STANDARD_DDL) await client.query(ddl)
    for (const s of SEED) await client.query(s)
    await client.end()
    console.error(`seeded standard schema → ${pgUrl}`)
  }

  // Short long-poll timeout: Electric's oracle harness polls each shape live and only detects "no more
  // changes" when an up-to-date response returns. With the 30s default, unchanged shapes stall a batch;
  // a short timeout returns up-to-date quickly (changed shapes still wake immediately on append).
  const longPollMs = Number(process.env.ADAPTER_LONGPOLL_MS || 1000)
  dsDataDir = mkdtempSync(join(tmpdir(), 'el-econf-ds-'))
  ds = new DurableStreamTestServer({ port: 0, dataDir: dsDataDir, longPollTimeout: longPollMs })
  const dsUrl = await ds.start()
  writeCleanupManifest()

  // The adapter's *overall* live deadline (how long a live=true request re-polls before its
  // up-to-date control response) is
  // decoupled from the ds long-poll timeout above (which just paces the engine's re-poll loop).
  // Default = ADAPTER_LONGPOLL_MS so the oracle harness keeps its fast up-to-date detection; the
  // benchmark runner sets ADAPTER_LIVE_TIMEOUT_MS=20000 for Electric-like ~20s live behavior.
  const liveTimeoutMs = Number(process.env.ADAPTER_LIVE_TIMEOUT_MS || longPollMs)

  engineProc = spawn(selectedEngineBin, [], {
    env: {
      ...process.env,
      ELECTRIC_LIVE_TIMEOUT_MS: String(liveTimeoutMs),
      ELECTRIC_CIRCUITS_DS_URL: dsUrl,
      ELECTRIC_CIRCUITS_BIND: '127.0.0.1:0',
      ELECTRIC_CIRCUITS_LOG: process.env.ADAPTER_LOG || 'warn',
      ELECTRIC_CIRCUITS_PG_URL: pgUrl,
      ELECTRIC_CIRCUITS_PG_TABLES: tables,
      ELECTRIC_CIRCUITS_PG_SLOT: SLOT,
      ELECTRIC_CIRCUITS_PG_POLL_MS: '25',
    },
    stdio: ['ignore', 'pipe', 'inherit'],
  })
  writeCleanupManifest()
  const url = await new Promise<string>((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('engine did not start')), 30000)
    let buf = ''
    engineProc!.stdout!.on('data', (d: Buffer) => {
      buf += d.toString()
      const m = buf.match(/ENGINE_LISTENING (\S+)/)
      if (m) {
        clearTimeout(t)
        resolve(m[1]!)
      }
    })
    engineProc!.on('exit', (c) => reject(new Error(`engine exited ${c}`)))
  })
  // Discovery lines on stdout (the Elixir setup reads these); diagnostics go to stderr.
  console.log(`ADAPTER_PG ${pgUrl}`)
  console.log(`ADAPTER_LISTENING ${url}`)
}

async function stopEngine(): Promise<void> {
  const proc = engineProc
  if (!proc || proc.exitCode !== null || proc.signalCode !== null) return
  const exited = new Promise<boolean>((resolve) => {
    const timer = setTimeout(() => resolve(false), 3_000)
    proc.once('exit', () => {
      clearTimeout(timer)
      resolve(true)
    })
  })
  proc.kill('SIGTERM')
  if (!(await exited) && proc.exitCode === null && proc.signalCode === null) {
    const killed = new Promise<void>((resolve) => proc.once('exit', () => resolve()))
    proc.kill('SIGKILL')
    await killed
  }
}

async function shutdown(): Promise<void> {
  if (shuttingDown) return shuttingDown
  shuttingDown = (async () => {
    await stopEngine()
    try {
      await ds?.stop()
    } finally {
      ds = undefined
      if (dsDataDir) {
        rmSync(dsDataDir, { recursive: true, force: true })
        dsDataDir = undefined
      }
    }
    if (pgData && pgDir) {
      // pnpm/tsx can hard-kill this process moments after a SIGTERM reaches the process group — before
      // a synchronous pg_ctl stop + rmSync would finish (which leaked the ephemeral PG + its tmpdir).
      // Hand the teardown to a detached helper: it survives us AND a group-wide SIGKILL backstop
      // (detached = its own process group).
      if (!postgres) throw new Error('ephemeral Postgres tools were not initialized')
      spawn('bash', ['-c', `'${postgres.pgCtl}' -D '${pgData}' -m immediate -w stop >/dev/null 2>&1; rm -rf '${pgDir}'`], {
        detached: true,
        stdio: 'ignore',
      }).unref()
    }
  })()
  return shuttingDown
}

function shutdownAndExit() {
  void shutdown()
    .catch((e) => console.error('adapter shutdown failed', e))
    .finally(() => process.exit(0))
}
process.on('SIGINT', shutdownAndExit)
process.on('SIGTERM', shutdownAndExit)

main().catch((e) => {
  console.error(e)
  shutdownAndExit()
})
