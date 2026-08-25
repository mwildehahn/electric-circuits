import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { SubsetResult } from '@electric-circuits/protocol'
import type { ElectricCore, ShapeHandle } from './core.js'
import { handleRestRequest } from './rest.js'

const handle: ShapeHandle = {
  shapeId: 's1',
  table: 'issues',
  streamPath: 'shape/s1',
  streamUrl: 'http://streams/shape/s1',
  subscription: 'ios-client',
  leaseSeconds: 60,
}

function fakeCore(): ElectricCore & {
  createShape: ReturnType<typeof vi.fn>
  getShape: ReturnType<typeof vi.fn>
  dropShape: ReturnType<typeof vi.fn>
} {
  return {
    dsUrl: 'http://streams',
    defineSchema: vi.fn(async () => undefined),
    write: vi.fn(async () => ({ txid: 'tx1' })),
    createShape: vi.fn(async () => handle),
    getShape: vi.fn(async () => handle),
    dropShape: vi.fn(async () => undefined),
    querySubset: vi.fn(async (): Promise<SubsetResult> => ({ rows: [], lsn: '0/1' })),
    createSubsetFeed: vi.fn(async () => handle),
    createAggregate: vi.fn(async () => handle),
  }
}

async function listen(core: ElectricCore): Promise<{ server: Server; url: string }> {
  const server = createServer((req, res) => {
    void handleRestRequest(req, res, core)
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address() as AddressInfo
  return { server, url: `http://127.0.0.1:${address.port}` }
}

async function close(server: Server): Promise<void> {
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
}

describe('REST adapter', () => {
  let server: Server | undefined

  afterEach(async () => {
    if (server) await close(server)
    server = undefined
  })

  it('passes the canonical predicate through shape creation', async () => {
    const core = fakeCore()
    const listening = await listen(core)
    server = listening.server
    const where = { and: [{ col: 'status', op: 'eq', value: 'open' }] }

    const response = await fetch(`${listening.url}/v1/shapes`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ table: 'issues', where, columns: ['id', 'status'], subscription: 'ios-client' }),
    })

    expect(response.status).toBe(200)
    expect(await response.json()).toEqual(handle)
    expect(core.createShape).toHaveBeenCalledWith(
      { table: 'issues', where, columns: ['id', 'status'] },
      'ios-client',
    )
  })

  it('maps shape lookup and idempotent release to the core service', async () => {
    const core = fakeCore()
    const listening = await listen(core)
    server = listening.server

    const get = await fetch(`${listening.url}/v1/shapes/s1`)
    expect(get.status).toBe(200)
    expect(await get.json()).toEqual(handle)

    const deleted = await fetch(`${listening.url}/v1/shapes/s1?subscription=ios-client`, { method: 'DELETE' })
    expect(deleted.status).toBe(204)
    expect(core.dropShape).toHaveBeenCalledWith('s1', 'ios-client')
  })

  it('returns problem details for invalid input and missing shapes', async () => {
    const core = fakeCore()
    core.getShape.mockResolvedValueOnce(null)
    const listening = await listen(core)
    server = listening.server

    const invalid = await fetch(`${listening.url}/v1/shapes`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ columns: ['id'] }),
    })
    expect(invalid.status).toBe(400)
    expect(((await invalid.json()) as { code: string }).code).toBe('invalid_argument')

    const missing = await fetch(`${listening.url}/v1/shapes/missing`)
    expect(missing.status).toBe(404)
    expect(((await missing.json()) as { code: string }).code).toBe('not_found')
  })
})
