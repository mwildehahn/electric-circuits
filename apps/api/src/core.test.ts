import { describe, expect, it } from 'vitest'
import { createCore } from './core.js'

function response(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

describe('Rust engine adapter', () => {
  it('uses the versioned native Axum routes for shape lifecycle', async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = []
    const core = createCore({
      dsUrl: 'http://streams',
      engineUrl: 'http://engine',
      fetch: async (input, init) => {
        calls.push({ url: String(input), init })
        return response({
          shapeId: 's1',
          table: 'public.items',
          streamPath: 'shape/s1',
          streamUrl: 'http://streams/shape/s1',
          subscription: 'ios-client',
          leaseSeconds: 60,
        })
      },
    })

    await core.createShape({ table: 'items' }, 'ios-client')
    await core.getShape('s1')
    await core.dropShape('s1', 'ios-client')

    expect(calls.map((call) => call.url)).toEqual([
      'http://engine/v1/shapes',
      'http://engine/v1/shapes/s1',
      'http://engine/v1/shapes/s1?subscription=ios-client',
    ])
  })

  it('uses the native subset-feed, aggregate, and query endpoints', async () => {
    const urls: string[] = []
    const core = createCore({
      dsUrl: 'http://streams',
      engineUrl: 'http://engine',
      fetch: async (input) => {
        urls.push(String(input))
        return response(
          String(input).endsWith('/subsets/query')
            ? { rows: [], lsn: '0/1' }
            : { shapeId: 's1', table: 'public.items', streamPath: 'shape/s1', streamUrl: 'http://streams/shape/s1' },
        )
      },
    })

    await core.createSubsetFeed({ table: 'items' })
    await core.createAggregate({ table: 'items', fn: 'count' })
    await core.querySubset({ table: 'items' })

    expect(urls).toEqual([
      'http://engine/v1/subset-feeds',
      'http://engine/v1/aggregates',
      'http://engine/v1/subsets/query',
    ])
  })
})
