// Subset queries — the non-materialized counterpart to a shape. Rows come from one-shot Postgres
// query-backs (`subset.query`); the loaded page is kept live by following a changes-only tail feed
// (`subset.live`) and re-checking each delta's membership in the loaded window *client-side*. The
// engine never holds per-page/per-range state, so a change is matched against one predicate (the base
// filter) and never fans out across ranges. This is our extension of Electric's static Subset: same
// non-materialized query-back, plus a single-range live tail.

import type { AppRouter } from '@electric-circuits/api'
import { canonicalTable } from '@electric-circuits/protocol'
import type {
  ColumnType,
  Predicate,
  Row,
  Schema,
  StreamEnvelope,
  SubsetDef,
  SubsetResult,
  Value,
} from '@electric-circuits/protocol'
import { stream } from '@durable-streams/client'
import { createCollection, type Collection } from '@tanstack/db'
import type { createTRPCClient } from '@trpc/client'

import { resolveTableDef } from './tables.js'

type Trpc = ReturnType<typeof createTRPCClient<AppRouter>>

export interface SubsetSubscription<T extends Row = Row> {
  /** Live collection: the query-back rows within the loaded window, kept current by the live tail. */
  collection: Collection<T, string>
  /** Fetch the next page from Postgres and append it; resolves to the rows added (0 once exhausted). */
  loadMore(pageSize?: number): Promise<number>
  /** False once a page returned fewer rows than requested (the set is fully loaded). */
  hasMore(): boolean
  /** Tear down the live feed (drops the server-side changes-only feed) and stop following the tail. */
  close(): Promise<void>
}

export interface SubsetDeps {
  trpc: Trpc
  schema: Schema
  /** Resolve a feed handle to a readable stream URL (honoring any dev-proxy base). */
  resolveStreamUrl(handle: { streamPath: string; streamUrl: string }): string
  liveMode?: boolean | 'sse' | 'long-poll'
}

/**
 * Compare two strings by Unicode **code point**, which is the order the engine's subset page arrives
 * in (`ORDER BY <col> COLLATE "C"`) and the order the engine's own predicate evaluation uses.
 *
 * JavaScript's `<` compares UTF-16 **code units**, which is a different order: a supplementary
 * character (U+1F600 → surrogates D83D DE00) sorts before the private-use area (U+E000) under `<`,
 * and after it by code point — so a live row could be admitted to, or dropped from, a loaded window
 * PostgreSQL puts on the other side of the boundary. Iterating with `for…of` yields code points.
 */
export function cmpCodePoints(a: string, b: string): number {
  if (a === b) return 0
  const ai = a[Symbol.iterator]()
  const bi = b[Symbol.iterator]()
  for (;;) {
    const x = ai.next()
    const y = bi.next()
    if (x.done) return y.done ? 0 : -1
    if (y.done) return 1
    const ax = x.value.codePointAt(0) as number
    const bx = y.value.codePointAt(0) as number
    if (ax !== bx) return ax < bx ? -1 : 1
  }
}

/**
 * Compare two cell values of a column of type `ty`. NULL sorts **last**, which is PostgreSQL's
 * default for an ascending `ORDER BY`; `makeCmp` multiplies by the order's direction, so a
 * descending order gets NULLS FIRST — Postgres's other default. The engine's page query uses
 * `ORDER BY <col> COLLATE "C" <dir>` for text, so this is the same order the page arrives in — see
 * `packages/client/README.md` and `docs/ARCHITECTURE.md` §7.
 *
 * The column TYPE decides the comparison, never the runtime shape of the value: an `int` cell is
 * `number | string` on the wire (a bigint beyond 2^53 arrives as an exact decimal string), so
 * comparing "whatever looks numeric" numerically would order a **text** column's `'10'` before
 * `'9'`, which PostgreSQL does not.
 */
function cmpVal(a: Value, b: Value, ty?: ColumnType): number {
  if (a === b) return 0
  if (a == null) return b == null ? 0 : 1
  if (b == null) return -1
  if (ty === 'int') {
    const ai = BigInt(a as number | string)
    const bi = BigInt(b as number | string)
    return ai < bi ? -1 : ai > bi ? 1 : 0
  }
  if (typeof a === 'number' && typeof b === 'number') return a - b
  return cmpCodePoints(String(a), String(b))
}

/**
 * Row comparator matching the engine's `ORDER BY <col> <dir>, <pk> <dir>` (pk tiebreaker, same
 * dir). `types` resolves a column's declared type so each comparison uses the column's own order
 * (see [`cmpVal`]); an unknown column falls back to the value-shape rules.
 */
export function makeCmp(
  pk: string,
  orderBy?: { col: string; desc?: boolean },
  types?: (col: string) => ColumnType | undefined,
): (a: Row, b: Row) => number {
  const dir = orderBy?.desc ? -1 : 1
  const col = orderBy?.col
  const colType = col ? types?.(col) : undefined
  const pkType = types?.(pk)
  return (a, b) => {
    if (col) {
      const d = cmpVal(a[col], b[col], colType)
      if (d !== 0) return dir * d
    }
    return dir * cmpVal(a[pk], b[pk], pkType)
  }
}

/**
 * Keyset predicate for rows strictly *after* `b` in the order — the cursor for the next page.
 *
 * NULL sort keys need their own arms, because no comparison is ever TRUE against or about a NULL
 * (three-valued logic), so a plain `col > b.col` cursor both fails to *reach* the NULL block and
 * fails to *leave* it — paging would stop at the first NULL-keyed row for ever. The NULL block's
 * position is the ORDER BY default this cursor has to agree with: ascending puts it last,
 * descending puts it first.
 */
function cursorPredicate(pk: string, orderBy: { col: string; desc?: boolean } | undefined, b: Row): Predicate {
  const pkOp = orderBy?.desc ? 'lt' : 'gt'
  if (!orderBy?.col) return { col: pk, op: pkOp, value: b[pk] }
  const col = orderBy.col
  const colOp = orderBy.desc ? 'lt' : 'gt'
  const nullBlockAfter: Predicate = { and: [{ col, isNull: true }, { col: pk, op: pkOp, value: b[pk] }] }
  if (b[col] == null) {
    // Inside the NULL block. Ascending (NULLS LAST): only later NULLs remain. Descending (NULLS
    // FIRST): later NULLs, and then the whole non-NULL body of the table.
    return orderBy.desc ? { or: [nullBlockAfter, { col, isNull: false }] } : nullBlockAfter
  }
  const after: Predicate = {
    or: [
      { col, op: colOp, value: b[col] },
      { and: [{ col, op: 'eq', value: b[col] }, { col: pk, op: pkOp, value: b[pk] }] },
    ],
  }
  // Ascending: the NULL block still lies ahead of a non-NULL boundary and must be paged into.
  // Descending: it sorted before the boundary, so it is already behind us.
  return orderBy.desc ? after : { or: [after, { col, isNull: true }] }
}

function andPredicate(base: Predicate | undefined, cursor: Predicate): Predicate {
  return base ? { and: [base, cursor] } : cursor
}

/** Does this error mean the shape/feed is already gone? Then a delete has nothing left to do. */
function isNotFoundError(e: unknown): boolean {
  const data = (e as { data?: { code?: string; httpStatus?: number } } | null)?.data
  if (data?.code === 'NOT_FOUND' || data?.httpStatus === 404) return true
  return /not[ _-]?found|404/i.test(e instanceof Error ? e.message : String(e))
}

/**
 * Release a server-side shape/feed subscriber ref, retrying transient failures. The engine
 * refcounts per identical create; the final release does not delete the shape (the retention
 * lifecycle — idle → dormant → evicted — retires it), but a swallowed release leaks a refcount
 * that pins the shape active forever, so retry with backoff and only warn once if the delete
 * never lands. "Not found" counts as success (the shape was already evicted by retention).
 */
export async function deleteShapeWithRetry(trpc: Trpc, id: string): Promise<void> {
  const attempts = 5
  for (let i = 0; i < attempts; i++) {
    try {
      await trpc.shapes.delete.mutate({ id })
      return
    } catch (e) {
      if (isNotFoundError(e)) return
      if (i === attempts - 1) {
        console.warn(`client: failed to delete shape ${id} after ${attempts} attempts:`, e)
        return
      }
      await new Promise((r) => setTimeout(r, 200 * 2 ** i))
    }
  }
}

/**
 * Parse a Postgres LSN (`"HI/LO"` hex) into a comparable bigint — mirrors the engine's
 * `pg::lsn_to_u64` (`(hi << 32) | lo`). `null`/empty/malformed → null (library/no-Postgres mode, or
 * an unparseable header): callers treat a null LSN as "apply fresh". Never throws — a throw here
 * would propagate out of the feed's async iterator and silently kill the live tail.
 */
export function lsnToU64(lsn: string | undefined | null): bigint | null {
  if (!lsn) return null
  const slash = lsn.indexOf('/')
  if (slash < 0) return null
  const hi = Number.parseInt(lsn.slice(0, slash), 16)
  const lo = Number.parseInt(lsn.slice(slash + 1), 16)
  if (Number.isNaN(hi) || Number.isNaN(lo)) return null
  return (BigInt(hi) << 32n) | BigInt(lo)
}

/** The loaded subset window's membership + per-row LSN watermark (the merge state). */
export interface SubsetView {
  snapshotLsn: bigint
  present: Set<string>
  applied: Map<string, bigint>
  /** Is the row within the currently-loaded keyset window? */
  inView: (row: Row) => boolean
}

/** A collection write to emit, or `null` to drop the delta. */
export type MergeAction =
  | { type: 'insert' | 'update'; value: Row }
  | { type: 'delete'; key: string }
  | null

/**
 * Decide how one live-feed delta updates the loaded subset view, applying **LSN positioning** +
 * **last-writer-wins**. Mutates `view.present`/`view.applied`. Returns the write to emit, or `null`
 * to drop the delta because it is: already reflected in the page (commit LSN < the row's watermark /
 * the snapshot floor), stale w.r.t. a newer page/delta, or out of the loaded window. Exported so the
 * no-double-count invariant can be unit-tested without the full stack.
 */
export function mergeFeedDelta(view: SubsetView, env: StreamEnvelope): MergeAction {
  const key = env.key
  const deltaLsn = lsnToU64(env.headers.lsn)
  // A null LSN (library/no-Postgres mode) always applies — the old idempotent-by-pk behaviour.
  const fresh = (): boolean => {
    if (deltaLsn === null) return true
    const w = view.applied.get(key)
    return w === undefined ? deltaLsn >= view.snapshotLsn : deltaLsn >= w
  }
  if (env.headers.operation === 'delete') {
    if (!fresh()) return null
    const wasPresent = view.present.has(key)
    view.present.delete(key)
    // Keep a tombstone watermark instead of clearing it: absence from `present` + watermark w means
    // "deleted at ≥ w", so an in-flight loadMore page snapshotted before the delete (pageLsn < w)
    // is skipped by the loadMore guard rather than resurrecting the row. Recorded even for a
    // never-seen pk — otherwise a stale page could insert a ghost row the feed already deleted.
    if (deltaLsn !== null) view.applied.set(key, deltaLsn)
    else view.applied.delete(key)
    return wasPresent ? { type: 'delete', key } : null
  }
  const value = env.value
  if (!value || !fresh()) return null
  if (view.inView(value)) {
    const type = view.present.has(key) ? 'update' : 'insert'
    view.present.add(key)
    if (deltaLsn !== null) view.applied.set(key, deltaLsn)
    return { type, value }
  }
  if (view.present.has(key)) {
    // Moved out of the loaded window (e.g. its sort key dropped below the boundary). Same tombstone
    // treatment as a delete: a stale in-flight page must not re-insert the pre-move version.
    view.present.delete(key)
    if (deltaLsn !== null) view.applied.set(key, deltaLsn)
    else view.applied.delete(key)
    return { type: 'delete', key }
  }
  return null
}

/** Manual-write handles captured from the collection's sync callback (used by load-more + the feed). */
interface SyncCtl {
  begin: () => void
  write: (m: { type: 'insert' | 'update' | 'delete'; value?: Row; key?: string }) => void
  commit: () => void
}

export async function createSubset<T extends Row = Row>(
  deps: SubsetDeps,
  def: SubsetDef,
): Promise<SubsetSubscription<T>> {
  // Either spelling — `items` or `public.items` — whichever one keyed the local schema.
  const tableDef = resolveTableDef(deps.schema, def.table)
  // Feed envelopes spell the table canonically (`schema.name`, ADR-0002) whatever spelling the
  // caller used, so the envelope filter below must compare against the canonical form.
  const feedType = canonicalTable(def.table)
  const pk = tableDef.primaryKey
  const cmp = makeCmp(pk, def.orderBy, (col) => tableDef.columns[col]?.type)
  const limit = def.limit ?? 100
  // The order column + pk must be present on every row so membership/cursoring can be evaluated, even
  // when the caller projects a narrower column set.
  const cols = def.columns
    ? Array.from(new Set([pk, ...(def.orderBy ? [def.orderBy.col] : []), ...def.columns]))
    : undefined

  // 1. Open the live tail FIRST so it captures every change from ~now. The feed may be SHARED with other
  //    subscriptions on the same predicate (the engine ref-counts identical changes-only feeds).
  const feed = await deps.trpc.subset.live.mutate({ table: def.table, where: def.where as never, columns: cols })
  const feedUrl = deps.resolveStreamUrl(feed)

  const ac = new AbortController()
  let closed = false

  // The feed above is the only server-side state we hold; if any of the remaining setup (offset
  // capture, page query-back, preload) throws, delete it before rethrowing — nobody else will.
  try {
    // 1b. Capture the feed's current tail offset BEFORE the page snapshot. Reading the live tail from
    //     here (rather than the stream origin) means a joiner to a SHARED, long-lived feed does not
    //     replay the whole backlog — it starts at "≈now". Everything at/before this offset committed
    //     before the snapshot LSN below and is already in the page; the `< snapshotLsn` drop covers the
    //     small [thisOffset, snapshot] overlap. Falls back to the stream origin if HEAD is unavailable.
    let feedOffset = '-1'
    try {
      const head = await fetch(feedUrl, { method: 'HEAD' })
      feedOffset = head.headers.get('stream-next-offset') ?? '-1'
    } catch {
      /* proxy/env without HEAD support → read from origin; correctness unaffected (only backlog). */
    }

    // 2. Query-back page 1 straight from Postgres (no stream, no materialization). `offset` applies
    //    to THIS page only: it is where the caller's window starts, and every later page is reached
    //    by moving the keyset cursor past the boundary — re-applying the offset there would skip
    //    that many rows again.
    const first = (await deps.trpc.subset.query.query({
      table: def.table,
      where: def.where as never,
      columns: cols,
      orderBy: def.orderBy,
      limit,
      offset: def.offset,
    })) as SubsetResult

    // `boundary` = the last (lowest-in-order) loaded row; the loaded window is everything sorting <= it.
    let boundary: Row | null = first.rows.length ? first.rows[first.rows.length - 1]! : null
    // ...and, WITH AN OFFSET, everything sorting >= the first loaded row. Without this lower bound
    // the window is open at the bottom, so a live delta that moves a row into the region the offset
    // deliberately skipped would insert it — the subset would silently grow past the page the
    // caller asked for. A row sorting strictly before the first loaded row is out of the window
    // under keyset semantics, and under OFFSET semantics it would SHIFT the window, which no live
    // delta may do; "not in view" is the right answer either way. (An empty first page leaves no
    // lower bound to take — nothing was loaded to anchor it to.)
    const lower: Row | null = def.offset && first.rows.length ? first.rows[0]! : null
    // A page shorter than the page size means the set is fully loaded. A page size of ZERO is never
    // short, and can never move the cursor either, so it ends the subset outright rather than
    // promising a next page that could never arrive.
    let ended = limit === 0 || first.rows.length < limit
    const present = new Set<string>()
    // LSN positioning: `snapshotLsn` is the page's read point in the engine's replication timeline.
    // `applied` is a per-present-row watermark — the snapshot LSN the row's current value was read at
    // (page or loadMore), bumped to a feed delta's LSN when applied. A feed delta is accepted only if
    // its commit LSN is at/after the relevant watermark, so deltas already reflected in the page (commit
    // LSN < snapshotLsn) are dropped — exactly-once after the snapshot, no double-count.
    const snapshotLsn = lsnToU64(first.lsn) ?? 0n
    const applied = new Map<string, bigint>()
    const inView = (row: Row): boolean => {
      // The lower bound holds even once `ended`: "fully loaded" means the pages ran out, not that
      // the rows below the offset joined the window.
      if (lower != null && cmp(row, lower) < 0) return false
      return ended || boundary == null || cmp(row, boundary) <= 0
    }

    let ctl: SyncCtl | null = null
    let loadsInFlight = 0

    const view: SubsetView = { snapshotLsn, present, applied, inView }
    const applyEnvelope = (env: StreamEnvelope): void => {
      if (!ctl || env.type !== feedType) return
      const action = mergeFeedDelta(view, env)
      if (!action) return
      ctl.begin()
      ctl.write(action)
      ctl.commit()
    }

    const collection = createCollection<T>({
      id: `subset:${def.table}:${feed.shapeId}`,
      getKey: (r) => String((r as Row)[pk]),
      sync: {
        sync: (params: SyncCtl & { markReady: () => void }) => {
          ctl = params
          // Seed the query-back page.
          params.begin()
          for (const r of first.rows) {
            const k = String(r[pk])
            params.write({ type: 'insert', value: r })
            present.add(k)
            applied.set(k, snapshotLsn)
          }
          params.commit()
          params.markReady()
          // 3. Follow the raw live tail from the offset captured before the snapshot, applying each change
          //    filtered by membership + LSN positioning (deltas already in the page are dropped).
          void (async () => {
            try {
              const resp = await stream<StreamEnvelope>({
                url: feedUrl,
                offset: feedOffset,
                live: deps.liveMode ?? 'long-poll',
                contentType: 'application/json',
                signal: ac.signal,
              })
              for await (const env of resp.jsonStream()) applyEnvelope(env)
            } catch (e) {
              // After close() the feed's durable stream may already be gone (the engine deletes it on
              // the final drop), so a racing read 404s — normal termination, not an error.
              if (!ac.signal.aborted && !closed) console.error('subset feed error', e)
            }
          })()
          return () => ac.abort()
        },
      },
    })
    await collection.preload()

    return {
      collection: collection as Collection<T, string>,
      hasMore: () => !ended,

      async loadMore(pageSize = limit) {
        // `pageSize <= 0` asks for nothing: answer 0 without a round-trip, and without concluding
        // the set is exhausted (a zero-row answer to a zero-row request proves nothing about it).
        if (closed || ended || pageSize <= 0 || !boundary || !ctl) return 0
        const where = andPredicate(def.where, cursorPredicate(pk, def.orderBy, boundary))
        loadsInFlight++
        try {
          const page = (await deps.trpc.subset.query.query({
            table: def.table,
            where: where as never,
            columns: cols,
            orderBy: def.orderBy,
            limit: pageSize,
          })) as SubsetResult
          if (page.rows.length) {
            // This page is a fresh Postgres snapshot at `pageLsn`; its rows are the authoritative state as
            // of that LSN. Don't let a stale page regress a row already advanced past `pageLsn` by the live
            // feed (the loadMore-vs-feed race), and set each row's watermark so older feed deltas drop.
            // Tombstoned rows (watermark without membership) are skipped the same way — a page older than
            // the delete must not resurrect the row.
            const pageLsn = lsnToU64(page.lsn) ?? snapshotLsn
            ctl.begin()
            for (const r of page.rows) {
              const k = String(r[pk])
              const w = applied.get(k)
              if (w !== undefined && pageLsn < w) continue
              ctl.write({ type: present.has(k) ? 'update' : 'insert', value: r })
              present.add(k)
              applied.set(k, pageLsn)
            }
            ctl.commit()
            boundary = page.rows[page.rows.length - 1]!
          }
          if (page.rows.length < pageSize) ended = true
          return page.rows.length
        } finally {
          loadsInFlight--
          // Tombstone watermarks only exist to guard in-flight loadMore pages; once none are in
          // flight, prune them so delete churn doesn't grow `applied` unboundedly.
          if (loadsInFlight === 0) {
            for (const k of applied.keys()) if (!present.has(k)) applied.delete(k)
          }
        }
      },

      async close() {
        // One-shot: the engine DELETE decrements a shared refcount per call, so a double close must
        // not steal another subscriber's reference on a shared feed.
        if (closed) return
        closed = true
        ac.abort()
        await deleteShapeWithRetry(deps.trpc, feed.shapeId)
      },
    }
  } catch (e) {
    closed = true
    ac.abort()
    await deleteShapeWithRetry(deps.trpc, feed.shapeId)
    throw e
  }
}
