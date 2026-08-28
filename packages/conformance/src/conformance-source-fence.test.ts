import type { Schema } from '@electric-circuits/protocol'
import pgpkg from 'pg'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { createShape, foldStream, waitFor } from './engine-native.js'
import { bootHarness, type Harness } from './harness.js'

const CONTROL_SECRET = 'conformance-controller-secret'
const SOURCE_COMMIT_ID = '018f5f4d-70c2-7d70-a4d5-5f7355078f85'

const schema: Schema = {
  tables: {
    items: {
      columns: {
        id: { type: 'int' },
        value: { type: 'text' },
      },
      primaryKey: 'id',
    },
  },
}

async function controlRequest(h: Harness, path: string, method = 'GET'): Promise<Response> {
  return fetch(`${h.engineUrl}${path}`, {
    method,
    headers: { authorization: `Bearer ${CONTROL_SECRET}` },
  })
}

describe('conformance: named source fence handoff', () => {
  let harness: Harness

  beforeAll(async () => {
    harness = await bootHarness(schema, {
      engineEnv: { ELECTRIC_CIRCUITS_CONTROL_SECRET: CONTROL_SECRET },
    })
  }, 60_000)

  afterAll(async () => {
    await harness?.shutdown()
  })

  it('publishes a queryable receipt only after the fenced transaction output is durable', async () => {
    const shape = await createShape(harness, { table: 'items' })
    const closed = await controlRequest(harness, '/_admin/control-admission/close', 'POST')
    expect(closed.status).toBe(200)

    const postgres = new pgpkg.Client({ connectionString: harness.pgUrl })
    await postgres.connect()
    try {
      await postgres.query('BEGIN')
      await postgres.query('INSERT INTO items (id, value) VALUES (1, $1)', ['fenced'])
      await postgres.query(
        'INSERT INTO circuits_source_fence (source_commit_id) VALUES ($1::uuid)',
        [SOURCE_COMMIT_ID],
      )
      await postgres.query('COMMIT')
    } finally {
      await postgres.end().catch(() => {})
    }

    let receipt: { drained: boolean; receipt?: { sourceCommitId: string; commitLsn: string } } | undefined
    await waitFor(async () => {
      const response = await controlRequest(
        harness,
        `/_admin/drained-through/${SOURCE_COMMIT_ID}`,
      )
      expect(response.status).toBe(200)
      receipt = (await response.json()) as typeof receipt
      return receipt?.drained === true
    }, 'durable named source-fence receipt')

    expect(receipt?.receipt?.sourceCommitId).toBe(SOURCE_COMMIT_ID)
    expect(receipt?.receipt?.commitLsn).toMatch(/^[0-9A-F]+\/[0-9A-F]+$/)
    expect(await foldStream(shape.streamUrl)).toEqual(
      new Map([['1', { id: 1, value: 'fenced' }]]),
    )
  }, 60_000)
})
