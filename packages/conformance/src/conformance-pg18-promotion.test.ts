// PG18-E2E-009 development regression: a physical standby without synchronized logical-slot
// support is an epoch boundary. This fixture intentionally uses no failover-slot settings. It is
// not release qualification: it drives the direct native Axum surface and folds the durable stream
// itself, while keeping all PostgreSQL, engine, durable-stream, and artifact state private to one
// test-owned directory.

import { execFileSync } from 'node:child_process'
import { appendFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from 'node:net'

import { DurableStreamTestServer } from '@electric-circuits/ds-rust'
import pgpkg from 'pg'
import { afterEach, describe, expect, it } from 'vitest'

import { postgres18Tools, type Postgres18Tools } from '../../../scripts/postgres18.js'
import { buildEngine, spawnRawEngine, type RawEngine } from './harness.js'
import { foldStream } from './engine-native.js'

const MARKER_TABLE = '__el_sync'
const ITEM_TABLE = 'items'
const TIMEOUT_MS = 30_000

interface ShapeResponse {
  shapeId: string
  streamUrl: string
}

interface Epoch {
  state: 'ok' | 'broken'
  reason: string | null
  systemIdentifier: string | null
  timelineId: number | null
  slot: string | null
  boundAt: string | null
}

interface EngineReceipt {
  marker: number
  sync: number
  epoch: Epoch
}

interface PromotionFixture {
  root: string
  primaryData: string
  standbyData: string
  primaryUrl: string
  standbyUrl: string
  dbName: string
  slot: string
  publication: string
  pg: Postgres18Tools
  ds: DurableStreamTestServer
  dsUrl: string
  engine: RawEngine | undefined
  engineUrl: string | undefined
  gates: string[]
  ownedPids: Set<number>
  cleanup(): Promise<void>
}

let fixture: PromotionFixture | undefined

function run(command: string, args: string[]): void {
  execFileSync(command, args, { stdio: 'ignore' })
}

async function freePort(): Promise<number> {
  return await new Promise<number>((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      server.close((error) => {
        if (error || address === null || typeof address === 'string') {
          reject(error ?? new Error('could not allocate a TCP port'))
          return
        }
        resolve(address.port)
      })
    })
  })
}

async function sql(url: string, statement: string, params: unknown[] = []): Promise<unknown[]> {
  const client = new pgpkg.Client({ connectionString: url })
  await client.connect()
  try {
    return (await client.query(statement, params)).rows
  } finally {
    await client.end().catch(() => {})
  }
}

async function scalar<T>(url: string, statement: string, params: unknown[] = []): Promise<T> {
  const rows = await sql(url, statement, params)
  const row = rows[0] as Record<string, T> | undefined
  if (!row) throw new Error(`expected one row from: ${statement}`)
  return Object.values(row)[0]!
}

async function waitFor(
  fx: PromotionFixture,
  gate: string,
  condition: () => Promise<boolean>,
  detail: () => Promise<string> = async () => '',
): Promise<void> {
  const deadline = Date.now() + TIMEOUT_MS
  while (Date.now() < deadline) {
    if (await condition()) {
      fx.gates.push(gate)
      return
    }
    await new Promise<void>((resolve) => setTimeout(resolve, 25))
  }
  throw new Error(`PG18-E2E-009 gate '${gate}' timed out; gates=${fx.gates.join(',')}; ${await detail()}`)
}

function pgCtl(pg: Postgres18Tools, data: string, log: string, action: 'start' | 'stop' | 'promote', mode?: 'immediate'): void {
  const args = ['-D', data]
  if (action === 'start') args.push('-l', log, '-w', 'start')
  else if (action === 'stop') args.push('-m', mode ?? 'immediate', '-w', 'stop')
  else args.push('promote', '-w')
  run(pg.pgCtl, args)
}

function postmasterPid(data: string): number {
  const pid = Number(readFileSync(join(data, 'postmaster.pid'), 'utf8').split('\n', 1)[0])
  if (!Number.isSafeInteger(pid) || pid <= 0) throw new Error(`invalid postmaster pid in ${data}`)
  return pid
}

function processAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function engineEnv(fx: PromotionFixture, pgUrl: string): Record<string, string> {
  return {
    ELECTRIC_CIRCUITS_DS_URL: fx.dsUrl,
    ELECTRIC_CIRCUITS_BIND: '127.0.0.1:0',
    ELECTRIC_CIRCUITS_LOG: 'warn',
    ELECTRIC_CIRCUITS_PG_URL: pgUrl,
    ELECTRIC_CIRCUITS_PG_TABLES: `public.${ITEM_TABLE}`,
    ELECTRIC_CIRCUITS_PG_SLOT: fx.slot,
    ELECTRIC_CIRCUITS_PG_POLL_MS: '25',
    // The test owns this directory and the engine only uses it for its metrics sampler.
    ELECTRIC_STORAGE_DIR: join(fx.root, 'engine-storage'),
  }
}

async function startEngine(fx: PromotionFixture, pgUrl: string, gate: string): Promise<void> {
  const raw = spawnRawEngine(engineEnv(fx, pgUrl))
  if (raw.proc.pid) fx.ownedPids.add(raw.proc.pid)
  const url = await raw.waitForListening(TIMEOUT_MS).catch((error: Error) => {
    raw.signal('SIGKILL')
    throw error
  })
  fx.engine = raw
  fx.engineUrl = url
  await waitFor(fx, gate, async () => (await fetch(`${url}/ready`)).status === 200)
}

async function stopEngine(fx: PromotionFixture): Promise<void> {
  const raw = fx.engine
  fx.engine = undefined
  fx.engineUrl = undefined
  if (!raw) return
  raw.signal('SIGTERM')
  try {
    await raw.waitForExit(TIMEOUT_MS)
  } catch {
    raw.signal('SIGKILL')
    await raw.waitForExit(TIMEOUT_MS)
  }
}

async function epoch(fx: PromotionFixture): Promise<Epoch> {
  if (!fx.engineUrl) throw new Error('engine URL missing')
  const response = await fetch(`${fx.engineUrl}/replication/lsn`)
  if (!response.ok) throw new Error(`GET /replication/lsn -> ${response.status}`)
  return ((await response.json()) as { epoch: Epoch }).epoch
}

async function replicationSync(fx: PromotionFixture): Promise<number> {
  if (!fx.engineUrl) throw new Error('engine URL missing')
  const response = await fetch(`${fx.engineUrl}/replication/lsn`)
  if (!response.ok) throw new Error(`GET /replication/lsn -> ${response.status}`)
  return Number(((await response.json()) as { sync: number }).sync)
}

async function counter(fx: PromotionFixture, name: string): Promise<number> {
  if (!fx.engineUrl) throw new Error('engine URL missing')
  const response = await fetch(`${fx.engineUrl}/metrics`)
  if (!response.ok) throw new Error(`GET /metrics -> ${response.status}`)
  return Number(((await response.json()) as { counters: Record<string, number> }).counters[name] ?? 0)
}

async function createShape(fx: PromotionFixture): Promise<ShapeResponse> {
  if (!fx.engineUrl) throw new Error('engine URL missing')
  const response = await fetch(`${fx.engineUrl}/v1/shapes`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ table: ITEM_TABLE, where: { col: 'n', op: 'gte', value: 0 } }),
  })
  if (!response.ok) throw new Error(`POST /v1/shapes -> ${response.status} ${await response.text()}`)
  return (await response.json()) as ShapeResponse
}

async function commitMarker(url: string, rows: Array<[number, number]>): Promise<number> {
  const client = new pgpkg.Client({ connectionString: url })
  await client.connect()
  try {
    await client.query('BEGIN')
    for (const [id, n] of rows) {
      await client.query(`INSERT INTO ${ITEM_TABLE} (id, n) VALUES ($1, $2)`, [id, n])
    }
    // The marker is the final mutation in this source transaction. The engine's development
    // receipt advances only after the terminal pgoutput envelope has been decoded and appended.
    const marker = Number((await client.query(`UPDATE ${MARKER_TABLE} SET n = n + 1 WHERE id = 1 RETURNING n`)).rows[0]!.n)
    await client.query('COMMIT')
    return marker
  } catch (error) {
    await client.query('ROLLBACK').catch(() => {})
    throw error
  } finally {
    await client.end().catch(() => {})
  }
}

async function awaitSourceToTargetReceipt(
  fx: PromotionFixture,
  marker: number,
  shape: ShapeResponse,
  oracleUrl: string,
  gatePrefix: string,
): Promise<EngineReceipt> {
  await waitFor(fx, `${gatePrefix}_server_drained`, async () => (await replicationSync(fx)) >= marker)
  await waitFor(
    fx,
    `${gatePrefix}_target_folded`,
    async () => {
      const expected = await oracleItems(oracleUrl)
      return JSON.stringify(await foldedItems(shape.streamUrl)) === JSON.stringify(expected)
    },
    async () => `expected=${JSON.stringify(await oracleItems(oracleUrl))} actual=${JSON.stringify(await foldedItems(shape.streamUrl))}`,
  )
  return { marker, sync: await replicationSync(fx), epoch: await epoch(fx) }
}

async function oracleItems(url: string): Promise<Array<{ id: number; n: number }>> {
  return (await sql(url, `SELECT id, n FROM ${ITEM_TABLE} ORDER BY id`)).map((row) => {
    const item = row as { id: number; n: number }
    return { id: Number(item.id), n: Number(item.n) }
  })
}

async function foldedItems(streamUrl: string): Promise<Array<{ id: number; n: number }>> {
  return [...(await foldStream(streamUrl)).values()]
    .map((row) => ({ id: Number(row.id), n: Number(row.n) }))
    .sort((a, b) => a.id - b.id)
}

async function waitForStandbyReplay(fx: PromotionFixture, targetLsn: string): Promise<void> {
  await waitFor(
    fx,
    'standby_caught_up_to_pre_promotion_marker',
    async () =>
      Boolean(
        await scalar<boolean>(
          fx.standbyUrl,
          'SELECT pg_is_in_recovery() AND pg_last_wal_replay_lsn() IS NOT NULL AND pg_wal_lsn_diff(pg_last_wal_replay_lsn(), $1::pg_lsn) >= 0',
          [targetLsn],
        ),
      ),
  )
}

async function bootFixture(): Promise<PromotionFixture> {
  const pg = postgres18Tools()
  buildEngine()
  const root = mkdtempSync(join(tmpdir(), 'electric-circuits-pg18-promotion-'))
  const primaryData = join(root, 'primary')
  const standbyData = join(root, 'standby')
  const primaryPort = await freePort()
  const standbyPort = await freePort()
  const nonce = `${process.pid}_${Date.now().toString(36)}`.toLowerCase()
  const dbName = `pg18_promotion_${nonce}`
  const slot = `pg18_prom_slot_${nonce}`
  const publication = `${slot}_pub`
  let ds: DurableStreamTestServer | undefined
  let fx: PromotionFixture | undefined

  try {
    run(pg.initdb, ['-D', primaryData, '-U', 'postgres', '--auth=trust', '--no-sync'])
    appendFileSync(
      join(primaryData, 'postgresql.conf'),
      `\nlisten_addresses = '127.0.0.1'\nport = ${primaryPort}\nwal_level = logical\nmax_wal_senders = 10\nmax_replication_slots = 10\n`,
    )
    appendFileSync(join(primaryData, 'pg_hba.conf'), '\nhost replication all 127.0.0.1/32 trust\nhost all all 127.0.0.1/32 trust\n')
    pgCtl(pg, primaryData, join(root, 'primary.log'), 'start')
    const primaryPid = postmasterPid(primaryData)
    const primaryUrl = `postgres://postgres@127.0.0.1:${primaryPort}/${dbName}`

    await sql(`postgres://postgres@127.0.0.1:${primaryPort}/postgres`, 'CREATE ROLE pg18_promotion_repl WITH REPLICATION LOGIN')
    await sql(`postgres://postgres@127.0.0.1:${primaryPort}/postgres`, `CREATE DATABASE ${dbName}`)
    await sql(
      primaryUrl,
      `CREATE TABLE ${ITEM_TABLE} (id integer PRIMARY KEY, n integer NOT NULL); CREATE TABLE ${MARKER_TABLE} (id integer PRIMARY KEY, n bigint NOT NULL); INSERT INTO ${MARKER_TABLE} (id, n) VALUES (1, 0);`,
    )

    run(pg.pgBasebackup, [
      '-D',
      standbyData,
      '-h',
      '127.0.0.1',
      '-p',
      String(primaryPort),
      '-U',
      'pg18_promotion_repl',
      '-R',
      '-X',
      'stream',
      '--checkpoint=fast',
    ])
    appendFileSync(join(standbyData, 'postgresql.conf'), `\nlisten_addresses = '127.0.0.1'\nport = ${standbyPort}\nhot_standby = on\n`)
    pgCtl(pg, standbyData, join(root, 'standby.log'), 'start')
    const standbyPid = postmasterPid(standbyData)
    const standbyUrl = `postgres://postgres@127.0.0.1:${standbyPort}/${dbName}`
    ds = new DurableStreamTestServer({ port: 0, dataDir: join(root, 'durable-streams') })
    const dsUrl = await ds.start()

    fx = {
      root,
      primaryData,
      standbyData,
      primaryUrl,
      standbyUrl,
      dbName,
      slot,
      publication,
      pg,
      ds,
      dsUrl,
      engine: undefined,
      engineUrl: undefined,
      gates: [],
      ownedPids: new Set([primaryPid, standbyPid, ...(ds.pid ? [ds.pid] : [])]),
      cleanup: async () => {
        await stopEngine(fx!)
        try {
          pgCtl(pg, standbyData, join(root, 'standby.log'), 'stop')
        } catch {
          // A promoted or already stopped cluster is still owned by this fixture directory.
        }
        try {
          pgCtl(pg, primaryData, join(root, 'primary.log'), 'stop')
        } catch {
          // Primary was deliberately isolated before promotion.
        }
        await ds?.stop().catch(() => {})
        rmSync(root, { recursive: true, force: true })
        await waitFor(
          fx!,
          'cleanup_owned_children_exited',
          async () => ![...fx!.ownedPids].some(processAlive),
          async () => `livePids=${[...fx!.ownedPids].filter(processAlive).join(',')}`,
        )
        expect(existsSync(root), 'cleanup must remove only the fixture root it owns').toBe(false)
      },
    }
    await waitFor(fx, 'primary_pg18_ready', async () => (await scalar<number>(primaryUrl, "SELECT current_setting('server_version_num')::int")) >= 180000)
    await waitFor(fx, 'standby_in_recovery', async () => Boolean(await scalar<boolean>(standbyUrl, 'SELECT pg_is_in_recovery()')))
    return fx
  } catch (error) {
    await fx?.cleanup().catch(() => {})
    await ds?.stop().catch(() => {})
    try {
      pgCtl(pg, standbyData, join(root, 'standby.log'), 'stop')
    } catch {}
    try {
      pgCtl(pg, primaryData, join(root, 'primary.log'), 'stop')
    } catch {}
    rmSync(root, { recursive: true, force: true })
    throw error
  }
}

afterEach(async () => {
  await fixture?.cleanup()
  fixture = undefined
})

describe('PG18-E2E-009: promotion without synchronized logical failover slots', () => {
  it('terminally retires the old epoch and reseeds a fresh native feed from the promoted primary', async () => {
    fixture = await bootFixture()
    const fx = fixture

    await startEngine(fx, fx.primaryUrl, 'primary_engine_ready')
    expect(await scalar<string>(fx.primaryUrl, 'SHOW server_version_num')).toMatch(/^18/)
    expect(await scalar<string>(fx.primaryUrl, 'SELECT pubname FROM pg_publication WHERE pubname = $1', [fx.publication])).toBe(
      fx.publication,
    )
    expect(await scalar<number>(fx.primaryUrl, 'SELECT count(*)::int FROM pg_replication_slots WHERE slot_name = $1', [fx.slot])).toBe(1)

    const oldShape = await createShape(fx)
    fx.gates.push('old_generation_materialized')
    const baselineMarker = await commitMarker(fx.primaryUrl, [
      [1, 10],
      [2, 20],
    ])
    const baselineReceipt = await awaitSourceToTargetReceipt(fx, baselineMarker, oldShape, fx.primaryUrl, 'baseline')
    const primaryLsn = await scalar<string>(fx.primaryUrl, 'SELECT pg_current_wal_lsn()::text')
    await waitForStandbyReplay(fx, primaryLsn)

    // Named outage gates: the source is stopped first, then the former engine is terminated before
    // its replacement is allowed to point at the promoted endpoint.
    pgCtl(fx.pg, fx.primaryData, join(fx.root, 'primary.log'), 'stop', 'immediate')
    fx.gates.push('primary_isolated')
    await stopEngine(fx)
    fx.gates.push('old_engine_stopped')
    pgCtl(fx.pg, fx.standbyData, join(fx.root, 'standby.log'), 'promote')
    await waitFor(fx, 'standby_promoted', async () => !(await scalar<boolean>(fx.standbyUrl, 'SELECT pg_is_in_recovery()')))
    expect(await scalar<string>(fx.standbyUrl, 'SHOW server_version_num')).toMatch(/^18/)
    await waitFor(
      fx,
      'unsynchronized_slot_absent_on_promoted_standby',
      async () => (await scalar<number>(fx.standbyUrl, 'SELECT count(*)::int FROM pg_replication_slots WHERE slot_name = $1', [fx.slot])) === 0,
    )
    expect(await scalar<string>(fx.standbyUrl, 'SHOW sync_replication_slots')).toBe('off')
    expect(await scalar<number>(fx.standbyUrl, 'SELECT count(*)::int FROM pg_replication_slots WHERE failover')).toBe(0)

    // The physical standby did not synchronize the logical slot. Reusing the durable catalog must
    // therefore park/retire old records and bind a new slot, never resume the old stream.
    await startEngine(fx, fx.standbyUrl, 'redirected_engine_ready')
    const replacementEpoch = await epoch(fx)
    await waitFor(
      fx,
      'old_epoch_terminal',
      async () => {
        const shape = await fetch(`${fx.engineUrl}/v1/shapes/${oldShape.shapeId}`)
        const stream = await fetch(oldShape.streamUrl, { method: 'HEAD' })
        return shape.status === 404 && stream.status === 404
      },
      async () => `epoch=${JSON.stringify(await epoch(fx))}`,
    )
    expect(replacementEpoch.state).toBe('ok')
    expect(replacementEpoch.reason).toBeNull()
    await waitFor(
      fx,
      'replacement_epoch_rebound',
      async () => (await epoch(fx)).boundAt !== baselineReceipt.epoch.boundAt,
      async () => `baseline=${JSON.stringify(baselineReceipt.epoch)} replacement=${JSON.stringify(await epoch(fx))}`,
    )
    const reboundEpoch = await epoch(fx)
    expect(baselineReceipt.epoch.systemIdentifier).toBeTruthy()
    expect(baselineReceipt.epoch.slot).toBe(fx.slot)
    expect(reboundEpoch.systemIdentifier).toBe(baselineReceipt.epoch.systemIdentifier)
    expect(reboundEpoch.slot).toBe(baselineReceipt.epoch.slot)
    expect(reboundEpoch.boundAt).not.toBe(baselineReceipt.epoch.boundAt)
    await waitFor(
      fx,
      'epoch_reset_observed',
      async () => (await counter(fx, 'epoch_breaks_total')) >= 1 && (await counter(fx, 'epoch_resets_total')) >= 1,
      async () => `breaks=${await counter(fx, 'epoch_breaks_total')} resets=${await counter(fx, 'epoch_resets_total')}`,
    )

    const freshShape = await createShape(fx)
    fx.gates.push('fresh_generation_created')
    const postPromotionMarker = await commitMarker(fx.standbyUrl, [[3, 30]])
    const postPromotionReceipt = await awaitSourceToTargetReceipt(
      fx,
      postPromotionMarker,
      freshShape,
      fx.standbyUrl,
      'post_promotion',
    )
    expect(await foldedItems(freshShape.streamUrl)).toEqual(await oracleItems(fx.standbyUrl))
    expect(postPromotionReceipt.epoch.state).toBe('ok')
    fx.gates.push('fresh_generation_matches_promoted_sql')
  }, 180_000)
})
