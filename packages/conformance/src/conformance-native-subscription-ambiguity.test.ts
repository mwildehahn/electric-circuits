// Native shape creation and release cross an HTTP boundary where a request can commit even though
// its response is lost. These tests put an ordinary reverse proxy in front of the public API and
// reproduce both ambiguous outcomes through the published client; no engine internals are used.

import { createServer, request } from 'node:http'

import { createClient, type ElectricIvmClient } from '@electric-circuits/client'
import type { Schema } from '@electric-circuits/protocol'
import { afterEach, describe, expect, it } from 'vitest'

import { waitFor } from './engine-native.js'
import { bootHarness, type Harness } from './harness.js'

const schema: Schema = {
  tables: {
    items: { columns: { id: { type: 'int' }, n: { type: 'int' } }, primaryKey: 'id' },
  },
}

interface ResponseLossProxy {
  url: string
  loseNextResponseFor(pathFragment: string): void
  lostResponses(): number
  forwarded(pathFragment: string): number
  close(): Promise<void>
}

async function startResponseLossProxy(upstreamUrl: string): Promise<ResponseLossProxy> {
  const upstream = new URL(upstreamUrl)
  let loseFor: string | undefined
  let lost = 0
  const forwardedPaths: string[] = []
  const server = createServer((incoming, outgoing) => {
    const target = new URL(incoming.url ?? '/', upstream)
    const path = `${target.pathname}${target.search}`
    forwardedPaths.push(path)
    const loseThisResponse = loseFor !== undefined && path.includes(loseFor)
    if (loseThisResponse) loseFor = undefined

    const forwarded = request(
      target,
      { method: incoming.method, headers: { ...incoming.headers, host: upstream.host } },
      (response) => {
        if (loseThisResponse && response.statusCode !== undefined && response.statusCode < 400) {
          // The public API has completed the native mutation. Consume that success response, then
          // reproduce a gateway failure between the service and its client.
          response.resume()
          response.once('end', () => {
            lost += 1
            outgoing.writeHead(503, { 'content-type': 'text/plain' })
            outgoing.end('upstream success response lost')
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
  if (!address || typeof address === 'string') throw new Error('response-loss proxy did not bind TCP')

  return {
    url: `http://127.0.0.1:${address.port}`,
    loseNextResponseFor: (pathFragment) => {
      loseFor = pathFragment
    },
    lostResponses: () => lost,
    forwarded: (pathFragment) => forwardedPaths.filter((path) => path.includes(pathFragment)).length,
    close: () => new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve()))),
  }
}

let h: Harness | undefined
const clients: ElectricIvmClient[] = []
const proxies: ResponseLossProxy[] = []

afterEach(async () => {
  for (const client of clients.splice(0)) await client.close().catch(() => {})
  for (const proxy of proxies.splice(0)) await proxy.close().catch(() => {})
  await h?.shutdown()
  h = undefined
})

async function bootWithShortRetention(): Promise<{ client: ElectricIvmClient; proxy: ResponseLossProxy }> {
  h = await bootHarness(schema, {
    engineEnv: {
      ELECTRIC_CIRCUITS_SHAPE_IDLE_SECS: '1',
      ELECTRIC_CIRCUITS_SHAPE_DORMANT_TTL_SECS: '1',
      ELECTRIC_CIRCUITS_RETENTION_SWEEP_SECS: '1',
    },
  })
  const proxy = await startResponseLossProxy(h.apiUrl)
  proxies.push(proxy)
  const client = createClient({ apiUrl: proxy.url, schema, liveMode: 'long-poll' })
  clients.push(client)
  return { client, proxy }
}

describe('native subscription mutations with an ambiguous HTTP result', () => {
  it('does not release two subscribers when one successful DELETE response is lost and retried', async () => {
    const { client, proxy } = await bootWithShortRetention()
    const first = await client.shape({ table: 'items' })
    const second = await client.shape({ table: 'items' })
    expect(second.handle.shapeId).toBe(first.handle.shapeId)

    // deleteShapeWithRetry is part of the published client. Its first DELETE reaches the native
    // engine and succeeds; only the response is lost, so its retry is the same logical release.
    proxy.loseNextResponseFor('shapes.delete')
    await first.close()
    expect(proxy.lostResponses()).toBe(1)
    expect(proxy.forwarded('shapes.delete')).toBeGreaterThanOrEqual(2)

    // The reproduction, inverted into the requirement: the duplicated release used to take the
    // shared refcount from two to zero and let retention delete the stream under the other live
    // materialization. It must now never happen, so this wait must TIME OUT.
    await expect(
      waitFor(
        async () => (await fetch(`${h!.engineUrl}/shapes/${second.handle.shapeId}`)).status === 404,
        'the retried release to evict the shared shape despite its other live subscriber',
        15000,
      ),
    ).rejects.toThrow()
    expect(
      (await fetch(`${h!.engineUrl}/shapes/${second.handle.shapeId}`)).status,
      'one logical close must leave the other native subscription and stream alive',
    ).toBe(200)
    expect((await fetch(second.handle.streamUrl)).status).toBe(200)
  }, 90000)

  it('does not pin an unreachable subscriber when a successful POST response is lost', async () => {
    const { client, proxy } = await bootWithShortRetention()

    // The engine creates and durably records this subscription, but the caller receives only 503
    // and therefore has no handle it could ever close.
    proxy.loseNextResponseFor('shapes.create')
    await expect(client.shape({ table: 'items' })).rejects.toThrow()
    expect(proxy.lostResponses()).toBe(1)

    // A later normal create joins the same native shape. Once that only returned subscription is
    // closed, retention must be able to reclaim it; the lost response must not create an immortal
    // phantom subscriber.
    const reachable = await client.shape({ table: 'items' })
    await reachable.close()
    await new Promise((resolve) => setTimeout(resolve, 5000))

    expect(
      (await fetch(`${h!.engineUrl}/shapes/${reachable.handle.shapeId}`)).status,
      'a subscription whose create response was lost cannot permanently pin the native shape',
    ).toBe(404)
  }, 90000)
})
