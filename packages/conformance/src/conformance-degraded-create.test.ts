import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Schema } from '@electric-circuits/protocol'

import { bootHarness, drainEngine, type Harness } from './harness.js'
import { createShape, lockTable, pgQuery, tableLockWaiters, waitFor } from './engine-native.js'

const schema: Schema = {
  tables: {
    parent: { columns: { id: { type: 'int' }, active: { type: 'bool' } }, primaryKey: 'id' },
    child: { columns: { id: { type: 'int' }, parent_id: { type: 'int' } }, primaryKey: 'id' },
    child2: { columns: { id: { type: 'int' }, parent_id: { type: 'int' } }, primaryKey: 'id' },
  },
}

const activeParents = { table: 'parent', project: 'id', where: { col: 'active', op: 'eq', value: true } } as const

async function healthStatus(h: Harness): Promise<number> {
  return (await fetch(`${h.engineUrl}/v1/health`)).status
}

async function streamStatus(streamUrl: string): Promise<number> {
  return (await fetch(`${streamUrl}?offset=-1`)).status
}

describe('native: degradation closes every membership stream, including an in-flight create', () => {
  let h: Harness
  beforeAll(async () => {
    h = await bootHarness(schema)
    await pgQuery(h, 'INSERT INTO parent (id, active) VALUES (1, true)')
    await pgQuery(h, 'INSERT INTO child (id, parent_id) VALUES (1, 1)')
    await drainEngine(h)
  }, 60000)
  afterAll(async () => await h?.shutdown())

  it('a create already in progress cannot return a live handle after degradation', async () => {
    const live = await createShape(h, { table: 'child', where: { col: 'parent_id', in: activeParents } })
    await drainEngine(h)
    expect(await streamStatus(live.streamUrl)).not.toBe(404)

    // Start a second shape before degradation and hold its outer backfill after its snapshot is fixed.
    const createLock = await lockTable(h, 'child2')
    const creating = createShape(h, { table: 'child2', where: { col: 'parent_id', in: activeParents } })
    await waitFor(async () => (await tableLockWaiters(h, 'child2')).length > 0, 'the second create to block')

    const queryBackLock = await lockTable(h, 'child')
    try {
      await pgQuery(h, 'UPDATE parent SET active = false WHERE id = 1')

      // Exhaust the production retry schedule through ordinary Postgres connection failures.
      const killed = new Set<number>()
      for (let attempt = 1; attempt <= 5; attempt++) {
        let pid = 0
        await waitFor(async () => {
          pid = (await tableLockWaiters(h, 'child')).find((candidate) => !killed.has(candidate)) ?? 0
          return pid !== 0
        }, `query-back retry ${attempt} to block`)
        killed.add(pid)
        await pgQuery(h, 'SELECT pg_terminate_backend($1)', [pid])
      }

      await waitFor(async () => (await healthStatus(h)) === 503, 'the engine to report degraded')
      await waitFor(async () => (await streamStatus(live.streamUrl)) === 404, 'the live stream to be reaped')
    } finally {
      await queryBackLock.release()
    }

    // The second create started while health was active. Keep it parked until the degradation
    // reaper has run, then let its empty backfill finish.
    await createLock.release()
    const created = await creating

    // A successful POST must never hand a normal client a stream the same engine has already reaped.
    expect(await streamStatus(created.streamUrl)).not.toBe(404)
  }, 60000)
})
