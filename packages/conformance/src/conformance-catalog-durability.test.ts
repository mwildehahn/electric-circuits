// The native shape catalog is the restart contract: once POST /shapes succeeds, a crash must not
// turn that acknowledged shape into an unmaintained durable stream. This test puts an ordinary HTTP
// reverse proxy between the engine and durable-streams and makes only catalog appends return 503.
// It uses no engine hooks or private APIs: PostgreSQL SQL, POST/GET /shapes, process restart, and the
// durable stream URL returned by the native create response are the complete reproduction.

import { createServer, request } from 'node:http'

import type { Schema } from '@electric-circuits/protocol'
import { afterEach, describe, expect, it } from 'vitest'

import { createShape, foldStream, pgQuery, waitFor } from './engine-native.js'
import { bootHarness, drainEngine, type Harness } from './harness.js'

const schema: Schema = {
  tables: {
    items: { columns: { id: { type: 'int' }, n: { type: 'int' } }, primaryKey: 'id' },
  },
}

interface CatalogFaultProxy {
  url: string
  failedCatalogAppends(): number
  failedShapeRetirements(): number
  failCatalogAppends(fail: boolean): void
  failShapeRetirements(fail: boolean): void
  close(): Promise<void>
}

async function startCatalogFaultProxy(upstreamUrl: string): Promise<CatalogFaultProxy> {
  let fail = false
  let failures = 0
  let failRetirements = false
  let retirementFailures = 0
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

    const forwarded = request(
      target,
      { method: incoming.method, headers: { ...incoming.headers, host: upstream.host } },
      (response) => {
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
    failCatalogAppends: (next) => {
      fail = next
    },
    failShapeRetirements: (next) => {
      failRetirements = next
    },
    close: () => new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
  }
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
})
