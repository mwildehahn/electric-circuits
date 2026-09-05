import type { Schema } from '@electric-circuits/protocol'
import pgpkg from 'pg'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { createShape, foldStream, lockTable, pgQuery, waitFor, waitForLockWaiter } from './engine-native.js'
import { bootHarness, type Harness } from './harness.js'

const GATEWAY_SECRET = 'runtime-fence-gateway-secret'
const CONTROL_SECRET = 'runtime-fence-controller-secret'
const USER = '018f5f4d-70c2-7d70-a4d5-5f7355078f81'
const OTHER_USER = '018f5f4d-70c2-7d70-a4d5-5f7355078f82'
const GENERATION = 'a'.repeat(64)
const NEXT_GENERATION = 'b'.repeat(64)
const MARKER = '018f5f4d-70c2-7d70-a4d5-5f7355078f85'
const DEPLOYMENT_MARKER = '018f5f4d-70c2-7d70-a4d5-5f7355078f86'
const DEFERRED_MARKER = '018f5f4d-70c2-7d70-a4d5-5f7355078f87'
const RESTART_MARKER = '018f5f4d-70c2-7d70-a4d5-5f7355078f88'

const schema: Schema = {
  tables: {
    items: {
      columns: { id: { type: 'int' }, group_id: { type: 'int' }, value: { type: 'text' } },
      primaryKey: 'id',
    },
    memberships: {
      columns: { id: { type: 'int' }, group_id: { type: 'int' } },
      primaryKey: 'id',
    },
  },
}

interface ReceiptResponse {
  sourceCommitId: string
  drained: boolean
  receipt: null | { sourceCommitId: string; userId: string; generation: string; commitLsn: string }
}

function request(h: Harness, path: string, secret?: string, method = 'GET'): Promise<Response> {
  return fetch(`${h.engineUrl}${path}`, {
    method,
    headers: secret ? { authorization: `Bearer ${secret}` } : {},
  })
}

function runtimePath(marker: string, user = USER, generation = GENERATION): string {
  return `/_runtime/drained-through/${marker}?user_id=${user}&generation=${generation}`
}

async function receipt(h: Harness, marker: string, user = USER, generation = GENERATION): Promise<ReceiptResponse> {
  const response = await request(h, runtimePath(marker, user, generation), GATEWAY_SECRET)
  expect(response.status).toBe(200)
  expect(response.headers.get('cache-control')).toContain('no-store')
  return await response.json() as ReceiptResponse
}

async function awaitReceipt(h: Harness, marker: string, generation = GENERATION): Promise<void> {
  await waitFor(async () => (await receipt(h, marker, USER, generation)).drained, `runtime receipt ${marker}`)
  const result = await receipt(h, marker, USER, generation)
  expect(result).toEqual({
    sourceCommitId: marker,
    drained: true,
    receipt: { sourceCommitId: marker, userId: USER, generation, commitLsn: expect.stringMatching(/^[0-9A-F]+\/[0-9A-F]+$/) },
  })
}

async function writeMarker(h: Harness, marker: string, generation = GENERATION): Promise<void> {
  await pgQuery(h, `
    INSERT INTO native_sync_authority_fence (user_id, generation, source_commit_id)
    VALUES ($1::uuid, $2, $3::uuid)
    ON CONFLICT (user_id) DO UPDATE SET generation = excluded.generation, source_commit_id = excluded.source_commit_id
  `, [USER, generation, marker])
}

describe('conformance: runtime authority receipts are separate from deployment handoff', () => {
  let h: Harness

  beforeAll(async () => {
    h = await bootHarness(schema, {
      durableStreamsDurability: 'wal',
      engineEnv: {
        ELECTRIC_SECRET: GATEWAY_SECRET,
        ELECTRIC_CIRCUITS_CONTROL_SECRET: CONTROL_SECRET,
        ELECTRIC_CIRCUITS_PG_TABLES: 'public.*,public.native_sync_authority_fence',
        ELECTRIC_CIRCUITS_SHUTDOWN_DRAIN_SECS: '0',
      },
      beforeEngine: async ({ pgUrl }) => {
        const postgres = new pgpkg.Client({ connectionString: pgUrl })
        await postgres.connect()
        try {
          await postgres.query(`CREATE TABLE native_sync_authority_fence (
            user_id uuid PRIMARY KEY, generation varchar(64) NOT NULL, source_commit_id uuid NOT NULL
          ); ALTER TABLE native_sync_authority_fence REPLICA IDENTITY FULL`)
        } finally {
          await postgres.end()
        }
      },
    })
  }, 60_000)

  afterAll(async () => { await h?.shutdown() })

  it('requires gateway authority and keeps the marker out of ordinary table admission', async () => {
    for (const secret of [undefined, CONTROL_SECRET, 'wrong-secret']) {
      expect((await request(h, runtimePath(MARKER), secret)).status).toBe(401)
    }
    expect(await receipt(h, MARKER)).toEqual({ sourceCommitId: MARKER, drained: false, receipt: null })
    expect((await request(h, `/_admin/drained-through/${MARKER}`, GATEWAY_SECRET)).status).toBe(401)
    const tables = await (await fetch(`${h.engineUrl}/tables`)).text()
    expect(tables).not.toContain('native_sync_authority_fence')
    const shape = await fetch(`${h.engineUrl}/v1/shapes`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ table: 'public.native_sync_authority_fence' }),
    })
    expect(shape.ok).toBe(false)
  })

  it('waits for commit and binds the receipt to the exact user, generation and marker', async () => {
    const shape = await createShape(h, { table: 'items' })
    const postgres = new pgpkg.Client({ connectionString: h.pgUrl })
    await postgres.connect()
    try {
      await postgres.query('BEGIN')
      await postgres.query("INSERT INTO items (id, group_id, value) VALUES (1, 7, 'committed')")
      await postgres.query('INSERT INTO native_sync_authority_fence VALUES ($1::uuid, $2, $3::uuid)', [USER, GENERATION, MARKER])
      expect((await receipt(h, MARKER)).drained).toBe(false)
      await postgres.query('COMMIT')
    } finally {
      await postgres.query('ROLLBACK').catch(() => {})
      await postgres.end()
    }
    await awaitReceipt(h, MARKER)
    expect(await foldStream(shape.streamUrl)).toEqual(new Map([['1', { id: 1, group_id: 7, value: 'committed' }]]))
    expect((await receipt(h, MARKER, OTHER_USER)).drained).toBe(false)
    expect((await receipt(h, MARKER, USER, NEXT_GENERATION)).drained).toBe(false)
    const deployment = await request(h, `/_admin/drained-through/${MARKER}`, CONTROL_SECRET)
    expect(deployment.status).toBe(200)
    expect((await deployment.json() as { drained: boolean }).drained).toBe(false)
  })

  it('cannot turn a deployment receipt into runtime authority, even for an identical UUID', async () => {
    for (const marker of [MARKER, DEPLOYMENT_MARKER]) {
      await pgQuery(h, 'INSERT INTO circuits_source_fence VALUES ($1::uuid)', [marker])
      await waitFor(async () => {
        const response = await request(h, `/_admin/drained-through/${marker}`, CONTROL_SECRET)
        expect(response.status).toBe(200)
        return (await response.json() as { drained: boolean }).drained
      }, `deployment receipt ${marker}`)
    }
    expect((await receipt(h, DEPLOYMENT_MARKER)).drained).toBe(false)
    await awaitReceipt(h, MARKER)
    await writeMarker(h, DEPLOYMENT_MARKER, NEXT_GENERATION)
    await awaitReceipt(h, DEPLOYMENT_MARKER, NEXT_GENERATION)
    expect((await receipt(h, DEPLOYMENT_MARKER)).drained).toBe(false)
  })

  it('holds the runtime receipt while a real deferred membership query-back is blocked', async () => {
    await pgQuery(h, 'INSERT INTO memberships VALUES (1, 7)')
    const shape = await createShape(h, {
      table: 'items',
      where: { col: 'group_id', in: { table: 'memberships', project: 'group_id' } },
    })
    expect(await foldStream(shape.streamUrl)).toEqual(new Map([['1', { id: 1, group_id: 7, value: 'committed' }]]))
    const held = await lockTable(h, 'items')
    try {
      await pgQuery(h, 'DELETE FROM memberships WHERE id = 1')
      await waitForLockWaiter(h, 'items')
      await writeMarker(h, DEFERRED_MARKER)
      expect((await receipt(h, DEFERRED_MARKER)).drained).toBe(false)
    } finally {
      await held.release()
    }
    await awaitReceipt(h, DEFERRED_MARKER)
    expect(await foldStream(shape.streamUrl)).toEqual(new Map())
  })

  it('does not restore runtime receipts from the catalog and recovers with a fresh marker after restart', async () => {
    await writeMarker(h, RESTART_MARKER, NEXT_GENERATION)
    await awaitReceipt(h, RESTART_MARKER, NEXT_GENERATION)
    h.signalEngine('SIGTERM')
    expect(await h.waitForEngineExit()).toEqual({ code: 0, signal: null })
    await h.startEngine()
    expect((await receipt(h, RESTART_MARKER, USER, NEXT_GENERATION)).drained).toBe(false)
    const fresh = '018f5f4d-70c2-7d70-a4d5-5f7355078f89'
    await writeMarker(h, fresh, NEXT_GENERATION)
    await awaitReceipt(h, fresh, NEXT_GENERATION)
  })
})
