import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { once } from 'node:events'
import { request } from 'node:https'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { mtlsAccess, testPhysicalPath, testStoreIdentity } from './ds-mtls-access.js'

const pki = join(process.cwd(), 'packages/conformance/test-pki')

interface TestResponse {
  status: number
  body: string
}

function httpsRequest(url: string, method: string, withClientCertificate: boolean): Promise<TestResponse> {
  return new Promise((resolve, reject) => {
    const req = request(url, {
      method,
      agent: false,
      ca: readFileSync(join(pki, 'ca.pem')),
      ...(withClientCertificate
        ? { cert: readFileSync(join(pki, 'client.pem')), key: readFileSync(join(pki, 'client.key')) }
        : {}),
    }, (response) => {
      let body = ''
      response.setEncoding('utf8')
      response.on('data', (chunk: string) => { body += chunk })
      response.on('end', () => resolve({ status: response.statusCode ?? 0, body }))
    })
    req.once('error', reject)
    req.end()
  })
}

let raw: ReturnType<typeof createServer> | undefined
let access: Awaited<ReturnType<typeof mtlsAccess>> | undefined

afterEach(async () => {
  await access?.close()
  access = undefined
  await new Promise<void>((resolve, reject) => raw?.close((error) => (error ? reject(error) : resolve())) ?? resolve())
  raw = undefined
})

describe('durable-streams mTLS conformance access façade', () => {
  it('returns deterministic readiness only to an authenticated engine client and proxies mutations', async () => {
    const requests: Array<{ method: string | undefined; url: string | undefined }> = []
    raw = createServer((incoming, outgoing) => {
      requests.push({ method: incoming.method, url: incoming.url })
      incoming.resume()
      outgoing.writeHead(204).end()
    })
    raw.listen(0, '127.0.0.1')
    await once(raw, 'listening')
    const address = raw.address()
    if (!address || typeof address === 'string') throw new Error('raw fixture did not bind')
    access = await mtlsAccess(`http://127.0.0.1:${address.port}`)

    const unauthorizedReady = await httpsRequest(`${access.url}/_admin/ready`, 'GET', false)
    const unauthorizedReadyWithQuery = await httpsRequest(`${access.url}/_admin/ready?probe=1`, 'GET', false)
    const unauthorizedForeignRead = await httpsRequest(`${access.url}/foreign/shape/s1`, 'GET', false)
    const unauthorizedMutation = await httpsRequest(`${access.url}/mutation`, 'POST', false)
    expect(unauthorizedReady.status).toBe(401)
    expect(unauthorizedReadyWithQuery.status).toBe(401)
    expect(unauthorizedForeignRead.status).toBe(401)
    expect(unauthorizedMutation.status).toBe(401)

    const readableWithoutCertificate = await httpsRequest(`${access.url}/${testPhysicalPath('shape/s1')}`, 'HEAD', false)
    const getWithoutCertificate = await httpsRequest(`${access.url}/${testPhysicalPath('shape/s1')}`, 'GET', false)
    expect(readableWithoutCertificate.status).toBe(204)
    expect(getWithoutCertificate.status).toBe(204)

    const ready = await httpsRequest(`${access.url}/_admin/ready`, 'GET', true)
    expect(ready.status).toBe(200)
    expect(JSON.parse(ready.body)).toMatchObject({
      contract_version: 'durable-streams-store-ready-v1',
      status: 'ready',
      manifest: { ...testStoreIdentity, creation_time: '2026-08-27T19:00:00Z' },
    })

    const mutation = await httpsRequest(`${access.url}/mutation`, 'POST', true)
    expect(mutation.status).toBe(204)
    expect(requests).toEqual([
      { method: 'HEAD', url: `/${testPhysicalPath('shape/s1')}` },
      { method: 'GET', url: `/${testPhysicalPath('shape/s1')}` },
      { method: 'POST', url: '/mutation' },
    ])
    await Promise.all([access.close(), access.close()])
  })
})
