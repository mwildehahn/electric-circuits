// A logical slot must release WAL even when the database is otherwise idle. Postgres advances the
// replication stream with keepalives between transactions; the engine may acknowledge that safe
// position because there is no buffered transaction whose durable-stream append is still pending.

import type { Schema } from '@electric-circuits/protocol'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { pgQuery, waitFor } from './engine-native.js'
import { bootHarness, drainEngine, type Harness } from './harness.js'

const schema: Schema = {
  tables: {
    users: {
      columns: {
        id: { type: 'int' },
      },
      primaryKey: 'id',
    },
  },
}

async function confirmedFlushReached(harness: Harness, targetLsn: string): Promise<boolean> {
  const [slot] = await pgQuery(
    harness,
    `SELECT confirmed_flush_lsn::text AS confirmed_flush_lsn,
            pg_wal_lsn_diff(confirmed_flush_lsn, $2::pg_lsn) >= 0 AS reached
       FROM pg_replication_slots
      WHERE slot_name = $1`,
    [harness.slot, targetLsn],
  )
  return slot?.reached === true
}

describe('conformance: idle logical-replication progress', () => {
  let harness: Harness

  beforeAll(async () => {
    harness = await bootHarness(schema)
  }, 60000)

  afterAll(async () => {
    await harness?.shutdown()
  })

  it('acknowledges a forced WAL switch without requiring a table write', async () => {
    await drainEngine(harness)

    const [switched] = await pgQuery(harness, 'SELECT pg_switch_wal()::text AS lsn')
    const switchLsn = String(switched?.lsn)
    expect(switchLsn).toMatch(/^[0-9A-F]+\/[0-9A-F]+$/)

    await waitFor(
      () => confirmedFlushReached(harness, switchLsn),
      `slot ${harness.slot} to acknowledge idle WAL through ${switchLsn}`,
      30000,
    )
  }, 60000)
})
