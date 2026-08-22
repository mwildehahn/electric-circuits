// Resolving a caller's table spelling against the local Schema.
//
// ADR-0002 makes `schema.name` canonical and a bare name shorthand for `public.<name>` — at the API
// boundary. A client's `Schema` is not the API boundary: it is local config the app wrote, keyed by
// whatever spelling it chose, and the engine never sees those keys. So `shape({ table: 'public.items' })`
// against a schema keyed `items` is not a mistake to reject; both spellings name the same table, and
// the engine accepts either one on the wire.

import type { Schema, TableDef } from '@electric-circuits/protocol'
import { canonicalTable, PUBLIC_SCHEMA, parseTableRef } from '@electric-circuits/protocol'

/**
 * The local schema entry for a table, whichever of its equivalent spellings the caller used and
 * whichever one keyed the schema: the key **as given** first, then the canonical `schema.name`, then
 * — for a table in `public` — the bare name. `undefined` when the schema genuinely has no such table.
 *
 * As-given wins, which only matters for a schema carrying BOTH `items` and `public.items`: they name
 * the same Postgres table but are two distinct entries here, and each spelling then resolves to its
 * own. That is a schema worth not writing; resolving it by preferring the caller's exact key is the
 * least surprising of the available answers.
 *
 * An invalid reference (empty, two dots, a quote) is NOT resolved to `undefined`: `parseTableRef`
 * throws, and a malformed table name should say so rather than read as "unknown table".
 */
export function lookupTableDef(schema: Schema, table: string): TableDef | undefined {
  const direct = schema.tables[table]
  if (direct) return direct
  const ref = parseTableRef(table)
  const canonical = schema.tables[canonicalTable(ref)]
  if (canonical) return canonical
  return ref.schema === PUBLIC_SCHEMA ? schema.tables[ref.name] : undefined
}

/** [`lookupTableDef`], but a miss is the error every caller would otherwise write itself. */
export function resolveTableDef(schema: Schema, table: string): TableDef {
  const def = lookupTableDef(schema, table)
  if (!def) throw new Error(`client: unknown table "${table}"`)
  return def
}
