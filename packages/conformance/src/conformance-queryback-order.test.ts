import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { Schema } from '@electric-circuits/protocol'

import { bootHarness, drainEngine, type Harness } from './harness.js'
import { createShape, foldStream, pgQuery, waitFor } from './engine-native.js'

const schema: Schema = {
  tables: {
    parent: { columns: { id: { type: 'int' }, active: { type: 'bool' } }, primaryKey: 'id' },
    child: {
      columns: { id: { type: 'int' }, parent_id: { type: 'int' }, payload: { type: 'text' } },
      primaryKey: 'id',
    },
  },
}

const activeParents = { table: 'parent', project: 'id', where: { col: 'active', op: 'eq', value: true } } as const

async function candidateQueryRunning(h: Harness): Promise<boolean> {
  const rows = await pgQuery(
    h,
    `SELECT 1
       FROM pg_stat_activity
      WHERE datname = current_database()
        AND state = 'active'
        AND query ILIKE '%from "child" t%'
        AND pid <> pg_backend_pid()
      LIMIT 1`,
  )
  return rows.length > 0
}

describe('native: a query-back cannot overwrite a newer outer-row decision', () => {
  let h: Harness
  beforeAll(async () => {
    h = await bootHarness(schema)
    await pgQuery(h, 'INSERT INTO parent (id, active) VALUES (1, false), (2, false)')
    await pgQuery(
      h,
      `INSERT INTO child (id, parent_id, payload)
       SELECT i, 1, repeat('x', 256) FROM generate_series(1, 100000) AS i`,
    )
    await drainEngine(h, 60000)
  }, 90000)
  afterAll(async () => await h?.shutdown())

  it('a row moved after the query snapshot does not re-enter from that stale snapshot', async () => {
    const shape = await createShape(h, { table: 'child', where: { col: 'parent_id', in: activeParents } })
    await drainEngine(h)
    expect((await foldStream(shape.streamUrl)).size).toBe(0)

    // Activating parent 1 starts a query-back for all 100,000 candidate rows. Once Postgres is
    // visibly executing that SELECT, move row 1 to inactive parent 2 in a newer transaction.
    await pgQuery(h, 'UPDATE parent SET active = true WHERE id = 1')
    await waitFor(() => candidateQueryRunning(h), 'the membership query-back to establish its snapshot')
    await pgQuery(h, 'UPDATE child SET parent_id = 2 WHERE id = 1')

    await drainEngine(h, 60000)
    const rows = await foldStream(shape.streamUrl)
    expect(rows.has('1')).toBe(false)
    expect(rows.size).toBe(99999)
  }, 90000)
})
