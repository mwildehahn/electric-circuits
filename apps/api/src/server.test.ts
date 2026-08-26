import { createServer, request as httpRequest, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { afterEach, describe, expect, it } from 'vitest'
import { createApiServer, type ApiServer } from './server.js'

const handle = {
  shapeId: 's1',
  table: 'issues',
  streamPath: 'shape/s1',
  streamUrl: 'http://streams/shape/s1',
  subscription: 'ios-client',
  leaseSeconds: 60,
}

async function listenEngine(): Promise<{ server: Server; url: string; requests: string[] }> {
  const requests: string[] = []
  const server = createServer((req, res) => {
    requests.push(`${req.method} ${req.url}`)
    if (req.method === 'POST' && req.url === '/v1/shapes') {
      res.setHeader('content-type', 'application/json')
      res.end(JSON.stringify(handle))
      return
    }
    res.statusCode = 404
    res.end()
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address() as AddressInfo
  return { server, url: `http://127.0.0.1:${address.port}`, requests }
}

async function closeServer(server: Server): Promise<void> {
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
}

async function rawRequest(url: string, headers: Record<string, string>): Promise<{ status: number; body: string }> {
  return await new Promise((resolve, reject) => {
    const req = httpRequest(url, { method: 'GET', headers }, (res) => {
      const chunks: Buffer[] = []
      res.on('data', (chunk) => chunks.push(Buffer.from(chunk)))
      res.on('end', () => resolve({ status: res.statusCode ?? 0, body: Buffer.concat(chunks).toString('utf8') }))
    })
    req.on('error', reject)
    req.end()
  })
}

describe('API server dispatch', () => {
  let api: ApiServer | undefined
  let engine: Server | undefined

  afterEach(async () => {
    if (api) await api.close()
    if (engine) await closeServer(engine)
    api = undefined
    engine = undefined
  })

  it('routes compat/v1 through REST while leaving canonical /v1 unclaimed by REST', async () => {
    const listening = await listenEngine()
    engine = listening.server
    api = await createApiServer({ dsUrl: listening.url, engineUrl: listening.url })

    const compat = await fetch(`${api.url}/compat/v1/shapes`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ table: 'issues', subscription: 'ios-client' }),
    })
    expect(compat.status).toBe(200)
    expect(await compat.json()).toEqual(handle)

    const canonical = await fetch(`${api.url}/v1/shapes`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{}',
    })
    expect(canonical.status).toBe(404)
    expect(listening.requests).toEqual(['POST /v1/shapes'])
  })

  it('survives a malformed Host header without throwing from dispatch', async () => {
    const listening = await listenEngine()
    engine = listening.server
    api = await createApiServer({ dsUrl: listening.url, engineUrl: listening.url })

    const malformed = await rawRequest(`${api.url}/compat/v1/shapes`, { host: 'foo:bar:baz' })
    expect(malformed.status).toBe(405)
    const healthy = await fetch(`${api.url}/v1/unknown`)
    expect(healthy.status).toBe(404)
  })
})
