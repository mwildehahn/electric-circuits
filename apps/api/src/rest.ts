// Stable REST/JSON adapter over ElectricCore. This module owns HTTP parsing, validation, status
// codes, and error serialization; ElectricCore remains protocol-neutral and can be called by tRPC
// or another adapter without importing any HTTP concerns.

import type { IncomingMessage, ServerResponse } from 'node:http'
import type { AggregateDef, ShapeDef, SubsetDef } from '@electric-circuits/protocol'
import type { ElectricCore } from './core.js'

const MAX_BODY_BYTES = 1024 * 1024

type JsonObject = Record<string, unknown>

class RestError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}

function isObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function requiredString(input: JsonObject, key: string): string {
  const value = input[key]
  if (typeof value !== 'string' || value.length === 0) {
    throw new RestError(400, 'invalid_argument', `${key} must be a non-empty string`)
  }
  return value
}

function optionalString(input: JsonObject, key: string): string | undefined {
  const value = input[key]
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'string') throw new RestError(400, 'invalid_argument', `${key} must be a string`)
  return value
}

function optionalStringArray(input: JsonObject, key: string): string[] | undefined {
  const value = input[key]
  if (value === undefined || value === null) return undefined
  if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) {
    throw new RestError(400, 'invalid_argument', `${key} must be an array of strings`)
  }
  return value
}

function optionalObject(input: JsonObject, key: string): JsonObject | undefined {
  const value = input[key]
  if (value === undefined || value === null) return undefined
  if (!isObject(value)) throw new RestError(400, 'invalid_argument', `${key} must be a JSON object`)
  return value
}

function optionalNonnegativeInt(input: JsonObject, key: string): number | undefined {
  const value = input[key]
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0) {
    throw new RestError(400, 'invalid_argument', `${key} must be a non-negative integer`)
  }
  return value
}

function optionalOrderBy(input: JsonObject): { col: string; desc?: boolean } | undefined {
  const value = optionalObject(input, 'orderBy')
  if (!value) return undefined
  const col = requiredString(value, 'col')
  const desc = value.desc
  if (desc !== undefined && typeof desc !== 'boolean') {
    throw new RestError(400, 'invalid_argument', 'orderBy.desc must be a boolean')
  }
  return { col, desc }
}

function jsonBody<T>(input: JsonObject): T {
  return input as T
}

async function readJson(req: IncomingMessage): Promise<JsonObject> {
  const chunks: Buffer[] = []
  let size = 0
  for await (const chunk of req) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)
    size += buffer.length
    if (size > MAX_BODY_BYTES) throw new RestError(413, 'payload_too_large', 'request body is too large')
    chunks.push(buffer)
  }
  if (chunks.length === 0) return {}
  let parsed: unknown
  try {
    parsed = JSON.parse(Buffer.concat(chunks).toString('utf8'))
  } catch {
    throw new RestError(400, 'invalid_json', 'request body must be valid JSON')
  }
  if (!isObject(parsed)) throw new RestError(400, 'invalid_json', 'request body must be a JSON object')
  return parsed
}

function writeJson(res: ServerResponse, status: number, body: unknown): void {
  const encoded = JSON.stringify(body)
  res.statusCode = status
  res.setHeader('content-type', 'application/json; charset=utf-8')
  res.setHeader('content-length', Buffer.byteLength(encoded))
  res.end(encoded)
}

function writeEmpty(res: ServerResponse, status: number): void {
  res.statusCode = status
  res.end()
}

function writeError(res: ServerResponse, error: unknown): void {
  if (error instanceof RestError) {
    writeJson(res, error.status, {
      type: `https://electric-sql.com/problems/${error.code}`,
      title: error.code,
      status: error.status,
      detail: error.message,
      code: error.code,
    })
    return
  }
  // Do not expose engine internals through the public adapter.
  writeJson(res, 500, {
    type: 'https://electric-sql.com/problems/internal',
    title: 'internal',
    status: 500,
    detail: 'internal server error',
    code: 'internal',
  })
}

function shapeInput(body: JsonObject): { def: ShapeDef; subscription?: string } {
  const table = requiredString(body, 'table')
  const where = optionalObject(body, 'where')
  const columns = optionalStringArray(body, 'columns')
  const subscription = optionalString(body, 'subscription')
  const def = jsonBody<ShapeDef>({ table, where, columns })
  return { def, subscription }
}

function subsetInput(body: JsonObject): SubsetDef {
  const table = requiredString(body, 'table')
  const where = optionalObject(body, 'where')
  const columns = optionalStringArray(body, 'columns')
  return jsonBody<SubsetDef>({
    table,
    where,
    columns,
    orderBy: optionalOrderBy(body),
    limit: optionalNonnegativeInt(body, 'limit'),
    offset: optionalNonnegativeInt(body, 'offset'),
  })
}

function aggregateInput(body: JsonObject): { def: AggregateDef; subscription?: string } {
  const table = requiredString(body, 'table')
  const where = optionalObject(body, 'where')
  const fn = requiredString(body, 'fn') as AggregateDef['fn']
  if (!['count', 'sum', 'avg', 'min', 'max'].includes(fn)) {
    throw new RestError(400, 'invalid_argument', 'fn must be one of count, sum, avg, min, max')
  }
  const subscription = optionalString(body, 'subscription')
  return {
    def: jsonBody<AggregateDef>({ table, where, fn, col: body.col }),
    subscription,
  }
}

function route(pathname: string): { resource: string; id?: string } | null {
  const parts = pathname.split('/').filter(Boolean)
  if (parts[0] !== 'compat' || parts[1] !== 'v1') return null
  if (parts.length === 3 && parts[2] === 'shapes') return { resource: 'shapes' }
  if (parts.length === 4 && parts[2] === 'shapes') return { resource: 'shape', id: decodeURIComponent(parts[3]!) }
  if (parts.length === 4 && parts[2] === 'subsets' && parts[3] === 'query') return { resource: 'subset_query' }
  if (parts.length === 3 && parts[2] === 'subset-feeds') return { resource: 'subset_feed' }
  if (parts.length === 3 && parts[2] === 'aggregates') return { resource: 'aggregate' }
  return null
}

export async function handleRestRequest(req: IncomingMessage, res: ServerResponse, core: ElectricCore): Promise<void> {
  try {
    const url = new URL(req.url ?? '/', 'http://localhost')
    const matched = route(url.pathname)
    if (!matched) {
      writeJson(res, 404, { type: 'about:blank', title: 'not_found', status: 404, code: 'not_found' })
      return
    }

    if (matched.resource === 'shapes' && req.method === 'POST') {
      const { def, subscription } = shapeInput(await readJson(req))
      writeJson(res, 200, await core.createShape(def, subscription))
      return
    }

    if (matched.resource === 'shape' && matched.id && req.method === 'GET') {
      const handle = await core.getShape(matched.id)
      if (!handle) throw new RestError(404, 'not_found', `shape ${matched.id} not found`)
      writeJson(res, 200, handle)
      return
    }

    if (matched.resource === 'shape' && matched.id && req.method === 'DELETE') {
      await core.dropShape(matched.id, url.searchParams.get('subscription') ?? undefined)
      writeEmpty(res, 204)
      return
    }

    if (matched.resource === 'subset_query' && req.method === 'POST') {
      writeJson(res, 200, await core.querySubset(subsetInput(await readJson(req))))
      return
    }

    if (matched.resource === 'subset_feed' && req.method === 'POST') {
      const body = await readJson(req)
      const { def, subscription } = shapeInput(body)
      writeJson(res, 200, await core.createSubsetFeed(def, subscription))
      return
    }

    if (matched.resource === 'aggregate' && req.method === 'POST') {
      const { def, subscription } = aggregateInput(await readJson(req))
      writeJson(res, 200, await core.createAggregate(def, subscription))
      return
    }

    writeJson(res, 405, { type: 'about:blank', title: 'method_not_allowed', status: 405, code: 'method_not_allowed' })
  } catch (error) {
    writeError(res, error)
  }
}
