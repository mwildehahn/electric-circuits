// ADR-0002 makes a bare table name ingress shorthand for public.<name>. The published native client
// must therefore accept either spelling independently of which spelling keyed its local Schema.

import type { Schema } from '@electric-circuits/protocol'
import { afterEach, describe, expect, it } from 'vitest'

import { bootHarness, type Harness } from './harness.js'

const schema: Schema = {
  tables: {
    items: { columns: { id: { type: 'int' }, n: { type: 'int' } }, primaryKey: 'id' },
  },
}

let h: Harness | undefined
afterEach(async () => {
  await h?.shutdown()
  h = undefined
})

describe('published native client table aliases', () => {
  it('accepts public.items when the local schema is keyed by its items shorthand', async () => {
    h = await bootHarness(schema)
    const shape = await h.client.shape({ table: 'public.items' })
    expect(shape.handle.table).toBe('public.items')
    await shape.close()
  }, 90000)
})
