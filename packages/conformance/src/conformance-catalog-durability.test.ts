// The native shape catalog is the restart contract: once POST /shapes succeeds, a crash must not
// turn that acknowledged shape into an unmaintained durable stream. This test puts an ordinary HTTP
// reverse proxy between the engine and durable-streams and makes only catalog appends return 503.
// It uses no engine hooks or private APIs: PostgreSQL SQL, POST/GET /shapes, process restart, and the
// durable stream URL returned by the native create response are the complete reproduction.

import { createServer, request } from 'node:http'

import type { Schema } from '@electric-circuits/protocol'
import { afterEach, describe, expect, it } from 'vitest'

import { createShape, foldStream, pgQuery, type ShapeResp, waitFor } from './engine-native.js'
import { bootHarness, drainEngine, type Harness } from './harness.js'

const schema: Schema = {
  tables: {
    items: { columns: { id: { type: 'int' }, n: { type: 'int' } }, primaryKey: 'id' },
  },
}

const subquerySchema: Schema = {
  tables: {
    parent: {
      columns: { id: { type: 'int' }, active: { type: 'bool' } },
      primaryKey: 'id',
    },
    child: {
      columns: { id: { type: 'int' }, parent_id: { type: 'int' } },
      primaryKey: 'id',
    },
  },
}

interface CatalogFaultProxy {
  url: string
  failedCatalogAppends(): number
  failedShapeRetirements(): number
  lostCatalogResponses(): number
  failCatalogAppends(fail: boolean): void
  failShapeRetirements(fail: boolean): void
  loseNextCatalogResponse(): void
  close(): Promise<void>
}

async function startCatalogFaultProxy(upstreamUrl: string): Promise<CatalogFaultProxy> {
  let fail = false
  let failures = 0
  let failRetirements = false
  let retirementFailures = 0
  let loseNextCatalogResponse = false
  let lostCatalogResponses = 0
  const upstream = new URL(upstreamUrl)
  const server = createServer((incoming, outgoing) => {
    const target = new URL(incoming.url ?? '/', upstream)
    if (fail && incoming.method === 'POST' && target.pathname === '/meta/catalog') {
      failures += 1
      incoming.resume()
      outgoing.writeHead(503, { 'content-type': 'text/plain' })
      outgoing.end('catalog temporarily unavailable')
      return
    }
    if (
      failRetirements &&
      target.pathname.startsWith('/shape/') &&
      (incoming.method === 'POST' || incoming.method === 'DELETE')
    ) {
      retirementFailures += 1
      incoming.resume()
      outgoing.writeHead(503, { 'content-type': 'text/plain' })
      outgoing.end('shape retirement temporarily unavailable')
      return
    }

    const loseThisResponse =
      loseNextCatalogResponse && incoming.method === 'POST' && target.pathname === '/meta/catalog'
    if (loseThisResponse) loseNextCatalogResponse = false
    const forwarded = request(
      target,
      { method: incoming.method, headers: { ...incoming.headers, host: upstream.host } },
      (response) => {
        if (loseThisResponse) {
          // The upstream append has already committed. Consume its success response but report an
          // ordinary transient gateway failure to the engine, reproducing a lost response after a
          // successful non-idempotent request.
          response.resume()
          response.once('end', () => {
            lostCatalogResponses += 1
            outgoing.writeHead(503, { 'content-type': 'text/plain' })
            outgoing.end('catalog response lost after upstream commit')
          })
          return
        }
        outgoing.writeHead(response.statusCode ?? 502, response.headers)
        response.pipe(outgoing)
      },
    )
    forwarded.on('error', (error) => {
      if (!outgoing.headersSent) outgoing.writeHead(502, { 'content-type': 'text/plain' })
      outgoing.end(String(error))
    })
    incoming.pipe(forwarded)
  })

  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => resolve())
  })
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('catalog fault proxy did not bind TCP')

  return {
    url: `http://127.0.0.1:${address.port}`,
    failedCatalogAppends: () => failures,
    failedShapeRetirements: () => retirementFailures,
    lostCatalogResponses: () => lostCatalogResponses,
    failCatalogAppends: (next) => {
      fail = next
    },
    failShapeRetirements: (next) => {
      failRetirements = next
    },
    loseNextCatalogResponse: () => {
      loseNextCatalogResponse = true
    },
    close: () => new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
  }
}

async function readCatalogEvents(dsUrl: string): Promise<Array<{ t?: string; id?: string }>> {
  const events: Array<{ t?: string; id?: string }> = []
  let offset = '-1'
  for (let i = 0; i < 100; i++) {
    const response = await fetch(`${dsUrl}/meta/catalog?offset=${encodeURIComponent(offset)}`)
    if (response.status === 204) break
    if (!response.ok) throw new Error(`GET meta/catalog -> ${response.status}`)
    const body = (await response.text()).trim()
    if (body) events.push(...(JSON.parse(body) as Array<{ t?: string; id?: string }>))
    const next = response.headers.get('stream-next-offset')
    const upToDate = response.headers.get('stream-up-to-date') !== null
    if (!next || next === offset) break
    offset = next
    if (upToDate) break
  }
  return events
}

async function createAggregate(harness: Harness): Promise<ShapeResp> {
  const response = await fetch(`${harness.engineUrl}/aggregate`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ table: 'items', fn: 'count' }),
  })
  if (!response.ok) throw new Error(`POST /aggregate -> ${response.status} ${await response.text()}`)
  return (await response.json()) as ShapeResp
}

let h: Harness | undefined
afterEach(async () => {
  await h?.shutdown()
  h = undefined
})

describe('native catalog durability under a durable-streams status failure', () => {
  it('does not acknowledge a shape whose Created record was rejected by storage', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
    })

    await pgQuery(h, 'INSERT INTO items (id, n) VALUES (1, 10)')
    await drainEngine(h)
    await new Promise((resolve) => setTimeout(resolve, 250))

    proxy!.failCatalogAppends(true)
    const creating = createShape(h, { table: 'items' })
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the proxy to reject the Created event')
    proxy!.failCatalogAppends(false)

    // Rejecting creation is a valid fail-closed answer. If creation was acknowledged, however, the
    // handle and stream must be durable across the crash.
    let shape: Awaited<ReturnType<typeof createShape>>
    try {
      shape = await creating
    } catch {
      return
    }

    // The public create already returned success. Recover storage, crash the engine, and use only
    // the handle it returned to check whether that acknowledged subscription survived.
    await h.restartEngine()
    await pgQuery(h, 'INSERT INTO items (id, n) VALUES (2, 20)')
    await drainEngine(h)

    const shapeStatus = await fetch(`${h.engineUrl}/shapes/${shape.shapeId}`)
    const rows = await foldStream(shape.streamUrl)
    expect.soft(shapeStatus.status, 'the acknowledged native shape must still be registered after restart').toBe(200)
    expect.soft(rows.has('2'), 'the returned durable stream must still receive PostgreSQL changes').toBe(true)
  }, 90000)

  it('retries mandatory TRUNCATE retirement after durable-streams recovers', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
    })

    await pgQuery(h, 'INSERT INTO items (id, n) VALUES (1, 10)')
    await drainEngine(h)
    const shape = await createShape(h, { table: 'items' })
    expect((await foldStream(shape.streamUrl)).has('1')).toBe(true)

    proxy!.failShapeRetirements(true)
    await pgQuery(h, 'TRUNCATE items')
    await waitFor(
      () => Promise.resolve(proxy!.failedShapeRetirements() >= 2),
      'the proxy to reject both close and delete',
    )
    await waitFor(
      async () => (await fetch(`${h!.engineUrl}/shapes/${shape.shapeId}`)).status === 404,
      'the engine to remove the retired shape record',
    )

    // Recovery is entirely external. The engine has already forgotten the shape, but its mandatory
    // retirement must still finish: otherwise the public stream URL remains open with rows that
    // PostgreSQL no longer contains, forever.
    proxy!.failShapeRetirements(false)
    await waitFor(async () => {
      const response = await fetch(shape.streamUrl)
      return response.status === 404 || response.status === 410
    }, 'the stale shape stream to be deleted after durable-streams recovers')
  }, 90000)

  it('does not resurrect an acknowledged purge when its catalog intent and retirement are interrupted', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
      engineEnv: {
        ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS: '1',
        ELECTRIC_CIRCUITS_SHAPE_DORMANT_TTL_SECS: '1',
        ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS: '1',
      },
    })

    await pgQuery(h, 'INSERT INTO items (id, n) VALUES (1, 10)')
    await drainEngine(h)
    const shape = await createShape(h, { table: 'items' })
    expect((await foldStream(shape.streamUrl)).has('1')).toBe(true)

    // Keep both halves of the promised retirement from reaching storage. The only interactions are
    // the native purge route and ordinary HTTP failures from the engine's storage dependency.
    proxy!.failCatalogAppends(true)
    proxy!.failShapeRetirements(true)
    const purged = await fetch(`${h.engineUrl}/shapes/${shape.shapeId}?purge=true`, { method: 'DELETE' })
    expect(purged.ok, 'the native API acknowledged that the shape was purged').toBe(true)
    await waitFor(
      () => Promise.resolve(proxy!.failedCatalogAppends() > 0),
      'the proxy to reject the Dropped catalog intent',
    )
    await waitFor(
      () => Promise.resolve(proxy!.failedShapeRetirements() >= 2),
      'the proxy to reject both close and delete',
    )
    expect((await fetch(`${h.engineUrl}/shapes/${shape.shapeId}`)).status).toBe(404)

    // Crash while the acknowledged Dropped event and retirement are both still retrying. Recovery
    // happens only after the old process is gone, so it cannot flush its in-memory catalog queue.
    await h.restartEngine(async () => {
      proxy!.failCatalogAppends(false)
      proxy!.failShapeRetirements(false)
    })

    // Under ADR-0008 a purge acknowledged while the durable catalog was unavailable is reconverged
    // by the LEASE after a restart, within one idle window: the shape comes back from its `Created`
    // record with its subscriptions' restored lease ages, nothing renews them, and the sweeper
    // reclaims it. (Waiting for the `Dropped` before answering is not an option — it would leave a
    // caller unable to purge anything at all during an outage, deadlocking against a create parked
    // on its own durability wait.)
    await waitFor(
      async () => (await fetch(`${h!.engineUrl}/shapes/${shape.shapeId}`)).status === 404,
      'the lease to reclaim the purged shape after restart',
    )
    await waitFor(async () => {
      const response = await fetch(shape.streamUrl)
      return response.status === 404 || response.status === 410
    }, 'the purged stream to remain retired after restart')
  }, 90000)

  it('does not acknowledge a join whose shared shape was purged during its durability wait', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
    })

    const first = await createShape(h, { table: 'items' })
    proxy!.failCatalogAppends(true)
    const joining = createShape(h, { table: 'items' })
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the Joined event to be blocked')

    // The join has passed its lifecycle/schema checks and is waiting only for catalog durability.
    // Purge the shared shape through the native API while that external storage wait is in flight.
    const purged = await fetch(`${h.engineUrl}/shapes/${first.shapeId}?purge=true`, { method: 'DELETE' })
    expect(purged.ok).toBe(true)
    await waitFor(
      async () => (await fetch(`${h!.engineUrl}/shapes/${first.shapeId}`)).status === 404,
      'the shared shape to be purged',
    )
    expect((await fetch(first.streamUrl)).status).toBe(404)

    proxy!.failCatalogAppends(false)
    const joined = await joining
    expect(
      (await fetch(`${h.engineUrl}/shapes/${joined.shapeId}`)).status,
      'a successful native join must return a shape that still exists',
    ).toBe(200)
    expect((await fetch(joined.streamUrl)).status, 'a successful native join must return a readable stream').toBe(200)
  }, 90000)

  it('does not acknowledge a first create retired by TRUNCATE during its durability wait', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
    })

    await pgQuery(h, 'INSERT INTO items (id, n) VALUES (1, 10)')
    await drainEngine(h)
    proxy!.failCatalogAppends(true)
    const creating = createShape(h, { table: 'items' })
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the Created event to be blocked')

    // The create has completed its snapshot and closing schema check, then waits for storage. A
    // public database operation can still retire it in that interval.
    await pgQuery(h, 'TRUNCATE items')
    await waitFor(
      async () => (await fetch(`${h!.engineUrl}/shapes/s1`)).status === 404,
      'TRUNCATE to retire the pending first shape',
    )

    proxy!.failCatalogAppends(false)
    const created = await creating
    expect(
      (await fetch(`${h.engineUrl}/shapes/${created.shapeId}`)).status,
      'a successful native create must still be registered after its durability wait',
    ).toBe(200)
    expect((await fetch(created.streamUrl)).status, 'a successful native create must return a readable stream').toBe(200)
  }, 90000)

  it('does not acknowledge an aggregate retired by TRUNCATE during its durability wait', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
    })

    await pgQuery(h, 'INSERT INTO items (id, n) VALUES (1, 10)')
    await drainEngine(h)
    proxy!.failCatalogAppends(true)
    const creating = createAggregate(h)
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the aggregate Created event to be blocked')

    await pgQuery(h, 'TRUNCATE items')
    await waitFor(
      async () => (await fetch(`${h!.engineUrl}/shapes/s1`)).status === 404,
      'TRUNCATE to retire the pending aggregate',
    )

    proxy!.failCatalogAppends(false)
    const created = await creating
    expect(
      (await fetch(`${h.engineUrl}/shapes/${created.shapeId}`)).status,
      'a successful native aggregate create must still be registered after its durability wait',
    ).toBe(200)
    expect((await fetch(created.streamUrl)).status, 'a successful aggregate must return a readable stream').toBe(200)
  }, 90000)

  it('does not acknowledge a subquery shape retired during its durability wait', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(subquerySchema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
    })

    await pgQuery(h, 'INSERT INTO parent (id, active) VALUES (1, true)')
    await pgQuery(h, 'INSERT INTO child (id, parent_id) VALUES (1, 1)')
    await drainEngine(h)
    proxy!.failCatalogAppends(true)
    const creating = createShape(h, {
      table: 'child',
      where: {
        col: 'parent_id',
        in: { table: 'parent', project: 'id', where: { col: 'active', op: 'eq', value: true } },
      },
    })
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the subquery Created event to be blocked')

    // Retiring an inner dependency retires the outer subquery shape as well.
    await pgQuery(h, 'TRUNCATE parent')
    await waitFor(
      async () => (await fetch(`${h!.engineUrl}/shapes/s1`)).status === 404,
      'the inner-table TRUNCATE to retire the pending subquery shape',
    )

    proxy!.failCatalogAppends(false)
    const created = await creating
    expect(
      (await fetch(`${h.engineUrl}/shapes/${created.shapeId}`)).status,
      'a successful native subquery create must still be registered after its durability wait',
    ).toBe(200)
    expect((await fetch(created.streamUrl)).status, 'a successful subquery create must return a readable stream').toBe(200)
  }, 90000)

  it('does not apply a non-idempotent catalog mutation twice when only its success response is lost', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
      engineEnv: {
        // A long idle window on purpose: these subscriptions are anonymous and never renewed, and
        // this test is about what the FOLD does with a duplicated `Left` — not about leases. Under a
        // second-scale window the sweeper would (correctly, ADR-0008) reclaim the survivor
        // mid-assertion and hide the very thing being measured.
        ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS: '120',
        ELECTRIC_CIRCUITS_SHAPE_DORMANT_TTL_SECS: '120',
        ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS: '1',
      },
    })

    const first = await createShape(h, { table: 'items' })
    const second = await createShape(h, { table: 'items' })
    const third = await createShape(h, { table: 'items' })
    expect(second.shapeId).toBe(first.shapeId)
    expect(third.shapeId).toBe(first.shapeId)

    // One subscriber leaves. Storage commits that one `Left`, but its success response disappears
    // at the proxy. Retrying the same unkeyed append must not turn one public DELETE into two leaves.
    proxy!.loseNextCatalogResponse()
    const released = await fetch(`${h.engineUrl}/shapes/${first.shapeId}`, { method: 'DELETE' })
    expect(released.ok).toBe(true)
    await waitFor(() => Promise.resolve(proxy!.lostCatalogResponses() === 1), 'the committed response to be lost')
    await waitFor(
      async () =>
        (await readCatalogEvents(h!.dsUrl)).filter((event) => event.t === 'left' && event.id === first.shapeId)
          .length >= 2,
      'the retry to duplicate the committed Left event',
    )

    await h.restartEngine()
    // Two subscribers still exist. Releasing one of them must leave the other subscribed, so the
    // retention lifecycle has no right to retire their shared stream.
    const releasedSecond = await fetch(`${h.engineUrl}/shapes/${first.shapeId}`, { method: 'DELETE' })
    expect(releasedSecond.ok).toBe(true)
    // The reproduction, inverted into the requirement: the duplicated `Left` used to fold as two
    // decrements, leaving a shape with a live subscriber at refcount 0 for retention to evict. It
    // must now never happen, so this wait must TIME OUT.
    await expect(
      waitFor(
        async () => (await fetch(`${h!.engineUrl}/shapes/${first.shapeId}`)).status === 404,
        'the duplicated Left to evict a shape that still has a subscriber',
        15000,
      ),
    ).rejects.toThrow()
    expect(
      (await fetch(`${h.engineUrl}/shapes/${first.shapeId}`)).status,
      'one remaining subscriber must keep its shared native shape alive',
    ).toBe(200)
    expect((await fetch(third.streamUrl)).status, 'the remaining subscriber stream must stay readable').toBe(200)
  }, 90000)

  it('does not leak a subscriber when its join request is aborted during the durability wait', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
      engineEnv: {
        ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS: '1',
        ELECTRIC_CIRCUITS_SHAPE_DORMANT_TTL_SECS: '1',
        ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS: '1',
      },
    })

    const first = await createShape(h, { table: 'items' })
    proxy!.failCatalogAppends(true)
    const abort = new AbortController()
    const abandonedJoin = createShape(h, { table: 'items' }, abort.signal)
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the Joined event to be blocked')

    // This caller never receives a handle and therefore can never release the refcount already
    // taken for it. Cancelling a public HTTP request must undo that provisional subscription.
    abort.abort()
    await expect(abandonedJoin).rejects.toThrow()
    proxy!.failCatalogAppends(false)
    await waitFor(
      async () => (await readCatalogEvents(h!.dsUrl)).some((event) => event.t === 'joined' && event.id === first.shapeId),
      'the abandoned join event to reach the public catalog stream',
    )

    const released = await fetch(`${h.engineUrl}/shapes/${first.shapeId}`, { method: 'DELETE' })
    expect(released.ok).toBe(true)
    await new Promise((resolve) => setTimeout(resolve, 5000))
    expect(
      (await fetch(`${h.engineUrl}/shapes/${first.shapeId}`)).status,
      'the only subscription that ever received a handle was released, so retention must reclaim the shape',
    ).toBe(404)
  }, 90000)

  it('does not forget an acknowledged release when shutdown is forced during a catalog outage', async () => {
    let proxy: CatalogFaultProxy | undefined
    h = await bootHarness(schema, {
      wrapEngineDs: async (upstreamUrl) => {
        proxy = await startCatalogFaultProxy(upstreamUrl)
        return proxy
      },
      engineEnv: {
        ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS: '1',
        ELECTRIC_CIRCUITS_SHAPE_DORMANT_TTL_SECS: '1',
        ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS: '1',
      },
    })

    const shape = await createShape(h, { table: 'items' })
    proxy!.failCatalogAppends(true)
    const released = await fetch(`${h.engineUrl}/shapes/${shape.shapeId}`, { method: 'DELETE' })
    expect(released.ok, 'the native API acknowledged the subscription release').toBe(true)
    await waitFor(() => Promise.resolve(proxy!.failedCatalogAppends() > 0), 'the Left event to be blocked')

    // A normal SIGTERM cannot drain an unavailable catalog. A second signal is the documented
    // external forced-shutdown path and discards the process's only copy of the acknowledged Left.
    h.signalEngine('SIGTERM')
    await waitFor(async () => {
      try {
        return (await fetch(`${h!.engineUrl}/ready`)).status === 503
      } catch {
        return false
      }
    }, 'shutdown to enter its readiness drain')
    h.signalEngine('SIGTERM')
    const exit = await h.waitForEngineExit(20000)
    expect(exit.code).toBe(70)

    proxy!.failCatalogAppends(false)
    await h.startEngine()
    expect(
      (await readCatalogEvents(h.dsUrl)).filter((event) => event.t === 'left' && event.id === shape.shapeId),
      'the forced exit occurred before the queued release reached durable storage',
    ).toHaveLength(0)
    await new Promise((resolve) => setTimeout(resolve, 5000))
    expect(
      (await fetch(`${h.engineUrl}/shapes/${shape.shapeId}`)).status,
      'the acknowledged release must remain effective after restart so retention can reclaim the shape',
    ).toBe(404)
    expect((await fetch(shape.streamUrl)).status).toBe(404)
  }, 90000)
})
