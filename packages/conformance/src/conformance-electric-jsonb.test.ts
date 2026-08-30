import { Shape, ShapeStream } from '@electric-sql/client'
import type { Schema } from '@electric-circuits/protocol'
import pgpkg from 'pg'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { bootHarness, drainEngine, type Harness } from './harness.js'

const ownerId = '00000000-0000-0000-0000-000000000001'

const schema: Schema = {
  tables: {
    conversation: {
      columns: {
        id: { type: 'int' },
        created_by_user_id: { type: 'text' },
        metadata: { type: 'text' },
        deleted_at: { type: 'text' },
      },
      primaryKey: 'id',
    },
  },
}

type ConversationRow = {
  id: bigint
  created_by_user_id: string
  metadata: { calendar?: { monitoring?: { unresolved_count: number } } } | null
  deleted_at: string | null
}

function waitForMonitoringCount(
  shape: Shape<ConversationRow>,
  unresolvedCount: number,
  timeoutMs = 10_000,
): { applied: Promise<void>; cancel: () => void } {
  let unsubscribe = () => {}
  let timeout: ReturnType<typeof setTimeout> | undefined
  let settled = false

  const applied = new Promise<void>((resolve, reject) => {
    const finish = (error?: Error) => {
      if (settled) return
      settled = true
      if (timeout) clearTimeout(timeout)
      unsubscribe()
      if (error) reject(error)
      else resolve()
    }

    unsubscribe = shape.subscribe(({ rows }) => {
      if (
        rows[0]?.metadata?.calendar?.monitoring?.unresolved_count ===
        unresolvedCount
      ) {
        finish()
      }
    })
    timeout = setTimeout(() => {
      const observed =
        shape.currentRows[0]?.metadata?.calendar?.monitoring?.unresolved_count
      finish(
        new Error(
          `timed out waiting for unresolved_count=${unresolvedCount}; observed ${String(observed)}`,
        ),
      )
    }, timeoutMs)
  })

  return {
    applied,
    cancel: () => {
      if (settled) return
      settled = true
      if (timeout) clearTimeout(timeout)
      unsubscribe()
    },
  }
}

describe('conformance: Electric client JSONB compatibility', () => {
  let harness: Harness

  beforeAll(async () => {
    harness = await bootHarness(schema, {
      ddl: `
        CREATE TABLE conversation (
          id bigint PRIMARY KEY,
          created_by_user_id uuid NOT NULL,
          metadata jsonb,
          deleted_at timestamptz
        );
        ALTER TABLE conversation REPLICA IDENTITY FULL;
        INSERT INTO conversation VALUES
          (1, '${ownerId}', '{"calendar":{"monitoring":{"unresolved_count":2}}}', now()),
          (2, '${ownerId}', NULL, now());
      `,
    })
  }, 60_000)

  afterAll(async () => {
    await harness?.shutdown()
  })

  it('materializes JSONB and retains a metadata-bearing tombstone after a live update', async () => {
    const abort = new AbortController()
    const stream = new ShapeStream<ConversationRow>({
      url: `${harness.engineUrl}/v1/shape`,
      params: {
        table: 'conversation',
        where:
          '(deleted_at IS NULL OR (deleted_at IS NOT NULL AND created_by_user_id = $1 AND metadata IS NOT NULL))',
        'params[1]': ownerId,
      },
      signal: abort.signal,
    })
    const shape = new Shape(stream)
    let cancelUpdateWait = () => {}

    try {
      const initialRows = await shape.rows
      expect(initialRows).toHaveLength(1)
      expect(initialRows[0]?.metadata).toEqual({
        calendar: { monitoring: { unresolved_count: 2 } },
      })

      const postgres = new pgpkg.Client({ connectionString: harness.pgUrl })
      const update = waitForMonitoringCount(shape, 3)
      cancelUpdateWait = update.cancel
      await postgres.connect()
      try {
        await postgres.query(
          `UPDATE conversation
           SET metadata = '{"calendar":{"monitoring":{"unresolved_count":3}}}'
           WHERE id = 1`,
        )
      } finally {
        await postgres.end()
      }

      await drainEngine(harness)
      await update.applied
      expect(shape.currentRows).toHaveLength(1)
    } finally {
      cancelUpdateWait()
      abort.abort()
      shape.unsubscribeAll()
    }
  }, 60_000)
})
