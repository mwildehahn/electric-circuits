import type { Schema } from '@electric-circuits/protocol'
import { describe, expect, it } from 'vitest'

import { lookupTableDef, resolveTableDef } from './tables.js'

const items = { columns: { id: { type: 'int' as const } }, primaryKey: 'id' }

const bareKeyed: Schema = { tables: { items } }
const canonicallyKeyed: Schema = { tables: { 'public.items': items } }
const otherSchema: Schema = { tables: { 'billing.items': items } }

describe('local schema lookup accepts either spelling (ADR-0002)', () => {
  it('resolves a canonical reference against a bare-keyed schema', () => {
    expect(lookupTableDef(bareKeyed, 'public.items')).toBe(items)
    expect(lookupTableDef(bareKeyed, 'items')).toBe(items)
  })

  it('resolves a bare reference against a canonically keyed schema', () => {
    expect(lookupTableDef(canonicallyKeyed, 'items')).toBe(items)
    expect(lookupTableDef(canonicallyKeyed, 'public.items')).toBe(items)
  })

  it('never resolves across schemas: only `public` has a bare shorthand', () => {
    expect(lookupTableDef(otherSchema, 'items')).toBeUndefined()
    expect(lookupTableDef(otherSchema, 'public.items')).toBeUndefined()
    expect(lookupTableDef(otherSchema, 'billing.items')).toBe(items)
    expect(lookupTableDef(bareKeyed, 'billing.items')).toBeUndefined()
  })

  it('a schema carrying both spellings resolves each to its own entry (as-given wins)', () => {
    const bare = { columns: { id: { type: 'int' as const } }, primaryKey: 'id' }
    const qualified = { columns: { id: { type: 'int' as const } }, primaryKey: 'id' }
    const both: Schema = { tables: { items: bare, 'public.items': qualified } }
    expect(lookupTableDef(both, 'items')).toBe(bare)
    expect(lookupTableDef(both, 'public.items')).toBe(qualified)
  })

  it('a genuinely unknown table is a miss, and the throwing form names it', () => {
    expect(lookupTableDef(bareKeyed, 'orders')).toBeUndefined()
    expect(() => resolveTableDef(bareKeyed, 'orders')).toThrow(/unknown table "orders"/)
  })

  it('a malformed reference throws as malformed, not as unknown', () => {
    // Reading `a.b.c` as "unknown table" would send an operator looking for a missing table.
    expect(() => lookupTableDef(bareKeyed, 'a.b.c')).toThrow(/separator/)
    expect(() => lookupTableDef(bareKeyed, '')).toThrow(/empty/)
  })
})
