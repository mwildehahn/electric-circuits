// External consumers read the engine's segmented change log through its public positioned route.
// This is deliberately a real-stack contract: Postgres is the independent source oracle, the
// engine ingests logical replication, and the Rust durable-streams server holds the pages/close
// headers the external reader receives.

import pgpkg from 'pg'
import type { Schema, StreamEnvelope } from '@electric-circuits/protocol'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { bootHarness, drainEngine, type Harness } from './harness.js'

const schema: Schema = {
  tables: { items: { columns: { id: { type: 'int' }, n: { type: 'int' } }, primaryKey: 'id' } },
}

// Rotate after each committed append, while keeping the sweep asleep long enough for this reader
// to start at segment zero. Retention/pin behavior belongs to the next consumer-pin contract.
const knobs = {
  ELECTRIC_CIRCUITS_CHANGES_SEGMENT_BYTES: '1',
  ELECTRIC_CIRCUITS_CHANGES_SEGMENT_SECS: '0',
  ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS: '3600',
}

let h: Harness
beforeEach(async () => {
  h = await bootHarness(schema, { engineEnv: knobs })
})
afterEach(async () => {
  await h.shutdown()
})

async function pg(sql: string, params: unknown[] = []): Promise<void> {
  const client = new pgpkg.Client({ connectionString: h.pgUrl })
  await client.connect()
  try {
    await client.query(sql, params)
  } finally {
    await client.end().catch(() => {})
  }
}

interface Position {
  generation: string
  position: { segment: number; offset: string }
  segments: Record<string, number>
}

interface Page {
  envelopes: StreamEnvelope[]
  closed: boolean
}

async function position(): Promise<Position> {
  const response = await fetch(`${h.engineUrl}/changes/position`)
  if (!response.ok) throw new Error(`GET /changes/position -> ${response.status} ${await response.text()}`)
  return (await response.json()) as Position
}

async function read(segment: number, offset: string, generation: string): Promise<Page> {
  const response = await fetch(
    `${h.engineUrl}/changes/${segment}?offset=${encodeURIComponent(offset)}&generation=${encodeURIComponent(generation)}`,
  )
  if (!response.ok) throw new Error(`GET /changes/${segment} -> ${response.status} ${await response.text()}`)
  return {
    envelopes: (await response.json()) as StreamEnvelope[],
    closed: response.headers.get('stream-closed') === 'true',
  }
}

function successor(page: Page): number | undefined {
  const pointer = [...page.envelopes]
    .reverse()
    .find((envelope) => envelope.type === '__circuits.control' && envelope.headers.operation === 'rotated')
  const segment = pointer?.value?.segment
  return typeof segment === 'number' ? segment : undefined
}

describe('conformance: external change-log reader', () => {
  it('reads source commits once across a closed segment rotation', async () => {
    await pg('INSERT INTO items (id, n) VALUES ($1, $2)', [1, 10])
    await drainEngine(h)
    await pg('INSERT INTO items (id, n) VALUES ($1, $2)', [2, 20])
    await drainEngine(h)

    // This is the independent source-of-record oracle, authored as SQL rather than from the
    // engine page. The reader must observe exactly this committed sequence.
    const source = new pgpkg.Client({ connectionString: h.pgUrl })
    await source.connect()
    const expected = (await source.query('SELECT id, n FROM items ORDER BY id')).rows
    await source.end()

    const start = await position()
    expect(start.generation).toBeTruthy()
    expect(start.position.segment).toBeGreaterThanOrEqual(2)
    expect(start.segments).toHaveProperty('0')

    const first = await read(0, '-1', start.generation)
    expect(first.closed).toBe(true)
    expect(successor(first)).toBe(1)
    const second = await read(1, '-1', start.generation)
    expect(second.closed).toBe(true)
    expect(successor(second)).toBe(2)

    const changes = [...first.envelopes, ...second.envelopes].filter((envelope) => envelope.type !== '__circuits.control')
    expect(changes.map((envelope) => envelope.value?.id)).toEqual(expected.map((row) => row.id))
    expect(changes.map((envelope) => envelope.headers.last)).toEqual([true, true])
    expect(changes.every((envelope) => typeof envelope.headers.lsn === 'string' && typeof envelope.headers.seq === 'number')).toBe(true)
  })
})
