import type { AddressInfo } from 'node:net'
import { createServer } from 'node:http'
import { createHTTPHandler } from '@trpc/server/adapters/standalone'
import { createCore, type ElectricCore } from './core.js'
import { handleRestRequest } from './rest.js'
import { appRouter } from './router.js'

export interface ApiServer {
  url: string
  core: ElectricCore
  close(): Promise<void>
}

/**
 * Start the API gateway. The tRPC adapter is the primary gateway surface; the `/compat/v1` handler is a
 * compatibility facade for callers that still address the gateway. The canonical native `/v1`
 * contract (and OpenAPI document) lives on the Rust engine, so Swift clients should use the engine
 * URL directly. Both gateway adapters receive the same TypeScript service adapter.
 */
export async function createApiServer(opts: {
  dsUrl: string
  engineUrl: string
  port?: number
  /** Bind host. Default `127.0.0.1`; pass `0.0.0.0` to accept connections from other hosts/containers. */
  host?: string
}): Promise<ApiServer> {
  const core = createCore({ dsUrl: opts.dsUrl, engineUrl: opts.engineUrl })
  const trpcHandler = createHTTPHandler({ router: appRouter, createContext: () => ({ core }) })
  const server = createServer((req, res) => {
    // `req.url` is the origin-form request target; no client-controlled Host header is needed
    // to route it. Avoid constructing a URL from Host, which can throw on malformed input.
    const pathname = (req.url ?? '/').split('?', 1)[0] ?? '/'
    if (pathname === '/compat/v1' || pathname.startsWith('/compat/v1/')) {
      void handleRestRequest(req, res, core)
      return
    }
    trpcHandler(req, res)
  })
  const bind = opts.host ?? '127.0.0.1'
  await new Promise<void>((resolve) => server.listen(opts.port ?? 0, bind, () => resolve()))
  const addr = server.address() as AddressInfo
  const host = bind === '0.0.0.0' || bind === '::' ? '127.0.0.1' : bind
  return {
    url: `http://${host}:${addr.port}`,
    core,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  }
}
